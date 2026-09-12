"use strict";
const byId = id => document.getElementById(id);
let view = null, busy = false, lastPath = null, lastSnapshot = "", localMessage = "", operation = 0, refreshPending = false;
const controls = ["browse", "detect", "check", "apply", "recover", "restart", "close"];
function render(next) {
  view = next;
  const report = view.report;
  if (report && report.path !== lastPath) { byId("folder").value = report.path; lastPath = report.path; }
  byId("title").textContent = view.running ? "Your game files." : "Your journey starts here.";
  const loading = view.initialising || view.starting;
  byId("badge").textContent = loading ? (view.starting ? "Opening…" : "Checking…") : report?.ready ? "Files ready" : "Needs a folder";
  byId("badge").classList.toggle("ready", !!report?.ready && !loading);
  byId("storage-error").hidden = !view.config_error;
  byId("storage-message").textContent = view.config_error || "";
  byId("recover").hidden = !view.recoverable;
  byId("config-path").textContent = view.config_path;
  byId("active-path").textContent = view.active_path ? "Current session: " + view.active_path : "";
  byId("checks").replaceChildren();
  if (report) {
    for (const check of report.checks) {
      const row = document.createElement("div"); row.className = "file-check" + (check.ready ? " ready" : "");
      const icon = document.createElement("span"); icon.className = "icon"; icon.textContent = check.ready ? "✓" : check.required ? "!" : "–"; icon.setAttribute("aria-hidden", "true");
      const text = document.createElement("div"), title = document.createElement("h3"), detail = document.createElement("p");
      title.textContent = check.name; detail.textContent = check.detail; text.append(title, detail);
      const label = document.createElement("span"); label.className = "label"; label.textContent = check.ready ? "Found" : check.required ? "Required" : "Optional";
      row.append(icon, text, label); byId("checks").append(row);
    }
  } else {
    const hint = document.createElement("p"); hint.className = "muted"; hint.textContent = view.initialising ? "Looking for your installation…" : "Choose a folder to check the files Anima needs."; byId("checks").append(hint);
  }
  byId("compatibility").hidden = !report || report.checks.find(c => c.name === "World and buildings")?.ready;
  byId("feedback").textContent = localMessage || view.startup_error || view.notice;
  byId("close").hidden = !view.running;
  byId("restart").hidden = !view.running || !view.saved_path || view.saved_path === view.active_path;
  byId("apply").textContent = view.starting ? "Opening…" : view.running ? "Save for next launch" : "Open Anima →";
  byId("next-step").textContent = view.running ? "Restarting disconnects your current game session. You can keep playing and restart later." : "Your saved folder will be used on future launches.";
  updateControls();
}
function updateControls() {
  const unavailable = busy || view?.initialising || view?.starting;
  for (const id of controls) byId(id).disabled = !!unavailable;
  byId("folder").disabled = !!unavailable;
  const checked = view?.report?.ready && byId("folder").value.trim() === view.report.path;
  byId("apply").disabled = !!unavailable || !checked || !!view?.config_error;
}
async function call(command, args) {
  if (busy) return;
  operation++;
  busy = true; localMessage = ""; byId("feedback").textContent = command === "setup_choose" ? "Choose a folder in the system picker…" : "Working…"; updateControls();
  try {
    const next = await window.__TAURI__.core.invoke(command, args);
    if (next) {
      if (["setup_choose", "setup_check", "setup_detect"].includes(command) && next.report && !next.notice) lastPath = null;
      render(next);
    }
  } catch (error) { localMessage = String(error); byId("feedback").textContent = localMessage; }
  finally { busy = false; updateControls(); }
}
byId("folder").addEventListener("input", updateControls);
byId("folder-form").addEventListener("submit", event => { event.preventDefault(); call("setup_check", { path: byId("folder").value }); });
for (const [id, command] of Object.entries({browse:"setup_choose",detect:"setup_detect",recover:"setup_recover",apply:"setup_apply",close:"setup_close",restart:"setup_restart"})) {
  byId(id).addEventListener("click", () => call(command));
}
async function refresh() {
  if (busy || refreshPending) return;
  refreshPending = true;
  const generation = operation;
  try {
    const next = await window.__TAURI__.core.invoke("setup_status");
    if (generation !== operation || busy) return;
    const snapshot = JSON.stringify(next);
    if (snapshot !== lastSnapshot) { lastSnapshot = snapshot; render(next); }
  } catch (error) { if (generation === operation && !busy) byId("feedback").textContent = "Could not read app setup: " + String(error); }
  finally { refreshPending = false; }
}
if (window.__TAURI__?.core) { refresh(); setInterval(refresh, 700); }
else { byId("feedback").textContent = "Open this screen from Anima's desktop app."; for (const id of controls) byId(id).disabled = true; }
