"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const STEAM_PICKER_LIMIT = 8;

// macOS keeps its own window buttons (see tauri.macos.conf.json), so ours step aside.
if (/Mac/.test(navigator.userAgent)) document.documentElement.classList.add("mac");
const STAGES = {
  closing: "closing steam",
  preparing: "setting things up",
  launching: "starting steam",
  forgetting: "removing account",
};

const $ = (id) => document.getElementById(id);
const els = {
  list: $("list"),
  search: $("search"),
  count: $("count"),
  status: $("status"),
  footer: $("footer"),
  menu: $("menu"),
  scrim: $("dialog-scrim"),
  dialog: $("dialog"),
  overlay: $("overlay"),
  overlayAvatar: $("overlay-avatar"),
  overlayName: $("overlay-name"),
  overlayStage: $("overlay-stage"),
  overlayOk: $("overlay-ok"),
  toast: $("toast"),
  refresh: $("btn-refresh"),
};

let state = null;
let stateKey = "";
let selectedId = null;
let busy = false;
let renderPending = false;
let menuFor = null;
let menuFocus = -1;
let menuItems = [];
let dialogCleanup = null;
let fastPollUntil = 0;
// Account to scroll into view once the list re-sorts after a switch.
let revealId = null;
const avatars = new Map();

/* ---------- helpers ---------- */

const ESC = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ESC[c]);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const displayName = (a) => a.nickname || a.personaName || a.accountName;
const findAccount = (id) => state?.accounts.find((a) => a.steamId === id) ?? null;

function initial(a) {
  const ch = [...displayName(a)].find((c) => /[\p{L}\p{N}]/u.test(c));
  return ch ? ch.toUpperCase() : "?";
}

function ago(ts) {
  if (!ts) return "—";
  const s = Math.max(0, Date.now() / 1000 - ts);
  const m = s / 60, h = m / 60, d = h / 24;
  if (m < 1) return "now";
  if (h < 1) return `${Math.floor(m)}m`;
  if (d < 1) return `${Math.floor(h)}h`;
  if (d < 7) return `${Math.floor(d)}d`;
  if (d < 35) return `${Math.floor(d / 7)}w`;
  if (d < 365) return `${Math.floor(d / 30.44)}mo`;
  return `${Math.floor(d / 365.25)}y`;
}

function fullDate(ts) {
  if (!ts) return "never";
  return new Date(ts * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function errText(e) {
  const s = typeof e === "string" ? e : e?.message ?? String(e);
  return s || "something went wrong.";
}

let toastTimer = 0;
function toast(msg) {
  els.toast.textContent = msg;
  els.toast.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => els.toast.classList.remove("show"), 2600);
}

/* ---------- data ---------- */

function sortedAccounts() {
  const list = [...(state?.accounts ?? [])];
  list.sort(
    (a, b) =>
      Number(b.pinned) - Number(a.pinned) ||
      b.timestamp - a.timestamp ||
      displayName(a).localeCompare(displayName(b))
  );
  return list;
}

function visibleAccounts() {
  const q = els.search.value.trim().toLowerCase();
  const all = sortedAccounts();
  if (!q) return all;
  return all.filter((a) =>
    [a.nickname, a.personaName, a.accountName, a.note].some((f) => f && f.toLowerCase().includes(q))
  );
}

async function refresh({ force = false } = {}) {
  let next;
  try {
    next = await invoke("get_state");
  } catch (e) {
    toast(errText(e));
    return;
  }
  const key = JSON.stringify(next);
  if (!force && key === stateKey) return;
  stateKey = key;
  state = next;
  requestRender();
}

function requestRender() {
  // Re-rendering under an open menu or dialog would yank it from the user.
  if (!els.menu.hidden || !els.scrim.hidden) {
    renderPending = true;
    return;
  }
  renderPending = false;
  render();
}

function loadAvatar(a) {
  if (!a.hasAvatar || avatars.has(a.steamId)) return;
  avatars.set(a.steamId, undefined);
  invoke("get_avatar", { steamId: a.steamId })
    .then((url) => {
      avatars.set(a.steamId, url || null);
      if (!url) return;
      document.querySelectorAll(`[data-avatar="${a.steamId}"]`).forEach((el) => {
        el.innerHTML = `<img src="${url}" alt="">`;
      });
    })
    .catch(() => avatars.set(a.steamId, null));
}

function avatarHtml(a) {
  const url = avatars.get(a.steamId);
  const inner = url ? `<img src="${url}" alt="">` : esc(initial(a));
  return `<div class="avatar" data-avatar="${esc(a.steamId)}">${inner}</div>`;
}

/* ---------- rendering ---------- */

const PIN_SVG = `<svg class="pin" data-tip="pinned" viewBox="0 0 16 16" aria-label="pinned"><path d="M6 2.5h4M7 2.5v4L4.5 9.5h7L9 6.5v-4M8 9.5V14"/></svg>`;
const DOTS_SVG = `<svg viewBox="0 0 16 16"><circle cx="3.5" cy="8" r="1.3"/><circle cx="8" cy="8" r="1.3"/><circle cx="12.5" cy="8" r="1.3"/></svg>`;

function rowHtml(a) {
  const subParts = [esc(a.accountName)];
  if (a.note) subParts.push(`<span class="note">${esc(a.note)}</span>`);
  else if (!a.rememberPassword) subParts.push("needs password");
  const status = a.signedIn
    ? `<span class="pill"><i></i>signed in</span>`
    : `<span data-tip="last signed in ${esc(fullDate(a.timestamp))}">${esc(ago(a.timestamp))}</span>`;
  const nameTip = a.nickname ? ` data-tip="steam name: ${esc(a.personaName || a.accountName)}"` : "";
  return `
    <div class="row${a.steamId === selectedId ? " selected" : ""}${a.signedIn ? " signed-in" : ""}"
         role="option" aria-selected="${a.steamId === selectedId}" data-id="${esc(a.steamId)}">
      ${avatarHtml(a)}
      <div class="info">
        <div class="name"><span${nameTip}>${esc(displayName(a))}</span></div>
        <div class="sub">${subParts.join(" · ")}</div>
      </div>
      <div class="meta">${a.pinned ? PIN_SVG : ""}${status}</div>
      <button class="more" data-more aria-label="More options for ${esc(displayName(a))}" aria-haspopup="menu" aria-expanded="false" data-tip="options" data-key="right-click">${DOTS_SVG}</button>
    </div>`;
}

function emptyHtml(big, text, button) {
  return `<div class="empty"><div class="big">${big}</div><p>${text}</p>${button ?? ""}</div>`;
}

function renderStatus() {
  if (!state?.steamDir) {
    els.status.innerHTML = "";
    return;
  }
  const signedIn = state.accounts.find((a) => a.signedIn);
  let html;
  if (signedIn) html = `<span class="dot on"></span><span class="label">signed in as <b>${esc(displayName(signedIn))}</b></span>`;
  else if (state.steamRunning) html = `<span class="dot on"></span><span class="label">steam is running</span>`;
  else html = `<span class="dot"></span><span class="label">steam is closed</span>`;
  const hidden = state.accounts.length - STEAM_PICKER_LIMIT;
  if (hidden > 0) html += `<span class="sep">·</span><span class="extra">+${hidden} hidden by steam</span>`;
  els.status.innerHTML = html;
}

function render() {
  hideTip();
  renderStatus();
  if (!state) return;

  const total = state.accounts.length;
  const shown = visibleAccounts();
  const query = els.search.value.trim();
  els.count.textContent = !state.steamDir || state.error ? "" : query ? `${shown.length} / ${total}` : String(total);

  let html = "";
  let listMode = false;
  if (!state.steamDir) {
    const where = {
      windows: "the one with steam.exe in it.",
      linux: "usually <code>~/.local/share/Steam</code> (press ctrl+h in the picker to see hidden folders).",
      macos: "usually <code>~/Library/Application Support/Steam</code>.",
    };
    html = emptyHtml(
      "steam not found",
      `unhidden couldn't find steam. point it to your steam folder, ${where[state.platform] ?? where.windows}`,
      `<button class="btn primary" data-action="locate">locate steam folder</button>`
    );
  } else if (state.error) {
    html = emptyHtml(
      "couldn't read your accounts",
      `${esc(state.error)}<br><br><code>${esc([state.steamDir, "config", "loginusers.vdf"].join(state.platform === "windows" ? "\\" : "/"))}</code>`,
      `<button class="btn" data-action="retry">try again</button>`
    );
  } else if (total === 0) {
    html = emptyHtml(
      "no saved accounts",
      "sign in to steam with “remember me” ticked and the account will show up here."
    );
  } else if (shown.length === 0) {
    html = emptyHtml("no matches", `nothing matches “${esc(query)}”.`);
  } else {
    listMode = true;
    const pinned = shown.filter((a) => a.pinned);
    const rest = shown.filter((a) => !a.pinned);
    if (pinned.length) {
      html += `<div class="group">pinned</div>` + pinned.map(rowHtml).join("");
      if (rest.length) html += `<div class="group">all</div>`;
    }
    html += rest.map(rowHtml).join("");
  }

  if (selectedId && !shown.some((a) => a.steamId === selectedId)) selectedId = null;
  els.list.innerHTML = html;
  els.footer.style.visibility = listMode ? "visible" : "hidden";
  shown.forEach(loadAvatar);
  if (revealId) {
    els.list.querySelector(`.row[data-id="${revealId}"]`)?.scrollIntoView({ block: "nearest" });
    revealId = null;
  }
}

function select(id, { scroll = true } = {}) {
  selectedId = id;
  els.list.querySelectorAll(".row").forEach((row) => {
    const on = row.dataset.id === id;
    row.classList.toggle("selected", on);
    row.setAttribute("aria-selected", String(on));
    if (on && scroll) row.scrollIntoView({ block: "nearest" });
  });
}

function moveSelection(delta) {
  const ids = visibleAccounts().map((a) => a.steamId);
  if (!ids.length) return;
  let i = ids.indexOf(selectedId);
  if (delta === Infinity) i = ids.length - 1;
  else if (delta === -Infinity) i = 0;
  else if (i === -1) i = delta > 0 ? 0 : ids.length - 1;
  else i = Math.min(ids.length - 1, Math.max(0, i + delta));
  select(ids[i]);
}

/* ---------- actions ---------- */

function showOverlay(a, stage) {
  els.overlayAvatar.innerHTML = avatarHtml(a);
  els.overlayName.textContent = displayName(a);
  setStage(stage);
  els.overlayStage.classList.remove("error");
  els.overlay.querySelector(".bar").className = "bar";
  els.overlayOk.hidden = true;
  els.overlay.classList.remove("leaving");
  els.overlay.hidden = false;
}

function setStage(text) {
  els.overlayStage.textContent = text;
}

async function hideOverlay(delay = 0) {
  if (delay) await sleep(delay);
  els.overlay.classList.add("leaving");
  await sleep(250);
  els.overlay.hidden = true;
  els.overlay.classList.remove("leaving");
}

function overlayError(message) {
  els.overlayStage.textContent = message;
  els.overlayStage.classList.add("error");
  els.overlay.querySelector(".bar").className = "bar hide";
  els.overlayOk.hidden = false;
  els.overlayOk.focus();
}

async function switchTo(id, offline) {
  const a = findAccount(id);
  if (!a || busy) return;
  if (a.signedIn && !offline) {
    toast(`already signed in as ${displayName(a)}`);
    return;
  }
  busy = true;
  closeMenu();
  showOverlay(a, offline ? "going offline" : "switching");
  try {
    await invoke("switch_account", { steamId: id, offline });
    setStage(offline ? "steam is starting in offline mode" : "steam is starting");
    els.overlay.querySelector(".bar").className = "bar done";
    fastPollUntil = Date.now() + 45000;
    revealId = id;
    await hideOverlay(1000);
    busy = false;
    refresh({ force: true });
  } catch (e) {
    overlayError(errText(e));
    refresh();
  }
}

async function togglePin(id) {
  const a = findAccount(id);
  if (!a) return;
  await saveMeta(a, { pinned: !a.pinned });
  toast(a.pinned ? "unpinned" : "pinned to top");
}

async function saveMeta(a, patch) {
  const meta = { pinned: a.pinned, nickname: a.nickname, note: a.note, ...patch };
  try {
    await invoke("update_meta", { steamId: a.steamId, meta });
    await refresh({ force: true });
    return true;
  } catch (e) {
    toast(errText(e));
    return false;
  }
}

async function forget(id) {
  const a = findAccount(id);
  if (!a || busy) return;
  busy = true;
  const slow = state.steamRunning;
  if (slow) showOverlay(a, "forgetting");
  try {
    await invoke("forget_account", { steamId: id });
    avatars.delete(id);
    if (slow) await hideOverlay();
    busy = false;
    toast(`forgot ${displayName(a)}`);
    refresh({ force: true });
  } catch (e) {
    if (slow) overlayError(errText(e));
    else {
      busy = false;
      toast(errText(e));
    }
    refresh();
  }
}

async function locateSteam() {
  try {
    if (await invoke("pick_steam_folder")) {
      toast("found steam");
      await refresh({ force: true });
    }
  } catch (e) {
    toast(errText(e));
  }
}

/* ---------- menu ---------- */

function openMenu(id, pos) {
  const a = findAccount(id);
  if (!a) return;
  closeMenu();
  menuFor = id;
  select(id, { scroll: false });
  menuItems = [
    { label: a.signedIn ? "signed in" : "switch", kbd: "↵", run: () => switchTo(id, false), disabled: a.signedIn },
    { label: "switch offline", kbd: "⇧↵", run: () => switchTo(id, true) },
    null,
    { label: a.pinned ? "unpin" : "pin to top", kbd: "P", run: () => togglePin(id) },
    { label: "nickname & note", kbd: "E", run: () => openEdit(id) },
    null,
    { label: "forget account…", kbd: "del", run: () => openForget(id), danger: true },
  ];
  els.menu.innerHTML = menuItems
    .map((it, i) =>
      it
        ? `<button class="item${it.danger ? " danger" : ""}" role="menuitem" data-index="${i}"${it.disabled ? " disabled" : ""}>
             <span>${esc(it.label)}</span><kbd>${esc(it.kbd)}</kbd></button>`
        : "<hr>"
    )
    .join("");
  menuFocus = -1;
  els.menu.hidden = false;

  const r = els.menu.getBoundingClientRect();
  const vw = window.innerWidth, vh = window.innerHeight, m = 8;
  let x, y;
  if (pos.anchor) {
    const b = pos.anchor.getBoundingClientRect();
    x = b.right - r.width;
    y = b.bottom + 4 + r.height > vh - m ? b.top - 4 - r.height : b.bottom + 4;
    pos.anchor.setAttribute("aria-expanded", "true");
  } else {
    x = pos.x;
    y = pos.y + r.height > vh - m ? pos.y - r.height : pos.y;
  }
  els.menu.style.left = `${Math.max(m, Math.min(x, vw - r.width - m))}px`;
  els.menu.style.top = `${Math.max(m, Math.min(y, vh - r.height - m))}px`;
}

function closeMenu() {
  if (els.menu.hidden) return;
  els.menu.hidden = true;
  menuFor = null;
  els.list.querySelectorAll('[aria-expanded="true"]').forEach((b) => b.setAttribute("aria-expanded", "false"));
  if (renderPending) requestRender();
}

function focusMenuItem(delta) {
  const usable = menuItems.map((it, i) => (it && !it.disabled ? i : -1)).filter((i) => i >= 0);
  let pos = usable.indexOf(menuFocus);
  pos = pos === -1 ? (delta > 0 ? 0 : usable.length - 1) : (pos + delta + usable.length) % usable.length;
  menuFocus = usable[pos];
  els.menu.querySelectorAll(".item").forEach((el) => el.classList.toggle("focus", Number(el.dataset.index) === menuFocus));
}

function runMenuItem(index) {
  const it = menuItems[index];
  closeMenu();
  if (it && !it.disabled) it.run();
}

els.menu.addEventListener("click", (e) => {
  const item = e.target.closest(".item");
  if (item) runMenuItem(Number(item.dataset.index));
});

/* ---------- dialogs ---------- */

function openDialog(html, mount) {
  closeMenu();
  els.dialog.innerHTML = html;
  els.scrim.hidden = false;
  dialogCleanup = mount?.(els.dialog) ?? null;
}

function closeDialog() {
  if (els.scrim.hidden) return;
  els.scrim.hidden = true;
  els.dialog.innerHTML = "";
  dialogCleanup = null;
  if (renderPending) requestRender();
}

els.scrim.addEventListener("mousedown", (e) => {
  if (e.target === els.scrim) closeDialog();
});
els.dialog.addEventListener("click", (e) => {
  if (e.target.closest("[data-close]")) closeDialog();
});

function openEdit(id) {
  const a = findAccount(id);
  if (!a) return;
  openDialog(
    `<h2>${esc(a.personaName || a.accountName)}</h2>
     <p>only you see these. they're saved in unhidden, not on steam.</p>
     <label class="field"><span>nickname</span><input id="f-nick" maxlength="40" spellcheck="false"></label>
     <label class="field"><span>note</span><textarea id="f-note" rows="3" maxlength="200" spellcheck="false" placeholder="e.g. cs2 smurf, trading, main"></textarea></label>
     <div class="actions"><button class="btn" data-close>cancel</button><button class="btn primary" id="f-save">save</button></div>`,
    (d) => {
      const nick = d.querySelector("#f-nick");
      const note = d.querySelector("#f-note");
      const save = d.querySelector("#f-save");
      nick.placeholder = a.personaName || a.accountName;
      nick.value = a.nickname;
      note.value = a.note;
      const submit = async () => {
        save.disabled = true;
        const ok = await saveMeta(a, { nickname: nick.value.trim(), note: note.value.trim() });
        if (ok) {
          closeDialog();
          toast("saved");
        } else {
          save.disabled = false;
        }
      };
      save.addEventListener("click", submit);
      nick.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          submit();
        }
      });
      note.addEventListener("keydown", (e) => {
        if (e.key === "Enter" && (e.ctrlKey || !e.shiftKey)) {
          e.preventDefault();
          submit();
        }
      });
      nick.focus();
      nick.select();
    }
  );
}

function openForget(id) {
  const a = findAccount(id);
  if (!a) return;
  openDialog(
    `<h2>forget ${esc(displayName(a))}?</h2>
     <p>this removes <b>${esc(a.accountName)}</b> from steam's saved accounts on this pc. you'll need its password to sign in again.</p>
     ${state.steamRunning ? "<p>steam will restart to apply this.</p>" : ""}
     <div class="actions"><button class="btn" data-close>cancel</button><button class="btn primary" id="f-forget">forget</button></div>`,
    (d) => {
      d.querySelector("#f-forget").addEventListener("click", () => {
        closeDialog();
        forget(id);
      });
      d.querySelector("[data-close]").focus();
    }
  );
}

/* ---------- list events ---------- */

els.list.addEventListener("click", (e) => {
  const action = e.target.closest("[data-action]")?.dataset.action;
  if (action === "locate") return locateSteam();
  if (action === "retry") return refresh({ force: true });

  const row = e.target.closest(".row");
  if (!row) return;
  const id = row.dataset.id;
  const more = e.target.closest("[data-more]");
  if (more) {
    if (menuFor === id) closeMenu();
    else openMenu(id, { anchor: more });
    return;
  }
  select(id, { scroll: false });
  switchTo(id, e.shiftKey);
});

els.list.addEventListener("contextmenu", (e) => {
  const row = e.target.closest(".row");
  if (!row) return;
  e.preventDefault();
  openMenu(row.dataset.id, { x: e.clientX, y: e.clientY });
});

// Close on user scrolling only; a scroll that is still settling must not close a menu that just opened.
els.list.addEventListener("wheel", closeMenu, { passive: true });

els.search.addEventListener("input", () => {
  selectedId = null;
  render();
  els.list.scrollTop = 0;
});

/* ---------- window chrome ---------- */

$("btn-min").addEventListener("click", () => appWindow.minimize());
$("btn-close").addEventListener("click", () => appWindow.close());
els.refresh.addEventListener("click", () => manualRefresh());

function manualRefresh() {
  els.refresh.classList.remove("spinning");
  void els.refresh.offsetWidth;
  els.refresh.classList.add("spinning");
  avatars.clear();
  refresh({ force: true });
}

els.overlayOk.addEventListener("click", () => {
  hideOverlay();
  busy = false;
});

/* ---------- tooltips ---------- */

const tip = $("tip");
let tipFor = null;
let tipShown = false;
let tipTimer = 0;
let tipWarmUntil = 0;

function showTip(el) {
  tip.innerHTML = `<span>${esc(el.dataset.tip)}</span>${el.dataset.key ? `<kbd>${esc(el.dataset.key)}</kbd>` : ""}`;
  tip.classList.remove("show");
  const a = el.getBoundingClientRect();
  const t = tip.getBoundingClientRect();
  const m = 6;
  const x = Math.max(m, Math.min(a.left + a.width / 2 - t.width / 2, window.innerWidth - t.width - m));
  const above = a.bottom + 6 + t.height > window.innerHeight - m;
  tip.classList.toggle("above", above);
  tip.style.left = `${x}px`;
  tip.style.top = `${above ? a.top - 6 - t.height : a.bottom + 6}px`;
  void tip.offsetWidth;
  tip.classList.add("show");
  tipShown = true;
}

function hideTip() {
  clearTimeout(tipTimer);
  // Moving straight to another button shows its tip without waiting again.
  if (tipShown) tipWarmUntil = Date.now() + 500;
  tipShown = false;
  tipFor = null;
  tip.classList.remove("show");
}

document.addEventListener("mouseover", (e) => {
  const el = e.target.closest?.("[data-tip]") ?? null;
  if (el === tipFor) return;
  hideTip();
  if (!el || !els.menu.hidden || !els.scrim.hidden || !els.overlay.hidden) return;
  tipFor = el;
  tipTimer = setTimeout(() => {
    if (tipFor === el && el.isConnected) showTip(el);
  }, Date.now() < tipWarmUntil ? 0 : 450);
});
document.addEventListener("mouseout", (e) => {
  if (tipFor && !tipFor.contains(e.relatedTarget)) hideTip();
});
for (const ev of ["mousedown", "wheel", "keydown"]) document.addEventListener(ev, hideTip, { capture: true, passive: true });
window.addEventListener("blur", hideTip);

/* ---------- keyboard ---------- */

const isTyping = (el) => el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA");

document.addEventListener("keydown", (e) => {
  const key = e.key;
  const ctrl = e.ctrlKey || e.metaKey;

  // Browser shortcuts that make no sense in an app.
  if (
    key === "F12" || key === "F3" || key === "F7" ||
    (ctrl && ["p", "u", "g", "j", "h", "s", "o", "n", "t", "w", "+", "-", "=", "0"].includes(key.toLowerCase())) ||
    (ctrl && e.shiftKey && ["i", "j", "c"].includes(key.toLowerCase())) ||
    (e.altKey && (key === "ArrowLeft" || key === "ArrowRight"))
  ) {
    e.preventDefault();
    return;
  }

  if (!els.overlay.hidden) {
    e.preventDefault();
    if (!els.overlayOk.hidden && (key === "Enter" || key === "Escape" || key === " ")) els.overlayOk.click();
    return;
  }

  if (!els.scrim.hidden) {
    if (key === "Escape") {
      e.preventDefault();
      closeDialog();
    } else if (key === "Tab") {
      const f = [...els.dialog.querySelectorAll("input, textarea, button:not([disabled])")];
      if (!f.length) return;
      const i = f.indexOf(document.activeElement);
      if (e.shiftKey && i <= 0) {
        e.preventDefault();
        f[f.length - 1].focus();
      } else if (!e.shiftKey && i === f.length - 1) {
        e.preventDefault();
        f[0].focus();
      }
    } else if (ctrl && (key === "r" || key === "f")) {
      e.preventDefault();
    } else if (key === "F5") {
      e.preventDefault();
    }
    return;
  }

  if (!els.menu.hidden) {
    e.preventDefault();
    if (key === "ArrowDown" || key === "Tab") focusMenuItem(e.shiftKey && key === "Tab" ? -1 : 1);
    else if (key === "ArrowUp") focusMenuItem(-1);
    else if ((key === "Enter" || key === " ") && menuFocus >= 0) runMenuItem(menuFocus);
    else closeMenu();
    return;
  }

  if (key === "F5" || (ctrl && key.toLowerCase() === "r")) {
    e.preventDefault();
    manualRefresh();
    return;
  }
  if ((ctrl && key.toLowerCase() === "f") || (key === "/" && !isTyping(document.activeElement))) {
    e.preventDefault();
    els.search.focus();
    els.search.select();
    return;
  }

  const moves = { ArrowDown: 1, ArrowUp: -1, PageDown: 5, PageUp: -5 };
  if (key in moves) {
    e.preventDefault();
    moveSelection(moves[key]);
    return;
  }
  if ((key === "Home" || key === "End") && !isTyping(document.activeElement)) {
    e.preventDefault();
    moveSelection(key === "Home" ? -Infinity : Infinity);
    return;
  }

  if (key === "Enter") {
    e.preventDefault();
    if (!selectedId) {
      const first = visibleAccounts()[0];
      if (els.search.value.trim() && first) select(first.steamId);
    }
    if (selectedId) switchTo(selectedId, e.shiftKey);
    return;
  }

  if (key === "Escape") {
    e.preventDefault();
    if (els.search.value) {
      els.search.value = "";
      selectedId = null;
      render();
    } else {
      els.search.blur();
      select(null);
    }
    return;
  }

  if ((key === "ContextMenu" || (e.shiftKey && key === "F10")) && selectedId) {
    e.preventDefault();
    const row = els.list.querySelector(`.row[data-id="${selectedId}"]`);
    if (row) {
      const more = row.querySelector("[data-more]");
      openMenu(selectedId, { anchor: more });
      focusMenuItem(1);
    }
    return;
  }

  if (isTyping(document.activeElement) || ctrl || e.altKey) return;

  if (selectedId && key.toLowerCase() === "p") {
    e.preventDefault();
    togglePin(selectedId);
  } else if (selectedId && key.toLowerCase() === "e") {
    e.preventDefault();
    openEdit(selectedId);
  } else if (selectedId && key === "Delete") {
    e.preventDefault();
    openForget(selectedId);
  } else if (key.length === 1 && key !== " ") {
    // Start typing anywhere to search.
    els.search.focus();
  }
});

// No browser context menu, zoom or file drops; this is an app, not a web page.
document.addEventListener("contextmenu", (e) => {
  if (!isTyping(e.target)) e.preventDefault();
});
document.addEventListener("wheel", (e) => { if (e.ctrlKey) e.preventDefault(); }, { passive: false });
document.addEventListener("dragover", (e) => e.preventDefault());
document.addEventListener("drop", (e) => e.preventDefault());
document.addEventListener("mousedown", (e) => {
  if (!els.menu.hidden && !els.menu.contains(e.target) && !e.target.closest("[data-more]")) closeMenu();
});
window.addEventListener("blur", closeMenu);
window.addEventListener("resize", closeMenu);
window.addEventListener("focus", () => refresh());

/* ---------- startup ---------- */

let slowTimer = 0;
listen("progress", (e) => {
  clearTimeout(slowTimer);
  if (els.overlay.hidden || !STAGES[e.payload]) return;
  setStage(STAGES[e.payload]);
  if (e.payload === "closing") {
    // Steam can take a while to close (or hang); after 15 s it is closed by force.
    slowTimer = setTimeout(() => {
      if (!els.overlay.hidden && els.overlayStage.textContent === STAGES.closing) {
        setStage("steam is slow to close, hang on");
      }
    }, 5000);
  }
});

async function poll() {
  if (document.visibilityState === "visible" && !busy) await refresh();
  setTimeout(poll, Date.now() < fastPollUntil ? 1500 : 4000);
}

refresh({ force: true }).then(() => setTimeout(poll, 4000));
