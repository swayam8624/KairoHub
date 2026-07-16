import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

type ProjectDescriptor = {
  name: string;
  assetManifest: string;
  startupScene: string;
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
      <div class="engine-status"><span class="status-dot"></span><div><strong>Local engine</strong><small>Environment discovery</small></div></div>
    </aside>
    <main>
      <header class="topbar">
        <div><p class="eyebrow">Project workspace</p><h1>Your projects</h1></div>
        <div class="header-actions">
          <button id="import-button" class="button secondary" type="button">Import project</button>
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
      <label>Descriptor path<input id="import-path" required placeholder="/path/to/Game.kproject" /></label>
      <p class="field-note">The descriptor and referenced startup files are validated before import.</p>
      <div class="dialog-actions"><button value="cancel" class="button secondary">Cancel</button><button id="confirm-import" value="default" class="button primary">Import</button></div>
    </form>
  </dialog>
  <dialog id="create-dialog">
    <form method="dialog" class="dialog-form">
      <div><p class="eyebrow">Blank template</p><h2>Create a Kairo project</h2></div>
      <label>Project name<input id="display-name" required placeholder="Skybound" /></label>
      <label>Folder name<input id="folder-name" required pattern="[A-Za-z0-9_-]+" placeholder="Skybound" /></label>
      <label>Parent directory<input id="parent-path" required placeholder="/Users/name/Projects" /></label>
      <div class="dialog-actions"><button value="cancel" class="button secondary">Cancel</button><button id="confirm-create" value="default" class="button primary">Create project</button></div>
    </form>
  </dialog>
  <div id="toast" class="toast" role="status" aria-live="polite"></div>
`;

const projectList = document.querySelector<HTMLDivElement>("#project-list")!;
const projectFilter = document.querySelector<HTMLInputElement>("#project-filter")!;
const importDialog = document.querySelector<HTMLDialogElement>("#import-dialog")!;
const createDialog = document.querySelector<HTMLDialogElement>("#create-dialog")!;
const toast = document.querySelector<HTMLDivElement>("#toast")!;
let state: HubState = { recentProjects: [], favorites: [] };
let healthByPath = new Map<string, ProjectHealth>();

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
      <button class="button secondary" type="button" data-action="recovery">Recovery</button>
      <button class="button primary" type="button" data-action="open" ${valid ? "" : "disabled"}>Open editor</button>
    </article>`;
  }).join("");
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
  await refreshHealth();
}

document.querySelector("#import-button")!.addEventListener("click", () => importDialog.showModal());
document.querySelector("#create-button")!.addEventListener("click", () => createDialog.showModal());
projectFilter.addEventListener("input", renderProjects);

document.querySelector("#confirm-import")!.addEventListener("click", async (event) => {
  event.preventDefault();
  const path = document.querySelector<HTMLInputElement>("#import-path")!.value.trim();
  if (!path) return;
  try {
    state = await invoke<HubState>("remember_project", { path });
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

projectList.addEventListener("click", async (event) => {
  const target = (event.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]");
  const row = target?.closest<HTMLElement>("[data-path]");
  if (!target || !row) return;
  const path = row.dataset.path!;
  try {
    if (target.dataset.action === "favorite") {
      state = await invoke<HubState>("set_favorite", { path, favorite: !state.favorites.includes(path) });
      renderProjects();
    } else {
      const recoveryMode = target.dataset.action === "recovery";
      const processId = await invoke<number>("launch_editor", { path, recoveryMode });
      notify(`KairoEditor launched (process ${processId})`);
    }
  } catch (error) { notify(String(error), true); }
});

loadState().catch((error) => notify(`Cannot load KairoHub state: ${String(error)}`, true));
