# KairoHub

KairoHub is the Tauri 2 project manager and process launcher for Kairo Game Engine. It validates projects before launch, creates canonical starter projects atomically, imports an existing local Kairo `.kproject`, clones a strict HTTPS GitHub/GitLab repository containing exactly one valid Kairo project, tracks recent and favorite projects, selects a registered engine installation, packages descriptor-defined build profiles, and starts that installation's KairoEditor or KairoPlayer. Editor recovery remains an explicit separate launch mode.

"Import" means an existing **Kairo project**. KairoHub does not migrate Unity,
Unreal, Godot, or arbitrary game executables/projects into Kairo. Running an
imported Kairo project requires a selected installation with a built
`KairoProjectCompiler` and `KairoPlayer`; Hub compiles attached logic before
starting Player.

## Development

```bash
npm install
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

Engine discovery validates a KairoGameEngine root, then resolves built tools
from the current sibling-superbuild layout first:

1. `build/dev-clang/components/KairoEditor/KairoEditorApp`
2. `build/dev/components/KairoEditor/KairoEditorApp`
3. `build/release/components/KairoEditor/KairoEditorApp`
4. legacy pre-sibling build locations only as compatibility fallbacks

`KairoProjectCompiler` follows the same `components/KairoEditor` layout;
`KairoPlayer` remains under `build/<preset>/Runtime/KairoPlayer`.

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
- Discovers KairoEditor, KairoProjectCompiler, and KairoPlayer from one selected engine build.
- Runs `<compiler> <project.kproject>` to publish source-bound attached logic, then spawns `<player> <project.kproject>` only when compilation succeeds; no command is routed through a shell.
- Lists build profiles from the validated project descriptor and runs `<player> <project.kproject> --package <profile>` only after the same compiler gate succeeds. Replacement is explicit, and success is reported only after Hub verifies the published package manifest.
- Imports a local `.kproject` only after its descriptor and required bootstrap files validate.
- Clones only strict HTTPS `github.com`/`gitlab.com` `.git` URLs without a shell, does not follow repository symlinks while discovering projects, deletes an invalid clone, and accepts exactly one validated `.kproject`.
- Repairs only missing bootstrap manifest/scene files after the descriptor itself parses successfully.

Platform signing, shared-library deployment, and deeper dependency repair remain
future release-engineering work; they are not represented as inert UI controls.


## External glTF / GLB import

KairoHub now distinguishes **Clone Kairo** from **Import glTF repo**.

`Clone Kairo` requires the repository to already contain exactly one valid
`.kproject`. It is not an engine-conversion feature.

`Import glTF repo` accepts an HTTPS GitHub/GitLab repository containing
portable glTF/GLB scene content. If the repository contains exactly one
`.gltf`/`.glb`, Hub selects it automatically; otherwise the user must provide
the project-relative entry scene. Hub generates `KairoImported.kproject` plus
bounded `.kairo` bootstrap files and then uses the same Kairo project
validation/compiler/player path as native projects.

This v1 importer preserves a truthful boundary: it imports portable scene
content, not arbitrary source gameplay from another engine. Unity, Godot and
Unreal projects require dedicated semantic importers before they can be claimed
as supported.
