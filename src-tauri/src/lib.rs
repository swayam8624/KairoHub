mod project;

use project::{HubState, ProjectHealth};
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
    let json = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&temporary, json).map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
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
fn launch_editor(path: PathBuf, recovery_mode: bool) -> Result<u32, String> {
    project::launch_editor(&path, recovery_mode).map(|child| child.id())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_file = app.path().app_data_dir()?.join("hub-state.json");
            let state = fs::read(&data_file)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
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
            create_project,
            launch_editor
        ])
        .run(tauri::generate_context!())
        .expect("KairoHub runtime failed");
}
