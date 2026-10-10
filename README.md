# KairoHub

KairoHub is the Tauri 2 project manager and process launcher for Kairo Game Engine. It validates projects before launch, creates canonical starter projects atomically, imports an existing local Kairo `.kproject`, clones a strict HTTPS GitHub/GitLab repository containing exactly one valid Kairo project, tracks recent and favorite projects, selects a registered engine installation, packages descriptor-defined build profiles, and starts that installation's KairoEditor or KairoPlayer. Editor recovery remains an explicit separate launch mode.

A local `.kproject` is a native Kairo project and launches through
`KairoProjectCompiler` and `KairoPlayer`. **Add existing game** separately
links a Godot, Unity, or Unreal source folder without copying or changing its
original authored files. The Hub identifies the original engine and produces
a source-level compatibility report. Unity/Unreal inspection is currently
analysis-only, not gameplay execution or conversion.

For Godot source projects with a valid `res://` startup scene, **Run original**
uses a locally installed Godot executable (set `KAIRO_GODOT_EXECUTABLE` if
needed). This preserves original gameplay because it starts Godot, NOT because
Kairo has reconstructed its GDScript/2D runtime. **Original editor** similarly
opens the source folder in Godot. No executable extracted from an imported
directory is launched automatically.

Where a source project contains exactly one portable glTF/GLB scene,
**Convert 3D scene** can create a native `KairoImported.kproject` with scene,
camera, lighting, assets and input. It writes new Kairo-generated files next
to the source (`KairoImported.kproject` and `.kairo/`); it refuses to overwrite
existing generated files. It does **not** convert foreign scenes, C#/GDScript,
Unreal Blueprints, physics logic, menus, collisions or commercial executable
binaries. A 2D Godot game without glTF/GLB cannot yet become native Kairo
gameplay; inspection reports that explicitly.

## First launch: game selection and one-click Run

KairoHub is the **entry application**. Starting the installed KairoHub desktop
app opens a project/game chooser rather than starting an empty editor.
The Hub checks direct directories near its executable/current working directory
for a validated KairoGameEngine checkout and honors `KAIRO_ENGINE_ROOT`.
It does not recursively search arbitrary private directories.

When a built KairoGameEngine checkout is discoverable and no projects are
registered yet, Hub offers the real
`Samples/PlayableVerticalSlice/Project.kproject` as a playable starting point.
The **Run game** button is available only when the selected engine includes
working KairoProjectCompiler and KairoPlayer binaries. Otherwise the project
row explains what is missing, and the user can choose a built installation
with the engine selector at the bottom left. **Edit project** opens authoring,
while **Run game** plays without ever opening the editor.

The Hub desktop package is a launcher, **not** a self-contained Kairo engine:
the build of KairoGameEngine still needs to be installed or bundled separately.
Linking a Godot/Unity/Unreal source folder is supported in the separate
**Games from other engines** section with explicit original-engine and
conversion boundaries.

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


## Reverse-game intake and migration diagnostics

Choose **Add existing game** in the Hub and select the directory containing
`project.godot`, Unity's `ProjectSettings/ProjectVersion.txt` + `Assets`,
or a single Unreal `.uproject` file. The game appears in a separate catalogue
so an external project can never accidentally be launched by KairoPlayer.

The inspector parses source-owned file metadata, caps recursion and entry
counts, rejects symbolic-link roots, doesn't follow symbolic links while
scanning, and never executes imported scripts. For Godot, the source scene
graph parser exposes node types, instanced scenes, external resources, script
references and signal connections. Unsupported behaviour is recorded, not
guessed or silently discarded.

```sh
# From the KairoHub checkout:
cargo run --manifest-path src-tauri/Cargo.toml \
  --bin kairo-hub-inspect-game -- /path/to/godot-game

# Show the normalized source scene structure:
cargo run --manifest-path src-tauri/Cargo.toml \
  --bin kairo-hub-inspect-game -- /path/to/godot-game --graph
```

**Real-world regression:** KairoHub CI retrieves the upstream official Godot
`2d/dodge_the_creeps` source from commit
`b761b4cd5718a7118f2c95eb55f181d0a5f004f1` (MIT-licensed source;
third-party art/audio/font licenses documented in the game's README), checks
the project entry scene, and validates actual source nodes, scripts, resources
and signal connections. That is an intake/information-preservation test, not
a claim that the original game was played through Kairo's renderer.

### Long-term engine reconstruction gates

1. **Inspection:** detect source engine, entry scenes, scripts, assets and
   incompatible formats with a non-destructive preview.
2. **Canonical IR:** preserve object identity, scene hierarchy, 2D/3D data,
   signals, input, animation, physics, audio, materials and script bindings.
3. **Semantic import:** translate only mappings with tested equivalent
   behaviour; mark each unresolved script/API/asset explicitly.
4. **Reference replay:** run the permitted original game and Kairo version
   against matching input traces, compare state and rendered evidence, and
   surface differences before claiming a playable port.
5. **Novice Hub:** choose Game → Run without ever opening an IDE; choose Edit
   for authoring; explain exactly which tool/asset is missing when launch fails.

Closed-source commercial releases such as FIFA or Spider-Man cannot be
generally recreated from an installed executable by a single importer. The
tool must only process content the user has the rights and technical means to
access, and no imported third-party content is included in Kairo releases.
