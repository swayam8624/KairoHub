# KairoHub — from a desktop click to a validated project and launched engine

This is a worked companion to the [Hub source-reading order](SOURCE_READING_ORDER.md). KairoHub is a Tauri desktop launcher. It is *not* the authority for game-engine scene semantics, and a project folder appearing in its UI does not authorize arbitrary code execution.

## 1. The two halves of the application

The frontend lives in [`src/main.ts`](../src/main.ts), with build configuration in [`package.json`](../package.json) and [`vite.config.ts`](../vite.config.ts). It displays the project list, favorites, installed engines, import/repair actions and user choices.

[`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs) registers Rust commands that the TypeScript UI can invoke through Tauri. The Rust side, rather than untrusted JavaScript state or a filename displayed in a dialog, validates filesystem paths and creates child processes. `ManagedHubState` uses a Mutex for concurrent backend commands and persists a user-oriented HubState snapshot.

The UI must present the action, while Rust determines whether the requested project and engine installation satisfy policy. Returning an error to the UI is not permission for a fallback shell launch.

## 2. The .kproject parser, source ownership and limits

[`src-tauri/src/project.rs`](../src-tauri/src/project.rs) parses the same portable schema described by [KairoEngineCore/ProjectDescriptor.cppm](https://github.com/swayam8624/KairoEngineCore/blob/docs/teaching-walkthrough-20261121/ProjectDescriptor.cppm). It reads a bounded descriptor, checks version/field names, validates relative paths, and produces `ProjectHealth` diagnostics.

A project descriptor identifies its asset manifest, startup scene and input map; those are persistent *authored documents*, not executable files to invoke automatically. The Hub also checks required project files and reports missing or incompatible resources before import.

The parser uses a 1 MiB descriptor cap. Recovery manifests, payloads and file counts have separately declared bounds; they are different trust surfaces with different allowed sizes.

**Exercise:** point Hub at a malformed descriptor, a directory with two plausible `.kproject` files, and a relative path containing traversal. Observe whether each action fails before any native Editor/Player process is spawned.

## 3. Import, repair and recovery are separate operations

`inspect_project` reads and reports health without mutating or launching the project. `import_project` requires a healthy descriptor; import-by-directory discovery should reject ambiguity instead of choosing an arbitrary first match.

`repair_project` and recovery-snapshot routines inspect and reassemble known project files under a validated root. A recovery list's checksum must match actual bytes; file names must not escape the selected project. This is not a global filesystem backup or a guarantee of recovery after arbitrary process termination.

[`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs) writes Hub's own preferences via a temporary state file and rename/backup scheme. That state file is separate from the engine's `.kproject` or `.kscene` documents.

## 4. Native Editor and Player launch boundaries

`inspect_engine` validates a chosen Kairo installation and expected binaries. `launch_editor_with` validates the project and optional recovery snapshot, then starts a selected Editor executable with explicit arguments. `launch_player_with` validates project/engine compatibility and invokes the native Player path. A configured Play executable is not to be interpreted as arbitrary host commands without verification.

The Editor owns authored state and KairoRenderer viewport resources; Player owns its runtime bridges and physics simulation. Hub is a *composition/launch* tool, not the renderer or solver.

**Exercise:** use a deliberately nonexistent engine installation and an incompatible project version. Confirm that launch fails with diagnostics rather than silently selecting some other executable or running a shell command.

## 5. Importing foreign-engine source is not game translation

[`src-tauri/src/source_game.rs`](../src-tauri/src/source_game.rs) inspects source-controlled projects authored in other engines, recording an original-engine identity and project structure. [`src-tauri/src/source_game_graph.rs`](../src-tauri/src/source_game_graph.rs) contains read-only scene inspection for supported sources.

Recognizing Godot project assets and metadata does **not** make the game natively playable in KairoPlayer. A separate 'run in original engine' adapter can launch an explicitly selected compatible runtime; supported glTF/GLB reconstruction can create a limited Kairo-native scene asset. It is not automatic transfer of arbitrary GDScript gameplay, animations, physics rules or third-party rights.

The difference matters for the Desktop Pets game and other future imports: asset reuse is not complete engine compatibility.

## 6. Tauri and process security

[`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json) and the capabilities/configuration define which desktop resources are exposed. Typed command input, path canonicalization and preflight validation should apply to every operation that reads files, creates project directories or launches processes.

The application should not use a natural-language string or model-proposed tool call as shell authority. If KairoAI is integrated, its [exact-call authorization policy](https://github.com/swayam8624/KairoAI/blob/docs/teaching-walkthrough-20261121/docs/AI_PROPOSAL_TO_APPROVED_ACTION.md) must remain host-enforced.

## 7. Verification and tests

Start with Rust unit tests under `src-tauri` and run the appropriate Tauri frontend checks. Record which tests only exercise string parsing and which require native OS dialogs, process launch and actual Editor/Player binaries.

For each workflow capture the exact project path policy, installation version, process exit/diagnostic result, and user-visible error. A successful Tauri build is not proof of verified external-source game translation or signed release packaging.

*Evidence note: the walkthrough is based on source review; no new native macOS/Windows installer, external engine, or end-to-end UI test result is claimed.*
