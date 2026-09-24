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
        <div class="section-heading"><h2>Recent</h2><input id="project-filter" type="search" placeholder="Filter projects" aria-label="Filter projects" /></div>
        <div id="project-list" class="project-list" aria-live="polite"></div>
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
let state: HubState = { recentProjects: [], favorites: [], engineRoots: [], selectedEngine: null };
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
      <button class="button secondary" type="button" data-action="run" ${valid ? "" : "disabled"}>Run</button>
      <button class="button primary" type="button" data-action="open" ${valid ? "" : "disabled"}>Open editor</button>
    </article>`;
  }).join("");
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

async function loadState(): Promise<void> {
  state = await invoke<HubState>("hub_state");
  engines = await invoke<EngineInstallation[]>("engine_installations");
  renderEngines();
  await refreshHealth();
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
document.querySelector("#create-button")!.addEventListener("click", () => createDialog.showModal());
document.querySelector("#clone-button")!.addEventListener("click", () => cloneDialog.showModal());
document.querySelector("#external-import-button")!.addEventListener("click", () => externalImportDialog.showModal());
document.querySelector("#engine-button")!.addEventListener("click", () => document.querySelector<HTMLDialogElement>("#engine-dialog")!.showModal());
document.querySelector("#close-engine")!.addEventListener("click", () => document.querySelector<HTMLDialogElement>("#engine-dialog")!.close());
document.querySelector("#close-recovery")!.addEventListener("click", () => recoveryDialog.close());
projectFilter.addEventListener("input", renderProjects);
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
    } else if (target.dataset.action === "run") {
      const processId = await invoke<number>("launch_player", { path });
      notify(`KairoPlayer launched (process ${processId})`);
    } else {
      const processId = await invoke<number>("launch_editor", { path, recoverySnapshot: null });
      notify(`KairoEditor launched (process ${processId})`);
    }
  } catch (error) { notify(String(error), true); }
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
