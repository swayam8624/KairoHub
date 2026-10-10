import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import "./styles.css";

type ProjectDescriptor = {
  name: string;
  assetManifest: string;
  startupScene: string;
  engineVersion: string;
  inputMap: string;
  renderingProfile: string;
  graphicsBackend: string;
  playExecutable: string | null;
  enabledPlugins: string[];
  buildProfiles: ProjectBuildProfile[];
};

type ProjectBuildProfile = {
  name: string;
  kind: "development" | "release";
  outputDirectory: string;
};

type PackageArtifact = {
  profileName: string;
  profileKind: string;
  outputDirectory: string;
  manifestPath: string;
};

type ProjectHealth = {
  descriptorPath: string;
  descriptor: ProjectDescriptor | null;
  errors: string[];
  warnings: string[];
};

type HubState = {
  recentProjects: string[];
  sourceGames: string[];
  godotExecutable: string | null;
  favorites: string[];
  engineRoots: string[];
  selectedEngine: string | null;
};

type EngineInstallation = {
  root: string;
  version: string;
  editor: string;
  editorAvailable: boolean;
  projectCompiler: string;
  projectCompilerAvailable: boolean;
  player: string;
  playerAvailable: boolean;
};

type SourceGameGraph = {
  sourceScene: string;
  nodes: { name: string; parent: string; nodeType: string; instanceResource: string | null; scriptResource: string | null }[];
  resources: { id: string; resourceType: string; path: string }[];
  signals: { signal: string; from: string; to: string; method: string }[];
  unsupportedBehaviors: string[];
};

type GameMigrationIssue = {
  code: string;
  severity: "blocker" | "error" | "warning";
  source: string;
  explanation: string;
  suggestedAction: string;
};

type GodotMigrationPlan = {
  schema: string;
  engine: string;
  title: string;
  mainScene: string;
  sourceObjects: {sourcePath: string; sourceType: string; authoredScript: string | null; packedScene: string | null}[];
  sourceAssets: {resourceId: string; resourceType: string; sourcePath: string; exists: boolean}[];
  signalCount: number;
  nativeGameplayReady: boolean;
  originalRuntimeAvailable: boolean;
  issues: GameMigrationIssue[];
};

type SourceGameInspection = {
  root: string;
  engine: "Godot" | "Unity" | "Unreal";
  displayName: string;
  mainScene: string | null;
  sceneFiles: number;
  scriptFiles: number;
  mediaFiles: number;
  scannedFiles: number;
  nativeSceneCandidate: string | null;
  nativeSceneCandidates: string[];
  originalRuntimeAvailable: boolean;
  notes: string[];
};

type RecoverySnapshotInfo = {
  directory: string;
  createdUnixMilliseconds: number;
  activeScene: string;
  fileCount: number;
  dirtyFileCount: number;
  textDraftCount: number;
  errors: string[];
};

const app = document.querySelector<HTMLDivElement>("#app");
if (!app) throw new Error("KairoHub root element is missing");

app.innerHTML = `
  <div class="shell">
    <aside class="sidebar">
      <div class="brand"><span class="brand-mark">K</span><span>KairoHub</span></div>
      <nav aria-label="Hub navigation">
        <button class="nav-item active" type="button"><span>⌂</span> Projects</button>
      </nav>
      <button id="engine-button" class="engine-status" type="button"><span id="engine-dot" class="status-dot unavailable"></span><div><strong id="engine-name">Engine not selected</strong><small id="engine-detail">Choose an installation</small></div></button>
    </aside>
    <main>
      <header class="topbar">
        <div><p class="eyebrow">Project workspace</p><h1>Your projects</h1></div>
        <div class="header-actions">
          <button id="clone-button" class="button secondary" type="button">Clone Kairo</button>
          <button id="external-import-button" class="button secondary" type="button">Import glTF repo</button>
          <button id="source-game-button" class="button secondary" type="button">Add existing game</button>
          <button id="import-folder-button" class="button secondary" type="button">Import folder</button>
          <button id="import-button" class="button secondary" type="button">Import .kproject</button>
          <button id="create-button" class="button primary" type="button">New project</button>
        </div>
      </header>
      <section class="summary" aria-label="Project summary">
        <div><span id="project-count">0</span><small>Recent projects</small></div>
        <div><span id="healthy-count">0</span><small>Ready to open</small></div>
        <div><span id="attention-count">0</span><small>Need attention</small></div>
      </section>
      <section class="project-section">
        <div class="section-heading"><h2>Kairo projects</h2><input id="project-filter" type="search" placeholder="Filter games" aria-label="Filter games" /></div>
        <div id="project-list" class="project-list" aria-live="polite"></div>
      </section>
      <section class="project-section source-section" aria-label="Linked original games">
        <div class="section-heading"><h2>Games from other engines</h2><button id="choose-godot" class="button secondary" type="button">Choose Godot executable</button></div>
        <div id="source-game-list" class="project-list" aria-live="polite"></div>
      </section>
    </main>
  </div>
  <dialog id="import-dialog">
    <form method="dialog" class="dialog-form">
      <div><p class="eyebrow">Existing project</p><h2>Import a Kairo project</h2></div>
      <label>Descriptor path<span class="path-field"><input id="import-path" required placeholder="/path/to/Game.kproject" /><button id="browse-project" class="button secondary" type="button">Browse</button></span></label>
      <p class="field-note">The descriptor and referenced startup files are validated before import.</p>
      <div class="dialog-actions"><button value="cancel" class="button secondary">Cancel</button><button id="confirm-import" value="default" class="button primary">Import</button></div>
    </form>
  </dialog>
  <dialog id="create-dialog">
    <form method="dialog" class="dialog-form">
      <div><p class="eyebrow">Blank template</p><h2>Create a Kairo project</h2></div>
      <label>Project name<input id="display-name" required placeholder="Skybound" /></label>
      <label>Folder name<input id="folder-name" required pattern="[A-Za-z0-9_-]+" placeholder="Skybound" /></label>
      <label>Parent directory<span class="path-field"><input id="parent-path" required placeholder="/Users/name/Projects" /><button id="browse-parent" class="button secondary" type="button">Browse</button></span></label>
      <div class="dialog-actions"><button value="cancel" class="button secondary">Cancel</button><button id="confirm-create" value="default" class="button primary">Create project</button></div>
    </form>
  </dialog>
  <dialog id="clone-dialog">
    <form method="dialog" class="dialog-form">
      <div><p class="eyebrow">Git repository</p><h2>Clone a Kairo project</h2></div>
      <label>HTTPS repository URL<input id="clone-url" required placeholder="https://github.com/owner/game.git" /></label>
      <label>Folder name<input id="clone-folder" required pattern="[A-Za-z0-9_.-]+" placeholder="game" /></label>
      <label>Parent directory<span class="path-field"><input id="clone-parent" required placeholder="/Users/name/Projects" /><button id="browse-clone-parent" class="button secondary" type="button">Browse</button></span></label>
      <div class="dialog-actions"><button value="cancel" class="button secondary">Cancel</button><button id="confirm-clone" value="default" class="button primary">Clone project</button></div>
    </form>
  </dialog>
  <dialog id="external-import-dialog">
    <form method="dialog" class="dialog-form">
      <div><p class="eyebrow">External repository</p><h2>Import a glTF / GLB project</h2></div>
      <label>HTTPS repository URL<input id="external-import-url" required placeholder="https://github.com/owner/demo.git" /></label>
      <label>Folder name<input id="external-import-folder" required pattern="[A-Za-z0-9_.-]+" placeholder="demo" /></label>
      <label>Entry scene <input id="external-import-scene" placeholder="Optional: path/to/world.glb" /></label>
      <p class="field-note">If the repository contains exactly one .gltf/.glb file, Kairo detects it automatically. Multiple scenes require an explicit relative path. This imports portable scene content; Unity/Godot/Unreal gameplay code is not converted.</p>
      <label>Parent directory<span class="path-field"><input id="external-import-parent" required placeholder="/Users/name/Projects" /><button id="browse-external-parent" class="button secondary" type="button">Browse</button></span></label>
      <div class="dialog-actions"><button value="cancel" class="button secondary">Cancel</button><button id="confirm-external-import" value="default" class="button primary">Clone and convert</button></div>
    </form>
  </dialog>
  <dialog id="reconstruct-dialog">
    <form method="dialog" class="dialog-form">
      <div><p class="eyebrow">Native asset migration</p><h2>Choose a 3D scene</h2></div>
      <label>Scene to import<select id="reconstruct-scene" required></select></label>
      <p class="field-note">Creates a new editable Kairo scene and project descriptor. Scripts, collisions, menus, and gameplay are not translated. The source files remain untouched.</p>
      <div class="dialog-actions">
        <button value="cancel" class="button secondary">Cancel</button>
        <button id="confirm-reconstruct" type="button" class="button primary">Create Kairo scene</button>
      </div>
    </form>
  </dialog>
  <dialog id="game-inspection-dialog" class="wide-dialog">
    <div class="dialog-form">
      <div><p class="eyebrow">Compatibility report</p><h2 id="game-inspection-title">Game analysis</h2></div>
      <div id="game-inspection-details" class="game-inspection-details"></div>
      <p class="field-note">Run original is not a Kairo-native conversion. Native scene import preserves only portable 3D content, not other engines’ gameplay scripts.</p>
      <div class="dialog-actions"><button id="close-game-inspection" class="button primary" type="button">Done</button></div>
    </div>
  </dialog>
  <dialog id="engine-dialog">
    <div class="dialog-form">
      <div><p class="eyebrow">Toolchain</p><h2>Kairo installations</h2></div>
      <div id="engine-list" class="engine-list"></div>
      <div class="dialog-actions"><button id="add-engine" class="button secondary" type="button">Add installation</button><button id="close-engine" class="button primary" type="button">Done</button></div>
    </div>
  </dialog>
  <dialog id="recovery-dialog" class="wide-dialog">
    <div class="dialog-form">
      <div><p class="eyebrow">Project recovery</p><h2>Choose a recovery point</h2></div>
      <p class="field-note">Kairo validates every payload before launch. Restoring creates a backup of current project files, then opens the saved scene, tabs, and text drafts.</p>
      <div id="recovery-list" class="recovery-list" aria-live="polite"></div>
      <div class="dialog-actions"><button id="close-recovery" class="button secondary" type="button">Cancel</button></div>
    </div>
  </dialog>
  <dialog id="package-dialog">
    <form method="dialog" class="dialog-form">
      <div><p class="eyebrow">Runtime artifact</p><h2>Package project</h2></div>
      <label>Build profile<select id="package-profile" required></select></label>
      <p id="package-destination" class="field-note"></p>
      <label class="checkbox-row"><input id="replace-package" type="checkbox" /><span>Replace an existing artifact atomically</span></label>
      <p class="field-note">Kairo compiles attached logic first, then validates the relocated runtime project before publishing the bundle.</p>
      <div class="dialog-actions"><button value="cancel" class="button secondary">Cancel</button><button id="confirm-package" value="default" class="button primary">Package</button></div>
    </form>
  </dialog>
  <div id="toast" class="toast" role="status" aria-live="polite"></div>
`;

const projectList = document.querySelector<HTMLDivElement>("#project-list")!;
const sourceGameList = document.querySelector<HTMLDivElement>("#source-game-list")!;
const gameInspectionDialog = document.querySelector<HTMLDialogElement>("#game-inspection-dialog")!;
const reconstructDialog = document.querySelector<HTMLDialogElement>("#reconstruct-dialog")!;
const reconstructScene = document.querySelector<HTMLSelectElement>("#reconstruct-scene")!;
let reconstructRoot: string | null = null;
const projectFilter = document.querySelector<HTMLInputElement>("#project-filter")!;
const importDialog = document.querySelector<HTMLDialogElement>("#import-dialog")!;
const createDialog = document.querySelector<HTMLDialogElement>("#create-dialog")!;
const cloneDialog = document.querySelector<HTMLDialogElement>("#clone-dialog")!;
const externalImportDialog = document.querySelector<HTMLDialogElement>("#external-import-dialog")!;
const recoveryDialog = document.querySelector<HTMLDialogElement>("#recovery-dialog")!;
const recoveryList = document.querySelector<HTMLDivElement>("#recovery-list")!;
const packageDialog = document.querySelector<HTMLDialogElement>("#package-dialog")!;
const packageProfile = document.querySelector<HTMLSelectElement>("#package-profile")!;
const packageDestination = document.querySelector<HTMLParagraphElement>("#package-destination")!;
const toast = document.querySelector<HTMLDivElement>("#toast")!;
let state: HubState = { recentProjects: [], sourceGames: [], godotExecutable: null, favorites: [], engineRoots: [], selectedEngine: null };
let sourceGamesByRoot = new Map<string, SourceGameInspection>();
let sourceGameErrors = new Map<string, string>();
let healthByPath = new Map<string, ProjectHealth>();
let engines: EngineInstallation[] = [];
let recoveryProject = "";
let packageProject = "";

function escapeHtml(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;"
  })[character] ?? character);
}

function notify(message: string, error = false): void {
  toast.textContent = message;
  toast.classList.toggle("error", error);
  toast.classList.add("visible");
  window.setTimeout(() => toast.classList.remove("visible"), 3500);
}

function projectTitle(path: string, health: ProjectHealth): string {
  return health.descriptor?.name ?? path.split(/[\\/]/).at(-1)?.replace(/\.kproject$/, "") ?? "Unknown project";
}

function formatRecoveryTime(milliseconds: number): string {
  if (!Number.isSafeInteger(milliseconds) || milliseconds < 0) return "Unknown time";
  const date = new Date(milliseconds);
  if (Number.isNaN(date.getTime())) return "Unknown time";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium", timeStyle: "short"
  }).format(date);
}

function renderRecoverySnapshots(snapshots: RecoverySnapshotInfo[]): void {
  if (!snapshots.length) {
    recoveryList.innerHTML = `<div class="compact-empty">No recovery points exist for this project yet.</div>`;
    return;
  }
  recoveryList.innerHTML = snapshots.map((snapshot) => {
    const valid = snapshot.errors.length === 0;
    const details = valid
      ? `${snapshot.fileCount} files · ${snapshot.dirtyFileCount} dirty · ${snapshot.textDraftCount} drafts`
      : snapshot.errors[0];
    return `<article class="recovery-row ${valid ? "" : "invalid"}">
      <div class="recovery-state ${valid ? "valid" : "invalid"}">${valid ? "✓" : "!"}</div>
      <div><strong>${escapeHtml(formatRecoveryTime(snapshot.createdUnixMilliseconds))}</strong><small>${escapeHtml(snapshot.activeScene || "Unknown scene")}</small><p>${escapeHtml(details)}</p></div>
      <button class="button primary" type="button" data-recovery="${escapeHtml(snapshot.directory)}" ${valid ? "" : "disabled"}>Restore and open</button>
    </article>`;
  }).join("");
}

function updatePackageDestination(): void {
  const health = healthByPath.get(packageProject);
  const profile = health?.descriptor?.buildProfiles.find((candidate) => candidate.name === packageProfile.value);
  packageDestination.textContent = profile
    ? `${profile.kind === "release" ? "Release" : "Development"} bundle · ${profile.outputDirectory}`
    : "Select an authored build profile.";
}

function showPackage(path: string): void {
  const descriptor = healthByPath.get(path)?.descriptor;
  if (!descriptor?.buildProfiles.length) throw new Error("Project has no build profiles");
  packageProject = path;
  packageProfile.innerHTML = descriptor.buildProfiles.map((profile) =>
    `<option value="${escapeHtml(profile.name)}">${escapeHtml(profile.name)} · ${escapeHtml(profile.kind)}</option>`
  ).join("");
  document.querySelector<HTMLInputElement>("#replace-package")!.checked = false;
  updatePackageDestination();
  packageDialog.showModal();
}

async function showRecovery(path: string): Promise<void> {
  recoveryProject = path;
  recoveryList.innerHTML = `<div class="compact-empty">Validating recovery points…</div>`;
  recoveryDialog.showModal();
  const snapshots = await invoke<RecoverySnapshotInfo[]>("recovery_snapshots", { path });
  renderRecoverySnapshots(snapshots);
}

function renderProjects(): void {
  const query = projectFilter.value.trim().toLocaleLowerCase();
  const projects = state.recentProjects.filter((path) => {
    const health = healthByPath.get(path);
    return !query || path.toLocaleLowerCase().includes(query) || (health && projectTitle(path, health).toLocaleLowerCase().includes(query));
  });
  const healthy = [...healthByPath.values()].filter((health) => health.errors.length === 0).length;
  document.querySelector("#project-count")!.textContent = String(state.recentProjects.length);
  document.querySelector("#healthy-count")!.textContent = String(healthy);
  document.querySelector("#attention-count")!.textContent = String(healthByPath.size - healthy);
  if (projects.length === 0) {
    projectList.innerHTML = `<div class="empty-state"><div class="empty-icon">K</div><h3>${state.recentProjects.length ? "No matching projects" : "Create your first project"}</h3><p>${state.recentProjects.length ? "Try another project name or path." : "KairoHub will validate it and keep the launch history here."}</p></div>`;
    return;
  }
  projectList.innerHTML = projects.map((path) => {
    const health = healthByPath.get(path);
    if (!health) return `<article class="project-row loading"><div class="project-glyph">…</div><div><strong>Inspecting project</strong><small>${escapeHtml(path)}</small></div></article>`;
    const valid = health.errors.length === 0;
    const favorite = state.favorites.includes(path);
    const message = valid ? (health.warnings[0] ?? "Ready to open") : health.errors[0];
    return `<article class="project-row" data-path="${escapeHtml(path)}">
      <div class="project-glyph">${escapeHtml(projectTitle(path, health).slice(0, 1).toUpperCase())}</div>
      <div class="project-info"><div><strong>${escapeHtml(projectTitle(path, health))}</strong><span class="health ${valid ? "ready" : "warning"}">${valid ? "Ready" : "Repair"}</span></div><small>${escapeHtml(path)}</small><p>${escapeHtml(message)}</p></div>
      <button class="icon-button favorite ${favorite ? "selected" : ""}" type="button" data-action="favorite" aria-label="${favorite ? "Remove from favorites" : "Add to favorites"}">★</button>
      <button class="button secondary" type="button" data-action="${valid ? "recovery" : "repair"}">${valid ? "Recovery" : "Repair"}</button>
      <button class="button secondary" type="button" data-action="package" ${valid ? "" : "disabled"}>Package</button>
      <button class="button primary" type="button" data-action="run" ${valid ? "" : "disabled"}>▶ Run game</button>
      <button class="button secondary" type="button" data-action="open" ${valid ? "" : "disabled"}>Edit project</button>
    </article>`;
  }).join("");
}

function renderSourceGames(): void {
  const query = projectFilter.value.trim().toLocaleLowerCase();
  const roots = state.sourceGames.filter((root) => {
    const info = sourceGamesByRoot.get(root);
    return !query || root.toLocaleLowerCase().includes(query) ||
      (info?.displayName.toLocaleLowerCase().includes(query) ?? false);
  });
  if (!roots.length) {
    sourceGameList.innerHTML = '<div class="compact-empty">Add a local Godot, Unity or Unreal source-game folder. Your original files are not copied or modified.</div>';
    return;
  }
  sourceGameList.innerHTML = roots.map((root) => {
    const info = sourceGamesByRoot.get(root);
    const error = sourceGameErrors.get(root);
    if (!info) return '<div class="compact-empty">' + escapeHtml(error ?? 'Inspecting ' + root) + '</div>';
    const canPlay = info.engine === "Godot" && Boolean(info.mainScene);
    const canConvert = info.nativeSceneCandidates.length > 0;
    const note = info.notes[0] ?? 'Source project detected';
    const escapedRoot = escapeHtml(root);
    const disabledPlay = canPlay ? "" : 'disabled title="Only Godot games with a startup scene can run through the original engine"';
    const disabledConvert = canConvert ? '' : 'disabled title="No single portable 3D scene"';
    return '<article class="project-row source-row" data-source-root="' + escapedRoot + '">'
      + '<div class="project-glyph">' + escapeHtml(info.engine.slice(0, 1)) + '</div>'
      + '<div class="project-info"><div><strong>' + escapeHtml(info.displayName)
      + '</strong><span class="health warning">' + escapeHtml(info.engine) + ' source</span></div>'
      + '<small>' + escapedRoot + '</small><p>' + escapeHtml(note) + '</p></div>'
      + '<button class="button secondary" type="button" data-source-action="inspect">Inspect</button>'
      + '<button class="button secondary" type="button" data-source-action="convert" ' + disabledConvert + '>Import 3D scene</button>'
      + '<button class="button secondary" type="button" data-source-action="edit" ' + disabledPlay + '>Original editor</button>'
      + '<button class="button primary" type="button" data-source-action="play" ' + disabledPlay + '>▶ Play</button>'
      + '</article>';
  }).join('');
}

async function showSourceInspection(info: SourceGameInspection): Promise<void> {
  document.querySelector("#game-inspection-title")!.textContent = info.displayName;
  const properties = [
    ["Engine", info.engine],
    ["Startup scene", info.mainScene ?? "Not resolved"],
    ["Scene files", String(info.sceneFiles)],
    ["Script files", String(info.scriptFiles)],
    ["Media files", String(info.mediaFiles)],
    ["3D scenes available to import", String(info.nativeSceneCandidates.length)],
    ["Original engine executable", info.originalRuntimeAvailable ? "Available" : "Unavailable"],
  ];
  document.querySelector("#game-inspection-details")!.innerHTML = properties.map(([key, value]) =>
    '<div><strong>' + escapeHtml(key) + '</strong><span>' + escapeHtml(value) + '</span></div>'
  ).join('') + '<p>' + info.notes.map(escapeHtml).join(' · ') + '</p>';
  gameInspectionDialog.showModal();
  if (info.engine !== "Godot" || !info.mainScene) return;
  try {
    const graph = await invoke<SourceGameGraph>("inspect_source_game_graph", { root: info.root });
    const graphDetail = '<p><strong>Scene graph:</strong> ' + graph.nodes.length + ' nodes, '
      + graph.resources.length + ' external resources, ' + graph.signals.length + ' signal connections</p>'
      + '<p><strong>Node structure:</strong> ' + graph.nodes.slice(0, 16)
        .map((node) => escapeHtml(node.name + ' (' + node.nodeType + ')')).join(', ')
      + (graph.nodes.length > 16 ? ', ...' : '') + '</p>'
      + '<p><strong>Migration gaps:</strong> ' + graph.unsupportedBehaviors.map(escapeHtml).join(' · ') + '</p>';
    document.querySelector("#game-inspection-details")!.insertAdjacentHTML("beforeend", graphDetail);
  } catch (error) {
    document.querySelector("#game-inspection-details")!.insertAdjacentHTML("beforeend",
      '<p>Scene graph detail unavailable: ' + escapeHtml(String(error)) + '</p>');
  }
  try {
    const plan = await invoke<GodotMigrationPlan>("plan_source_game_migration", {root: info.root});
    const blocking = plan.issues.filter((entry) => entry.severity === "blocker").length;
    const missing = plan.sourceAssets.filter((asset) => !asset.exists).length;
    const sample = plan.issues.slice(0, 12);
    const items = sample.map((entry) =>
      '<li><strong>' + escapeHtml(entry.code.replace(/_/g, " ")) + '</strong> — ' +
      escapeHtml(entry.explanation) + '<small>' +
      escapeHtml(entry.source + " · " + entry.suggestedAction) + '</small></li>').join("");
    const heading = '<p><strong>Game reconstruction readiness:</strong> ' +
      plan.sourceObjects.length + ' scene objects, ' + plan.sourceAssets.length +
      ' referenced resources (' + missing + ' unavailable), ' + plan.signalCount +
      ' signals, ' + blocking + ' native-play blockers. ' +
      (plan.nativeGameplayReady ? "Native gameplay certified" :
        "Original gameplay only; a Kairo-native port is not yet verified.") + '</p>';
    document.querySelector("#game-inspection-details")!.insertAdjacentHTML("beforeend",
      heading + '<div class="migration-plan"><strong>What needs converting</strong><ol>' +
      items + '</ol>' +
      (plan.issues.length > sample.length ?
        '<p>+' + (plan.issues.length - sample.length) +
        ' additional migration findings. Export the complete JSON using the inspector CLI.</p>' : '') +
      '</div>');
  } catch (error) {
    document.querySelector("#game-inspection-details")!.insertAdjacentHTML("beforeend",
      '<p>Migration assessment unavailable: ' + escapeHtml(String(error)) + '</p>');
  }
}

async function refreshSourceGames(): Promise<void> {
  sourceGamesByRoot = new Map();
  sourceGameErrors = new Map();
  renderSourceGames();
  await Promise.all(state.sourceGames.map(async (root) => {
    try {
      const inspected = await invoke<SourceGameInspection>("inspect_source_game", { root });
      sourceGamesByRoot.set(root, inspected);
    } catch (error) { sourceGameErrors.set(root, String(error)); }
    renderSourceGames();
  }));
}

function renderEngines(): void {
  const selected = engines.find((engine) => engine.root === state.selectedEngine);
  document.querySelector("#engine-name")!.textContent = selected ? `Kairo ${selected.version}` : "Engine not selected";
  document.querySelector("#engine-detail")!.textContent = selected
    ? (selected.editorAvailable && selected.projectCompilerAvailable && selected.playerAvailable ? "Editor, compiler and player ready" : "Build incomplete") : "Choose an installation";
  document.querySelector("#engine-dot")!.classList.toggle("unavailable", !selected?.editorAvailable || !selected?.projectCompilerAvailable || !selected?.playerAvailable);
  const list = document.querySelector<HTMLDivElement>("#engine-list")!;
  list.innerHTML = engines.length ? engines.map((engine) => `<button class="engine-row ${engine.root === state.selectedEngine ? "selected" : ""}" data-engine="${escapeHtml(engine.root)}" type="button"><span><strong>Kairo ${escapeHtml(engine.version)}</strong><small>${escapeHtml(engine.root)}</small></span><em>${engine.editorAvailable && engine.projectCompilerAvailable && engine.playerAvailable ? "Ready" : "Build incomplete"}</em></button>`).join("")
    : `<div class="compact-empty">No Kairo installations registered.</div>`;
}

async function refreshHealth(): Promise<void> {
  healthByPath = new Map();
  renderProjects();
  await Promise.all(state.recentProjects.map(async (path) => {
    const health = await invoke<ProjectHealth>("inspect_project", { path });
    healthByPath.set(path, health);
    renderProjects();
  }));
}

// A novice should never need to open Settings before their first Play.
// Resolve the selected installation at the point of action and describe the
// exact missing tool instead of a generic failed-process notification.
async function ensureNativeTools(mode: "play" | "edit"): Promise<boolean> {
  let selected = engines.find((engine) => engine.root === state.selectedEngine);
  if (!selected) {
    const root = await open({ multiple: false, directory: true,
      title: "Locate your KairoGameEngine installation to " + mode });
    if (!root) return false;
    const installed = await invoke<EngineInstallation>("register_engine", { root });
    state = await invoke<HubState>("hub_state");
    engines = await invoke<EngineInstallation[]>("engine_installations");
    selected = installed;
    renderEngines();
  }
  if (mode === "play" && (!selected.projectCompilerAvailable || !selected.playerAvailable)) {
    throw new Error("This Kairo engine is not built for Play yet. Missing: " +
      [!selected.projectCompilerAvailable ? "Project Compiler" : "",
       !selected.playerAvailable ? "KairoPlayer" : ""].filter(Boolean).join(", ") +
      ". Build KairoGameEngine once, then return to Play.");
  }
  if (mode === "edit" && !selected.editorAvailable) {
    throw new Error("KairoEditor is not built in this installation. Build KairoGameEngine once, then choose Edit again.");
  }
  return true;
}

async function loadState(): Promise<void> {
  state = await invoke<HubState>("hub_state");
  engines = await invoke<EngineInstallation[]>("engine_installations");
  renderEngines();
  await Promise.all([refreshHealth(), refreshSourceGames()]);
}

document.querySelector("#import-button")!.addEventListener("click", () => importDialog.showModal());
document.querySelector("#import-folder-button")!.addEventListener("click", async () => {
  const root = await open({ multiple: false, directory: true });
  if (!root) return;
  try {
    const project = await invoke<string>("import_project_directory", { root });
    state = await invoke<HubState>("hub_state");
    await refreshHealth();
    notify(`Project imported from folder: ${project}`);
  } catch (error) { notify(String(error), true); }
});
document.querySelector("#choose-godot")!.addEventListener("click", async () => {
  const executable = await open({ multiple: false, directory: false, title: "Select the Godot editor executable" });
  if (!executable) return;
  try {
    state = await invoke<HubState>("register_godot_runtime", { executable });
    renderSourceGames();
    notify("Godot executable saved. Linked Godot games can now be run from the Hub.");
  } catch (error) { notify(String(error), true); }
});

document.querySelector("#source-game-button")!.addEventListener("click", async () => {
  const root = await open({ multiple: false, directory: true, title: "Choose a source-game folder" });
  if (!root) return;
  try {
    state = await invoke<HubState>("link_source_game", { root });
    await refreshSourceGames();
    notify("Game linked. Check its compatibility report before conversion.");
  } catch (error) { notify(String(error), true); }
});
document.querySelector("#close-game-inspection")!.addEventListener("click", () => gameInspectionDialog.close());
document.querySelector("#create-button")!.addEventListener("click", () => createDialog.showModal());
document.querySelector("#clone-button")!.addEventListener("click", () => cloneDialog.showModal());
document.querySelector("#external-import-button")!.addEventListener("click", () => externalImportDialog.showModal());
document.querySelector("#engine-button")!.addEventListener("click", () => document.querySelector<HTMLDialogElement>("#engine-dialog")!.showModal());
document.querySelector("#close-engine")!.addEventListener("click", () => document.querySelector<HTMLDialogElement>("#engine-dialog")!.close());
document.querySelector("#close-recovery")!.addEventListener("click", () => recoveryDialog.close());
projectFilter.addEventListener("input", () => { renderProjects(); renderSourceGames(); });
packageProfile.addEventListener("change", updatePackageDestination);

document.querySelector("#browse-project")!.addEventListener("click", async () => {
  const selected = await open({ multiple: false, directory: false, filters: [{ name: "Kairo project", extensions: ["kproject"] }] });
  if (selected) document.querySelector<HTMLInputElement>("#import-path")!.value = selected;
});

document.querySelector("#browse-parent")!.addEventListener("click", async () => {
  const selected = await open({ multiple: false, directory: true });
  if (selected) document.querySelector<HTMLInputElement>("#parent-path")!.value = selected;
});

document.querySelector("#browse-clone-parent")!.addEventListener("click", async () => {
  const selected = await open({ multiple: false, directory: true });
  if (selected) document.querySelector<HTMLInputElement>("#clone-parent")!.value = selected;
});

document.querySelector("#browse-external-parent")!.addEventListener("click", async () => {
  const selected = await open({ multiple: false, directory: true });
  if (selected) document.querySelector<HTMLInputElement>("#external-import-parent")!.value = selected;
});

document.querySelector("#add-engine")!.addEventListener("click", async () => {
  const root = await open({ multiple: false, directory: true, title: "Select KairoGameEngine root" });
  if (!root) return;
  try {
    await invoke<EngineInstallation>("register_engine", { root });
    state = await invoke<HubState>("hub_state");
    engines = await invoke<EngineInstallation[]>("engine_installations");
    renderEngines();
  } catch (error) { notify(String(error), true); }
});

document.querySelector("#engine-list")!.addEventListener("click", async (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-engine]");
  if (!button) return;
  try {
    state = await invoke<HubState>("select_engine", { root: button.dataset.engine! });
    renderEngines();
  } catch (error) { notify(String(error), true); }
});

document.querySelector("#confirm-import")!.addEventListener("click", async (event) => {
  event.preventDefault();
  const path = document.querySelector<HTMLInputElement>("#import-path")!.value.trim();
  if (!path) return;
  try {
    state = await invoke<HubState>("import_project", { path });
    importDialog.close();
    await refreshHealth();
    notify("Project imported");
  } catch (error) { notify(String(error), true); }
});

document.querySelector("#confirm-create")!.addEventListener("click", async (event) => {
  event.preventDefault();
  const displayName = document.querySelector<HTMLInputElement>("#display-name")!.value.trim();
  const folderName = document.querySelector<HTMLInputElement>("#folder-name")!.value.trim();
  const parent = document.querySelector<HTMLInputElement>("#parent-path")!.value.trim();
  if (!displayName || !folderName || !parent) return;
  try {
    await invoke<string>("create_project", { parent, folderName, displayName });
    state = await invoke<HubState>("hub_state");
    createDialog.close();
    await refreshHealth();
    notify("Project created");
  } catch (error) { notify(String(error), true); }
});

document.querySelector("#confirm-clone")!.addEventListener("click", async (event) => {
  event.preventDefault();
  const repository = document.querySelector<HTMLInputElement>("#clone-url")!.value.trim();
  const folderName = document.querySelector<HTMLInputElement>("#clone-folder")!.value.trim();
  const parent = document.querySelector<HTMLInputElement>("#clone-parent")!.value.trim();
  if (!repository || !folderName || !parent) return;
  try {
    await invoke<string>("clone_project", { repository, parent, folderName });
    state = await invoke<HubState>("hub_state");
    cloneDialog.close();
    await refreshHealth();
    notify("Repository cloned and project imported");
  } catch (error) { notify(String(error), true); }
});

document.querySelector("#confirm-external-import")!.addEventListener("click", async (event) => {
  event.preventDefault();
  const repository = document.querySelector<HTMLInputElement>("#external-import-url")!.value.trim();
  const folderName = document.querySelector<HTMLInputElement>("#external-import-folder")!.value.trim();
  const parent = document.querySelector<HTMLInputElement>("#external-import-parent")!.value.trim();
  const entrySceneValue = document.querySelector<HTMLInputElement>("#external-import-scene")!.value.trim();
  if (!repository || !folderName || !parent) return;
  try {
    const project = await invoke<string>("import_external_gltf_project", {
      repository,
      parent,
      folderName,
      entryScene: entrySceneValue || null
    });
    state = await invoke<HubState>("hub_state");
    externalImportDialog.close();
    await refreshHealth();
    notify(`External scene converted to Kairo project: ${project}`);
  } catch (error) { notify(String(error), true); }
});

document.querySelector("#confirm-package")!.addEventListener("click", async (event) => {
  event.preventDefault();
  if (!packageProject || !packageProfile.value) return;
  const button = event.currentTarget as HTMLButtonElement;
  button.disabled = true;
  button.textContent = "Packaging…";
  try {
    const artifact = await invoke<PackageArtifact>("package_project", {
      path: packageProject,
      profileName: packageProfile.value,
      replace: document.querySelector<HTMLInputElement>("#replace-package")!.checked
    });
    packageDialog.close();
    notify(`${artifact.profileName} package ready at ${artifact.outputDirectory}`);
  } catch (error) {
    notify(String(error), true);
  } finally {
    button.disabled = false;
    button.textContent = "Package";
  }
});

projectList.addEventListener("click", async (event) => {
  const target = (event.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]");
  const row = target?.closest<HTMLElement>("[data-path]");
  if (!target || !row) return;
  const path = row.dataset.path!;
  try {
    if (target.dataset.action === "favorite") {
      state = await invoke<HubState>("set_favorite", { path, favorite: !state.favorites.includes(path) });
      renderProjects();
    } else if (target.dataset.action === "repair") {
      const health = await invoke<ProjectHealth>("repair_project", { path });
      healthByPath.set(path, health);
      renderProjects();
      notify(health.errors.length ? health.errors.join("; ") : "Missing project files repaired");
    } else if (target.dataset.action === "recovery") {
      await showRecovery(path);
    } else if (target.dataset.action === "package") {
      showPackage(path);
    } else if (target.dataset.action === "run" || target.dataset.action === "open") {
      const mode = target.dataset.action === "run" ? "play" : "edit";
      const shippedExecutable = mode === "play" &&
        Boolean(healthByPath.get(path)?.descriptor?.playExecutable);
      if (!shippedExecutable && !await ensureNativeTools(mode)) return;
      target.disabled = true;
      const originalLabel = target.textContent;
      target.textContent = mode === "play" ? "Starting…" : "Opening…";
      try {
        const processId = mode === "play"
          ? await invoke<number>(shippedExecutable ? "launch_prebuilt_game" : "launch_player", { path })
          : await invoke<number>("launch_editor", { path, recoverySnapshot: null });
        notify(mode === "play"
          ? `Game runtime started (process ${processId})`
          : `Editor opened (process ${processId})`);
      } finally {
        target.disabled = false;
        target.textContent = originalLabel;
      }
    }
  } catch (error) { notify(String(error), true); }
});

sourceGameList.addEventListener("click", async (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("button[data-source-action]");
  const row = button?.closest<HTMLElement>("[data-source-root]");
  if (!button || !row) return;
  const root = row.dataset.sourceRoot!;
  const action = button.dataset.sourceAction;
  const info = sourceGamesByRoot.get(root);
  if (!info) return;
  if (action === "inspect") { await showSourceInspection(info); return; }
  button.disabled = true;
  try {
    if (action === "play" || action === "edit") {
      // The novice path is a single Play click. If Godot isn't configured,
      // offer its file chooser at the point of need rather than disabling Play.
      if (!info.originalRuntimeAvailable && !state.godotExecutable) {
        const executable = await open({ multiple: false, directory: false,
          title: "Select Godot to play this existing game" });
        if (!executable) return;
        state = await invoke<HubState>("register_godot_runtime", { executable });
      }
      const pid = await invoke<number>("launch_source_game", { root, edit: action === "edit" });
      notify("Playing " + info.displayName + " in original " + info.engine +
        " runtime (process " + pid + "). Conversion is not required to play.");
    } else if (action === "convert") {
      reconstructRoot = root;
      reconstructScene.innerHTML = info.nativeSceneCandidates
        .map((candidate) => '<option value="' + escapeHtml(candidate) + '">' +
          escapeHtml(candidate) + '</option>').join("");
      reconstructDialog.showModal();
    }
  } catch (error) { notify(String(error), true); }
  finally { button.disabled = false; }
});

// Never silently choose one scene from a multi-scene imported game.
document.querySelector("#confirm-reconstruct")!.addEventListener("click", async (event) => {
  const root = reconstructRoot;
  const entryScene = reconstructScene.value;
  if (!root || !entryScene) return;
  const button = event.currentTarget as HTMLButtonElement;
  button.disabled = true;
  try {
    const project = await invoke<string>("reconstruct_source_game", { root, entryScene });
    reconstructDialog.close();
    reconstructRoot = null;
    state = await invoke<HubState>("hub_state");
    await refreshHealth();
    notify("Kairo 3D scene created: " + project +
      ". Gameplay is not ported. Use Edit to modify the scene.");
  } catch (error) {
    notify(String(error), true);
  } finally {
    button.disabled = false;
  }
});

recoveryList.addEventListener("click", async (event) => {
  const target = (event.target as HTMLElement).closest<HTMLButtonElement>("button[data-recovery]");
  if (!target || !recoveryProject) return;
  target.disabled = true;
  try {
    const processId = await invoke<number>("launch_editor", {
      path: recoveryProject,
      recoverySnapshot: target.dataset.recovery!
    });
    recoveryDialog.close();
    notify(`Recovered KairoEditor launched (process ${processId})`);
  } catch (error) {
    target.disabled = false;
    notify(String(error), true);
  }
});

if ("__TAURI_INTERNALS__" in window) {
  loadState().catch((error) => notify(`Cannot load KairoHub state: ${String(error)}`, true));
} else {
  renderProjects();
}
