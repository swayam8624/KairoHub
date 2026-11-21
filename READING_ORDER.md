# KairoHub: source reading order

Start with Rust/Tauri host initialization and the web UI, then project descriptor validation, project creation/import/launch, and external source-game inspection. The Hub invokes KairoProjectCompiler, KairoEditor and KairoPlayer as child processes; it does not execute Unity/Unreal/Godot gameplay inside Kairo.

## Key APIs and interactions

Tauri commands bridge frontend TypeScript to Rust host operations. Trace project paths, refusal to execute arbitrary imported binaries, safe clone URLs, explicit snapshot recovery, and installed engine selection; never conflate migration diagnostics with full foreign-engine compatibility.

## Linked source files (11)

1. [`src-tauri/src/main.rs`](src-tauri/src/main.rs) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
2. [`src-tauri/src/lib.rs`](src-tauri/src/lib.rs) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
3. [`src-tauri/src/project.rs`](src-tauri/src/project.rs) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
4. [`src-tauri/src/source_game.rs`](src-tauri/src/source_game.rs) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
5. [`src-tauri/src/source_game_graph.rs`](src-tauri/src/source_game_graph.rs) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
6. [`src/main.ts`](src/main.ts) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
7. [`src-tauri/src/bin/kairo-hub-import.rs`](src-tauri/src/bin/kairo-hub-import.rs) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
8. [`src-tauri/build.rs`](src-tauri/build.rs) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
9. [`src-tauri/src/bin/kairo-hub-inspect-game.rs`](src-tauri/src/bin/kairo-hub-inspect-game.rs) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
10. [`src/vite-env.d.ts`](src/vite-env.d.ts) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.
11. [`vite.config.ts`](vite.config.ts) — read public contracts, lifetime ownership, imports/callers, error behavior and associated tests.

For setup consult [README.md](README.md), and for downstream consumers inspect [KairoGameEngine](https://github.com/swayam8624/KairoGameEngine). Original code is [MIT licensed](LICENSE) for study, modification and further development; framework binaries/SDKs retain their original licenses. Function-level commentary and full native tests are separate ongoing verification tasks.
