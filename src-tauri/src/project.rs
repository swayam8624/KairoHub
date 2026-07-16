use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command};

const MAX_PROJECT_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDescriptor {
    pub name: String,
    pub asset_manifest: PathBuf,
    pub startup_scene: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectHealth {
    pub descriptor_path: PathBuf,
    pub descriptor: Option<ProjectDescriptor>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
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
    let mut header = false;
    let mut name = None;
    let mut assets = None;
    let mut startup_scene = None;
    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let tokens = tokenize(line, line_number)?;
        if tokens.is_empty() {
            continue;
        }
        if !header {
            if tokens.as_slice() != ["kairo-project", "1"] {
                return Err(format!(
                    "{line_number}:1: expected 'kairo-project 1' header"
                ));
            }
            header = true;
            continue;
        }
        if tokens.len() != 2 {
            return Err(format!(
                "{line_number}:1: '{}' requires one value",
                tokens[0]
            ));
        }
        match tokens[0].as_str() {
            "name" if name.is_none() => name = Some(tokens[1].clone()),
            "assets" if assets.is_none() => assets = Some(PathBuf::from(&tokens[1])),
            "startup-scene" if startup_scene.is_none() => {
                startup_scene = Some(PathBuf::from(&tokens[1]))
            }
            "name" | "assets" | "startup-scene" => {
                return Err(format!(
                    "{line_number}:1: duplicate '{}' statement",
                    tokens[0]
                ));
            }
            unknown => return Err(format!("{line_number}:1: unknown statement '{unknown}'")),
        }
    }
    if !header {
        return Err("1:1: missing kairo-project header".into());
    }
    let descriptor = ProjectDescriptor {
        name: name.ok_or("project requires a name statement")?,
        asset_manifest: assets.ok_or("project requires an assets statement")?,
        startup_scene: startup_scene.ok_or("project requires a startup-scene statement")?,
    };
    if descriptor.name.trim().is_empty() || descriptor.name.contains(['\n', '\r']) {
        return Err("project name must be non-empty and single-line".into());
    }
    validate_relative_path(&descriptor.asset_manifest, "assets")?;
    validate_relative_path(&descriptor.startup_scene, "startup-scene")?;
    if descriptor.asset_manifest == descriptor.startup_scene {
        return Err("asset manifest and startup scene must be different".into());
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
            "kairo-project 1\nname {}\nassets \"Assets.kassets\"\nstartup-scene \"Scenes/Main.kscene\"\n",
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
    let editor = if cfg!(windows) {
        root.join("KairoEditor/build/KairoEditorApp.exe")
    } else {
        root.join("KairoEditor/build/KairoEditorApp")
    };
    Ok(EngineInstallation {
        root: root.to_path_buf(),
        version,
        editor_available: editor.is_file(),
        editor,
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
    recovery_mode: bool,
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
    if recovery_mode {
        command.arg("--no-layout-persistence");
    }
    command
        .spawn()
        .map_err(|error| format!("Cannot launch KairoEditor: {error}"))
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
    }
}
