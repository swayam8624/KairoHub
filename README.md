# KairoHub

KairoHub is the Tauri 2 project manager and process launcher for Kairo Game Engine. It validates projects before launch, creates canonical starter projects atomically, tracks recent and favorite projects, selects a registered engine installation, and starts that installation's KairoEditor in normal or explicit snapshot-recovery mode.

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

Recovery lists project-owned `.kairo/recovery/snapshot-*` journals newest first,
stream-validates bounded payload sizes and checksums, and reports damaged entries
without enabling them. An explicitly selected valid snapshot launches the editor
with `--recovery-snapshot`; the editor backs up current targets, restores the
canonical project state, disables persisted docking, and rehydrates raw text
drafts. Recovery is never selected automatically. Missing manifest or startup
scene files can be recreated explicitly through Repair; existing files are never
overwritten by Repair.

## Current Contract

- Reads the existing `kairo-project 1` descriptor without inventing incompatible fields.
- Requires referenced asset manifest and startup scene files for normal launch.
- Writes new project files through same-directory temporary files.
- Stores Hub recents/favorites in the operating system application-data directory, never in the project.
- Spawns KairoEditor as a child process through the documented `--project` and `--no-layout-persistence` CLI.
- Clones shallow HTTPS GitHub/GitLab repositories without invoking a shell and imports a single discovered `.kproject`.
- Repairs only missing bootstrap manifest/scene files after the descriptor itself parses successfully.

Build-and-run profiles and deeper dependency repair remain future domain work;
they are not represented as inert UI controls.
