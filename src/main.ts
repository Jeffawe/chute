import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

type Target = {
  ip: string;
  name: string;
  online: boolean;
  status: string | null;
};

type Config = { download_dir: string; auto_receive: boolean; shell_menus: boolean };

type Received = { name: string; bytes: number; path: string };

// The Rust side serialises its error enum as { kind, message }.
type TsError = { kind: string; message?: string };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

let staged: string[] = [];
let selected: string | null = null;

function describe(err: unknown): string {
  const e = err as TsError;
  if (e && typeof e === "object" && "kind" in e) {
    switch (e.kind) {
      case "MissingBinary":
        return "Tailscale isn't installed, or isn't on PATH.";
      case "DaemonUnreachable":
        return "tailscaled isn't running. Start it and try again.";
      default:
        return e.message ?? "Something went wrong.";
    }
  }
  return String(err);
}

function humanBytes(n: number): string {
  const units = ["B", "KB", "MB", "GB"];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return `${u === 0 ? v : v.toFixed(1)} ${units[u]}`;
}

function basename(p: string): string {
  const parts = p.split("/");
  return parts[parts.length - 1] || p;
}

/* ---------- devices ---------- */

async function refreshTargets() {
  const list = $("devices");
  try {
    const targets = await invoke<Target[]>("list_targets");
    setDaemon("ok", "connected");

    if (targets.length === 0) {
      list.innerHTML = `<li class="empty">No devices available to send to.</li>`;
      return;
    }

    list.innerHTML = "";
    for (const t of targets) {
      const li = document.createElement("li");
      li.className = "device" + (t.online ? "" : " is-offline");
      li.dataset.name = t.name;
      li.innerHTML = `
        <span class="dot"></span>
        <span class="device-name"></span>
        <span class="device-meta"></span>`;
      li.querySelector(".device-name")!.textContent = t.name;
      li.querySelector(".device-meta")!.textContent = t.online ? t.ip : (t.status ?? "offline");
      li.addEventListener("click", () => {
        selected = t.name;
        for (const el of list.querySelectorAll(".device")) el.classList.remove("is-selected");
        li.classList.add("is-selected");
        updateSendButton();
      });
      list.appendChild(li);
    }

    // Keep the previous selection if that device is still listed.
    if (selected && !targets.some((t) => t.name === selected)) selected = null;
    if (selected) {
      list.querySelector(`[data-name="${CSS.escape(selected)}"]`)?.classList.add("is-selected");
    }
    updateSendButton();
  } catch (err) {
    const e = err as TsError;
    setDaemon(e?.kind === "DaemonUnreachable" ? "down" : "error", describe(err));
    list.innerHTML = `<li class="empty">${describe(err)}</li>`;
  }
}

function setDaemon(state: string, text: string) {
  const el = $("daemon");
  el.dataset.state = state;
  el.textContent = text;
}

/* ---------- staging + send ---------- */

function renderStaged() {
  const ul = $("staged");
  ul.innerHTML = "";
  for (const [i, p] of staged.entries()) {
    const li = document.createElement("li");
    const name = document.createElement("span");
    name.textContent = basename(p);
    name.title = p;
    const rm = document.createElement("button");
    rm.className = "link";
    rm.textContent = "remove";
    rm.addEventListener("click", () => {
      staged.splice(i, 1);
      renderStaged();
    });
    li.append(name, rm);
    ul.appendChild(li);
  }
  updateSendButton();
}

function updateSendButton() {
  const btn = $<HTMLButtonElement>("send");
  btn.disabled = staged.length === 0 || selected === null;
  btn.textContent =
    staged.length && selected
      ? `Send ${staged.length} file${staged.length > 1 ? "s" : ""} to ${selected}`
      : "Send";
}

function addFiles(paths: string[]) {
  for (const p of paths) if (!staged.includes(p)) staged.push(p);
  renderStaged();
}

async function pickFiles() {
  const picked = await open({ multiple: true, directory: false });
  if (!picked) return;
  addFiles(Array.isArray(picked) ? picked : [picked]);
}

async function doSend() {
  if (!selected || staged.length === 0) return;
  const btn = $<HTMLButtonElement>("send");
  const status = $("send-status");
  btn.disabled = true;
  status.className = "status";
  status.textContent = "Sending…";
  try {
    await invoke("send_files", { paths: staged, target: selected });
    const n = staged.length;
    staged = [];
    renderStaged();
    status.className = "status is-ok";
    status.textContent = `Sent ${n} file${n > 1 ? "s" : ""} to ${selected}.`;
  } catch (err) {
    status.className = "status is-error";
    status.textContent = describe(err);
  } finally {
    updateSendButton();
  }
}

/* ---------- settings ---------- */

async function loadSettings() {
  const cfg = await invoke<Config>("get_config");
  $<HTMLInputElement>("download-dir").value = cfg.download_dir;
  $<HTMLInputElement>("auto-receive").checked = cfg.auto_receive;
  $<HTMLInputElement>("shell-menus").checked = cfg.shell_menus;
  // Autostart lives in the plugin's own state, not our config file, so it is
  // read back separately rather than mirrored.
  $<HTMLInputElement>("autostart").checked = await invoke<boolean>("get_autostart");
}

async function saveSettings() {
  const status = $("settings-status");
  const cfg: Config = {
    download_dir: $<HTMLInputElement>("download-dir").value.trim(),
    auto_receive: $<HTMLInputElement>("auto-receive").checked,
    shell_menus: $<HTMLInputElement>("shell-menus").checked,
  };
  try {
    await invoke("set_config", { cfg });
    await invoke("set_autostart", {
      enabled: $<HTMLInputElement>("autostart").checked,
    });
    status.className = "status is-ok";
    status.textContent = "Saved.";
  } catch (err) {
    status.className = "status is-error";
    status.textContent = describe(err);
  }
}

/* ---------- wiring ---------- */

function initTabs() {
  for (const tab of document.querySelectorAll<HTMLButtonElement>(".tab")) {
    tab.addEventListener("click", () => {
      for (const t of document.querySelectorAll(".tab")) t.classList.remove("is-active");
      for (const p of document.querySelectorAll(".panel")) p.classList.remove("is-active");
      tab.classList.add("is-active");
      $(`panel-${tab.dataset.tab}`).classList.add("is-active");
    });
  }
}

async function initDragDrop() {
  const zone = $("dropzone");
  await getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "over") zone.classList.add("is-over");
    else if (event.payload.type === "drop") {
      zone.classList.remove("is-over");
      addFiles(event.payload.paths);
    } else zone.classList.remove("is-over");
  });
}

/// Report a frontend failure to the backend log and the status pill, so a
/// broken step is visible instead of silently aborting startup.
function reportError(where: string, err: unknown) {
  const msg = `${where}: ${err instanceof Error ? err.message : String(err)}`;
  void invoke("log_js", { message: msg }).catch(() => {});
  setDaemon("error", msg);
}

window.addEventListener("error", (e) => reportError("uncaught", e.error ?? e.message));
window.addEventListener("unhandledrejection", (e) => reportError("unhandled", e.reason));

/// Run a startup step in isolation: one failure must not abort the rest.
async function step(name: string, fn: () => Promise<void>) {
  try {
    await fn();
  } catch (err) {
    reportError(name, err);
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  initTabs();
  $("refresh").addEventListener("click", refreshTargets);
  $("pick").addEventListener("click", pickFiles);
  $("send").addEventListener("click", doSend);
  $("save").addEventListener("click", saveSettings);
  $("browse").addEventListener("click", async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") $<HTMLInputElement>("download-dir").value = dir;
  });

  await listen<Received>("file-received", (e) => {
    const ul = $("arrivals");
    ul.querySelector(".empty")?.remove();

    const li = document.createElement("li");
    li.className = "arrival";
    li.tabIndex = 0;
    li.title = `Show ${e.payload.path} in file manager`;

    const name = document.createElement("span");
    name.textContent = e.payload.name;

    const meta = document.createElement("span");
    meta.className = "device-meta";
    meta.textContent = `${humanBytes(e.payload.bytes)} · ${new Date().toLocaleTimeString()}`;

    const reveal = async () => {
      try {
        await revealItemInDir(e.payload.path);
      } catch {
        // Most likely the file has since been moved or deleted.
        meta.textContent = "could not open — file may have moved";
        meta.classList.add("is-error");
      }
    };
    li.addEventListener("click", reveal);
    li.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        void reveal();
      }
    });

    li.append(name, meta);
    ul.prepend(li);
  });

  // Files handed over by a second launch (the right-click entry point).
  await listen<string[]>("files-queued", (e) => {
    addFiles(e.payload);
    document.querySelector<HTMLButtonElement>('.tab[data-tab="send"]')?.click();
  });

  await listen<string>("receiver-error", (e) => {
    setDaemon("error", `receiver: ${e.payload}`);
  });

  // Devices first: it is the only step the app is useless without.
  await step("refreshTargets", refreshTargets);
  await step("loadSettings", loadSettings);
  await step("initDragDrop", initDragDrop);
  // Devices go up and down; keep the list roughly current without a refresh click.
  setInterval(refreshTargets, 15000);
});
