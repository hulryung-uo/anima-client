// Backup/recovery stays available at login, even when game settings need repair.
let preferencePreview = null, preferenceFileGeneration = 0;
let preferenceDownloadUrl = null;
function preferenceMessage(text) { document.getElementById("pref-message").textContent = text; }
function preferenceDownloadResult(success) {
  preferenceMessage(success ? "Backup saved in your Downloads folder." : "The backup could not be saved. Check your Downloads folder and try the Save link again.");
}
function preferenceInGame() { return !!scene?.player; }
function renderPreferenceStatus(status = preferenceStorage.status()) {
  const warning = document.getElementById("preference-warning");
  if (!warning) return;
  warning.hidden = !status.message;
  document.getElementById("preference-warning-text").textContent = status.message;
  document.getElementById("pref-status").textContent = status.message || (status.pending ? "Some changes have not been saved yet." : "Your settings are saved on this device.");
  document.getElementById("pref-recover").hidden = !status.recoverable;
  document.getElementById("pref-retry").hidden = !status.pending || status.recoverable;
  document.getElementById("pref-original").hidden = !status.recovery;
  document.getElementById("pref-previous").hidden = !status.recovery;
  document.getElementById("pref-recover").disabled = preferenceInGame();
  document.getElementById("pref-apply").disabled = !preferencePreview || preferenceInGame();
  document.getElementById("pref-session-note").textContent = preferenceInGame()
    ? "You can export while playing. Log out before restoring or recovering settings."
    : "Restoring replaces these preferences and reloads Anima. A copy of the previous settings is kept.";
}
function openPreferencePanel() {
  const panel = document.getElementById("preference-panel");
  if (panel.open) return;
  if (typeof releaseMoveKeys === "function") releaseMoveKeys();
  if (typeof stopMacro === "function") stopMacro();
  if (typeof stopFollowing === "function") stopFollowing();
  preferenceMessage(""); renderPreferenceStatus(); panel.showModal();
}
function previewPreferences(text, name) {
  preferencePreview = null;
  document.getElementById("pref-preview").hidden = true;
  try {
    const values = preferenceStorage.parseBackup(text);
    preferencePreview = { values, source: preferenceStorage.source() };
    const count = key => JSON.parse(values["anima." + key] || "[]").length;
    document.getElementById("pref-file-name").textContent = name;
    document.getElementById("pref-summary").textContent = `${Object.keys(values).length} saved groups · ${count("macros")} macros · ${count("markers")} map markers`;
    document.getElementById("pref-preview").hidden = false;
    preferenceMessage("Review the backup, then apply it when you are ready. Missing groups return to their defaults.");
  } catch (error) { preferenceMessage(error.message); }
  renderPreferenceStatus();
}
function downloadPreferences(text, name) {
  if (preferenceDownloadUrl) URL.revokeObjectURL(preferenceDownloadUrl);
  preferenceDownloadUrl = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  // Keep the link available if the browser needs a second explicit save action.
  const link = document.getElementById("pref-download");
  link.href = preferenceDownloadUrl; link.download = name;
  link.textContent = "Save " + name; link.hidden = false; link.click();
}
function applyPreferencePreview() {
  if (!preferencePreview) return;
  if (preferenceInGame()) { preferenceMessage("Log out before restoring settings."); return; }
  try {
    preferenceStorage.replace(preferencePreview.values, preferencePreview.source);
    preferenceMessage("Settings restored. Reloading Anima…"); location.reload();
  } catch (error) { preferenceMessage(error.message || "Could not save the backup. Your saved settings are unchanged."); }
}
function wirePreferencePanel() {
  const panel = document.getElementById("preference-panel");
  if (!panel || panel.dataset.wired) return;
  panel.dataset.wired = "1";
  preferenceStorage.onChange(renderPreferenceStatus);
  document.getElementById("lg-settings-data")?.addEventListener("click", openPreferencePanel);
  document.getElementById("preference-warning-open").addEventListener("click", openPreferencePanel);
  document.getElementById("pref-close").addEventListener("click", () => panel.close());
  panel.addEventListener("close", () => {
    if (preferenceDownloadUrl) URL.revokeObjectURL(preferenceDownloadUrl);
    preferenceDownloadUrl = null; document.getElementById("pref-download").hidden = true;
  });
  for (const type of ["keydown", "keyup", "mousedown", "mouseup", "wheel"]) panel.addEventListener(type, e => e.stopPropagation());
  document.getElementById("pref-export").addEventListener("click", () => {
    try { downloadPreferences(preferenceStorage.exportData(), "anima-settings.json"); preferenceMessage("Settings backup prepared for download."); }
    catch (error) { preferenceMessage(error.message || "The settings backup could not be prepared."); }
  });
  document.getElementById("pref-original").addEventListener("click", () => {
    try { downloadPreferences(preferenceStorage.recoveryData(), "anima-settings-recovery.json"); preferenceMessage("Original settings prepared for download."); }
    catch (error) { preferenceMessage(error.message); }
  });
  document.getElementById("pref-previous").addEventListener("click", () => {
    preferenceFileGeneration++; preferencePreview = null;
    document.getElementById("pref-preview").hidden = true; renderPreferenceStatus();
    try { previewPreferences(preferenceStorage.previousData(), "Previous settings"); }
    catch (error) { preferenceMessage(error.message); }
  });
  document.getElementById("pref-retry").addEventListener("click", () => {
    preferenceMessage(preferenceStorage.flush() ? "Your changes have been saved." : "Changes could not be saved yet.");
  });
  document.getElementById("pref-recover").addEventListener("click", () => {
    if (preferenceInGame()) { preferenceMessage("Log out before recovering settings."); return; }
    try { preferenceStorage.recover(); preferenceMessage("Settings recovered. Reloading Anima…"); location.reload(); }
    catch (error) { preferenceMessage(error.message || "Recovery could not be saved. The original settings are unchanged."); }
  });
  document.getElementById("pref-file").addEventListener("change", async e => {
    const generation = ++preferenceFileGeneration, file = e.target.files?.[0];
    if (!file) return;
    preferencePreview = null; document.getElementById("pref-preview").hidden = true; renderPreferenceStatus();
    try {
      if (file.size > preferenceStorage.limit) throw new Error("Choose a settings backup smaller than 4 MB.");
      const text = await file.text();
      if (generation === preferenceFileGeneration) previewPreferences(text, file.name);
    } catch (error) { if (generation === preferenceFileGeneration) preferenceMessage(error.message || "The selected file could not be read."); }
    finally { if (generation === preferenceFileGeneration) e.target.value = ""; }
  });
  document.getElementById("pref-apply").addEventListener("click", applyPreferencePreview);
  window.addEventListener("storage", e => {
    if (!preferenceStorage.relevant(e.key)) return;
    renderPreferenceStatus();
    if (panel.open) preferenceMessage("Device settings changed in another window. Review your backup again before restoring.");
  });
}
wirePreferencePanel();
