// Saved worlds/accounts. Native passwords are resolved by /login, never fetched.
const LAUNCHER_SELECTION_KEY = "anima.launcher.selection.v1";
const LAUNCHER_BROWSER_KEY = "anima.launcher.browser.v1";
let launcherData = { servers: [], accounts: [], passwords: false, persistent: false };
let launcherServerId = "", launcherAccountId = "";
let launcherReady = false, launcherWorking = false, launcherConnecting = false;
let launcherInitPromise = null, launcherAuthKey = "";
let launcherSelection = { server: "", accounts: {} };
const launcherEl = id => document.getElementById(id);
const launcherText = (id, text) => { const el = launcherEl(id); if (el) el.textContent = text; };
const launcherUid = () => typeof crypto !== "undefined" && crypto.randomUUID ? crypto.randomUUID() : "p-" + Date.now().toString(36) + "-" + Math.random().toString(36).slice(2);
const launcherServer = () => launcherData.servers.find(s => s.id === launcherServerId);
const launcherAccount = () => launcherData.accounts.find(a => a.id === launcherAccountId && a.server_id === launcherServerId);
const launcherValue = id => (launcherEl(id)?.value || "").trim();
function launcherBusy() { return launcherWorking || launcherConnecting; }
function launcherDate(time) { return time ? new Date(time).toLocaleString() : "Not yet"; }
function launcherRememberSelection() {
  launcherSelection.server = launcherServerId;
  launcherSelection.accounts[launcherServerId] = launcherAccountId;
  try { localStorage.setItem(LAUNCHER_SELECTION_KEY, JSON.stringify(launcherSelection)); } catch (_) {}
}
async function launcherRequest(body) {
  const res = await fetch("launcher", {
    method: body ? "POST" : "GET", headers: { "X-Anima-Launcher": "1", ...(body ? { "Content-Type": "application/json" } : {}) },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data.error || "Profile storage is unavailable. Reopen Anima and try again.");
  if (!Array.isArray(data.servers) || !Array.isArray(data.accounts)) throw new Error("Invalid profile response.");
  return data;
}
function launcherBrowserCommand(body) {
  // WASM has no OS vault. Whitelist all persisted fields, even if callers pass a password.
  const next = JSON.parse(JSON.stringify(launcherData));
  if (body.op === "save_server") {
    const old = next.servers.find(s => s.id === body.id);
    const server = { id: body.id, name: body.name, host: body.host, port: body.port, shard: body.shard, notes: body.notes, relay: body.relay, cache: null };
    const changed = old && (old.host !== server.host || old.port !== server.port || old.shard !== server.shard || old.relay !== server.relay);
    if (old && !changed) server.cache = old.cache;
    if (changed) next.accounts.filter(a => a.server_id === body.id).forEach(a => { a.characters = []; a.last_used = null; });
    if (old) next.servers[next.servers.indexOf(old)] = server; else next.servers.push(server);
  } else if (body.op === "save_account") {
    if (body.remember_password) throw new Error("Password saving is available in the desktop app.");
    const old = next.accounts.find(a => a.id === body.id);
    const same = old && old.server_id === body.server_id && old.username === body.username;
    const account = { id: body.id, server_id: body.server_id, label: body.label, username: body.username, remember_password: false, characters: same ? old.characters : [], last_used: same ? old.last_used : null };
    if (old) next.accounts[next.accounts.indexOf(old)] = account; else next.accounts.push(account);
  } else if (body.op === "delete_account") next.accounts = next.accounts.filter(a => a.id !== body.id);
  else if (body.op === "delete_server") { next.servers = next.servers.filter(s => s.id !== body.id); next.accounts = next.accounts.filter(a => a.server_id !== body.id); }
  localStorage.setItem(LAUNCHER_BROWSER_KEY, JSON.stringify({ version: 1, servers: next.servers, accounts: next.accounts }));
  return next;
}
async function launcherCommand(body) {
  launcherData = WASM_MODE ? launcherBrowserCommand(body) : await launcherRequest(body);
}
function launcherSetBusy(value) {
  launcherWorking = value;
  for (const el of document.querySelectorAll("#lg-account-fields input, #lg-account-fields select, #lg-account-fields textarea, #lg-account-fields button, #lg-library button")) el.disabled = value || launcherConnecting || !launcherReady;
  launcherPasswordHint();
  const refresh = launcherEl("lg-refresh-server");
  if (refresh) refresh.disabled = value || launcherConnecting || !launcherServer() || WASM_MODE || !launcherReady;
  if (!value) {
    launcherEl("lg-remove-server").disabled = !launcherServer() || launcherConnecting || !launcherReady;
    launcherEl("lg-remove-account").disabled = !launcherAccount() || launcherConnecting || !launcherReady;
  }
  if (WASM_MODE) launcherEl("lg-shard").disabled = true;
  const retry = launcherEl("lg-retry-profiles");
  if (retry) { retry.hidden = launcherReady; retry.disabled = value; }
}
function launcherPasswordHint() {
  const checkbox = launcherEl("lg-save-password"), input = launcherEl("lg-pass");
  if (!checkbox || !input) return;
  checkbox.disabled = !launcherData.passwords || launcherBusy() || !launcherReady;
  if (!launcherData.passwords) checkbox.checked = false;
  const account = launcherAccount(), server = launcherServer();
  const matching = account && server && account.username === launcherValue("lg-user") && launcherMatchesServer(server);
  input.placeholder = matching && account.remember_password ? "Saved securely · type to replace" : "Enter password";
  launcherText("lg-password-note", !launcherData.passwords
    ? "Password saving is available in the desktop app. Browser storage contains no passwords."
    : matching && account.remember_password
      ? "A password is saved in the system vault. Uncheck and save to remove it."
      : "Optional. Stored in macOS Keychain or Windows Credential Manager.");
}
function launcherMatchesServer(server) {
  return server.host === launcherValue("lg-host").replace(/^\[|\]$/g, "").toLowerCase() && server.port === Number(launcherValue("lg-port")) && server.shard === Number(launcherValue("lg-shard"));
}
function launcherRenderLibrary() {
  const list = launcherEl("lg-server-list"); if (!list) return;
  list.replaceChildren();
  for (const server of launcherData.servers) {
    const button = document.createElement("button"); button.type = "button"; button.className = "launcher-server";
    button.setAttribute("aria-pressed", String(server.id === launcherServerId)); button.disabled = launcherBusy();
    const name = document.createElement("strong"); name.textContent = server.name;
    const note = document.createElement("small");
    const count = launcherData.accounts.filter(a => a.server_id === server.id).length;
    note.textContent = `${count} account${count === 1 ? "" : "s"} · ${server.cache ? server.cache.reachable ? "last check reachable" : "last check failed" : "not checked"}`;
    button.append(name, note); button.addEventListener("click", () => { if (!launcherBusy()) launcherSelectServer(server.id); }); list.append(button);
  }
  if (!launcherData.servers.length) { const empty = document.createElement("p"); empty.className = "launcher-muted"; empty.textContent = "Your saved servers will appear here."; list.append(empty); }
}
function launcherRenderAccounts() {
  const select = launcherEl("lg-account-list"); select.replaceChildren();
  const option = document.createElement("option"); option.value = ""; option.textContent = "New account"; select.append(option);
  for (const account of launcherData.accounts.filter(a => a.server_id === launcherServerId)) {
    const row = document.createElement("option"); row.value = account.id;
    row.textContent = account.label === account.username ? account.username : `${account.label} · ${account.username}`;
    select.append(row);
  }
  select.value = launcherAccountId;
}
function launcherSelectServer(id) {
  const server = launcherData.servers.find(s => s.id === id);
  launcherServerId = server?.id || "";
  launcherEl("lg-server-name").value = server?.name || "";
  launcherEl("lg-host").value = server?.host || "127.0.0.1";
  launcherEl("lg-port").value = String(server?.port || 2594);
  launcherEl("lg-shard").value = String(server?.shard || 0);
  launcherEl("lg-server-notes").value = server?.notes || "";
  if (WASM_MODE) {
    launcherEl("lg-relay").value = server?.relay || "ws://127.0.0.1:2595/relay?target=1";
    launcherEl("lg-shard").value = "0";
  }
  launcherSelectAccount(launcherSelection.accounts[launcherServerId] || "");
  launcherRenderLibrary(); launcherRenderInfo(); launcherRememberSelection(); launcherText("lg-profile-msg", "");
}
function launcherSelectAccount(id) {
  const account = launcherData.accounts.find(a => a.id === id && a.server_id === launcherServerId);
  launcherAccountId = account?.id || "";
  launcherEl("lg-user").value = account?.username || "";
  launcherEl("lg-account-label").value = account?.label || "";
  launcherEl("lg-pass").value = "";
  launcherEl("lg-save-password").checked = !!account?.remember_password;
  launcherRenderAccounts(); launcherRenderInfo(); launcherPasswordHint(); launcherRememberSelection();
  launcherEl("lg-remove-account").disabled = !account;
  launcherEl("lg-remove-server").disabled = !launcherServer();
}
function launcherRenderInfo() {
  const server = launcherServer(), cache = server?.cache, account = launcherAccount();
  launcherText("lg-info-name", server?.name || "A world awaits");
  launcherText("lg-info-address", server ? `${server.host}:${server.port}${server.shard ? " · shard " + server.shard : ""}` : "Save a server to keep its details here.");
  launcherText("lg-info-status", cache ? cache.reachable ? "Reachable at last check" : "Unreachable at last check" : "Not checked");
  const status = launcherEl("lg-info-status"); if (status) status.dataset.state = cache ? cache.reachable ? "reachable" : "unreachable" : "unknown";
  const stats = launcherEl("lg-info-stats"); stats.replaceChildren();
  const pairs = cache ? [["TCP response", cache.latency_ms == null ? "—" : `${cache.latency_ms} ms`], ["Reported clients", cache.clients == null ? "Not reported" : String(cache.clients)], ["Server uptime", cache.uptime_hours == null ? "Not reported" : `${cache.uptime_hours} hours`]] : [];
  if (cache?.reported_name) pairs.unshift(["Server name", cache.reported_name]);
  for (const [label, value] of pairs) { const row = document.createElement("div"), dt = document.createElement("dt"), dd = document.createElement("dd"); dt.textContent = label; dd.textContent = value; row.append(dt, dd); stats.append(row); }
  launcherText("lg-info-time", cache ? `Checked ${launcherDate(cache.checked_at)}. ${cache.details_at ? "Reported details: " + launcherDate(cache.details_at) + "." : "This server did not provide public status details."}` : WASM_MODE ? "Live checks are available in the desktop/native client." : "A manual check uses no account or password. Results are cached, not live.");
  launcherText("lg-info-notes", server?.notes || "");
  const chars = launcherEl("lg-cached-characters"); chars.replaceChildren();
  for (const slot of account?.characters || []) { const row = document.createElement("div"); row.textContent = `${slot.name} · slot ${slot.index + 1}`; chars.append(row); }
  launcherText("lg-character-time", account?.last_used ? `Cached at last authentication: ${launcherDate(account.last_used)}. Log in to refresh.` : "Log in with this account to cache its character list.");
  launcherEl("lg-refresh-server").disabled = !server || WASM_MODE || launcherBusy() || !launcherReady;
}
function launcherServerForm() {
  const host = launcherValue("lg-host").replace(/^\[|\]$/g, "").toLowerCase(), port = Number(launcherValue("lg-port")), shard = Number(launcherValue("lg-shard"));
  if (!host || host.length > 253 || !/^[a-z0-9.:-]+$/i.test(host)) throw new Error("Enter a hostname or IP address without a URL or port.");
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error("Port must be between 1 and 65535.");
  if (!Number.isInteger(shard) || shard < 0 || shard > 65535) throw new Error("Shard index must be between 0 and 65535.");
  const server = { op: "save_server", id: launcherServerId || launcherUid(), name: launcherValue("lg-server-name") || host, host, port, shard, notes: launcherEl("lg-server-notes").value || "" };
  if (WASM_MODE) { const relay = new URL(launcherValue("lg-relay")); if (!["ws:", "wss:"].includes(relay.protocol) || relay.username || relay.password) throw new Error("Enter a ws:// or wss:// relay URL without credentials."); server.relay = relay.href; }
  return server;
}
async function launcherSaveServer() {
  const form = launcherServerForm(), previous = launcherServer();
  if (previous && !launcherMatchesServer(previous) && launcherData.accounts.some(a => a.server_id === previous.id && a.remember_password)) {
    if (!confirm("Changing this server address or shard removes its saved passwords and cached characters. Continue?")) throw new Error("Server changes were not saved.");
  }
  await launcherCommand(form); launcherServerId = form.id;
  launcherEl("lg-server-name").value = form.name; launcherEl("lg-host").value = form.host;
  if (previous && !launcherMatchesServer(previous)) { launcherEl("lg-save-password").checked = false; launcherAccountId = ""; }
  launcherRenderLibrary(); launcherRenderAccounts(); launcherRenderInfo(); launcherRememberSelection();
}
async function launcherSaveAccount() {
  const username = launcherValue("lg-user"), password = launcherEl("lg-pass").value || "";
  if (!username || username.length > 30 || /[^\x20-\x7e]/.test(username)) throw new Error("Enter a UO username using up to 30 ASCII characters.");
  if (password.length > 30 || /[^\x20-\x7e]/.test(password)) throw new Error("UO passwords support up to 30 ASCII characters.");
  await launcherSaveServer();
  // Reusing an existing username updates it instead of creating duplicate profiles.
  const same = launcherData.accounts.find(a => a.server_id === launcherServerId && a.username === username);
  const id = same?.id || launcherAccountId || launcherUid();
  const form = { op: "save_account", id, server_id: launcherServerId, label: launcherValue("lg-account-label") || username, username, password, remember_password: !!launcherEl("lg-save-password").checked };
  await launcherCommand(form); launcherAccountId = id;
  launcherEl("lg-account-label").value = form.label;
  launcherRenderLibrary(); launcherRenderAccounts(); launcherRenderInfo(); launcherPasswordHint(); launcherRememberSelection();
}
async function launcherAction(action, message) {
  if (launcherBusy() || !launcherReady) return;
  launcherSetBusy(true); launcherText("lg-profile-msg", "Saving…");
  try { await action(); launcherText("lg-profile-msg", message); }
  catch (e) { launcherText("lg-profile-msg", e.message); }
  finally { launcherSetBusy(false); }
}
async function launcherPrepareLogin() {
  if (launcherInitPromise) await launcherInitPromise;
  if (!launcherReady) throw new Error("Profiles are not available. Reopen Anima before connecting.");
  launcherSetBusy(true);
  try {
    await launcherSaveAccount();
    return { account_id: WASM_MODE ? null : launcherAccountId, host: launcherValue("lg-host"), port: Number(launcherValue("lg-port")), shard: Number(launcherValue("lg-shard")), username: launcherValue("lg-user"), password: launcherEl("lg-pass").value || "" };
  } finally { launcherSetBusy(false); }
}
function launcherOnAuth(auth, slots) {
  if (!launcherEl("lg-shell")) return;
  launcherConnecting = auth === "connecting";
  launcherEl("lg-shell").classList.toggle("character-stage", auth === "characters");
  launcherSetBusy(launcherWorking);
  if (auth !== "characters") { if (auth === "login" || auth === "error") launcherAuthKey = ""; return; }
  launcherEl("lg-pass").value = "";
  const key = launcherAccountId + JSON.stringify(slots || []);
  if (!launcherReady || key === launcherAuthKey) return;
  launcherAuthKey = key;
  if (WASM_MODE) {
    const account = launcherAccount();
    if (account) {
      account.characters = (slots || []).map(s => ({ index: s.index, name: s.name })); account.last_used = Date.now();
      try { localStorage.setItem(LAUNCHER_BROWSER_KEY, JSON.stringify({ version: 1, servers: launcherData.servers, accounts: launcherData.accounts })); } catch (_) {}
      launcherRenderInfo();
    }
  } else {
    launcherRequest().then(data => { launcherData = data; launcherRenderInfo(); }).catch(e => launcherText("lg-profile-msg", e.message));
  }
}
function initLauncher() {
  if (launcherInitPromise || !launcherEl("lg-server-list")) return;
  launcherEl("lg-new-server").addEventListener("click", () => { if (!launcherBusy()) launcherSelectServer(""); });
  launcherEl("lg-new-account").addEventListener("click", () => { if (!launcherBusy()) { launcherSelectAccount(""); launcherText("lg-profile-msg", ""); } });
  launcherEl("lg-account-list").addEventListener("change", e => launcherSelectAccount(e.target.value));
  for (const id of ["lg-user", "lg-host", "lg-port", "lg-shard"]) launcherEl(id).addEventListener("input", launcherPasswordHint);
  launcherEl("lg-save-server").addEventListener("click", () => launcherAction(launcherSaveServer, "Server saved."));
  launcherEl("lg-save-account").addEventListener("click", () => launcherAction(launcherSaveAccount, "Account saved."));
  launcherEl("lg-remove-account").addEventListener("click", () => launcherAction(async () => {
    const account = launcherAccount(); if (!account || !confirm(`Remove ${account.label} from this device, including its saved password? Your game account is not deleted.`)) return;
    await launcherCommand({ op: "delete_account", id: account.id }); launcherSelectAccount(""); launcherRenderLibrary();
  }, "Account library updated."));
  launcherEl("lg-remove-server").addEventListener("click", () => launcherAction(async () => {
    const server = launcherServer(); if (!server || !confirm(`Remove ${server.name} and its saved accounts and passwords from this device? Game accounts are not deleted.`)) return;
    await launcherCommand({ op: "delete_server", id: server.id }); launcherSelectServer(launcherData.servers[0]?.id || "");
  }, "Server library updated."));
  launcherEl("lg-refresh-server").addEventListener("click", () => launcherAction(async () => {
    launcherText("lg-profile-msg", "Checking server without logging in…");
    await launcherCommand({ op: "refresh", id: launcherServerId }); launcherRenderLibrary(); launcherRenderInfo();
  }, "Server check cached."));
  launcherEl("lg-retry-profiles")?.addEventListener("click", launcherLoadProfiles);
  launcherLoadProfiles();
}
function launcherLoadProfiles() {
  launcherSetBusy(true);
  launcherInitPromise = (async () => {
    try {
      try { const selected = JSON.parse(localStorage.getItem(LAUNCHER_SELECTION_KEY) || "null"); if (selected && typeof selected.accounts === "object" && selected.accounts) launcherSelection = selected; } catch (_) {}
      if (WASM_MODE) {
        const stored = JSON.parse(localStorage.getItem(LAUNCHER_BROWSER_KEY) || '{"version":1,"servers":[],"accounts":[]}');
        if (stored.version !== 1 || !Array.isArray(stored.servers) || !Array.isArray(stored.accounts)) throw new Error("Saved browser profiles could not be loaded.");
        launcherData = { servers: stored.servers, accounts: stored.accounts, persistent: true, passwords: false };
      } else launcherData = await launcherRequest();
      launcherReady = true;
      launcherSelectServer(launcherData.servers.some(s => s.id === launcherSelection.server) ? launcherSelection.server : launcherData.servers[0]?.id || "");
      launcherText("lg-storage-note", WASM_MODE ? "Profiles stay in this browser. Passwords are not stored." : launcherData.persistent ? "Saved on this device, shared by your Anima windows." : "Profiles last for this session only.");
    } catch (e) { launcherText("lg-profile-msg", e.message); launcherText("lg-storage-note", "Profiles could not be loaded. Existing files have not been changed."); }
    finally { launcherSetBusy(false); }
  })();
}
