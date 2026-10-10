//! Read-only intake of source-controlled games from other engines.
//!
//! This is source inspection and an explicit "run in original engine" adapter.
//! It is NOT binary decompilation or gameplay translation. Only glTF/GLB
//! content can currently be reconstructed as a Kairo-native scene.

use serde::Serialize;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_SCAN_ENTRIES: usize = 40_000;
const MAX_SCAN_DEPTH: usize = 12;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceGameInspection {
    pub root: PathBuf,
    pub engine: String,
    pub display_name: String,
    pub main_scene: Option<String>,
    pub scene_files: usize,
    pub script_files: usize,
    pub media_files: usize,
    pub scanned_files: usize,
    /// A single glTF/GLB file is a candidate for Kairo-native SCENE import.
    /// Game logic, scripts, animation state machines etc. are NOT translated.
    pub native_scene_candidate: Option<String>,
    /// Deterministic bounded list for source games containing multiple
    /// portable glTF/GLB scenes, requiring explicit selection.
    pub native_scene_candidates: Vec<String>,
    /// Godot runs through Godot, never through KairoPlayer.
    pub original_runtime_available: bool,
    pub notes: Vec<String>,
}

fn is_regular_marker(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|v| v.is_file() && !v.file_type().is_symlink())
        .unwrap_or(false)
}

fn ensure_safe_root(root: &Path) -> Result<PathBuf, String> {
    if root.as_os_str().is_empty() {
        return Err("Choose a source-game directory".into());
    }
    let metadata =
        fs::symlink_metadata(root).map_err(|e| format!("Cannot inspect game directory: {e}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Choose a real directory, not a file or symbolic link".into());
    }
    fs::canonicalize(root).map_err(|e| format!("Cannot resolve game directory: {e}"))
}

fn godot_setting(content: &str, section: &str, key: &str) -> Option<String> {
    let mut active = "";
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            active = &line[1..line.len() - 1];
            continue;
        }
        if active != section {
            continue;
        }
        if let Some((found, value)) = line.split_once('=') {
            if found.trim() == key {
                let value = value.trim();
                // We deliberately reject multiline and escaped paths instead of
                // interpreting arbitrary Godot expressions or executing GDScript.
                if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                    return Some(value[1..value.len() - 1].to_owned());
                }
            }
        }
    }
    None
}

/// Resolve Godot 4 UID-based startup references from text-scene headers.
////
//// We intentionally do not parse Godot's generated binary UID cache. Instead,
//// search project-owned .tscn headers with strict resource limits. Returning
//// an ambiguous UID is worse than refusing to play the wrong scene.
fn resolve_godot_scene_uid(root: &Path, uid: &str) -> Result<String, String> {
    if !uid.starts_with("uid://")
        || uid.len() > 160
        || !uid[6..].chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err("Godot startup UID has an unsupported format".into());
    }
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    let mut match_path: Option<String> = None;
    while let Some((directory, depth)) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            visited += 1;
            if visited > MAX_SCAN_ENTRIES {
                return Err("Godot UID lookup exceeded the bounded project scan".into());
            }
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                let name = entry.file_name();
                if matches!(
                    name.to_str().unwrap_or(""),
                    ".git" | ".godot" | ".kairo" | "node_modules"
                ) {
                    continue;
                }
                if depth < MAX_SCAN_DEPTH {
                    pending.push((path, depth + 1));
                }
                continue;
            }
            if !meta.is_file()
                || path.extension() != Some(OsStr::new("tscn"))
                || meta.len() > 8 * 1024 * 1024
            {
                continue;
            }
            // Only the scene header may declare the scene's UID.
            let mut file = fs::File::open(&path).map_err(|e| e.to_string())?;
            let mut prefix = [0u8; 4096];
            use std::io::Read;
            let bytes = file.read(&mut prefix).map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&prefix[..bytes]);
            let header = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            if !header.starts_with("[gd_scene ") || !header.ends_with(']') {
                continue;
            }
            // Delimit UID token to avoid accepting a prefix of another UID.
            let expected = format!("uid=\"{uid}\"");
            if !header
                .split_ascii_whitespace()
                .any(|field| field == expected || field == format!("{expected}]"))
            {
                continue;
            }
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
            let portable = format!("res://{}", relative.to_string_lossy().replace('\\', "/"));
            if match_path.replace(portable).is_some() {
                return Err(format!(
                    "Godot scene UID {uid} is ambiguous between multiple scene files"
                ));
            }
        }
    }
    match_path
        .ok_or_else(|| format!("Godot scene UID {uid} was not found in project-owned text scenes"))
}

fn inspect_godot_scene(root: &Path, scene: &str) -> Result<String, String> {
    let owned_scene = if scene.starts_with("uid://") {
        resolve_godot_scene_uid(root, scene)?
    } else {
        scene.to_owned()
    };
    let path = owned_scene.strip_prefix("res://").ok_or(
        "Godot main_scene must be a project-relative res:// path or resolvable uid:// text scene",
    )?;
    if path.is_empty() || path.contains('\\') || path.contains('\0') {
        return Err("Godot startup scene must have a safe non-empty res:// path".into());
    }
    let relative = Path::new(path);
    if relative
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Godot startup scene must not contain traversal or absolute paths".into());
    }
    let absolute = root.join(relative);
    if !is_regular_marker(&absolute) {
        return Err(format!("Godot startup scene is missing: {scene}"));
    }
    let canonical = fs::canonicalize(&absolute).map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("Godot startup scene escapes the source-game directory".into());
    }
    Ok(owned_scene)
}

fn executable_at(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

pub fn find_godot_runtime() -> Option<PathBuf> {
    // An explicit user-configured Godot installation has priority.
    if let Some(value) = std::env::var_os("KAIRO_GODOT_EXECUTABLE") {
        let path = PathBuf::from(value);
        if executable_at(&path) {
            return Some(path);
        }
    }
    let executables: &[&str] = if cfg!(windows) {
        &["godot.exe", "godot4.exe", "Godot.exe"]
    } else {
        &["godot", "godot4", "Godot"]
    };
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            for executable in executables {
                let candidate = directory.join(executable);
                if executable_at(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    if cfg!(target_os = "macos") {
        let bundle = Path::new("/Applications/Godot.app/Contents/MacOS/Godot");
        if executable_at(bundle) {
            return Some(bundle.to_path_buf());
        }
    }
    None
}

pub fn inspect_source_game(input: &Path) -> Result<SourceGameInspection, String> {
    let root = ensure_safe_root(input)?;
    let godot = is_regular_marker(&root.join("project.godot"));
    let unity_marker = root.join("ProjectSettings/ProjectVersion.txt");
    let unity = is_regular_marker(&unity_marker)
        && root.join("Assets").is_dir()
        && !fs::symlink_metadata(root.join("Assets"))
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true)
        && !fs::symlink_metadata(root.join("ProjectSettings"))
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true);
    let mut unreal_markers = Vec::new();
    for entry in fs::read_dir(&root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().extension() == Some(OsStr::new("uproject"))
            && is_regular_marker(&entry.path())
        {
            unreal_markers.push(entry.path());
        }
    }
    if unreal_markers.len() > 1 {
        return Err("Found multiple .uproject files; choose a single Unreal game root".into());
    }
    let unreal = !unreal_markers.is_empty();
    if (godot as usize + unity as usize + unreal as usize) != 1 {
        return Err("Select a source root with exactly one recognized engine marker (Godot, Unity or Unreal)".into());
    }

    let engine = if godot {
        "Godot"
    } else if unity {
        "Unity"
    } else {
        "Unreal"
    };
    let mut display_name = root
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("Imported game")
        .to_owned();
    let mut main_scene = None;
    let mut notes = Vec::new();
    let mut main_valid = false;

    if godot {
        let marker = root.join("project.godot");
        let size = fs::metadata(&marker).map_err(|e| e.to_string())?.len();
        if size > MAX_CONFIG_BYTES {
            return Err("Godot project.godot is larger than the 1 MiB inspection limit".into());
        }
        let content = fs::read_to_string(&marker)
            .map_err(|e| format!("Cannot read Godot project.godot as UTF-8: {e}"))?;
        if let Some(name) = godot_setting(&content, "application", "config/name") {
            if !name.trim().is_empty() {
                display_name = name;
            }
        }
        if let Some(scene) = godot_setting(&content, "application", "run/main_scene") {
            match inspect_godot_scene(&root, &scene) {
                Ok(scene) => {
                    main_scene = Some(scene);
                    main_valid = true;
                }
                Err(reason) => notes.push(reason),
            }
        } else {
            notes.push(
                "No readable run/main_scene; configure a Godot entry scene before playing".into(),
            );
        }
    } else if unreal {
        display_name = unreal_markers[0]
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or(&display_name)
            .to_owned();
    }

    let mut pending = vec![(root.clone(), 0_usize)];
    let mut scanned_files = 0_usize;
    let mut scene_files = 0_usize;
    let mut script_files = 0_usize;
    let mut media_files = 0_usize;
    let mut native_candidates = Vec::new();
    let mut candidates_truncated = false;
    let mut walked = 0_usize;
    let mut truncated = false;

    'scan: while let Some((directory, depth)) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|e| format!("Cannot inspect {}: {e}", directory.display()))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            walked += 1;
            if walked > MAX_SCAN_ENTRIES {
                // Large commercial-sized source trees should still be
                // recognized and launchable. Counts below are partial.
                truncated = true;
                break 'scan;
            }
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if meta.is_dir() {
                if matches!(
                    name.as_ref(),
                    ".git"
                        | ".godot"
                        | ".kairo"
                        | "Library"
                        | "Temp"
                        | "Intermediate"
                        | "DerivedDataCache"
                        | "node_modules"
                        | "Build"
                        | "Binaries"
                        | ".svn"
                ) {
                    continue;
                }
                if depth < MAX_SCAN_DEPTH {
                    pending.push((path, depth + 1));
                }
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            scanned_files += 1;
            let extension = path
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(extension.as_str(), "tscn" | "scn" | "unity" | "umap") {
                scene_files += 1;
            }
            if matches!(
                extension.as_str(),
                "gd" | "cs" | "cpp" | "h" | "hpp" | "hlsl" | "shader" | "gdshader" | "lua"
            ) {
                script_files += 1;
            }
            if matches!(
                extension.as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "wav"
                    | "ogg"
                    | "mp3"
                    | "glb"
                    | "gltf"
                    | "fbx"
                    | "obj"
                    | "tga"
            ) {
                media_files += 1;
            }
            if matches!(extension.as_str(), "glb" | "gltf") {
                // Only record portable scene paths suitable for existing Kairo
                // glTF import. Never follow symbolic links or arbitrary paths.
                if let Ok(relative) = path.strip_prefix(&root) {
                    let portable = relative.components().all(|part| match part {
                        std::path::Component::Normal(value) => value.to_str().is_some_and(|s| {
                            !s.is_empty()
                                && !s.ends_with('.')
                                && !s.ends_with(' ')
                                && !s.chars().any(|ch| {
                                    matches!(ch, ':' | '\\' | '"' | '<' | '>' | '|' | '?' | '*')
                                })
                        }),
                        _ => false,
                    });
                    if portable {
                        if native_candidates.len() < 256 {
                            native_candidates.push(relative.to_string_lossy().replace('\\', "/"));
                        } else {
                            candidates_truncated = true;
                        }
                    }
                }
            }
        }
    }

    if truncated {
        notes.push(format!(
            "Source scan inspected the first {MAX_SCAN_ENTRIES} entries; file counts and 3D scene candidates are incomplete"
        ));
    }

    native_candidates.sort();
    native_candidates.dedup();
    if candidates_truncated {
        notes.push(
            "More than 256 3D scene candidates found; explicit scene discovery is incomplete"
                .into(),
        );
    }
    let native_scene_candidate =
        if !truncated && !candidates_truncated && native_candidates.len() == 1 {
            Some(native_candidates[0].clone())
        } else {
            None
        };
    if native_scene_candidate.is_some() {
        notes.push("One portable 3D scene is available for native Kairo scene-only import; gameplay is not translated".into());
    } else if native_candidates.len() > 1 {
        notes.push(
            "Multiple glTF/GLB scenes found; native import needs an explicit scene selection"
                .into(),
        );
    } else {
        notes.push("No automatically convertible glTF/GLB scene; a full gameplay migration is not currently supported".into());
    }
    if unity || unreal {
        notes.push("Unity/Unreal gameplay and proprietary serialized assets are not converted or executed by Kairo in this version".into());
    }
    let runtime_available = godot && main_valid && find_godot_runtime().is_some();
    if godot && main_valid && !runtime_available {
        notes.push(
            "Install Godot 4 or set KAIRO_GODOT_EXECUTABLE to Run using the original Godot engine"
                .into(),
        );
    }
    Ok(SourceGameInspection {
        root,
        engine: engine.into(),
        display_name,
        main_scene,
        scene_files,
        script_files,
        media_files,
        scanned_files,
        native_scene_candidate,
        native_scene_candidates: native_candidates,
        original_runtime_available: runtime_available,
        notes,
    })
}

/// Launch only a recognized Godot SOURCE project through its original engine.
/// Never invoke shell interpreters, unknown binaries found inside imported
/// folders, downloaded executables, or reconstructed code from source.
pub fn validate_godot_binary(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if !name.starts_with("godot") {
        return Err("Choose the Godot editor executable, not an arbitrary program".into());
    }
    let info = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect Godot executable: {error}"))?;
    if !info.is_file() || info.file_type().is_symlink() {
        return Err("Godot executable must be a real, existing file".into());
    }
    fs::canonicalize(path).map_err(|error| error.to_string())
}

pub fn launch_source_game_with(
    root: &Path,
    edit: bool,
    chosen_runtime: Option<&Path>,
) -> Result<Child, String> {
    let inspection = inspect_source_game(root)?;
    if inspection.engine != "Godot" || inspection.main_scene.is_none() {
        return Err("Only Godot source projects with a validated startup scene can launch in this version; Unity/Unreal migration is analysis-only".into());
    }
    let godot = if let Some(chosen) = chosen_runtime {
        validate_godot_binary(chosen)?
    } else {
        find_godot_runtime()
            .ok_or("Godot was not found; choose it from Hub or set KAIRO_GODOT_EXECUTABLE")?
    };
    let mut command = Command::new(godot);
    command.arg("--path").arg(&inspection.root);
    if edit {
        command.arg("--editor");
    }
    command
        .spawn()
        .map_err(|e| format!("Cannot launch original Godot game: {e}"))
}

pub fn launch_source_game(root: &Path, edit: bool) -> Result<Child, String> {
    launch_source_game_with(root, edit, None)
}

/// Native conversion is explicitly LIMITED to a portable 3D scene. There is no
/// script, logic, physics, UI, audio behaviour or pixel-game translation.
pub fn reconstruct_native_scene(
    root: &Path,
    display_name: &str,
    engine_version: &str,
) -> Result<PathBuf, String> {
    reconstruct_native_scene_selected(root, display_name, engine_version, None)
}

/// Convert exactly one explicitly selected source-owned glTF/GLB scene.
/// The chosen path MUST match the bounded, symlink-safe inventory.
pub fn reconstruct_native_scene_selected(
    root: &Path,
    display_name: &str,
    engine_version: &str,
    selected: Option<&str>,
) -> Result<PathBuf, String> {
    let info = inspect_source_game(root)?;
    let chosen = match selected {
        Some(scene) if info.native_scene_candidates.iter().any(|v| v == scene) => {
            scene.to_owned()
        }
        Some(_) => return Err("Chosen 3D scene is absent or unsafe; re-inspect the source game".into()),
        None => info.native_scene_candidate.ok_or(
            "Choose a specific glTF/GLB scene. A 2D Godot game needs a separate 2D gameplay converter; Run Original preserves its existing gameplay",
        )?,
    };
    crate::project::import_external_gltf_directory(
        &info.root,
        Some(&chosen),
        display_name,
        engine_version,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(root: &Path) {
        fs::write(root.join("project.godot"),
            "config_version=5\n[application]\nconfig/name=\"Platform Runner\"\nrun/main_scene=\"res://main.tscn\"\n").unwrap();
        fs::write(
            root.join("main.tscn"),
            "[gd_scene format=3]\n[node name=\"Main\" type=\"Node2D\"]\n",
        )
        .unwrap();
        fs::write(root.join("player.gd"), "extends CharacterBody2D\n").unwrap();
    }

    #[test]
    fn godot_project_is_classified_without_claiming_conversion() {
        let dir = tempfile::tempdir().unwrap();
        game(dir.path());
        let info = inspect_source_game(dir.path()).unwrap();
        assert_eq!(info.engine, "Godot");
        assert_eq!(info.display_name, "Platform Runner");
        assert_eq!(info.main_scene.as_deref(), Some("res://main.tscn"));
        assert_eq!(info.scene_files, 1);
        assert_eq!(info.script_files, 1);
        assert!(info.native_scene_candidate.is_none());
        assert!(reconstruct_native_scene(dir.path(), "Runner", "0.1.0").is_err());
    }

    #[test]
    fn native_scene_requires_a_single_glb_and_does_not_translate_scripts() {
        let dir = tempfile::tempdir().unwrap();
        game(dir.path());
        fs::write(dir.path().join("level.glb"), b"glTF").unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert_eq!(info.native_scene_candidate.as_deref(), Some("level.glb"));
        fs::write(dir.path().join("second.gltf"), "{}").unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert!(info.native_scene_candidate.is_none());
    }

    #[test]
    fn source_marker_ambiguity_and_path_escape_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        game(dir.path());
        fs::create_dir_all(dir.path().join("ProjectSettings")).unwrap();
        fs::create_dir_all(dir.path().join("Assets")).unwrap();
        fs::write(
            dir.path().join("ProjectSettings/ProjectVersion.txt"),
            "m_EditorVersion: 2022.3",
        )
        .unwrap();
        assert!(inspect_source_game(dir.path()).is_err());
        fs::remove_file(dir.path().join("ProjectSettings/ProjectVersion.txt")).unwrap();
        fs::write(
            dir.path().join("project.godot"),
            "config_version=5\n[application]\nrun/main_scene=\"res://../secret.tscn\"\n",
        )
        .unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert!(info.main_scene.is_none());
        assert!(!info.original_runtime_available);
    }

    #[test]
    fn godot_uid_entry_scene_resolves_to_safe_project_owned_text_scene() {
        let dir = tempfile::tempdir().unwrap();
        game(dir.path());
        fs::write(
            dir.path().join("project.godot"),
            "config_version=5\n[application]\nrun/main_scene=\"uid://abc123\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("Levels")).unwrap();
        fs::write(
            dir.path().join("Levels/Start.tscn"),
            "[gd_scene format=3 uid=\"uid://abc123\"]\n[node name=\"Start\" type=\"Node2D\"]\n",
        )
        .unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert_eq!(info.main_scene.as_deref(), Some("res://Levels/Start.tscn"));
        // Resolving to a concrete path also enables the structural game graph.
        let graph = crate::source_game_graph::inspect_godot_graph(dir.path()).unwrap();
        assert_eq!(graph.source_scene, "res://Levels/Start.tscn");
        assert_eq!(graph.nodes[0].name, "Start");
    }

    #[test]
    fn godot_uid_duplicate_or_missing_entries_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        game(dir.path());
        fs::write(
            dir.path().join("project.godot"),
            "config_version=5\n[application]\nrun/main_scene=\"uid://abc123\"\n",
        )
        .unwrap();
        let scene =
            "[gd_scene uid=\"uid://abc123\" format=3]\n[node name=\"Main\" type=\"Node2D\"]\n";
        fs::write(dir.path().join("main.tscn"), scene).unwrap();
        fs::write(dir.path().join("duplicate.tscn"), scene).unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert!(info.main_scene.is_none());
        assert!(info.notes.iter().any(|s| s.contains("ambiguous")));
        fs::remove_file(dir.path().join("duplicate.tscn")).unwrap();
        fs::write(
            dir.path().join("project.godot"),
            "config_version=5\n[application]\nrun/main_scene=\"uid://missing\"\n",
        )
        .unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert!(info.main_scene.is_none());
        assert!(info.notes.iter().any(|s| s.contains("not found")));
    }

    #[test]
    fn choose_one_of_multiple_source_owned_3d_scenes_explicitly() {
        let dir = tempfile::tempdir().unwrap();
        game(dir.path());
        fs::create_dir_all(dir.path().join("models")).unwrap();
        for name in ["a.gltf", "b.gltf"] {
            fs::write(
                dir.path().join("models").join(name),
                "{\"asset\":{\"version\":\"2.0\"}}",
            )
            .unwrap();
        }
        let info = inspect_source_game(dir.path()).unwrap();
        assert_eq!(info.native_scene_candidate, None);
        assert_eq!(
            info.native_scene_candidates,
            ["models/a.gltf".to_owned(), "models/b.gltf".to_owned()]
        );
        assert!(
            reconstruct_native_scene_selected(dir.path(), "Demo", "0.1.0", Some("../outside.gltf"))
                .is_err()
        );
        assert!(
            reconstruct_native_scene_selected(
                dir.path(),
                "Demo",
                "0.1.0",
                Some("models/absent.gltf")
            )
            .is_err()
        );
        let imported =
            reconstruct_native_scene_selected(dir.path(), "Demo", "0.1.0", Some("models/b.gltf"))
                .unwrap();
        assert!(imported.ends_with("KairoImported.kproject"));
        let manifest = fs::read_to_string(dir.path().join(".kairo/Assets.kassets")).unwrap();
        assert!(manifest.contains("models/b.gltf"));
        // Never replace an existing author's generated project on repeat import.
        assert!(
            reconstruct_native_scene_selected(dir.path(), "Demo", "0.1.0", Some("models/a.gltf"))
                .is_err()
        );
    }

    #[test]
    fn unity_project_is_inspected_without_pretending_to_run_csharp() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("ProjectSettings")).unwrap();
        fs::create_dir_all(dir.path().join("Assets/Scenes")).unwrap();
        fs::write(
            dir.path().join("ProjectSettings/ProjectVersion.txt"),
            "m_EditorVersion: 2022.3.62f1",
        )
        .unwrap();
        fs::write(dir.path().join("Assets/Scenes/Level.unity"), "%YAML 1.1").unwrap();
        fs::write(dir.path().join("Assets/Player.cs"), "class Player {}").unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert_eq!(info.engine, "Unity");
        assert_eq!(info.scene_files, 1);
        assert_eq!(info.script_files, 1);
        assert!(info.main_scene.is_none());
        assert!(!info.original_runtime_available);
        assert!(launch_source_game(dir.path(), false).is_err());
    }

    #[test]
    fn unreal_project_is_inspected_without_launching_foreign_executables() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Orbit.uproject"), "{\"FileVersion\":3}").unwrap();
        fs::create_dir_all(dir.path().join("Content")).unwrap();
        fs::write(dir.path().join("Content/World.umap"), b"UE4 cooked data").unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert_eq!(info.engine, "Unreal");
        assert_eq!(info.display_name, "Orbit");
        assert_eq!(info.scene_files, 1);
        assert!(!info.original_runtime_available);
        assert!(launch_source_game(dir.path(), false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_recursed_or_selected_as_entry_scenes() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        game(dir.path());
        fs::write(outside.path().join("hidden.glb"), b"glTF").unwrap();
        symlink(outside.path(), dir.path().join("linked")).unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert!(info.native_scene_candidate.is_none());
        fs::remove_file(dir.path().join("main.tscn")).unwrap();
        symlink(
            outside.path().join("hidden.glb"),
            dir.path().join("main.tscn"),
        )
        .unwrap();
        let info = inspect_source_game(dir.path()).unwrap();
        assert!(info.main_scene.is_none());
    }
}
