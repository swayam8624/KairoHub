# Teaching source map — KairoHub

> A guided source-reading itinerary. It is not a test result or claim that every file is documented yet.

## How to use this guide

KairoHub is a Tauri desktop project manager. Read the TypeScript UI and Rust commands as a permission boundary: selecting a project is not equivalent to executing arbitrary source projects.

Read each phase in order. Every link goes to a source file or directory in this repository. For a directory, inspect its public interface and then its implementation files. After each phase, identify the caller, the state it owns, the inputs it validates, and its observable outputs.

## 1. User experience and build

1. [`README.md`](../README.md)
2. [`package.json`](../package.json)
3. [`vite.config.ts`](../vite.config.ts)
4. [`src`](../src)
5. [`src-tauri/Cargo.toml`](../src-tauri/Cargo.toml)

## 2. Rust command boundary

1. [`src-tauri/src`](../src-tauri/src)
2. [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json)

## 3. Validation and test provenance

1. [`src-tauri`](../src-tauri)
2. [`README.md`](../README.md)

## End-to-end interaction map

The frontend asks Rust commands to discover or create project descriptors; the Rust backend validates paths, project manifests and configured engine installs, then launches an explicit KairoEditor/Player binary. Imported Unity/Unreal games are inspected, not transparently converted into native Kairo execution.

## APIs and code-reading questions

For each function, distinguish **contract** (preconditions, postconditions, failure modes), **implementation** (state/algorithms), **collaborators** (imports, calls and consumers), **lifetime/ownership**, and **verification** (test or evidence that checks it). For C++ modules, `export module` names the public compiled interface and `import` expresses module dependencies; an implementation unit's `module ...;` contributes definitions without exporting them.

## Boundaries and honesty

Native game conversion, foreign scripting and full release packaging are not claimed. Check subprocess and clone validation paths before extending external project support.

## Documentation rules

- Teaching comments describe *why*, invariants, edge cases, API ownership and cross-file effects; avoid narrating obvious assignments.
- Preserve behavior during documentation-only edits; keep correctness changes in separate reviewed commits.
- Link function names and tests where possible; mark claims as source-present, CI-observed, host-verified or research-evidenced.
- License grants apply only to material for which the repository owner holds rights; third-party source, models, media and datasets keep their own notices.
