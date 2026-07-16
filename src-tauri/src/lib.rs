mod project;

use project::{EngineInstallation, HubState, ProjectHealth, RecoverySnapshotInfo};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Manager, State};

struct ManagedHubState {
    data_file: PathBuf,
    value: Mutex<HubState>,
}

fn save_state(path: &Path, state: &HubState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.bak");
    let json = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&temporary, json).map_err(|error| error.to_string())?;
    if !path.exists() {
        return fs::rename(temporary, path).map_err(|error| error.to_string());
    }
    let _ = fs::remove_file(&backup);
    fs::rename(path, &backup)
        .map_err(|error| format!("Cannot preserve prior Hub state: {error}"))?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::rename(&backup, path);
        return Err(format!("Cannot publish Hub state: {error}"));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

fn register_discovered_engine(state: &mut HubState) {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("KAIRO_ENGINE_ROOT") {
        candidates.push(PathBuf::from(root));
    }
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.clone());
        if let Some(parent) = current.parent() {
            candidates.push(parent.to_path_buf());
        }
    }
    if let Some(root) = candidates
        .into_iter()
        .find(|candidate| project::inspect_engine(candidate).is_ok())
    {
        if !state.engine_roots.contains(&root) {
            state.engine_roots.push(root.clone());
        }
        if state.selected_engine.is_none() {
            state.selected_engine = Some(root);
        }
    }
}

#[tauri::command]
fn hub_state(state: State<'_, ManagedHubState>) -> Result<HubState, String> {
    state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())
        .map(|state| state.clone())
}

#[tauri::command]
fn inspect_project(path: PathBuf) -> ProjectHealth {
    project::inspect_project(&path)
}

#[tauri::command]
fn remember_project(path: PathBuf, state: State<'_, ManagedHubState>) -> Result<HubState, String> {
    let health = project::inspect_project(&path);
    if health.descriptor.is_none() {
        return Err(health.errors.join("; "));
    }
    let mut value = state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())?;
    value.remember(path);
    save_state(&state.data_file, &value)?;
    Ok(value.clone())
}

#[tauri::command]
fn set_favorite(
    path: PathBuf,
    favorite: bool,
    state: State<'_, ManagedHubState>,
) -> Result<HubState, String> {
    let mut value = state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())?;
    value.set_favorite(path, favorite);
    save_state(&state.data_file, &value)?;
    Ok(value.clone())
}

#[tauri::command]
fn engine_installations(
    state: State<'_, ManagedHubState>,
) -> Result<Vec<EngineInstallation>, String> {
    let value = state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())?;
    Ok(value
        .engine_roots
        .iter()
        .filter_map(|root| project::inspect_engine(root).ok())
        .collect())
}

#[tauri::command]
fn register_engine(
    root: PathBuf,
    state: State<'_, ManagedHubState>,
) -> Result<EngineInstallation, String> {
    let installation = project::inspect_engine(&root)?;
    let mut value = state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())?;
    value.register_engine(root);
    save_state(&state.data_file, &value)?;
    Ok(installation)
}

#[tauri::command]
fn select_engine(root: PathBuf, state: State<'_, ManagedHubState>) -> Result<HubState, String> {
    project::inspect_engine(&root)?;
    let mut value = state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())?;
    if !value.engine_roots.contains(&root) {
        return Err("Engine installation must be registered before selection".into());
    }
    value.selected_engine = Some(root);
    save_state(&state.data_file, &value)?;
    Ok(value.clone())
}

#[tauri::command]
fn create_project(
    parent: PathBuf,
    folder_name: String,
    display_name: String,
    state: State<'_, ManagedHubState>,
) -> Result<PathBuf, String> {
    let path = project::create_project(&parent, &folder_name, &display_name)?;
    let mut value = state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())?;
    value.remember(path.clone());
    save_state(&state.data_file, &value)?;
    Ok(path)
}

#[tauri::command]
fn repair_project(path: PathBuf) -> Result<ProjectHealth, String> {
    project::repair_project(&path)
}

#[tauri::command]
fn recovery_snapshots(path: PathBuf) -> Result<Vec<RecoverySnapshotInfo>, String> {
    project::recovery_snapshots(&path)
}

#[tauri::command]
fn clone_project(
    repository: String,
    parent: PathBuf,
    folder_name: String,
    state: State<'_, ManagedHubState>,
) -> Result<PathBuf, String> {
    let path = project::clone_project(&repository, &parent, &folder_name)?;
    let health = project::inspect_project(&path);
    if health.descriptor.is_none() {
        return Err(health.errors.join("; "));
    }
    let mut value = state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())?;
    value.remember(path.clone());
    save_state(&state.data_file, &value)?;
    Ok(path)
}

#[tauri::command]
fn launch_editor(
    path: PathBuf,
    recovery_snapshot: Option<PathBuf>,
    state: State<'_, ManagedHubState>,
) -> Result<u32, String> {
    let installation = state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())?
        .selected_engine
        .as_ref()
        .and_then(|root| project::inspect_engine(root).ok());
    if let Some(selected) = installation.as_ref() {
        let health = project::inspect_project(&path);
        project::validate_project_engine_version(&health, selected)?;
    }
    let editor = installation
        .as_ref()
        .map(|selected| selected.editor.as_path());
    project::launch_editor_with(&path, recovery_snapshot.as_deref(), editor).map(|child| child.id())
}

#[tauri::command]
fn launch_player(path: PathBuf, state: State<'_, ManagedHubState>) -> Result<u32, String> {
    let root = state
        .value
        .lock()
        .map_err(|_| "KairoHub state lock was poisoned".to_string())?
        .selected_engine
        .clone()
        .ok_or_else(|| "Select a Kairo engine installation before running a project".to_string())?;
    let installation = project::inspect_engine(&root)?;
    project::launch_player_with(&path, &installation).map(|child| child.id())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_file = app.path().app_data_dir()?.join("hub-state.json");
            let mut state: HubState = fs::read(&data_file)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            register_discovered_engine(&mut state);
            save_state(&data_file, &state)?;
            app.manage(ManagedHubState {
                data_file,
                value: Mutex::new(state),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            hub_state,
            inspect_project,
            remember_project,
            set_favorite,
            engine_installations,
            register_engine,
            select_engine,
            create_project,
            repair_project,
            recovery_snapshots,
            clone_project,
            launch_editor,
            launch_player
        ])
        .run(tauri::generate_context!())
        .expect("KairoHub runtime failed");
}
