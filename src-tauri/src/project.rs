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
const STARTER_INPUT_MAP: &str = "kairo-input 1\n\
action \"Move\" axis2d\n\
action \"Look\" axis2d\n\
action \"Jump\" button\n\
action \"Quit\" button\n\
bind \"Move\" key W 0 1 0\n\
bind \"Move\" key S 0 -1 0\n\
bind \"Move\" key A -1 0 0\n\
bind \"Move\" key D 1 0 0\n\
bind \"Move\" gamepad-axis LeftX 1 0 0.15\n\
bind \"Move\" gamepad-axis LeftY 0 -1 0.15\n\
bind \"Look\" gamepad-axis RightX 1 0 0.15\n\
bind \"Look\" gamepad-axis RightY 0 -1 0.15\n\
bind \"Jump\" key Space 1 0 0\n\
bind \"Jump\" gamepad-button A 1 0 0\n\
bind \"Quit\" key Escape 1 0 0\n";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDescriptor {
    pub name: String,
    pub asset_manifest: PathBuf,
    pub startup_scene: PathBuf,
    pub engine_version: String,
    pub input_map: PathBuf,
    pub rendering_profile: String,
    pub graphics_backend: String,
    pub runtime_executable: Option<PathBuf>,
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

/// A verified package result returned to the Hub UI. Paths refer to the
/// profile output actually published by KairoPlayer, not merely the authored
/// destination that was requested.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageArtifact {
    pub profile_name: String,
    pub profile_kind: String,
    pub output_directory: PathBuf,
    pub manifest_path: PathBuf,
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
    let mut graphics_backend = None;
    let mut runtime_executable = None;
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
            "graphics-backend"
                if tokens.len() == 2 && version == Some(2) && graphics_backend.is_none() =>
            {
                graphics_backend = Some(tokens[1].clone())
            }
            "runtime-executable"
                if tokens.len() == 2 && version == Some(2) && runtime_executable.is_none() =>
            {
                runtime_executable = Some(PathBuf::from(&tokens[1]))
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
            "engine-version" | "input-map" | "rendering-profile" | "graphics-backend"
            | "runtime-executable" | "plugin" | "build-profile" => {
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
        graphics_backend: graphics_backend.unwrap_or_else(|| "auto".into()),
        runtime_executable,
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
    if let Some(runtime) = descriptor.runtime_executable.as_deref() {
        validate_relative_path(runtime, "runtime-executable")?;
    }
    if descriptor.asset_manifest == descriptor.startup_scene {
        return Err("asset manifest and startup scene must be different".into());
    }
    if descriptor.engine_version.trim().is_empty() || descriptor.rendering_profile.trim().is_empty()
    {
        return Err("engine version and rendering profile must be non-empty".into());
    }
    if !matches!(
        descriptor.graphics_backend.as_str(),
        "auto" | "vulkan" | "metal" | "d3d12" | "opengl"
    ) {
        return Err("graphics backend must be auto, vulkan, metal, d3d12, or opengl".into());
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
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            health
                .errors
                .push(format!("Cannot read project descriptor metadata: {error}"));
            return health;
        }
    };
    if metadata.file_type().is_symlink() {
        health
            .errors
            .push("Project descriptor cannot be a symbolic link".into());
        return health;
    }
    if !metadata.is_file() {
        health
            .errors
            .push("Project descriptor must be a regular file".into());
        return health;
    }
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
    inspect_required_project_file(
        root,
        &descriptor.asset_manifest,
        "asset manifest",
        &mut health.errors,
    );
    inspect_required_project_file(
        root,
        &descriptor.startup_scene,
        "startup scene",
        &mut health.errors,
    );
    inspect_required_project_file(root, &descriptor.input_map, "input map", &mut health.errors);
    if !root.join(".git").exists() {
        health
            .warnings
            .push("Project is not under local Git version control".into());
    }
    health.descriptor = Some(descriptor);
    health
}

/// Imports an existing Kairo project descriptor after validating the complete
/// bootstrap contract. This function performs no mutation; HubState decides
/// whether to remember the project only after validation succeeds.
pub fn import_project(path: &Path) -> Result<ProjectHealth, String> {
    let health = inspect_project(path);
    if !health.is_valid() {
        return Err(format!(
            "Project import failed: {}",
            health.errors.join("; ")
        ));
    }
    Ok(health)
}

/// Required project files must be regular files whose resolved location stays
/// under the project root. This matches KairoPlayer's runtime boundary and
/// prevents an apparently healthy project from reaching outside itself through
/// a symlinked scene, manifest, or input map.
fn inspect_required_project_file(
    root: &Path,
    relative: &Path,
    role: &str,
    errors: &mut Vec<String>,
) {
    let candidate = root.join(relative);
    if !candidate.is_file() {
        errors.push(format!("Missing {role}: {}", relative.display()));
        return;
    }
    let Ok(canonical_root) = fs::canonicalize(root) else {
        errors.push(format!("Cannot resolve project root while checking {role}"));
        return;
    };
    match fs::canonicalize(&candidate) {
        Ok(resolved) if resolved.starts_with(&canonical_root) => {}
        Ok(_) => errors.push(format!(
            "{role} resolves outside the project: {}",
            relative.display()
        )),
        Err(error) => errors.push(format!(
            "Cannot resolve {role} {}: {error}",
            relative.display()
        )),
    }
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
    let input_map = root.join(&descriptor.input_map);
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
    if !input_map.exists() {
        if let Some(parent) = input_map.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        write_atomic(&input_map, STARTER_INPUT_MAP)?;
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
    fs::create_dir_all(root.join("Config")).map_err(|error| error.to_string())?;
    fs::create_dir_all(root.join(".kairo")).map_err(|error| error.to_string())?;
    let result = (|| {
        write_atomic(
            &root.join("Assets.kassets"),
            "kairo-assets 1\nasset 00000000-0000-4000-8000-000000000202 material builtin 1 \"builtin/default-material\" \"kairo.builtin\"\nend\n",
        )?;
        write_atomic(&root.join("Scenes/Main.kscene"), "kairo-scene 1\n")?;
        write_atomic(&root.join("Config/Input.kinput"), STARTER_INPUT_MAP)?;
        let descriptor = format!(
            "kairo-project 2\nname {}\nengine-version \"0.1.0\"\nassets \"Assets.kassets\"\nstartup-scene \"Scenes/Main.kscene\"\ninput-map \"Config/Input.kinput\"\nrendering-profile \"desktop\"\ngraphics-backend \"auto\"\nbuild-profile \"Development\" development \"Build/Development\"\nbuild-profile \"Release\" release \"Build/Release\"\n",
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
    pub project_compiler: PathBuf,
    pub project_compiler_available: bool,
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
            "build/dev-clang/components/KairoEditor/KairoEditorApp",
            "build/dev/components/KairoEditor/KairoEditorApp",
            "build/release/components/KairoEditor/KairoEditorApp",
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
    let project_compiler = engine_executable(
        root,
        &[
            "build/dev-clang/components/KairoEditor/KairoProjectCompiler",
            "build/dev/components/KairoEditor/KairoProjectCompiler",
            "build/release/components/KairoEditor/KairoProjectCompiler",
            "build/dev-clang/KairoEditor/KairoProjectCompiler",
            "build/dev/KairoEditor/KairoProjectCompiler",
            "build/release/KairoEditor/KairoProjectCompiler",
            "KairoEditor/build/KairoProjectCompiler",
        ],
    );
    Ok(EngineInstallation {
        root: root.to_path_buf(),
        version,
        editor_available: editor.is_file(),
        editor,
        project_compiler_available: project_compiler.is_file(),
        project_compiler,
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
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            find_project_descriptors(&path, depth + 1, output)?;
        } else if metadata.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("kproject")
        {
            output.push(path);
        }
    }
    Ok(())
}

fn validate_clone_repository(repository: &str) -> Result<(), String> {
    if repository.is_empty()
        || repository.len() > 2048
        || repository
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
        || repository.contains('?')
        || repository.contains('#')
    {
        return Err("Repository URL is malformed".into());
    }
    let path = repository
        .strip_prefix("https://github.com/")
        .or_else(|| repository.strip_prefix("https://gitlab.com/"))
        .ok_or_else(|| "Repository must use HTTPS on github.com or gitlab.com".to_string())?;
    if !path.ends_with(".git") {
        return Err("Repository URL must end in .git".into());
    }
    let project_path = &path[..path.len() - 4];
    let parts = project_path.split('/').collect::<Vec<_>>();
    if parts.len() < 2
        || project_path.starts_with('/')
        || project_path.ends_with('/')
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.starts_with('-')
                || !part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        })
    {
        return Err("Repository URL has an invalid owner/project path".into());
    }
    Ok(())
}

fn validate_external_scene_path(path: &Path) -> Result<(), String> {
    validate_relative_path(path, "external scene")?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .ok_or_else(|| "External scene must have a .gltf or .glb extension".to_string())?;
    if !matches!(extension.as_str(), "gltf" | "glb") {
        return Err("External scene must have a .gltf or .glb extension".into());
    }
    Ok(())
}

fn portable_relative_path(path: &Path) -> Result<String, String> {
    validate_relative_path(path, "external scene")?;
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(value) = component else {
            return Err("External scene path contains a non-normal component".into());
        };
        parts.push(
            value
                .to_str()
                .ok_or_else(|| "External scene path must be valid UTF-8".to_string())?,
        );
    }
    if parts.is_empty() {
        return Err("External scene path cannot be empty".into());
    }
    Ok(parts.join("/"))
}

fn find_external_scene_candidates(
    root: &Path,
    current: &Path,
    depth: usize,
    output: &mut Vec<PathBuf>,
) -> Result<(), String> {
    const MAX_DEPTH: usize = 8;
    const MAX_CANDIDATES: usize = 256;
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for entry in fs::read_dir(current).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if matches!(name, ".git" | ".kairo" | "Build" | "build" | "node_modules") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            find_external_scene_candidates(root, &path, depth + 1, output)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        if !matches!(extension.as_deref(), Some("gltf" | "glb")) {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "External scene escaped the cloned repository".to_string())?
            .to_path_buf();
        validate_external_scene_path(&relative)?;
        output.push(relative);
        if output.len() > MAX_CANDIDATES {
            return Err(format!(
                "External repository contains more than {MAX_CANDIDATES} glTF/GLB scenes; specify a narrower entry scene"
            ));
        }
    }
    Ok(())
}

fn resolve_external_scene(root: &Path, requested: Option<&str>) -> Result<PathBuf, String> {
    let relative = if let Some(requested) = requested.filter(|value| !value.trim().is_empty()) {
        let path = PathBuf::from(requested.trim());
        validate_external_scene_path(&path)?;
        path
    } else {
        let mut candidates = Vec::new();
        find_external_scene_candidates(root, root, 0, &mut candidates)?;
        candidates.sort();
        match candidates.len() {
            0 => {
                return Err(
                    "External repository contains no .gltf or .glb scene. Kairo's external importer v1 supports glTF/GLB content repositories only."
                        .into(),
                )
            }
            1 => candidates.remove(0),
            _ => {
                let preview = candidates
                    .iter()
                    .take(8)
                    .map(|path| portable_relative_path(path).unwrap_or_else(|_| path.display().to_string()))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "External repository contains {} glTF/GLB scenes. Specify the entry scene explicitly. Candidates: {}{}",
                    candidates.len(),
                    preview,
                    if candidates.len() > 8 { ", ..." } else { "" }
                ));
            }
        }
    };

    let candidate = root.join(&relative);
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        format!(
            "Cannot inspect external scene {}: {error}",
            relative.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("External scene must be a regular non-symlink file".into());
    }
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| format!("Cannot resolve external repository root: {error}"))?;
    let canonical_scene = fs::canonicalize(&candidate)
        .map_err(|error| format!("Cannot resolve external scene: {error}"))?;
    if !canonical_scene.starts_with(&canonical_root) {
        return Err("External scene resolves outside the cloned repository".into());
    }
    Ok(relative)
}

fn generate_external_gltf_project(
    root: &Path,
    scene_relative: &Path,
    display_name: &str,
    engine_version: &str,
) -> Result<PathBuf, String> {
    validate_external_scene_path(scene_relative)?;
    if display_name.trim().is_empty() || display_name.contains(['\n', '\r']) {
        return Err("Imported project display name must be non-empty and single-line".into());
    }
    if engine_version.trim().is_empty() || engine_version.contains(['\n', '\r']) {
        return Err("Imported project engine version must be non-empty and single-line".into());
    }
    let portable_scene = portable_relative_path(scene_relative)?;
    let asset_id = "90000000-0000-4000-8000-000000000001";

    let generated_root = root.join(".kairo");
    let scenes_dir = generated_root.join("Scenes");
    let config_dir = generated_root.join("Config");
    let project = root.join("KairoImported.kproject");
    let generated_files = [
        project.clone(),
        generated_root.join("Assets.kassets"),
        scenes_dir.join("Imported.kscene"),
        config_dir.join("Input.kinput"),
    ];
    if let Some(existing) = generated_files.iter().find(|path| path.exists()) {
        return Err(format!(
            "External import would overwrite an existing generated file: {}",
            existing.display()
        ));
    }

    fs::create_dir_all(&scenes_dir).map_err(|error| error.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|error| error.to_string())?;

    let manifest = format!(
        "kairo-assets 1\nasset {asset_id} scene source 1 {} \"kairo.gltf.scene\"\nend\n",
        quote(&portable_scene)
    );
    write_atomic(&generated_root.join("Assets.kassets"), &manifest)?;

    let scene = format!(
        "kairo-scene 4\n\
entity 1 \"Imported Scene\"\n\
enabled true\n\
layer 0\n\
transform 0 0 0 0 0 0 1 1 1 1\n\
scene-instance {asset_id} true true true 18446744073709551615\n\
end\n\
entity 2 \"Main Camera\"\n\
enabled true\n\
layer 0\n\
transform 0 2.5 6 0 0 0 1 1 1 1\n\
camera perspective 0.87266463 10 0.1 500 0 true environment 0.02 0.025 0.035 1 18446744073709551615\n\
end\n\
entity 3 \"Sun\"\n\
enabled true\n\
layer 0\n\
transform 0 5 3 -0.38268343 0 0 0.9238795 1 1 1\n\
light directional 1 0.95 0.85 60000 lux 100 0.34906585 0.52359878 1 1 soft 0.001 0.01 18446744073709551615\n\
end\n\
entity 4 \"World\"\n\
enabled true\n\
layer 0\n\
transform 0 0 0 0 0 0 1 1 1 1\n\
environment true 10 0.02 0.03 0.05 none 0.12 1 disabled 0.5 0.5 0.5 0.01 0 1000 0 aces global\n\
end\n"
    );
    write_atomic(&scenes_dir.join("Imported.kscene"), &scene)?;
    write_atomic(
        &config_dir.join("Input.kinput"),
        "kairo-input 1\naction \"Quit\" button\nbind \"Quit\" key Escape 1 0 0\n",
    )?;

    let descriptor = format!(
        "kairo-project 2\nname {}\nengine-version {}\nassets \".kairo/Assets.kassets\"\nstartup-scene \".kairo/Scenes/Imported.kscene\"\ninput-map \".kairo/Config/Input.kinput\"\nrendering-profile \"desktop\"\ngraphics-backend \"auto\"\nbuild-profile \"Development\" development \"Build/Development\"\nbuild-profile \"Release\" release \"Build/Release\"\n",
        quote(display_name.trim()),
        quote(engine_version.trim())
    );
    write_atomic(&project, &descriptor)?;

    let health = import_project(&project)?;
    if !health.is_valid() {
        return Err(format!(
            "Generated external Kairo project is invalid: {}",
            health.errors.join("; ")
        ));
    }
    Ok(project)
}

/// Converts one existing local content repository into a Kairo project without
/// copying or rewriting the authored glTF/GLB payload. Generated Kairo bootstrap
/// files live at the repository root/.kairo boundary.
pub fn import_external_gltf_directory(
    root: &Path,
    entry_scene: Option<&str>,
    display_name: &str,
    engine_version: &str,
) -> Result<PathBuf, String> {
    if !root.is_dir() {
        return Err(format!(
            "External import root is not a directory: {}",
            root.display()
        ));
    }
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("Cannot inspect external import root: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("External import root cannot be a symbolic link".into());
    }
    let scene = resolve_external_scene(root, entry_scene)?;
    generate_external_gltf_project(root, &scene, display_name, engine_version)
}

/// Clones a non-Kairo repository and converts one glTF/GLB scene into a real
/// runnable Kairo project. This is intentionally not an arbitrary Unity/Godot/
/// Unreal converter: gameplay code and proprietary engine metadata are not
/// silently translated.
pub fn clone_external_gltf_project(
    repository: &str,
    parent: &Path,
    folder_name: &str,
    entry_scene: Option<&str>,
    engine_version: &str,
) -> Result<PathBuf, String> {
    validate_clone_repository(repository)?;
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
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["clone", "--depth", "1", "--"])
        .arg(repository)
        .arg(&destination)
        .status()
        .map_err(|error| format!("Cannot start git clone: {error}"))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&destination);
        return Err(format!("git clone failed with status {status}"));
    }

    let result =
        import_external_gltf_directory(&destination, entry_scene, folder_name, engine_version);
    if result.is_err() {
        let _ = fs::remove_dir_all(&destination);
    }
    result
}

pub fn clone_project(
    repository: &str,
    parent: &Path,
    folder_name: &str,
) -> Result<PathBuf, String> {
    validate_clone_repository(repository)?;
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
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["clone", "--depth", "1", "--"])
        .arg(repository)
        .arg(&destination)
        .status()
        .map_err(|error| format!("Cannot start git clone: {error}"))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&destination);
        return Err(format!("git clone failed with status {status}"));
    }

    let result = (|| {
        let mut descriptors = Vec::new();
        find_project_descriptors(&destination, 0, &mut descriptors)?;
        if descriptors.len() != 1 {
            return Err(match descriptors.len() {
                0 => "Cloned repository contains no .kproject descriptor".into(),
                count => format!(
                    "Cloned repository contains {count} .kproject descriptors; select one through local import instead"
                ),
            });
        }
        let descriptor = descriptors.remove(0);
        import_project(&descriptor)?;
        Ok(descriptor)
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&destination);
    }
    result
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

fn validate_player_operation(
    project: &Path,
    installation: &EngineInstallation,
) -> Result<ProjectHealth, String> {
    let health = inspect_project(project);
    if !health.is_valid() {
        return Err(format!(
            "Project validation failed: {}",
            health.errors.join("; ")
        ));
    }
    validate_project_engine_version(&health, installation)?;
    if health
        .descriptor
        .as_ref()
        .and_then(|descriptor| descriptor.runtime_executable.as_ref())
        .is_none()
        && (!installation.player_available || !installation.player.is_file())
    {
        return Err(format!(
            "Selected KairoPlayer build is missing: {}",
            installation.player.display()
        ));
    }
    if !installation.project_compiler_available || !installation.project_compiler.is_file() {
        return Err(format!(
            "Selected KairoProjectCompiler build is missing: {}",
            installation.project_compiler.display()
        ));
    }
    Ok(health)
}

fn compile_project_logic(project: &Path, installation: &EngineInstallation) -> Result<(), String> {
    let build = Command::new(&installation.project_compiler)
        .arg(project)
        .status()
        .map_err(|error| format!("Cannot run KairoProjectCompiler: {error}"))?;
    if !build.success() {
        return Err(format!(
            "Project logic build failed with status {}. Fix compiler diagnostics before running.",
            build
        ));
    }
    Ok(())
}

/// Builds attached visual logic, then launches the selected engine's player
/// after KairoHub's structural inspection and KairoPlayer's runtime boundary.
/// Command arguments are passed directly, never through a host shell.
pub fn launch_player_with(
    project: &Path,
    installation: &EngineInstallation,
) -> Result<Child, String> {
    let health = validate_player_operation(project, installation)?;
    compile_project_logic(project, installation)?;

    let descriptor = health
        .descriptor
        .as_ref()
        .ok_or_else(|| "Project descriptor is unavailable after validation".to_string())?;

    let executable = if let Some(relative) = descriptor.runtime_executable.as_deref() {
        let project_root = project
            .parent()
            .ok_or_else(|| "Project descriptor has no parent directory".to_string())?;
        let requested = project_root.join(relative);
        let resolved = fs::canonicalize(&requested).map_err(|error| {
            format!(
                "Project runtime executable is missing. Build the Development target first: {} ({error})",
                requested.display()
            )
        })?;
        let root = fs::canonicalize(project_root)
            .map_err(|error| format!("Cannot canonicalize project root: {error}"))?;
        if !resolved.starts_with(&root) || !resolved.is_file() {
            return Err(format!(
                "Project runtime executable must be a regular file inside the project root: {}",
                resolved.display()
            ));
        }
        resolved
    } else {
        installation.player.clone()
    };

    Command::new(&executable)
        .arg(project)
        .current_dir(
            project
                .parent()
                .ok_or_else(|| "Project descriptor has no parent directory".to_string())?,
        )
        .spawn()
        .map_err(|error| {
            format!(
                "Cannot launch project runtime {}: {error}",
                executable.display()
            )
        })
}

fn bounded_process_diagnostics(output: &[u8]) -> String {
    const MAX_DIAGNOSTIC_BYTES: usize = 8 * 1024;
    let visible = &output[..output.len().min(MAX_DIAGNOSTIC_BYTES)];
    String::from_utf8_lossy(visible).trim().to_string()
}

/// Builds project logic and asks the selected KairoPlayer to package one exact
/// descriptor-defined profile. KairoPlayer owns staging, traversal safety, and
/// atomic publication; Hub owns engine selection and process orchestration.
pub fn package_project_with(
    project: &Path,
    profile_name: &str,
    replace: bool,
    installation: &EngineInstallation,
) -> Result<PackageArtifact, String> {
    let health = validate_player_operation(project, installation)?;
    let descriptor = health
        .descriptor
        .as_ref()
        .ok_or_else(|| "Project descriptor is unavailable after validation".to_string())?;
    let profile = descriptor
        .build_profiles
        .iter()
        .find(|profile| profile.name == profile_name)
        .ok_or_else(|| format!("Unknown project build profile: {profile_name}"))?
        .clone();

    compile_project_logic(project, installation)?;
    let mut command = Command::new(&installation.player);
    command.arg(project).arg("--package").arg(&profile.name);
    if replace {
        command.arg("--replace");
    }
    let output = command
        .output()
        .map_err(|error| format!("Cannot run KairoPlayer package operation: {error}"))?;
    if !output.status.success() {
        let stderr = bounded_process_diagnostics(&output.stderr);
        let stdout = bounded_process_diagnostics(&output.stdout);
        let diagnostics = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if diagnostics.is_empty() {
            format!(
                "KairoPlayer package operation failed with status {}",
                output.status
            )
        } else {
            format!(
                "KairoPlayer package operation failed with status {}: {diagnostics}",
                output.status
            )
        });
    }

    let project_root = project
        .parent()
        .ok_or_else(|| "Project descriptor has no parent directory".to_string())?;
    let requested_output = project_root.join(&profile.output_directory);
    let output_directory = fs::canonicalize(&requested_output).map_err(|error| {
        format!(
            "KairoPlayer reported success but profile output {} is unavailable: {error}",
            requested_output.display()
        )
    })?;
    if !output_directory.is_dir() {
        return Err(format!(
            "KairoPlayer profile output is not a directory: {}",
            output_directory.display()
        ));
    }
    let manifest_path = output_directory.join("package.kmanifest");
    if !manifest_path.is_file() {
        return Err(format!(
            "KairoPlayer package manifest is missing: {}",
            manifest_path.display()
        ));
    }
    Ok(PackageArtifact {
        profile_name: profile.name,
        profile_kind: profile.kind,
        output_directory,
        manifest_path,
    })
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
        let input =
            fs::read_to_string(temporary.path().join("TestGame/Config/Input.kinput")).unwrap();
        assert!(input.contains("action \"Move\" axis2d"));
        assert!(input.contains("action \"Quit\" button"));
    }

    #[test]
    fn project_v2_round_trips_runtime_and_build_metadata() {
        let descriptor = parse_project(
            "kairo-project 2\nname \"V2\"\nengine-version \"0.1.0\"\nassets \"Assets.kassets\"\nstartup-scene \"Scenes/Main.kscene\"\ninput-map \"Config/Input.kinput\"\nrendering-profile \"desktop\"\nplugin \"kairo.physics\"\nbuild-profile \"Shipping\" release \"Artifacts/Shipping\"\n",
        )
        .unwrap();
        assert_eq!(descriptor.enabled_plugins, vec!["kairo.physics"]);
        assert_eq!(descriptor.build_profiles[0].kind, "release");
        assert_eq!(descriptor.graphics_backend, "auto");

        let explicit = parse_project(
            "kairo-project 2\nname \"GL\"\nengine-version \"0.1.0\"\nassets \"Assets.kassets\"\nstartup-scene \"Scenes/Main.kscene\"\ninput-map \"Config/Input.kinput\"\nrendering-profile \"desktop\"\ngraphics-backend \"opengl\"\nbuild-profile \"Shipping\" release \"Artifacts/Shipping\"\n",
        )
        .unwrap();
        assert_eq!(explicit.graphics_backend, "opengl");
        assert!(parse_project(
            "kairo-project 2\nname \"Bad\"\nengine-version \"0.1.0\"\nassets \"Assets.kassets\"\nstartup-scene \"Scenes/Main.kscene\"\ninput-map \"Config/Input.kinput\"\nrendering-profile \"desktop\"\ngraphics-backend \"software\"\nbuild-profile \"Shipping\" release \"Artifacts/Shipping\"\n"
        ).is_err());
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
        fs::write(
            temporary.path().join("Config/Input.kinput"),
            "kairo-input 1\naction \"Custom\" button\nbind \"Custom\" key K 1 0 0\n",
        )
        .unwrap();
        repair_project(&descriptor).unwrap();
        assert!(
            fs::read_to_string(temporary.path().join("Config/Input.kinput"))
                .unwrap()
                .contains("Custom")
        );
    }

    #[test]
    fn clone_inputs_reject_option_and_untrusted_transport_injection() {
        assert!(validate_clone_repository("--upload-pack=bad").is_err());
        assert!(validate_clone_repository("git@github.com:owner/game.git").is_err());
        assert!(validate_clone_repository("http://github.com/owner/game.git").is_err());
        assert!(validate_clone_repository("https://example.com/owner/game").is_err());
        assert!(validate_clone_repository("https://example.com/owner/game.git").is_err());
        assert!(validate_clone_repository("https://github.com/owner/game").is_err());
        assert!(validate_clone_repository("https://github.com/../game.git").is_err());
        assert!(validate_clone_repository("https://github.com/-owner/game.git").is_err());
        assert!(validate_clone_repository("https://github.com/owner/game.git?x=1").is_err());
        assert!(validate_clone_repository("https://github.com/owner/game.git#fragment").is_err());
        assert!(validate_clone_repository("https://github.com/game.git").is_err());
        assert!(validate_clone_repository("https://github.com/owner/%2e%2e/game.git").is_err());
        assert!(validate_clone_repository("https://github.com/owner/game.git").is_ok());
        assert!(validate_clone_repository("https://gitlab.com/group/subgroup/game.git").is_ok());
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
    fn local_import_requires_a_complete_runnable_kairo_project_contract() {
        let temporary = tempfile::tempdir().unwrap();
        let project = create_project(temporary.path(), "Imported", "Imported Game").unwrap();
        let imported = import_project(&project).unwrap();
        assert_eq!(imported.descriptor.unwrap().name, "Imported Game");

        fs::remove_file(temporary.path().join("Imported/Scenes/Main.kscene")).unwrap();
        let error = import_project(&project).unwrap_err();
        assert!(error.contains("Missing startup scene"));
    }

    #[test]
    fn external_gltf_generation_creates_a_runnable_kairo_bootstrap() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("ExternalGame");
        fs::create_dir_all(root.join("content")).unwrap();
        fs::write(root.join("content/world.glb"), b"glb fixture placeholder").unwrap();

        let project = generate_external_gltf_project(
            &root,
            Path::new("content/world.glb"),
            "External Game",
            "0.1.0",
        )
        .unwrap();

        let health = inspect_project(&project);
        assert!(health.is_valid(), "{:?}", health.errors);
        assert_eq!(health.descriptor.unwrap().name, "External Game");

        let manifest = fs::read_to_string(root.join(".kairo/Assets.kassets")).unwrap();
        assert!(manifest.contains("content/world.glb"));
        assert!(manifest.contains("kairo.gltf.scene"));

        let scene = fs::read_to_string(root.join(".kairo/Scenes/Imported.kscene")).unwrap();
        assert!(scene.contains("scene-instance 90000000-0000-4000-8000-000000000001"));
        assert!(scene.contains("camera perspective"));
        assert!(scene.contains("light directional"));

        fs::write(root.join(".kairo/Assets.kassets"), "sentinel").unwrap();
        let error = generate_external_gltf_project(
            &root,
            Path::new("content/world.glb"),
            "External Game",
            "0.1.0",
        )
        .unwrap_err();
        assert!(error.contains("overwrite"));
        assert_eq!(
            fs::read_to_string(root.join(".kairo/Assets.kassets")).unwrap(),
            "sentinel"
        );
    }

    #[test]
    fn external_gltf_generation_rejects_escape_and_wrong_format() {
        let temporary = tempfile::tempdir().unwrap();
        assert!(
            generate_external_gltf_project(
                temporary.path(),
                Path::new("../outside.glb"),
                "Bad",
                "0.1.0",
            )
            .is_err()
        );
        assert!(
            generate_external_gltf_project(
                temporary.path(),
                Path::new("scene.fbx"),
                "Bad",
                "0.1.0",
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
        assert!(!installation.project_compiler_available);
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
            .join("build/dev-clang/components/KairoEditor/KairoEditorApp");
        let player = temporary
            .path()
            .join("build/dev-clang/Runtime/KairoPlayer/KairoPlayer");
        let project_compiler = temporary
            .path()
            .join("build/dev-clang/components/KairoEditor/KairoProjectCompiler");
        fs::create_dir_all(editor.parent().unwrap()).unwrap();
        fs::create_dir_all(player.parent().unwrap()).unwrap();
        fs::write(&editor, b"fixture").unwrap();
        fs::write(&project_compiler, b"fixture").unwrap();
        fs::write(&player, b"fixture").unwrap();
        let installation = inspect_engine(temporary.path()).unwrap();
        assert!(installation.editor_available);
        assert!(installation.project_compiler_available);
        assert!(installation.player_available);
        assert_eq!(installation.editor, editor);
        assert_eq!(installation.project_compiler, project_compiler);
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
            project_compiler: temporary.path().join("ProjectCompiler"),
            project_compiler_available: false,
            player: temporary.path().join("Player"),
            player_available: false,
        };
        let error = validate_project_engine_version(&health, &installation).unwrap_err();
        assert!(error.contains("requires Kairo 0.1.0"));
        assert!(error.contains("selected installation is Kairo 9.0.0"));
    }

    #[cfg(unix)]
    #[test]
    fn player_launch_requires_a_successful_project_logic_build() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let project = create_project(temporary.path(), "Runnable", "Runnable").unwrap();
        import_project(&project).expect("fresh Kairo project must import before launch");
        let compiler = temporary.path().join("compiler.sh");
        let player = temporary.path().join("player.sh");
        let marker = temporary.path().join("player-started");
        fs::write(&compiler, "#!/bin/sh\nexit 7\n").unwrap();
        fs::write(
            &player,
            format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&player, fs::Permissions::from_mode(0o755)).unwrap();
        let installation = EngineInstallation {
            root: temporary.path().to_path_buf(),
            version: "0.1.0".into(),
            editor: temporary.path().join("Editor"),
            editor_available: false,
            project_compiler: compiler.clone(),
            project_compiler_available: true,
            player: player.clone(),
            player_available: true,
        };
        let failure = launch_player_with(&project, &installation).unwrap_err();
        assert!(failure.contains("Project logic build failed"));
        assert!(!marker.exists(), "player started after compiler failure");

        fs::write(&compiler, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).unwrap();
        let mut child = launch_player_with(&project, &installation).unwrap();
        assert!(child.wait().unwrap().success());
        assert!(
            marker.exists(),
            "player did not start after successful compilation"
        );
    }

    #[test]
    fn project_v2_parses_optional_runtime_executable() {
        let descriptor = parse_project(
            "kairo-project 2\nname \"Custom Runtime\"\nengine-version \"0.1.0\"\nassets \"Assets.kassets\"\nstartup-scene \"Scenes/Main.kscene\"\ninput-map \"Config/Input.kinput\"\nrendering-profile \"desktop\"\ngraphics-backend \"auto\"\nruntime-executable \"Build/Development/Game\"\nbuild-profile \"Development\" development \"Build/Development\"\n"
        )
        .unwrap();
        assert_eq!(
            descriptor.runtime_executable,
            Some(PathBuf::from("Build/Development/Game"))
        );
        assert!(parse_project(
            "kairo-project 2\nname \"Bad\"\nengine-version \"0.1.0\"\nassets \"Assets.kassets\"\nstartup-scene \"Scenes/Main.kscene\"\ninput-map \"Config/Input.kinput\"\nrendering-profile \"desktop\"\nruntime-executable \"../escape\"\nbuild-profile \"Development\" development \"Build/Development\"\n"
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn project_run_prefers_custom_runtime_over_generic_player() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let project = create_project(temporary.path(), "CustomRuntime", "Custom Runtime").unwrap();
        let root = project.parent().unwrap();
        let descriptor_source = fs::read_to_string(&project).unwrap();
        let descriptor_source = descriptor_source.replace(
            "graphics-backend \"auto\"\n",
            "graphics-backend \"auto\"\nruntime-executable \"Build/Development/CustomGame\"\n",
        );
        fs::write(&project, descriptor_source).unwrap();

        let runtime = root.join("Build/Development/CustomGame");
        fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        let runtime_marker = root.join("custom-runtime-started");
        fs::write(
            &runtime,
            format!("#!/bin/sh\ntouch '{}'\n", runtime_marker.display()),
        )
        .unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();

        let compiler = temporary.path().join("compiler.sh");
        fs::write(&compiler, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).unwrap();

        let generic_player = temporary.path().join("generic-player.sh");
        let generic_marker = temporary.path().join("generic-player-started");
        fs::write(
            &generic_player,
            format!("#!/bin/sh\ntouch '{}'\n", generic_marker.display()),
        )
        .unwrap();
        fs::set_permissions(&generic_player, fs::Permissions::from_mode(0o755)).unwrap();

        let installation = EngineInstallation {
            root: temporary.path().to_path_buf(),
            version: "0.1.0".into(),
            editor: temporary.path().join("Editor"),
            editor_available: false,
            project_compiler: compiler,
            project_compiler_available: true,
            player: generic_player,
            player_available: true,
        };

        let mut child = launch_player_with(&project, &installation).unwrap();
        assert!(child.wait().unwrap().success());
        assert!(runtime_marker.is_file());
        assert!(!generic_marker.exists());
    }

    #[cfg(unix)]
    #[test]
    fn packaging_uses_exact_profile_and_verified_player_artifact() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let project = create_project(temporary.path(), "Packaged", "Packaged").unwrap();
        let compiler = temporary.path().join("compiler.sh");
        let player = temporary.path().join("player.sh");
        let compiler_marker = temporary.path().join("compiler-ran");
        let arguments_marker = temporary.path().join("player-arguments");
        fs::write(
            &compiler,
            format!("#!/bin/sh\ntouch '{}'\n", compiler_marker.display()),
        )
        .unwrap();
        fs::write(
            &player,
            format!(
                "#!/bin/sh\nproject=$1\nshift\nprintf '%s\\n' \"$@\" > '{}'\nroot=$(dirname -- \"$project\")\nmkdir -p \"$root/Build/Release\"\nprintf 'kairo-package 1\\n' > \"$root/Build/Release/package.kmanifest\"\n",
                arguments_marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&player, fs::Permissions::from_mode(0o755)).unwrap();
        let installation = EngineInstallation {
            root: temporary.path().to_path_buf(),
            version: "0.1.0".into(),
            editor: temporary.path().join("Editor"),
            editor_available: false,
            project_compiler: compiler.clone(),
            project_compiler_available: true,
            player: player.clone(),
            player_available: true,
        };

        let unknown = package_project_with(&project, "Shipping", false, &installation).unwrap_err();
        assert!(unknown.contains("Unknown project build profile"));
        assert!(
            !compiler_marker.exists(),
            "compiler ran before the requested profile was validated"
        );

        let artifact = package_project_with(&project, "Release", true, &installation).unwrap();
        assert!(compiler_marker.is_file(), "project compiler did not run");
        assert_eq!(artifact.profile_name, "Release");
        assert_eq!(artifact.profile_kind, "release");
        assert_eq!(
            artifact.output_directory,
            fs::canonicalize(temporary.path().join("Packaged/Build/Release")).unwrap()
        );
        assert!(artifact.manifest_path.is_file());
        assert_eq!(
            fs::read_to_string(arguments_marker).unwrap(),
            "--package\nRelease\n--replace\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn packaging_surfaces_bounded_player_diagnostics() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let project = create_project(temporary.path(), "BrokenPackage", "Broken Package").unwrap();
        let compiler = temporary.path().join("compiler.sh");
        let player = temporary.path().join("player.sh");
        fs::write(&compiler, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(
            &player,
            "#!/bin/sh\necho 'specific package failure' >&2\nexit 9\n",
        )
        .unwrap();
        fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&player, fs::Permissions::from_mode(0o755)).unwrap();
        let installation = EngineInstallation {
            root: temporary.path().to_path_buf(),
            version: "0.1.0".into(),
            editor: temporary.path().join("Editor"),
            editor_available: false,
            project_compiler: compiler,
            project_compiler_available: true,
            player,
            player_available: true,
        };
        let error =
            package_project_with(&project, "Development", false, &installation).unwrap_err();
        assert!(error.contains("specific package failure"));
        assert!(error.contains("status"));
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
