use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command};

const MAX_PROJECT_BYTES: u64 = 1024 * 1024;
const MAX_RECOVERY_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RECOVERY_PAYLOAD_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RECOVERY_FILES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDescriptor {
    pub name: String,
    pub asset_manifest: PathBuf,
    pub startup_scene: PathBuf,
    pub engine_version: String,
    pub input_map: PathBuf,
    pub rendering_profile: String,
    pub enabled_plugins: Vec<String>,
    pub build_profiles: Vec<ProjectBuildProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBuildProfile {
    pub name: String,
    pub kind: String,
    pub output_directory: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectHealth {
    pub descriptor_path: PathBuf,
    pub descriptor: Option<ProjectDescriptor>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySnapshotInfo {
    pub directory: PathBuf,
    pub created_unix_milliseconds: i64,
    pub active_scene: PathBuf,
    pub file_count: usize,
    pub dirty_file_count: usize,
    pub text_draft_count: usize,
    pub errors: Vec<String>,
}

impl RecoverySnapshotInfo {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Debug)]
struct RecoveryFileRecord {
    role: String,
    target: PathBuf,
    payload: PathBuf,
    byte_count: u64,
    checksum: u64,
    dirty: bool,
}

impl ProjectHealth {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty() && self.descriptor.is_some()
    }
}

fn tokenize(line: &str, line_number: usize) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = line.char_indices().peekable();
    while let Some((column, character)) = chars.next() {
        if character == '#' && current.is_empty() {
            break;
        }
        if character.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        if character == '"' {
            if !current.is_empty() {
                return Err(format!(
                    "{line_number}:{}: quote must begin a token",
                    column + 1
                ));
            }
            let mut closed = false;
            while let Some((_, quoted)) = chars.next() {
                match quoted {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next().map(|(_, escaped)| escaped) {
                        Some('"') => current.push('"'),
                        Some('\\') => current.push('\\'),
                        Some('n') => current.push('\n'),
                        Some(escaped) => {
                            return Err(format!(
                                "{line_number}:{}: unsupported escape \\{escaped}",
                                column + 1
                            ));
                        }
                        None => {
                            return Err(format!("{line_number}:{}: incomplete escape", column + 1));
                        }
                    },
                    value => current.push(value),
                }
            }
            if !closed {
                return Err(format!(
                    "{line_number}:{}: unterminated quoted token",
                    column + 1
                ));
            }
            if chars
                .peek()
                .is_some_and(|(_, next)| !next.is_whitespace() && *next != '#')
            {
                return Err(format!(
                    "{line_number}:{}: expected whitespace after quote",
                    column + 1
                ));
            }
            tokens.push(std::mem::take(&mut current));
            continue;
        }
        current.push(character);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn validate_relative_path(path: &Path, field: &str) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(format!("{field} must be a non-empty project-relative path"));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!("{field} cannot escape the project root"));
    }
    Ok(())
}

pub fn parse_project(source: &str) -> Result<ProjectDescriptor, String> {
    if source.len() as u64 > MAX_PROJECT_BYTES {
        return Err("project descriptor exceeds the 1 MiB safety limit".into());
    }
    let mut version = None;
    let mut name = None;
    let mut assets = None;
    let mut startup_scene = None;
    let mut engine_version = None;
    let mut input_map = None;
    let mut rendering_profile = None;
    let mut enabled_plugins = Vec::new();
    let mut build_profiles = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let tokens = tokenize(line, line_number)?;
        if tokens.is_empty() {
            continue;
        }
        if version.is_none() {
            if tokens.len() != 2
                || tokens[0] != "kairo-project"
                || !matches!(tokens[1].as_str(), "1" | "2")
            {
                return Err(format!(
                    "{line_number}:1: expected supported 'kairo-project 1|2' header"
                ));
            }
            version = Some(tokens[1].parse::<u32>().expect("validated project version"));
            continue;
        }
        match tokens[0].as_str() {
            "name" if tokens.len() == 2 && name.is_none() => name = Some(tokens[1].clone()),
            "assets" if tokens.len() == 2 && assets.is_none() => {
                assets = Some(PathBuf::from(&tokens[1]))
            }
            "startup-scene" if tokens.len() == 2 && startup_scene.is_none() => {
                startup_scene = Some(PathBuf::from(&tokens[1]))
            }
            "engine-version"
                if tokens.len() == 2 && version == Some(2) && engine_version.is_none() =>
            {
                engine_version = Some(tokens[1].clone())
            }
            "input-map" if tokens.len() == 2 && version == Some(2) && input_map.is_none() => {
                input_map = Some(PathBuf::from(&tokens[1]))
            }
            "rendering-profile"
                if tokens.len() == 2 && version == Some(2) && rendering_profile.is_none() =>
            {
                rendering_profile = Some(tokens[1].clone())
            }
            "plugin" if tokens.len() == 2 && version == Some(2) => {
                enabled_plugins.push(tokens[1].clone())
            }
            "build-profile" if tokens.len() == 4 && version == Some(2) => {
                build_profiles.push(ProjectBuildProfile {
                    name: tokens[1].clone(),
                    kind: tokens[2].clone(),
                    output_directory: PathBuf::from(&tokens[3]),
                })
            }
            "name" | "assets" | "startup-scene" => {
                return Err(format!(
                    "{line_number}:1: duplicate or malformed '{}' statement",
                    tokens[0]
                ));
            }
            "engine-version" | "input-map" | "rendering-profile" | "plugin" | "build-profile" => {
                return Err(format!(
                    "{line_number}:1: malformed or version-incompatible '{}' statement",
                    tokens[0]
                ));
            }
            unknown => return Err(format!("{line_number}:1: unknown statement '{unknown}'")),
        }
    }
    if version.is_none() {
        return Err("1:1: missing kairo-project header".into());
    }
    if version == Some(2)
        && (engine_version.is_none()
            || input_map.is_none()
            || rendering_profile.is_none()
            || build_profiles.is_empty())
    {
        return Err("project format 2 requires engine-version, input-map, rendering-profile, and build-profile statements".into());
    }
    let descriptor = ProjectDescriptor {
        name: name.ok_or("project requires a name statement")?,
        asset_manifest: assets.ok_or("project requires an assets statement")?,
        startup_scene: startup_scene.ok_or("project requires a startup-scene statement")?,
        engine_version: engine_version.unwrap_or_else(|| "0.1.0".into()),
        input_map: input_map.unwrap_or_else(|| PathBuf::from("Config/Input.kinput")),
        rendering_profile: rendering_profile.unwrap_or_else(|| "desktop".into()),
        enabled_plugins,
        build_profiles: if build_profiles.is_empty() {
            vec![
                ProjectBuildProfile {
                    name: "Development".into(),
                    kind: "development".into(),
                    output_directory: PathBuf::from("Build/Development"),
                },
                ProjectBuildProfile {
                    name: "Release".into(),
                    kind: "release".into(),
                    output_directory: PathBuf::from("Build/Release"),
                },
            ]
        } else {
            build_profiles
        },
    };
    if descriptor.name.trim().is_empty() || descriptor.name.contains(['\n', '\r']) {
        return Err("project name must be non-empty and single-line".into());
    }
    validate_relative_path(&descriptor.asset_manifest, "assets")?;
    validate_relative_path(&descriptor.startup_scene, "startup-scene")?;
    validate_relative_path(&descriptor.input_map, "input-map")?;
    if descriptor.asset_manifest == descriptor.startup_scene {
        return Err("asset manifest and startup scene must be different".into());
    }
    if descriptor.engine_version.trim().is_empty() || descriptor.rendering_profile.trim().is_empty()
    {
        return Err("engine version and rendering profile must be non-empty".into());
    }
    let mut profile_names = BTreeSet::new();
    for profile in &descriptor.build_profiles {
        if profile.name.trim().is_empty()
            || !matches!(profile.kind.as_str(), "development" | "release")
        {
            return Err("build profiles require a name and development or release kind".into());
        }
        validate_relative_path(&profile.output_directory, "build profile output")?;
        if !profile_names.insert(&profile.name) {
            return Err("build profile names must be unique".into());
        }
    }
    let mut plugins = BTreeSet::new();
    if descriptor
        .enabled_plugins
        .iter()
        .any(|plugin| plugin.trim().is_empty() || !plugins.insert(plugin))
    {
        return Err("plugin identifiers must be non-empty and unique".into());
    }
    Ok(descriptor)
}

pub fn inspect_project(path: &Path) -> ProjectHealth {
    let mut health = ProjectHealth {
        descriptor_path: path.to_path_buf(),
        descriptor: None,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    if path.extension().and_then(|extension| extension.to_str()) != Some("kproject") {
        health
            .errors
            .push("Project descriptor must use the .kproject extension".into());
        return health;
    }
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            health
                .errors
                .push(format!("Cannot read project descriptor metadata: {error}"));
            return health;
        }
    };
    if metadata.len() > MAX_PROJECT_BYTES {
        health
            .errors
            .push("Project descriptor exceeds the 1 MiB safety limit".into());
        return health;
    }
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            health
                .errors
                .push(format!("Cannot read project descriptor: {error}"));
            return health;
        }
    };
    let descriptor = match parse_project(&source) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            health.errors.push(error);
            return health;
        }
    };
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    if !root.join(&descriptor.asset_manifest).is_file() {
        health.errors.push(format!(
            "Missing asset manifest: {}",
            descriptor.asset_manifest.display()
        ));
    }
    if !root.join(&descriptor.startup_scene).is_file() {
        health.errors.push(format!(
            "Missing startup scene: {}",
            descriptor.startup_scene.display()
        ));
    }
    if !root.join(".git").exists() {
        health
            .warnings
            .push("Project is not under local Git version control".into());
    }
    health.descriptor = Some(descriptor);
    health
}

fn recovery_checksum(path: &Path) -> Result<(u64, u64), String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("Cannot open recovery payload {}: {error}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hash = 14_695_981_039_346_656_037u64;
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("Cannot read recovery payload {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or_else(|| "Recovery payload byte count overflowed".to_string())?;
        if bytes > MAX_RECOVERY_PAYLOAD_BYTES {
            return Err("Recovery payload exceeds its 512 MiB safety limit".into());
        }
        for byte in &buffer[..count] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(1_099_511_628_211u64);
        }
    }
    Ok((bytes, hash))
}

fn inspect_recovery_snapshot(project: &Path, directory: &Path) -> RecoverySnapshotInfo {
    let mut result = RecoverySnapshotInfo {
        directory: directory.to_path_buf(),
        created_unix_milliseconds: 0,
        active_scene: PathBuf::new(),
        file_count: 0,
        dirty_file_count: 0,
        text_draft_count: 0,
        errors: Vec::new(),
    };
    let project_root = match project
        .parent()
        .and_then(|path| fs::canonicalize(path).ok())
    {
        Some(path) => path,
        None => {
            result.errors.push("Cannot resolve project root".into());
            return result;
        }
    };
    let directory = match fs::canonicalize(directory) {
        Ok(path) => path,
        Err(error) => {
            result
                .errors
                .push(format!("Cannot resolve snapshot directory: {error}"));
            return result;
        }
    };
    result.directory = directory.clone();
    if directory.parent() != Some(project_root.join(".kairo/recovery").as_path()) {
        result
            .errors
            .push("Snapshot is outside this project's recovery directory".into());
        return result;
    }
    let manifest = directory.join("manifest.krecover");
    let metadata = match fs::metadata(&manifest) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            result
                .errors
                .push("Recovery manifest is not a regular file".into());
            return result;
        }
        Err(error) => {
            result
                .errors
                .push(format!("Cannot inspect recovery manifest: {error}"));
            return result;
        }
    };
    if metadata.len() > MAX_RECOVERY_MANIFEST_BYTES {
        result
            .errors
            .push("Recovery manifest exceeds its 4 MiB safety limit".into());
        return result;
    }
    let source = match fs::read_to_string(&manifest) {
        Ok(source) => source,
        Err(error) => {
            result
                .errors
                .push(format!("Cannot read recovery manifest: {error}"));
            return result;
        }
    };
    let mut header = false;
    let mut created = None;
    let mut declared_root = None;
    let mut project_file = None;
    let mut active_scene = None;
    let mut files = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let tokens = match tokenize(line, line_number) {
            Ok(tokens) => tokens,
            Err(error) => {
                result.errors.push(error);
                continue;
            }
        };
        if tokens.is_empty() {
            continue;
        }
        if !header {
            if tokens.as_slice() != ["kairo-recovery", "1"] {
                result.errors.push(format!(
                    "{line_number}:1: expected supported 'kairo-recovery 1' header"
                ));
            }
            header = true;
            continue;
        }
        match tokens[0].as_str() {
            "created-unix-ms" if tokens.len() == 2 && created.is_none() => {
                created = tokens[1].parse::<i64>().ok().filter(|value| *value >= 0);
                if created.is_none() {
                    result
                        .errors
                        .push(format!("{line_number}: invalid creation timestamp"));
                }
            }
            "project-root" if tokens.len() == 2 && declared_root.is_none() => {
                declared_root = Some(PathBuf::from(&tokens[1]));
            }
            "project-file" if tokens.len() == 2 && project_file.is_none() => {
                let path = PathBuf::from(&tokens[1]);
                if let Err(error) = validate_relative_path(&path, "recovery project file") {
                    result.errors.push(format!("{line_number}: {error}"));
                }
                project_file = Some(path);
            }
            "active-scene" if tokens.len() == 2 && active_scene.is_none() => {
                let path = PathBuf::from(&tokens[1]);
                if let Err(error) = validate_relative_path(&path, "recovery active scene") {
                    result.errors.push(format!("{line_number}: {error}"));
                }
                active_scene = Some(path);
            }
            "file" if tokens.len() == 8 && files.len() < MAX_RECOVERY_FILES => {
                let role = tokens[1].clone();
                if !matches!(
                    role.as_str(),
                    "project" | "assets" | "scene" | "document" | "text-draft"
                ) {
                    result
                        .errors
                        .push(format!("{line_number}: unknown recovery file role"));
                    continue;
                }
                let target = PathBuf::from(&tokens[2]);
                let payload = PathBuf::from(&tokens[3]);
                if let Err(error) = validate_relative_path(&target, "recovery target") {
                    result.errors.push(format!("{line_number}: {error}"));
                    continue;
                }
                if let Err(error) = validate_relative_path(&payload, "recovery payload") {
                    result.errors.push(format!("{line_number}: {error}"));
                    continue;
                }
                if payload.components().next() != Some(Component::Normal("payload".as_ref())) {
                    result.errors.push(format!(
                        "{line_number}: payload must remain inside payload/"
                    ));
                    continue;
                }
                let Some(byte_count) = tokens[4].parse::<u64>().ok() else {
                    result
                        .errors
                        .push(format!("{line_number}: invalid payload byte count"));
                    continue;
                };
                let Some(checksum) = tokens[5].parse::<u64>().ok() else {
                    result
                        .errors
                        .push(format!("{line_number}: invalid payload checksum"));
                    continue;
                };
                let dirty = match tokens[6].as_str() {
                    "true" => true,
                    "false" => false,
                    _ => {
                        result
                            .errors
                            .push(format!("{line_number}: invalid dirty flag"));
                        continue;
                    }
                };
                if !matches!(tokens[7].as_str(), "true" | "false") {
                    result
                        .errors
                        .push(format!("{line_number}: invalid active flag"));
                    continue;
                }
                files.push(RecoveryFileRecord {
                    role,
                    target,
                    payload,
                    byte_count,
                    checksum,
                    dirty,
                });
            }
            "file" if files.len() >= MAX_RECOVERY_FILES => {
                result
                    .errors
                    .push("Recovery snapshot exceeds its 512-file safety limit".into());
            }
            _ => result.errors.push(format!(
                "{line_number}: malformed or duplicate recovery statement"
            )),
        }
    }
    if !header
        || created.is_none()
        || declared_root.is_none()
        || project_file.is_none()
        || active_scene.is_none()
    {
        result.errors.push("Recovery manifest is incomplete".into());
    }
    if declared_root.as_deref() != Some(project_root.as_path()) {
        result
            .errors
            .push("Recovery manifest declares another project root".into());
    }
    let expected_project = project.file_name().map(PathBuf::from);
    if project_file != expected_project {
        result
            .errors
            .push("Recovery manifest declares another project descriptor".into());
    }
    for role in ["project", "assets", "scene"] {
        if files.iter().filter(|file| file.role == role).count() != 1 {
            result
                .errors
                .push(format!("Recovery requires exactly one {role} payload"));
        }
    }
    let mut payloads = BTreeSet::new();
    for file in &files {
        if !payloads.insert(file.payload.clone()) {
            result.errors.push(format!(
                "Duplicate recovery payload: {}",
                file.payload.display()
            ));
            continue;
        }
        match recovery_checksum(&directory.join(&file.payload)) {
            Ok((bytes, checksum)) if bytes == file.byte_count && checksum == file.checksum => {}
            Ok(_) => result.errors.push(format!(
                "Recovery payload failed size/checksum validation: {}",
                file.target.display()
            )),
            Err(error) => result.errors.push(error),
        }
    }
    result.created_unix_milliseconds = created.unwrap_or_default();
    result.active_scene = active_scene.unwrap_or_default();
    result.file_count = files.len();
    result.dirty_file_count = files.iter().filter(|file| file.dirty).count();
    result.text_draft_count = files
        .iter()
        .filter(|file| file.role == "text-draft")
        .count();
    result
}

/// Returns newest-first snapshots. Invalid published directories remain in the
/// result with diagnostics so Hub can report damage instead of hiding it.
pub fn recovery_snapshots(project: &Path) -> Result<Vec<RecoverySnapshotInfo>, String> {
    let health = inspect_project(project);
    if health.descriptor.is_none() {
        return Err(format!(
            "Cannot inspect recovery for invalid project: {}",
            health.errors.join("; ")
        ));
    }
    let root = project.parent().unwrap_or_else(|| Path::new("."));
    let recovery = root.join(".kairo/recovery");
    if !recovery.exists() {
        return Ok(Vec::new());
    }
    let mut snapshots = Vec::new();
    for entry in fs::read_dir(&recovery)
        .map_err(|error| format!("Cannot read recovery directory: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Cannot read recovery entry: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
            && entry.file_name().to_string_lossy().starts_with("snapshot-")
        {
            snapshots.push(inspect_recovery_snapshot(project, &entry.path()));
        }
    }
    snapshots.sort_by(|left, right| {
        right
            .created_unix_milliseconds
            .cmp(&left.created_unix_milliseconds)
            .then_with(|| right.directory.cmp(&left.directory))
    });
    Ok(snapshots)
}

/// Task: recreate only missing generated bootstrap files referenced by a valid
/// descriptor. Existing user files are never overwritten or normalized.
pub fn repair_project(path: &Path) -> Result<ProjectHealth, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("Cannot read project descriptor: {error}"))?;
    let descriptor = parse_project(&source)?;
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let manifest = root.join(&descriptor.asset_manifest);
    let scene = root.join(&descriptor.startup_scene);
    if !manifest.exists() {
        if let Some(parent) = manifest.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        write_atomic(&manifest, "kairo-assets 1\n")?;
    }
    if !scene.exists() {
        if let Some(parent) = scene.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        write_atomic(&scene, "kairo-scene 1\n")?;
    }
    Ok(inspect_project(path))
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn write_atomic(path: &Path, source: &str) -> Result<(), String> {
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("file")
    ));
    let mut output = fs::File::create(&temporary).map_err(|error| error.to_string())?;
    output
        .write_all(source.as_bytes())
        .map_err(|error| error.to_string())?;
    output.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

pub fn create_project(
    parent: &Path,
    folder_name: &str,
    display_name: &str,
) -> Result<PathBuf, String> {
    if folder_name.is_empty()
        || !folder_name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("Project folder must contain only letters, digits, '-' or '_'".into());
    }
    if display_name.trim().is_empty() || display_name.contains(['\n', '\r']) {
        return Err("Project display name must be non-empty and single-line".into());
    }
    let root = parent.join(folder_name);
    if root.exists() {
        return Err(format!(
            "Project directory already exists: {}",
            root.display()
        ));
    }
    fs::create_dir_all(root.join("Scenes")).map_err(|error| error.to_string())?;
    fs::create_dir_all(root.join(".kairo")).map_err(|error| error.to_string())?;
    let result = (|| {
        write_atomic(
            &root.join("Assets.kassets"),
            "kairo-assets 1\nasset 00000000-0000-4000-8000-000000000202 material builtin 1 \"builtin/default-material\" \"kairo.builtin\"\nend\n",
        )?;
        write_atomic(&root.join("Scenes/Main.kscene"), "kairo-scene 1\n")?;
        let descriptor = format!(
            "kairo-project 2\nname {}\nengine-version \"0.1.0\"\nassets \"Assets.kassets\"\nstartup-scene \"Scenes/Main.kscene\"\ninput-map \"Config/Input.kinput\"\nrendering-profile \"desktop\"\nbuild-profile \"Development\" development \"Build/Development\"\nbuild-profile \"Release\" release \"Build/Release\"\n",
            quote(display_name.trim())
        );
        let project = root.join(format!("{folder_name}.kproject"));
        write_atomic(&project, &descriptor)?;
        Ok(project)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&root);
    }
    result
}

pub fn discover_editor() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(explicit) = std::env::var_os("KAIRO_EDITOR_EXECUTABLE") {
        candidates.push(PathBuf::from(explicit));
    }
    if let Some(root) = std::env::var_os("KAIRO_ENGINE_ROOT") {
        candidates.push(PathBuf::from(root).join("KairoEditor/build/KairoEditorApp"));
    }
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.join("KairoEditor/build/KairoEditorApp"));
        candidates.push(current.join("../KairoEditor/build/KairoEditorApp"));
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            "KairoEditor executable was not found. Set KAIRO_EDITOR_EXECUTABLE or KAIRO_ENGINE_ROOT."
                .into()
        })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInstallation {
    pub root: PathBuf,
    pub version: String,
    pub editor: PathBuf,
    pub editor_available: bool,
    pub player: PathBuf,
    pub player_available: bool,
}

fn engine_executable(root: &Path, candidates: &[&str]) -> PathBuf {
    let extension = if cfg!(windows) { ".exe" } else { "" };
    candidates
        .iter()
        .map(|candidate| root.join(format!("{candidate}{extension}")))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| root.join(format!("{}{}", candidates[0], extension)))
}

pub fn inspect_engine(root: &Path) -> Result<EngineInstallation, String> {
    let manifest = root.join("CMakeLists.txt");
    let source = fs::read_to_string(&manifest).map_err(|error| {
        format!(
            "Cannot read engine manifest {}: {error}",
            manifest.display()
        )
    })?;
    if !source.contains("project(KairoGameEngine") {
        return Err("Selected directory is not a KairoGameEngine umbrella checkout".into());
    }
    let version = source
        .lines()
        .find(|line| line.contains("project(KairoGameEngine") && line.contains("VERSION"))
        .and_then(|line| line.split("VERSION").nth(1))
        .and_then(|tail| tail.split_whitespace().next())
        .map(|value| value.trim_end_matches(')').to_string())
        .unwrap_or_else(|| "development".into());
    let editor = engine_executable(
        root,
        &[
            "build/dev-clang/KairoEditor/KairoEditorApp",
            "build/dev/KairoEditor/KairoEditorApp",
            "build/release/KairoEditor/KairoEditorApp",
            "KairoEditor/build/KairoEditorApp",
        ],
    );
    let player = engine_executable(
        root,
        &[
            "build/dev-clang/Runtime/KairoPlayer/KairoPlayer",
            "build/dev/Runtime/KairoPlayer/KairoPlayer",
            "build/release/Runtime/KairoPlayer/KairoPlayer",
        ],
    );
    Ok(EngineInstallation {
        root: root.to_path_buf(),
        version,
        editor_available: editor.is_file(),
        editor,
        player_available: player.is_file(),
        player,
    })
}

fn validate_clone_folder(folder_name: &str) -> Result<(), String> {
    if folder_name.is_empty()
        || !folder_name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        || matches!(folder_name, "." | "..")
    {
        return Err("Clone folder must contain only letters, digits, '-', '_' or '.'".into());
    }
    Ok(())
}

fn find_project_descriptors(
    root: &Path,
    depth: usize,
    output: &mut Vec<PathBuf>,
) -> Result<(), String> {
    if depth > 4 {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(".git") {
            continue;
        }
        if path.is_dir() {
            find_project_descriptors(&path, depth + 1, output)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("kproject") {
            output.push(path);
        }
    }
    Ok(())
}

pub fn clone_project(
    repository: &str,
    parent: &Path,
    folder_name: &str,
) -> Result<PathBuf, String> {
    if !(repository.starts_with("https://github.com/")
        || repository.starts_with("https://gitlab.com/"))
        || !repository.ends_with(".git")
    {
        return Err("Repository must be an HTTPS GitHub or GitLab .git URL".into());
    }
    validate_clone_folder(folder_name)?;
    let destination = parent.join(folder_name);
    if destination.exists() {
        return Err(format!(
            "Clone destination already exists: {}",
            destination.display()
        ));
    }
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let status = Command::new("git")
        .args(["clone", "--depth", "1", "--"])
        .arg(repository)
        .arg(&destination)
        .status()
        .map_err(|error| format!("Cannot start git clone: {error}"))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&destination);
        return Err(format!("git clone failed with status {status}"));
    }
    let mut descriptors = Vec::new();
    find_project_descriptors(&destination, 0, &mut descriptors)?;
    match descriptors.len() {
        1 => Ok(descriptors.remove(0)),
        0 => Err("Cloned repository contains no .kproject descriptor".into()),
        count => Err(format!(
            "Cloned repository contains {count} .kproject descriptors; import one explicitly"
        )),
    }
}

pub fn launch_editor_with(
    project: &Path,
    recovery_snapshot: Option<&Path>,
    editor_override: Option<&Path>,
) -> Result<Child, String> {
    let health = inspect_project(project);
    if !health.is_valid() {
        return Err(format!(
            "Project validation failed: {}",
            health.errors.join("; ")
        ));
    }
    let editor = match editor_override {
        Some(editor) if editor.is_file() => editor.to_path_buf(),
        Some(editor) => {
            return Err(format!(
                "Selected KairoEditor build is missing: {}",
                editor.display()
            ));
        }
        None => discover_editor()?,
    };
    let mut command = Command::new(editor);
    command.arg("--project").arg(project);
    if let Some(snapshot) = recovery_snapshot {
        let selected = inspect_recovery_snapshot(project, snapshot);
        if !selected.is_valid() {
            return Err(format!(
                "Recovery snapshot validation failed: {}",
                selected.errors.join("; ")
            ));
        }
        command.arg("--recovery-snapshot").arg(&selected.directory);
    }
    command
        .spawn()
        .map_err(|error| format!("Cannot launch KairoEditor: {error}"))
}

/// Launches the selected engine's standalone player after both KairoHub's
/// structural inspection and KairoPlayer's own runtime parsing boundary.
/// Command arguments are passed directly, never through a host shell.
pub fn launch_player_with(
    project: &Path,
    installation: &EngineInstallation,
) -> Result<Child, String> {
    let health = inspect_project(project);
    if !health.is_valid() {
        return Err(format!(
            "Project validation failed: {}",
            health.errors.join("; ")
        ));
    }
    validate_project_engine_version(&health, installation)?;
    if !installation.player_available || !installation.player.is_file() {
        return Err(format!(
            "Selected KairoPlayer build is missing: {}",
            installation.player.display()
        ));
    }
    Command::new(&installation.player)
        .arg(project)
        .spawn()
        .map_err(|error| format!("Cannot launch KairoPlayer: {error}"))
}

/// Ensures an authored project is not opened or run with a different engine
/// contract than the one selected in KairoHub.
pub fn validate_project_engine_version(
    health: &ProjectHealth,
    installation: &EngineInstallation,
) -> Result<(), String> {
    let descriptor = health
        .descriptor
        .as_ref()
        .ok_or_else(|| "Project descriptor is unavailable after validation".to_string())?;
    if descriptor.engine_version != installation.version {
        return Err(format!(
            "Project requires Kairo {}, but selected installation is Kairo {}",
            descriptor.engine_version, installation.version
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct HubState {
    pub recent_projects: Vec<PathBuf>,
    pub favorites: BTreeSet<PathBuf>,
    pub engine_roots: Vec<PathBuf>,
    pub selected_engine: Option<PathBuf>,
}

impl HubState {
    pub fn remember(&mut self, project: PathBuf) {
        self.recent_projects.retain(|existing| existing != &project);
        self.recent_projects.insert(0, project);
        self.recent_projects.truncate(24);
    }

    pub fn set_favorite(&mut self, project: PathBuf, favorite: bool) {
        if favorite {
            self.favorites.insert(project);
        } else {
            self.favorites.remove(&project);
        }
    }

    pub fn register_engine(&mut self, root: PathBuf) {
        self.engine_roots.retain(|existing| existing != &root);
        self.engine_roots.insert(0, root.clone());
        self.selected_engine = Some(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish_test_snapshot(project: &Path, timestamp: i64) -> PathBuf {
        let root = fs::canonicalize(project.parent().unwrap()).unwrap();
        let project_name = project.file_name().unwrap().to_string_lossy().into_owned();
        let directory = root
            .join(".kairo/recovery")
            .join(format!("snapshot-{timestamp}-test"));
        let payload = directory.join("payload");
        fs::create_dir_all(payload.join("Scenes")).unwrap();
        let records = [
            (
                "project",
                PathBuf::from(&project_name),
                fs::read_to_string(project).unwrap(),
            ),
            (
                "assets",
                PathBuf::from("Assets.kassets"),
                fs::read_to_string(root.join("Assets.kassets")).unwrap(),
            ),
            (
                "scene",
                PathBuf::from("Scenes/Main.kscene"),
                fs::read_to_string(root.join("Scenes/Main.kscene")).unwrap(),
            ),
        ];
        let mut manifest = format!(
            "kairo-recovery 1\ncreated-unix-ms {timestamp}\nproject-root {}\nproject-file {}\nactive-scene \"Scenes/Main.kscene\"\n",
            quote(&root.to_string_lossy()),
            quote(&project_name)
        );
        for (role, target, source) in records {
            let payload_path = PathBuf::from("payload").join(&target);
            let path = directory.join(&payload_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, source.as_bytes()).unwrap();
            let (bytes, checksum) = recovery_checksum(&path).unwrap();
            manifest.push_str(&format!(
                "file {role} {} {} {bytes} {checksum} true false\n",
                quote(&target.to_string_lossy()),
                quote(&payload_path.to_string_lossy())
            ));
        }
        fs::write(directory.join("manifest.krecover"), manifest).unwrap();
        directory
    }

    #[test]
    fn parser_rejects_unknown_and_escaping_paths() {
        assert!(
            parse_project(
                "kairo-project 1\nname \"Demo\"\nassets \"../bad\"\nstartup-scene \"Main.kscene\"\n"
            )
            .is_err()
        );
        assert!(parse_project("kairo-project 1\nname \"Demo\"\nunknown \"x\"\n").is_err());
    }

    #[test]
    fn project_creation_is_immediately_healthy() {
        let temporary = tempfile::tempdir().unwrap();
        let project = create_project(temporary.path(), "TestGame", "Test Game").unwrap();
        let health = inspect_project(&project);
        assert!(health.is_valid(), "{:?}", health.errors);
        assert_eq!(health.descriptor.unwrap().name, "Test Game");
    }

    #[test]
    fn project_v2_round_trips_runtime_and_build_metadata() {
        let descriptor = parse_project(
            "kairo-project 2\nname \"V2\"\nengine-version \"0.1.0\"\nassets \"Assets.kassets\"\nstartup-scene \"Scenes/Main.kscene\"\ninput-map \"Config/Input.kinput\"\nrendering-profile \"desktop\"\nplugin \"kairo.physics\"\nbuild-profile \"Shipping\" release \"Artifacts/Shipping\"\n",
        )
        .unwrap();
        assert_eq!(descriptor.enabled_plugins, vec!["kairo.physics"]);
        assert_eq!(descriptor.build_profiles[0].kind, "release");
    }

    #[test]
    fn repair_recreates_only_missing_bootstrap_files() {
        let temporary = tempfile::tempdir().unwrap();
        let descriptor = temporary.path().join("Broken.kproject");
        fs::write(
            &descriptor,
            "kairo-project 1\nname \"Broken\"\nassets \"Data/Assets.kassets\"\nstartup-scene \"World/Main.kscene\"\n",
        )
        .unwrap();
        assert!(!inspect_project(&descriptor).is_valid());
        let repaired = repair_project(&descriptor).unwrap();
        assert!(repaired.is_valid(), "{:?}", repaired.errors);
        fs::write(temporary.path().join("World/Main.kscene"), "user content\n").unwrap();
        repair_project(&descriptor).unwrap();
        assert_eq!(
            fs::read_to_string(temporary.path().join("World/Main.kscene")).unwrap(),
            "user content\n"
        );
    }

    #[test]
    fn clone_inputs_reject_option_and_untrusted_transport_injection() {
        assert!(clone_project("--upload-pack=bad", Path::new("/tmp"), "Safe").is_err());
        assert!(clone_project("git@github.com:owner/game.git", Path::new("/tmp"), "Safe").is_err());
        assert!(
            clone_project(
                "https://github.com/owner/game.git",
                Path::new("/tmp"),
                "../bad"
            )
            .is_err()
        );
    }

    #[test]
    fn recent_projects_are_unique_and_bounded() {
        let mut state = HubState::default();
        for index in 0..30 {
            state.remember(PathBuf::from(format!("Project{index}.kproject")));
        }
        state.remember(PathBuf::from("Project12.kproject"));
        assert_eq!(state.recent_projects.len(), 24);
        assert_eq!(
            state.recent_projects[0],
            PathBuf::from("Project12.kproject")
        );
    }

    #[test]
    fn engine_registration_is_unique_and_selected() {
        let mut state = HubState::default();
        state.register_engine(PathBuf::from("/engines/kairo-a"));
        state.register_engine(PathBuf::from("/engines/kairo-b"));
        state.register_engine(PathBuf::from("/engines/kairo-a"));
        assert_eq!(state.engine_roots.len(), 2);
        assert_eq!(
            state.selected_engine,
            Some(PathBuf::from("/engines/kairo-a"))
        );
    }

    #[test]
    fn engine_inspection_reports_version_and_editor_health() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("CMakeLists.txt"),
            "project(KairoGameEngine VERSION 4.2.1 LANGUAGES CXX)\n",
        )
        .unwrap();
        let installation = inspect_engine(temporary.path()).unwrap();
        assert_eq!(installation.version, "4.2.1");
        assert!(!installation.editor_available);
        assert!(!installation.player_available);
    }

    #[test]
    fn engine_inspection_discovers_umbrella_preset_binaries() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("CMakeLists.txt"),
            "project(KairoGameEngine VERSION 0.1.0 LANGUAGES CXX)\n",
        )
        .unwrap();
        let editor = temporary
            .path()
            .join("build/dev-clang/KairoEditor/KairoEditorApp");
        let player = temporary
            .path()
            .join("build/dev-clang/Runtime/KairoPlayer/KairoPlayer");
        fs::create_dir_all(editor.parent().unwrap()).unwrap();
        fs::create_dir_all(player.parent().unwrap()).unwrap();
        fs::write(&editor, b"fixture").unwrap();
        fs::write(&player, b"fixture").unwrap();
        let installation = inspect_engine(temporary.path()).unwrap();
        assert!(installation.editor_available);
        assert!(installation.player_available);
        assert_eq!(installation.editor, editor);
        assert_eq!(installation.player, player);
    }

    #[test]
    fn project_launch_rejects_a_different_engine_version() {
        let temporary = tempfile::tempdir().unwrap();
        let project = create_project(temporary.path(), "Versioned", "Versioned").unwrap();
        let health = inspect_project(&project);
        let installation = EngineInstallation {
            root: temporary.path().to_path_buf(),
            version: "9.0.0".into(),
            editor: temporary.path().join("Editor"),
            editor_available: false,
            player: temporary.path().join("Player"),
            player_available: false,
        };
        let error = validate_project_engine_version(&health, &installation).unwrap_err();
        assert!(error.contains("requires Kairo 0.1.0"));
        assert!(error.contains("selected installation is Kairo 9.0.0"));
    }

    #[test]
    fn recovery_discovery_validates_payloads_and_reports_corruption() {
        let temporary = tempfile::tempdir().unwrap();
        let project = create_project(temporary.path(), "Recovery", "Recovery").unwrap();
        let older = publish_test_snapshot(&project, 1000);
        let newer = publish_test_snapshot(&project, 2000);
        let snapshots = recovery_snapshots(&project).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].directory, fs::canonicalize(&newer).unwrap());
        assert!(snapshots[0].is_valid(), "{:?}", snapshots[0].errors);
        assert_eq!(snapshots[0].file_count, 3);
        assert_eq!(snapshots[0].dirty_file_count, 3);

        fs::write(older.join("payload/Scenes/Main.kscene"), "corrupt\n").unwrap();
        let snapshots = recovery_snapshots(&project).unwrap();
        let damaged = snapshots
            .iter()
            .find(|snapshot| snapshot.directory == fs::canonicalize(&older).unwrap())
            .unwrap();
        assert!(!damaged.is_valid());
        assert!(
            damaged
                .errors
                .iter()
                .any(|error| error.contains("size/checksum"))
        );
    }
}
