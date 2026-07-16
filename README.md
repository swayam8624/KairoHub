# KairoHub

KairoHub is the Tauri 2 project manager and process launcher for Kairo Game Engine. It validates projects before launch, creates canonical starter projects atomically, tracks recent and favorite projects, and can start KairoEditor in normal or recovery mode.

## Development

```bash
npm install
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

KairoEditor discovery uses the first executable found in:

1. `KAIRO_EDITOR_EXECUTABLE`
2. `$KAIRO_ENGINE_ROOT/KairoEditor/build/KairoEditorApp`
3. `KairoEditor/build/KairoEditorApp` relative to the launch directory

Recovery mode validates what it can, disables persisted editor docking state, and still opens a damaged project descriptor for repair only when KairoEditor can parse it. It does not modify project files.

## Current Contract

- Reads the existing `kairo-project 1` descriptor without inventing incompatible fields.
- Requires referenced asset manifest and startup scene files for normal launch.
- Writes new project files through same-directory temporary files.
- Stores Hub recents/favorites in the operating system application-data directory, never in the project.
- Spawns KairoEditor as a child process through the documented `--project` and `--no-layout-persistence` CLI.

Engine-version selection, dependency repair, Git clone, build profiles, and session recovery will extend this domain layer; they are not represented as inert UI controls.
