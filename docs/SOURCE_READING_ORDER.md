# KairoHub — linked teaching source order

Read [README](../README.md) first for platform setup and supported capabilities. This is a **conceptual study sequence**; compile/launch order is defined by CMake (Editor) or Cargo/Tauri/Vite (Hub).

1. [src/main.ts](../src/main.ts) — trace its inputs, ownership and next consumer.
2. [src-tauri/src/main.rs](../src-tauri/src/main.rs) — trace its inputs, ownership and next consumer.
3. [src-tauri/src/lib.rs](../src-tauri/src/lib.rs) — trace its inputs, ownership and next consumer.
4. [src-tauri/src/project.rs](../src-tauri/src/project.rs) — trace its inputs, ownership and next consumer.
5. [src-tauri/src/source_game.rs](../src-tauri/src/source_game.rs) — trace its inputs, ownership and next consumer.
6. [src-tauri/src/source_game_graph.rs](../src-tauri/src/source_game_graph.rs) — trace its inputs, ownership and next consumer.
7. [src-tauri/src/bin/kairo-hub-import.rs](../src-tauri/src/bin/kairo-hub-import.rs) — trace its inputs, ownership and next consumer.
8. [src-tauri/src/bin/kairo-hub-inspect-game.rs](../src-tauri/src/bin/kairo-hub-inspect-game.rs) — trace its inputs, ownership and next consumer.
9. [src-tauri/capabilities/default.json](../src-tauri/capabilities/default.json) — trace its inputs, ownership and next consumer.
10. [src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json) — trace its inputs, ownership and next consumer.

## Integration flow

Tauri web UI in src/main.ts → invoke(...) typed host commands → Rust lib.rs command handlers/state → project.rs validation, installation discovery and process launching → KairoEditor or KairoPlayer child process. source_game/source_game_graph inspect external game inputs and do not guarantee complete engine conversion.

## API concepts and questions

For each file, identify state ownership, valid input formats, resource lifetimes, validation and failure paths, and which system consumes the result. Follow import statements and `invoke` command names to their actual definitions, rather than guessing at behavior. Do not confuse editor documents, source-game inspection, runtime scene objects and persistent asset references.

## Practice

1. Follow a project path from user interaction to validated descriptor and either authoring or runtime.
2. Trace one user action to an undoable document mutation, or, for Hub, an invocation into Rust.
3. Find an invalid path/security boundary and explain how it rejects escape or malformed data.
4. Differentiate durable editor/project state from process-local renderer handles and transient UI state.
5. Run applicable unit/integration tests and record which functionality is actually implemented.

Original project code/docs are proposed to be MIT-licensed under [LICENSE](../LICENSE); independently licensed dependencies, icons, sample content and third-party tools retain their own terms.
