"use strict";

// ============================================================
// 云盘 WebDisk 前端 (web-v2 重构版)
// 交互:
//   单击 = 勾选;双击 = 打开;右键 = 菜单;回车 = 打开选中;
//   悬停文件卡片 = 快捷操作;拖拽文件到窗口 = 上传到当前目录;
//   工具栏搜索框 = 过滤当前目录文件;Delete = 删除选中。
// ============================================================

// ---------------- 工具函数 ----------------

const esc = (s) =>
  String(s == null ? "" : s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

const TOKEN_KEY = "wp_token";
const COOKIE_KEY = "wp_token";

function setTokenCookie(token) {
  document.cookie = `${COOKIE_KEY}=${token}; path=/; SameSite=Lax`;
}
function clearTokenCookie() {
  document.cookie = `${COOKIE_KEY}=; path=/; max-age=0`;
}

const state = {
  token: localStorage.getItem(TOKEN_KEY) || "",
  user: null,
  shares: [],
  share: null,
  path: "",
  can: {},
  view: localStorage.getItem("wp_view") || "grid",
  sort: localStorage.getItem("wp_sort") || "name",
  filter: "",
  lastEntries: [],
  sel: new Set(),
  selDir: new Set(),
};

async function api(method, path, body) {
  const headers = {};
  if (state.token) headers["Authorization"] = "Bearer " + state.token;
  let payload;
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    payload = JSON.stringify(body);
  }
  const resp = await fetch(path, { method, headers, body: payload });
  const ct = resp.headers.get("content-type") || "";
  const data = ct.includes("json") ? await resp.json().catch(() => null) : await resp.text().catch(() => null);
  if (!resp.ok) {
    const e = new Error(data && data.error ? data.error : "请求失败 (" + resp.status + ")");
    e.status = resp.status;
    throw e;
  }
  return data;
}

function toast(msg, ok) {
  let box = document.getElementById("toasts");
  if (!box) {
    box = document.createElement("div");
    box.id = "toasts";
    document.body.appendChild(box);
  }
  const t = document.createElement("div");
  t.className = "toast " + (ok ? "ok" : "err");
  t.textContent = msg;
  box.appendChild(t);
  setTimeout(() => t.remove(), 3200);
}

function formatSize(n) {
  if (n == null) return "";
  if (n < 1024) return n + " B";
  const u = ["KB", "MB", "GB", "TB"];
  let v = n;
  let i = -1;
  do {
    v /= 1024;
    i++;
  } while (v >= 1024 && i < u.length - 1);
  return v.toFixed(1) + " " + u[i];
}

function formatTime(ts) {
  if (!ts) return "—";
  const d = new Date(ts * 1000);
  const p = (n) => String(n).padStart(2, "0");
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  if (sameDay) return `今天 ${p(d.getHours())}:${p(d.getMinutes())}`;
  const yest = new Date(now.getTime() - 86400000);
  if (d.toDateString() === yest.toDateString()) return `昨天 ${p(d.getHours())}:${p(d.getMinutes())}`;
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

// ---------------- 文件图标(SVG,按格式分类) ----------------

const FICO_FOLDER = `<svg class="fico" viewBox="0 0 64 64" xmlns="http://www.w3.org/2000/svg"><path d="M10 21.2a3.4 3.4 0 0 1 3.4-3.4h11.6l2.8 3.1h23.8a3.4 3.4 0 0 1 3.4 3.4v1.8H10z" fill="#f59e0b"/><path d="M10 24.6h44v17.2a3.4 3.4 0 0 1-3.4 3.4H13.4a3.4 3.4 0 0 1-3.4-3.4z" fill="#fbbf24"/><path d="M10 32.5h44" stroke="#fcd34d" stroke-width="2"/></svg>`;
const FICO_IMG = `<circle cx="25" cy="46.6" r="2.5" fill="#fff"/><path d="M19.5 53.4 26.5 46.4 30.8 50.7 34.2 47.3 41 53.4Z" fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round"/>`;
const FICO_VID = `<path d="M21 45.6a2 2 0 0 1 2-2h18a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H23a2 2 0 0 1-2-2z" fill="none" stroke="#fff" stroke-width="2"/><path d="M30.2 46.7v5.4l4.6-2.7z" fill="#fff"/>`;
const FICO_MUS = `<circle cx="28.2" cy="49.7" r="2.9" fill="#fff"/><rect x="29.8" y="43.8" width="2" height="6.4" fill="#fff"/><path d="M31.6 44.1c2.4-.2 4.3 1.2 4.5 3h-4.1z" fill="#fff"/>`;
const FICO_ZIP = `<rect x="20.5" y="45.6" width="10.5" height="8.8" rx="1.5" fill="none" stroke="#fff" stroke-width="1.8"/><rect x="33" y="45.6" width="10.5" height="8.8" rx="1.5" fill="none" stroke="#fff" stroke-width="1.8"/><rect x="31.1" y="46.5" width="1.8" height="7.2" fill="#fff"/><path d="M33.2 48.8h2.6M33.2 51.6h2.6" stroke="#fff" stroke-width="1.5"/>`;
const FICO_LINES = `<path d="M21 47.7h24M21 51.4h17" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/>`;

function ficoPage(color, tag, glyph) {
  const longTag = tag && tag.length > 2;
  const label = tag
    ? `<text x="32" y="${longTag ? 50.5 : 51.3}" text-anchor="middle" font-family="Arial, Helvetica, sans-serif" font-weight="700" font-size="${longTag ? 8 : 11.5}" fill="#fff" letter-spacing="0.4">${tag}</text>`
    : (glyph || "");
  return `<svg class="fico" viewBox="0 0 64 64" xmlns="http://www.w3.org/2000/svg"><path d="M15 5h24l10 10v44a2 2 0 0 1-2 2H15a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z" fill="#fff" stroke="#d9dfee" stroke-width="1.6"/><path d="M39 5v10h10z" fill="#eef1fb" stroke="#d9dfee" stroke-width="1.2"/><rect x="16" y="42.5" width="32" height="11.5" rx="3" fill="${color}"/>${label}</svg>`;
}

const FICO_GROUPS = [
  [["pdf"], ficoPage("#ef4444", "PDF")],
  [["doc", "docx", "docm", "odt", "rtf"], ficoPage("#2563eb", "W")],
  [["xls", "xlsx", "xlsm", "ods", "csv", "tsv"], ficoPage("#16a34a", "X")],
  [["ppt", "pptx", "odp"], ficoPage("#ea580c", "P")],
  [["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "avif", "ico", "tif", "tiff", "heic"], ficoPage("#8b5cf6", null, FICO_IMG)],
  [["mp4", "m4v", "webm", "ogv", "mov", "avi", "mkv", "flv", "wmv", "mpg", "mpeg", "ts", "3gp", "m2ts", "m2ts"], ficoPage("#7c3aed", null, FICO_VID)],
  [["mp3", "wav", "ogg", "oga", "flac", "m4a", "aac", "opus", "wma", "amr"], ficoPage("#0d9488", null, FICO_MUS)],
  [["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"], ficoPage("#d97706", null, FICO_ZIP)],
  [["txt", "log", "md", "markdown", "json", "yaml", "yml", "ini", "conf", "toml", "sh", "bash", "py", "js", "mjs", "ts", "html", "htm", "css", "xml", "sql", "properties", "env"], ficoPage("#64748b", null, FICO_LINES)],
];

function fileIco(x) {
  if (x.is_dir) return FICO_FOLDER;
  const e = (x.ext || "").toLowerCase();
  for (const [exts, svg] of FICO_GROUPS) {
    if (exts.includes(e)) return svg;
  }
  return ficoPage("#9ca3af", "");
}

function kindOf(x) {
  if (x.is_dir) return "dir";
  const e = x.ext;
  if (["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "avif", "ico"].includes(e)) return "image";
  if (["mp4", "m4v", "webm", "ogv", "mov", "avi", "mkv", "flv", "wmv", "mpg", "mpeg", "ts", "3gp", "m2ts"].includes(e)) return "video";
  if (["mp3", "wav", "ogg", "oga", "flac", "m4a", "aac", "opus", "wma"].includes(e)) return "audio";
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz"].includes(e)) return "archive";
  return "doc";
}

function accessName(a) {
  if (a === "guest") return "游客可访问";
  if (a === "login") return "登录可访问";
  if (a === "admin") return "仅管理员";
  return a || "登录";
}

function childPath(name) {
  return state.path ? state.path + "/" + name : name;
}

function mediaUrl(shareId, path, extra) {
  let u = `/api/media/${shareId}?path=${encodeURIComponent(path)}`;
  if (extra) u += "&" + extra;
  return u;
}

function thumbUrl(shareId, path) {
  return `/api/media/thumb/${shareId}?path=${encodeURIComponent(path)}`;
}

window.thumbErr = (img) => {
  const n = parseInt(img.dataset.tr || "0", 10);
  if (n <= 0) {
    img.closest(".thumb").classList.add("fail");
    return;
  }
  img.dataset.tr = String(n - 1);
  const wait = n > 2 ? 1200 : n > 1 ? 3000 : 6000;
  setTimeout(() => {
    if (img.isConnected) {
      img.src = img.dataset.src + (img.dataset.src.includes("?") ? "&" : "?") + "r=" + Date.now();
    }
  }, wait);
};

function download(shareId, path, name) {
  const a = document.createElement("a");
  a.href = mediaUrl(shareId, path, "dl=1");
  a.download = name || "";
  a.className = "dl-anchor";
  document.body.appendChild(a);
  a.click();
  setTimeout(() => a.remove(), 500);
}

async function copyText(text, okMsg) {
  try {
    await navigator.clipboard.writeText(text);
    toast(okMsg || "已复制", true);
    return;
  } catch (e) {}
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    toast(okMsg || "已复制", true);
    ta.remove();
    return;
  } catch (e2) {}
  toast("复制失败,请手动复制: " + text);
}

// ---------------- 登录 / 启动 ----------------

function renderLogin() {
  document.getElementById("app").innerHTML = `
    <div class="login-wrap">
      <div class="login-card">
        <div class="login-logo">云</div>
        <h1>云盘</h1>
        <div class="sub">多用户 · 断点续传 · 在线播放 · 安全分享</div>
        <label>用户名</label>
        <div class="fld"><span class="f-ico">👤</span><input id="in-user" placeholder="请输入用户名" autocomplete="username"></div>
        <label>密码</label>
        <div class="fld"><span class="f-ico">🔒</span><input id="in-pass" type="password" placeholder="请输入密码" autocomplete="current-password"></div>
        <label>验证码</label>
        <div class="fld"><span class="f-ico">🗝️</span><input id="in-captcha" placeholder="输入右侧字符" maxlength="4" autocomplete="off"></div>
        <div class="captcha-row">
          <img id="cap-img" alt="验证码" title="看不清?点击刷新">
          <input type="hidden" id="cap-id">
          <button type="button" class="btn mini" id="cap-refresh">换一张</button>
        </div>
        <button class="btn full" id="btn-login">登 录</button>
        <div class="err" id="login-err"></div>
      </div>
    </div>`;
  loadCaptcha();
  document.getElementById("btn-login").onclick = doLogin;
  document.getElementById("in-captcha").addEventListener("keydown", (e) => {
    if (e.key === "Enter") doLogin();
  });
  document.getElementById("in-pass").addEventListener("keydown", (e) => {
    if (e.key === "Enter") doLogin();
  });
  document.getElementById("in-user").addEventListener("keydown", (e) => {
    if (e.key === "Enter") document.getElementById("in-pass").focus();
  });
  document.getElementById("cap-img").onclick = loadCaptcha;
  document.getElementById("cap-refresh").onclick = loadCaptcha;
}

async function loadCaptcha() {
  const img = document.getElementById("cap-img");
  try {
    const r = await api("GET", "/api/captcha");
    document.getElementById("cap-id").value = r.id;
    img.src = "data:image/svg+xml;base64," + btoa(r.svg);
    img.style.opacity = 1;
  } catch (e) {
    img.style.opacity = 0.4;
  }
}

async function doLogin() {
  const u = document.getElementById("in-user").value.trim();
  const p = document.getElementById("in-pass").value;
  const capId = document.getElementById("cap-id").value;
  const cap = document.getElementById("in-captcha").value.trim();
  try {
    const r = await api("POST", "/api/auth/login", {
      username: u,
      password: p,
      captcha_id: capId,
      captcha: cap,
    });
    state.token = r.token;
    state.user = r.user;
    localStorage.setItem(TOKEN_KEY, r.token);
    setTokenCookie(r.token);
    await boot();
  } catch (e) {
    document.getElementById("login-err").textContent = e.message;
    document.getElementById("in-captcha").value = "";
    loadCaptcha();
  }
}

function logout() {
  api("POST", "/api/auth/logout").catch(() => {});
  localStorage.removeItem(TOKEN_KEY);
  clearTokenCookie();
  state.token = "";
  state.user = null;
  state.shares = [];
  state.share = null;
  renderLogin();
}

async function boot() {
  if (state.token) {
    try {
      state.user = await api("GET", "/api/auth/me");
    } catch (e) {
      localStorage.removeItem(TOKEN_KEY);
      state.token = "";
      clearTokenCookie();
      state.user = null;
      state.shares = [];
      state.share = null;
    }
  }
  if (!state.token) {
    try {
      const r = await api("GET", "/api/shares");
      if (r.shares && r.shares.length) {
        state.shares = r.shares;
        state.share = state.shares[0];
        state.path = "";
        render();
        return;
      }
    } catch (e) {}
    return renderLogin();
  }
  await reloadShares();
  if (state.shares.length) state.share = state.shares[0];
  state.path = "";
  render();
}

async function reloadShares() {
  const r = await api("GET", "/api/shares");
  state.shares = r.shares || [];
}

// ---------------- 外壳:侧边栏 + 主区 ----------------

function render() {
  document.getElementById("app").innerHTML = `
  <div class="shell">
    <div class="sidebar">
      <div class="brand">
        <span class="brand-mark">云</span>
        <span class="brand-name">云盘</span>
        ${state.user && state.user.is_admin ? '<span class="me">管理员</span>' : ""}
      </div>
      <div class="side-title">我的空间</div>
      <div class="share-list" id="shareList"></div>
      <div class="foot">
        <span class="who">
          <span class="who-avatar">${state.user ? esc(state.user.username.slice(0, 1).toUpperCase()) : "客"}</span>
          ${state.user ? esc(state.user.username) : "游客模式"}
        </span>
        <div class="foot-btns">
          ${state.user && state.user.is_admin ? '<button class="btn mini ghost" id="btn-admin">⚙ 管理后台</button>' : ""}
          ${state.user ? '<button class="btn mini ghost" id="btn-logout">退出</button>' : '<button class="btn mini ghost" id="btn-login2">登 录</button>'}
        </div>
      </div>
    </div>
    <div class="main">
      <div class="toolbar" id="toolbar"></div>
      <div id="selbar" class="selbar hidden"></div>
      <div class="files" id="files"></div>
    </div>
  </div>
  <div class="upload-panel hidden" id="uploadPanel"></div>`;
  const lo = document.getElementById("btn-logout");
  if (lo) lo.onclick = logout;
  const l2 = document.getElementById("btn-login2");
  if (l2) l2.onclick = renderLogin;
  const ab = document.getElementById("btn-admin");
  if (ab) ab.onclick = () => renderAdmin();
  renderShareList();
  loadFiles();
  bindDropUpload();
}

function renderShareList() {
  const el = document.getElementById("shareList");
  if (!state.shares.length) {
    el.innerHTML = '<div style="padding:14px;color:var(--muted);font-size:13px">无可用空间</div>';
    return;
  }
  el.innerHTML = state.shares
    .map(
      (s) => `
      <div class="share-item ${state.share && state.share.id === s.id ? "active" : ""}" data-share="${s.id}">
        <span class="ico">${s.kind === "home" ? "🏠" : "📂"}</span>
        <span>${esc(s.name)}</span>
        <span class="share-badge">${s.kind === "home" ? "我的" : accessName(s.access)}</span>
      </div>`
    )
    .join("");
  el.querySelectorAll(".share-item").forEach((node) => {
    node.onclick = () => {
      const s = state.shares.find((x) => x.id == node.dataset.share);
      if (s) {
        state.share = s;
        state.path = "";
        state.filter = "";
        loadFiles();
      }
    };
  });
}

function renderToolbar() {
  const parts = state.path ? state.path.split("/") : [];
  let crumb = '<a class="crumbs" data-path="" href="#">根目录</a>';
  let acc = "";
  parts.forEach((p) => {
    acc = acc ? acc + "/" + p : p;
    crumb += ` <span class="sp">›</span> <a class="crumbs" data-path="${esc(acc)}" href="#${esc(acc)}">${esc(p)}</a>`;
  });
  const can = state.can;
  const adm = state.user && state.user.is_admin;
  document.getElementById("toolbar").innerHTML = `
    <div class="crumb">${crumb}</div>
    <input id="Fsearch" class="search-box" placeholder="搜索当前文件夹…" value="${esc(state.filter)}">
    <div class="spacer"></div>
    <div class="view-switch" title="切换视图">
      <button class="btn mini ghost view-btn ${state.view === "grid" ? "active" : ""}" data-view="grid">▦</button>
      <button class="btn mini ghost view-btn ${state.view === "list" ? "active" : ""}" data-view="list">☰</button>
    </div>
    <select id="sort-by" class="sort-select" title="排序">
      <option value="name" ${state.sort === "name" ? "selected" : ""}>按名称</option>
      <option value="size" ${state.sort === "size" ? "selected" : ""}>按大小</option>
      <option value="mtime" ${state.sort === "mtime" ? "selected" : ""}>按修改时间</option>
      <option value="kind" ${state.sort === "kind" ? "selected" : ""}>按类型</option>
    </select>
    ${can.mkdir ? '<button class="btn ghost mini" id="btn-mkdir">＋ 新建文件夹</button>' : ""}
    ${can.upload ? '<button class="btn btn-upload" id="btn-upload">⬆ 上传文件</button><input type="file" id="file-input" multiple style="display:none">' : ""}
    ${adm ? '<button class="btn mini ghost" id="btn-perm">🔐 权限</button>' : ""}
    <button class="btn mini ghost" id="btn-refresh" title="刷新">↻</button>`;
  document.querySelectorAll(".crumbs").forEach((a) => {
    a.onclick = (e) => {
      e.preventDefault();
      state.path = a.dataset.path || "";
      const fi = document.getElementById("Fsearch");
      if (fi) fi.value = "";
      state.filter = "";
      loadFiles();
    };
  });
  document.querySelectorAll(".view-btn").forEach((b) => {
    b.onclick = () => {
      state.view = b.dataset.view;
      localStorage.setItem("wp_view", state.view);
      renderToolbar();
      renderFiles(state.lastEntries);
    };
  });
  document.getElementById("sort-by").onchange = (e) => {
    state.sort = e.target.value;
    localStorage.setItem("wp_sort", state.sort);
    renderFiles(state.lastEntries);
  };
  const fi = document.getElementById("Fsearch");
  if (fi) {
    fi.oninput = () => {
      state.filter = fi.value.trim();
      renderFiles(state.lastEntries);
    };
  }
  const mk = document.getElementById("btn-mkdir");
  if (mk) mk.onclick = mkdirModal;
  const up = document.getElementById("btn-upload");
  if (up) up.onclick = () => { const i = ensureFileInput(); i.click(); };
  const pm = document.getElementById("btn-perm");
  if (pm) pm.onclick = openFolderPerm;
  document.getElementById("btn-refresh").onclick = loadFiles;

  const hiddenBound = document.getElementById("file-input");
  if (hiddenBound) hiddenBound.onchange = onPickFiles;
}

function ensureFileInput() {
  let i = document.getElementById("file-input");
  if (!i) {
    i = document.createElement("input");
    i.type = "file";
    i.id = "file-input";
    i.multiple = true;
    i.style.display = "none";
    document.body.appendChild(i);
  }
  i.onchange = onFileInputChange;
  return i;
}
function onFileInputChange(ev) {
  onPickFiles(ev);
  document.getElementById("file-input").value = "";
}

function bindDropUpload() {
  const el = document.getElementById("files");
  if (!el) return;
  el.addEventListener("dragover", (ev) => {
    ev.preventDefault();
    el.classList.add("drag-over");
  });
  el.addEventListener("dragleave", () => el.classList.remove("drag-over"));
  el.addEventListener("drop", (ev) => {
    ev.preventDefault();
    el.classList.remove("drag-over");
    const files = ev.dataTransfer && ev.dataTransfer.files;
    if (!files || !files.length) return;
    if (!state.can || !state.can.upload) return toast("当前没有上传权限");
    onPickFiles({ target: { files } });
  });
}

// ---------------- 浏览与渲染 ----------------

async function loadFiles() {
  const s = state.share;
  const el = document.getElementById("files");
  clearSelection();
  if (!s) {
    el.innerHTML = '<div class="empty"><span class="empty-ico">🗂️</span>没有可访问的空间</div>';
    return;
  }
  el.innerHTML = '<div class="center"><div class="spinner"></div></div>';
  try {
    const r = await api("GET", `/api/browse/${s.id}?path=${encodeURIComponent(state.path)}`);
    state.path = r.path || "";
    state.can = r.can || {};
    renderToolbar();
    renderFiles(r.entries || []);
  } catch (e) {
    el.innerHTML = `<div class="center" style="color:var(--danger)">${esc(e.message)}</div>`;
  }
}

function sortFiles(entries) {
  return entries.slice().sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    switch (state.sort) {
      case "size":
        return b.size - a.size;
      case "mtime":
        return b.mtime - a.mtime;
      case "kind":
        return a.kind.localeCompare(b.kind);
      default:
        return a.name.localeCompare(b.name, "zh-Hans-CN");
    }
  });
}

function filterEntries(entries) {
  if (!state.filter) return entries;
  const q = state.filter.toLowerCase();
  return entries.filter((x) => x.name.toLowerCase().includes(q));
}

function renderFiles(entries) {
  state.lastEntries = entries;
  const el = document.getElementById("files");
  const shown = filterEntries(entries);
  if (!shown.length) {
    if (state.filter) {
      el.innerHTML = `<div class="empty"><span class="empty-ico">🔍</span><div>未找到与 <b style="color:var(--primary)">${esc(state.filter)}</b> 相关的文件</div></div>`;
    } else {
      el.innerHTML = '<div class="empty"><span class="empty-ico">📁</span>空文件夹</div>';
    }
    return;
  }
  const sorted = sortFiles(shown);
  if (state.view === "list") {
    el.innerHTML = `<div class="list-view-card">
      <div class="list-caption">共 ${sorted.length} 项${state.filter ? ` · 筛选:"${esc(state.filter)}"` : ""}</div>
      <table class="files-table"><thead><tr><th>名称</th><th>大小</th><th>修改时间</th><th style="width:120px"></th></tr></thead><tbody>
      ${sorted
        .map(
          (x) => `<tr class="ftr" data-name="${esc(x.name)}" data-isdir="${x.is_dir ? 1 : 0}" data-kind="${esc(x.kind || "other")}">
        <td class="fname"><label class="sel-cb"><input type="checkbox" class="sel-in"></label><span class="f-ico">${fileIco(x)}</span><span>${esc(x.name)}</span></td>
        <td>${x.is_dir ? "—" : formatSize(x.size)}</td>
        <td class="fdate">${formatTime(x.mtime)}</td>
        <td><span class="ftr-acts">${!x.is_dir && state.can.download ? `<button class="btn mini ghost" data-qdl="${esc(x.name)}" title="下载">⬇</button>` : ""}
        ${!x.is_dir && state.user ? `<button class="btn mini ghost" data-qshare="${esc(x.name)}" title="分享">🔗</button>` : ""}
        ${state.can.delete ? `<button class="btn mini ghost" data-qdel="${esc(x.name)}" title="删除">✕</button>` : ""}</span></td>
      </tr>`
        )
        .join("")}
      </tbody></table></div>`;
    el.querySelectorAll("[data-qdl]").forEach((b) => (b.onclick = () => download(state.share.id, childPath(b.dataset.qdl), b.dataset.qdl)));
    const qShare = el.querySelectorAll("[data-qshare]");
    qShare.forEach((b) => (b.onclick = () => shareModal(state.share.id, childPath(b.dataset.qshare), b.dataset.qshare)));
    el.querySelectorAll("[data-qdel]").forEach((b) => (b.onclick = () => deleteModal([b.dataset.qdel])));
    return;
  }
  el.innerHTML = `<div class="grid">${sorted
    .map((x) => {
      const isImg = !x.is_dir && ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "avif", "ico"].includes(x.ext);
      const isVid = !x.is_dir && x.kind === "video" && x.ext !== "avi";
      const kc = kindOf(x);
      const showThumb = state.share && (isImg || isVid);
      const turl = isImg ? mediaUrl(state.share.id, childPath(x.name)) : thumbUrl(state.share.id, childPath(x.name));
      const thumb = showThumb
        ? `<div class="thumb"><img src="${esc(turl)}" loading="lazy" data-src="${esc(turl)}" data-tr="3" onerror="thumbErr(this)"><span class="tfb">${fileIco(x)}</span></div>`
        : `<div class="thumb"><span style="filter:drop-shadow(0 6px 10px rgba(24,32,64,.14))">${fileIco(x)}</span></div>`;
      return `<div class="tile ${kc}" data-name="${esc(x.name)}" data-isdir="${x.is_dir ? 1 : 0}" data-kind="${esc(x.kind || "other")}">
        <label class="sel-cb"><input type="checkbox" class="sel-in"></label>
        <div class="tile-acts">
          ${!x.is_dir && state.can.download ? `<button class="tile-act" data-qdl="${esc(x.name)}" title="下载">⬇</button>` : ""}
          ${!x.is_dir && state.user ? `<button class="tile-act" data-qshare="${esc(x.name)}" title="分享">🔗</button>` : ""}
        </div>
        ${thumb}
        <div class="tname" title="${esc(x.name)}">${esc(x.name)}</div>
        <div class="tmeta">${x.is_dir ? "文件夹" : formatSize(x.size)}</div>
      </div>`;
    })
    .join("")}</div>`;
  const qdl = el.querySelectorAll("[data-qdl]");
  qdl.forEach((b) => { b.onclick = (ev) => { ev.stopPropagation(); download(state.share.id, childPath(b.dataset.qdl), b.dataset.qdl); }; });
  const qshare = el.querySelectorAll("[data-qshare]");
  qshare.forEach((b) => { b.onclick = (ev) => { ev.stopPropagation(); shareModal(state.share.id, childPath(b.dataset.qshare), b.dataset.qshare); }; });
}

// ---------------- 选中(单击勾选)与打开(双击) ----------------

function itemOf(ev) {
  return ev.target.closest(".tile, .ftr");
}

document.addEventListener("click", (ev) => {
  if (ev.target.closest(".sel-cb")) return;
  if (ev.target.closest(".tile, .ftr")) return;
  if (!ev.target.closest("#selbar, .toolbar, .modal-mask, .share-item, .crumbs, .upload-panel, .dl-anchor, .ctx-menu")) {
    clearSelection();
  }
});

document.addEventListener("dblclick", (ev) => {
  const item = itemOf(ev);
  if (item) openItem(item.dataset.name, item.dataset.isdir === "1", item.dataset.kind);
});

document.addEventListener("change", (ev) => {
  const cb = ev.target.closest(".sel-in");
  if (!cb) return;
  const item = cb.closest(".tile, .ftr");
  if (!item) return;
  const name = item.dataset.name;
  if (cb.checked) {
    state.sel.add(name);
    if (item.dataset.isdir === "1") state.selDir.add(name);
    else state.selDir.delete(name);
  } else {
    state.sel.delete(name);
    state.selDir.delete(name);
  }
  syncSelClass(item);
  renderSelectionBar();
});

function syncAllSelClasses() {
  document.querySelectorAll(".tile, .ftr").forEach((n) => syncSelClass(n));
}

function syncSelClass(item) {
  const on = state.sel.has(item.dataset.name);
  item.classList.toggle("sel", on);
  const cb = item.querySelector(".sel-in");
  if (cb) cb.checked = on;
}

function clearSelection() {
  state.sel.clear();
  state.selDir.clear();
  syncAllSelClasses();
  renderSelectionBar();
}

function renderSelectionBar() {
  const el = document.getElementById("selbar");
  if (!el) return;
  const n = state.sel.size;
  el.classList.toggle("hidden", !n);
  if (!n) {
    el.innerHTML = "";
    return;
  }
  const can = state.can;
  const names = [...state.sel];
  const one = n === 1 ? names[0] : null;
  el.innerHTML = `
    <span class="sb-count">已选 <b>${n}</b> 项</span>
    <span class="sel-sep"></span>
    ${one ? '<button class="btn mini" id="sb-open">📂 打开</button>' : ""}
    ${can.download ? `<button class="btn mini" id="sb-dl">⬇ 下载</button>
    <button class="btn mini ghost" id="sb-link">🔗 复制下载链接</button>` : ""}
    ${one && !state.selDir.has(one) && state.user ? '<button class="btn mini ghost" id="sb-share">🔗 分享</button>' : ""}
    ${can.delete ? `<button class="btn mini ghost" id="sb-rn"${one ? "" : ' disabled'} title="重命名">✏ 重命名</button>
    <button class="btn mini danger" id="sb-del">🗑 删除</button>` : ""}
    <button class="btn mini" id="sb-clear" style="background:transparent;color:var(--muted);box-shadow:none;border:none">取消选择</button>`;
  el.querySelector("#sb-clear").onclick = clearSelection;
  const op = el.querySelector("#sb-open");
  if (op) op.onclick = () => openItem(one, state.selDir.has(one), "");
  const dl = el.querySelector("#sb-dl");
  if (dl) dl.onclick = batchDownload;
  const link = el.querySelector("#sb-link");
  if (link) link.onclick = copyDownloadLink;
  const share = el.querySelector("#sb-share");
  if (share) share.onclick = () => shareModal(state.share.id, childPath(one), one);
  const rn = el.querySelector("#sb-rn");
  if (rn && one) rn.onclick = () => renameModal(one);
  const del = el.querySelector("#sb-del");
  if (del) del.onclick = () => deleteModal(names);
}

async function copyDownloadLink() {
  const can = state.can;
  const names = [...state.sel];
  if (!names.length || !state.share || !can || !can.download) return;
  const one = names.length === 1 && !state.selDir.has(names[0]) ? names[0] : null;
  try {
    const r = await api("POST", "/api/file/dlticket", {
      share_id: state.share.id,
      path: one ? childPath(one) : state.path,
      names: one ? [] : names,
    });
    copyText(location.origin + r.url, "下载链接已复制,7 天内免登录可下载");
  } catch (e) {
    toast(e.message);
  }
}

function openItem(name, isdir, kind) {
  if (isdir === true || isdir === "1") {
    state.path = childPath(name);
    state.filter = "";
    loadFiles();
    return;
  }
  if (kind === "image" || kind === "video" || kind === "audio") openMedia(name);
  else openFile(name);
}

// ---------------- 右键菜单 ----------------

let ctxMenuEl = null;

function closeCtxMenu() {
  if (ctxMenuEl) {
    ctxMenuEl.remove();
    ctxMenuEl = null;
  }
}

document.addEventListener("contextmenu", (ev) => {
  const item = itemOf(ev);
  if (!item) return;
  ev.preventDefault();
  closeCtxMenu();
  const name = item.dataset.name;
  const isdir = item.dataset.isdir === "1";
  const kind = item.dataset.kind;
  const items = [];
  items.push({ icon: "📂", label: "打开", act: () => openItem(name, isdir, kind) });
  if (!isdir) {
    if (state.can.download) items.push({ icon: "⬇", label: "下载", act: () => download(state.share.id, childPath(name), name) });
    if (state.user) {
      items.push({ icon: "🔗", label: "分享链接", act: () => shareModal(state.share.id, childPath(name), name) });
      items.push({ icon: "📋", label: "复制下载链接", act: () => copyText(location.origin + mediaUrl(state.share.id, childPath(name), "dl=1"), "下载链接已复制") });
    }
  }
  if (state.can.delete) {
    items.push({ sep: true });
    items.push({ icon: "✏", label: "重命名", act: () => renameModal(name) });
    items.push({ icon: "🗑", label: "删除", act: () => deleteModal([name]) });
  }
  openCtxAt(ev, items);
});

document.addEventListener("contextmenu", (ev) => {
  if (itemOf(ev) || ev.target.closest(".sel-cb")) return;
  ev.preventDefault();
  const items = [];
  if (state.can.upload) items.push({ icon: "⬆", label: "上传文件到当前目录", act: () => { const i = ensureFileInput(); i.click(); } });
  if (state.can.mkdir) items.push({ icon: "＋", label: "新建文件夹", act: mkdirModal });
  items.push({ icon: "↻", label: "刷新", act: loadFiles });
  if (items.length) openCtxAt(ev, items);
});

function openCtxAt(ev, items) {
  closeCtxMenu();
  const menu = document.createElement("div");
  menu.className = "ctx-menu";
  let html = "";
  items.forEach((it) => {
    if (it.sep) html += '<div class="ctx-sep"></div>';
    else html += `<button class="ctx-item"><span class="ctx-ico">${it.icon || ""}</span><span>${esc(it.label)}</span></button>`;
  });
  menu.innerHTML = html;
  const btns = menu.querySelectorAll(".ctx-item");
  btns.forEach((b, i) => {
    b.onclick = () => {
      closeCtxMenu();
      const acts = items.filter((x) => !x.sep);
      const it = acts[i];
      if (it && it.act) it.act();
    };
  });
  document.body.appendChild(menu);
  const r = menu.getBoundingClientRect();
  menu.style.left = Math.max(6, Math.min(ev.clientX, window.innerWidth - r.width - 10)) + "px";
  menu.style.top = Math.max(6, Math.min(ev.clientY, window.innerHeight - r.height - 10)) + "px";
  ctxMenuEl = menu;
}

document.addEventListener("click", closeCtxMenu);
document.addEventListener("scroll", closeCtxMenu, true);

// ---------------- 弹窗 ----------------

function openModal(html, cls) {
  const mask = document.createElement("div");
  mask.className = "modal-mask";
  mask.innerHTML = `<div class="modal${cls ? " " + cls : ""}"><div class="mhead"><span class="mt">预览</span><span class="close" data-close="1">×</span></div>${html}</div>`;
  document.body.appendChild(mask);
  mask.addEventListener("click", (ev) => {
    if (ev.target === mask || ev.target.closest("[data-close]")) mask.remove();
  });
  return mask;
}

// ---------------- 操作:下载 / 重命名 / 删除 / 新建 / 权限 ----------------

async function batchDownload() {
  const share = state.share;
  const files = [...state.sel];
  if (files.length === 1 && !state.selDir.has(files[0])) {
    download(share.id, childPath(files[0]), files[0]);
    toast("开始下载", true);
    return;
  }
  const zipName = files.length === 1 ? files[0] + ".zip" : "批量下载.zip";
  try {
    const headers = { "Content-Type": "application/json" };
    if (state.token) headers["Authorization"] = "Bearer " + state.token;
    const resp = await fetch("/api/zip", {
      method: "POST",
      headers,
      body: JSON.stringify({ share_id: share.id, path: state.path, names: files }),
    });
    if (!resp.ok) {
      const j = await resp.json().catch(() => null);
      throw new Error(j && j.error ? j.error : "打包失败");
    }
    const blob = await resp.blob();
    if (!blob.size) throw new Error("打包结果为空");
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = zipName;
    a.className = "dl-anchor";
    document.body.appendChild(a);
    a.click();
    setTimeout(() => {
      URL.revokeObjectURL(url);
      a.remove();
    }, 2000);
    toast("开始下载", true);
  } catch (e) {
    toast(e.message || "下载失败");
  }
}

function renameModal(name) {
  const mask = openModal(`
    <div class="pf">
      <div class="pf-row"><label>原名称</label><span style="font-size:13px">${esc(name)}</span></div>
      <div class="pf-row"><label>新名称</label><input id="rn-name" value="${esc(name)}"></div>
      <div class="pf-actions">
        <button class="btn mini ghost" data-close="1">取消</button>
        <button class="btn" id="rn-ok">重命名</button>
      </div>
    </div>`);
  mask.querySelector(".mhead .mt").textContent = "重命名";
  const inp = mask.querySelector("#rn-name");
  const okBtn = mask.querySelector("#rn-ok");
  inp.focus();
  inp.select();
  const go = async () => {
    const val = inp.value.trim();
    if (!val) return toast("名称不能为空");
    if (val === name) return mask.remove();
    okBtn.disabled = true;
    try {
      await api("POST", "/api/rename", { share_id: state.share.id, path: state.path, old: name, new: val });
      mask.remove();
      toast("已重命名", true);
      loadFiles();
    } catch (e) {
      okBtn.disabled = false;
      toast(e.message || "重命名失败");
    }
  };
  okBtn.onclick = go;
  inp.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") go();
  });
}

function deleteModal(names) {
  const hasDir = names.some((n) => state.selDir.has(n));
  const mask = openModal(`
    <div class="pf">
      <p style="font-size:14px;margin-bottom:10px">确认删除以下 <b>${names.length}</b> 项?此操作<b>不可恢复</b>:</p>
      <ul style="font-size:13px;color:var(--muted);margin:0 0 14px 18px;line-height:1.8">${names.map((n) => `<li>${esc(n)}</li>`).join("")}</ul>
      ${hasDir ? '<p class="pf-hint" style="margin-bottom:14px">包含文件夹,其下所有内容将一并删除</p>' : ""}
      <div class="pf-actions">
        <button class="btn mini ghost" data-close="1">取消</button>
        <button class="btn danger" id="del-ok">确认删除</button>
      </div>
    </div>`);
  mask.querySelector(".mhead .mt").textContent = "删除";
  const okBtn = mask.querySelector("#del-ok");
  okBtn.onclick = async () => {
    okBtn.disabled = true;
    try {
      for (const n of names) {
        await api("POST", "/api/delete", { share_id: state.share.id, path: childPath(n), recursive: true });
      }
      mask.remove();
      toast("已删除 " + names.length + " 项", true);
      loadFiles();
    } catch (e) {
      okBtn.disabled = false;
      toast(e.message || "删除失败");
    }
  };
}

function mkdirModal() {
  const isAdmin = state.user && state.user.is_admin;
  const mask = openModal(`
    <div class="pf">
      <div class="pf-row"><label>文件夹名称</label><input id="mkdir-name" placeholder="请输入名称"></div>
      ${isAdmin ? `<div class="pf-row"><label>谁能访问</label>
        <select id="mkdir-access">
          <option value="inherit">跟随上级(默认)</option>
          <option value="guest">访客可访问(无需登录)</option>
          <option value="login">登录用户可访问</option>
          <option value="admin">仅管理员可访问</option>
        </select></div>` : ""}
      <div class="pf-actions">
        <button class="btn mini ghost" data-close="1">取消</button>
        <button class="btn" id="mkdir-ok">创建</button>
      </div>
    </div>`);
  mask.querySelector(".mhead .mt").textContent = "新建文件夹";
  const inp = mask.querySelector("#mkdir-name");
  const okBtn = mask.querySelector("#mkdir-ok");
  inp.focus();
  const go = async () => {
    const name = inp.value.trim();
    if (!name) return toast("请输入文件夹名称");
    const acc = isAdmin ? mask.querySelector("#mkdir-access").value : "inherit";
    try {
      await api("POST", "/api/mkdir", { share_id: state.share.id, path: state.path, name });
      if (acc !== "inherit") {
        await api("POST", `/api/shares/${state.share.id}/rules`, { rel_path: childPath(name), access: acc });
      }
      toast("已创建", true);
    } catch (e) {
      toast(e.message);
      return;
    }
    mask.remove();
    loadFiles();
  };
  okBtn.onclick = go;
  inp.addEventListener("keydown", (e) => {
    if (e.key === "Enter") go();
  });
}

async function openFolderPerm() {
  const s = state.share;
  if (!s || !state.user || !state.user.is_admin) return;
  const target = state.path;
  const disp = target || "(根目录)";
  const mask = openModal(`
    <div class="pf">
      <div class="pf-row" style="flex-wrap:wrap"><label>文件夹</label><b>${esc(disp)}</b></div>
      <div class="pf-row"><label>谁能访问</label>
        <select id="perm-access">
          <option value="inherit">跟随上级(无单独设置)</option>
          <option value="guest">访客可访问(无需登录)</option>
          <option value="login">登录用户可访问</option>
          <option value="admin">仅管理员可访问</option>
        </select>
      </div>
      <div class="pf-hint">此设置对该文件夹及其下所有子目录生效,子目录可单独覆盖。未登录的访客只能看到标记为"访客可访问"的内容;根目录设为"访客可访问"后,未登录用户即可在侧边栏看到该共享。</div>
      <div class="pf-actions">
        <button class="btn mini ghost" data-close="1">关闭</button>
        <button class="btn" id="perm-save">保存</button>
      </div>
    </div>`);
  mask.querySelector(".mhead .mt").textContent = "文件夹权限";
  let mine = null;
  try {
    const r = await api("GET", `/api/shares/${s.id}/rules`);
    mine = (r.rules || []).find((x) => x.rel_path === target) || null;
  } catch (e) {}
  const sel = mask.querySelector("#perm-access");
  sel.value = mine ? mine.access : "inherit";
  mask.querySelector("#perm-save").onclick = async () => {
    const v = sel.value;
    try {
      if (v === "inherit") {
        if (mine) await api("DELETE", `/api/shares/${s.id}/rules/${mine.id}`);
      } else {
        await api("POST", `/api/shares/${s.id}/rules`, { rel_path: target, access: v });
      }
      toast("权限已更新", true);
    } catch (e) {
      toast(e.message);
      return;
    }
    mask.remove();
    loadFiles();
  };
}

// ---------------- 打开 / 预览 ----------------

let pvList = [];
let pvSel = -1;
let pvTimer = null;

function pvCancel() {
  if (pvTimer) { clearTimeout(pvTimer); pvTimer = null; }
}

function pvBuildList() {
  pvList = sortFiles(state.lastEntries).filter((e) => !e.is_dir);
  return pvList;
}

function pvShell() {
  const mask = openModal(`
    <div class="mbody" id="pv-body" style="padding:16px;display:flex;align-items:center;justify-content:center;min-height:180px"></div>
    <div class="meta">
      <span class="pv-nav" id="pv-nav"></span>
      <b id="pv-name" style="font-size:13px"></b>
      <span id="pv-size"></span>
      ${state.user ? '<button class="btn mini ghost" data-share>🔗 分享</button>' : ""}
      <button class="btn mini" data-dl style="margin-left:auto">下载</button>
    </div>`, "pv");
  return mask;
}

function pvNavButtons(mask) {
  const cur = mask.dataset.cur;
  const idx = pvList.findIndex((e) => e.name === cur);
  pvSel = idx;
  const prev = idx > 0 ? pvList[idx - 1] : null;
  const next = idx >= 0 && idx < pvList.length - 1 ? pvList[idx + 1] : null;
  const nav = mask.querySelector("#pv-nav");
  if (nav) {
    nav.innerHTML = `${prev ? '<button class="btn mini btn-nav" title="上一个">‹</button>' : ""}${next ? '<button class="btn mini btn-nav" title="下一个">›</button>' : ""}`;
    const [pb, nb] = nav.children;
    if (pb) pb.onclick = (ev) => { ev.stopPropagation(); pvNav(-1); };
    if (nb) nb.onclick = (ev) => { ev.stopPropagation(); pvNav(1); };
  }
}

function pvNav(dir) {
  const pv = document.querySelector(".modal.pv");
  const mask = pv ? pv.closest(".modal-mask") : null;
  if (!mask) return;
  const idx = pvList.findIndex((e) => e.name === mask.dataset.cur);
  const next = idx + dir;
  if (next < 0 || next >= pvList.length) return;
  const target = pvList[next];
  if (!target) return;
  if (target.kind === "image" || target.kind === "video" || target.kind === "audio") openMedia(target.name, mask);
  else openFile(target.name, mask);
}

function pvPrepare(mask, name) {
  if (!mask) {
    pvBuildList();
    mask = pvShell();
  }
  mask.dataset.cur = name;
  const nEl = mask.querySelector("#pv-name");
  if (nEl) nEl.textContent = name;
  return mask;
}
async function openMedia(name, mask) {
  const share = state.share;
  const path = childPath(name);
  let meta;
  try {
    meta = await api("GET", `/api/file/meta/${share.id}?path=${encodeURIComponent(path)}`);
  } catch (e) {
    if (mask) mask.closest(".modal-mask").remove();
    return toast(e.message);
  }
  mask = pvPrepare(mask, name);
  const sEl = mask.querySelector("#pv-size");
  if (sEl) sEl.textContent = `${formatSize(meta.size)} · ${esc(meta.mime)}`;
  mask.querySelector("[data-dl]").onclick = () => download(share.id, path, name);
  const sh = mask.querySelector("[data-share]");
  if (sh) sh.onclick = () => shareModal(share.id, path, name);
  pvNavButtons(mask);
  const kind = meta.kind;
  const body = mask.querySelector("#pv-body");
  pvCancel();
  if (kind === "image") {
    body.innerHTML = `<img src="${esc(mediaUrl(share.id, path))}" style="max-width:100%;max-height:78vh">`;
    return;
  }
  if (kind === "audio") {
    body.innerHTML = `<audio controls autoplay src="${esc(mediaUrl(share.id, path))}"></audio>`;
    return;
  }
  const st = await api("GET", `/api/media/status/${share.id}?path=${encodeURIComponent(path)}`);
  if (st.state === "unsupported") {
    body.innerHTML = `<p style="padding:30px;color:var(--muted)">服务器未安装 ffmpeg,该格式无法在线播放(仍可下载)</p>`;
    return;
  }
  const play = (src) => {
    body.innerHTML = `<video controls autoplay src="${esc(src)}"></video>`;
    body.querySelector("video").style.maxWidth = "100%";
    body.querySelector("video").style.maxHeight = "78vh";
    body.querySelector("video").style.borderRadius = "10px";
  };
  if ((st.direct || st.state === "ready") && st.src) {
    play(st.src);
    return;
  }
  body.innerHTML = '<div style="display:flex;flex-direction:column;align-items:center;gap:12px"><div class="spinner"></div><span style="color:var(--muted);font-size:13px">首次播放需要转码,请稍候...</span></div>';
  const poll = async () => {
    const st2 = await api("GET", `/api/media/status/${share.id}?path=${encodeURIComponent(path)}`).catch(() => null);
    if (st2 && st2.state === "ready" && st2.src) { pvTimer = null; play(st2.src); }
    else if (st2 && (st2.state === "converting" || st2.state === undefined)) pvTimer = setTimeout(poll, 2500);
    else { pvTimer = null; body.innerHTML = `<p style="padding:30px;color:var(--danger)">转码失败,无法播放该文件(可下载)</p>`; }
  };
  pvTimer = setTimeout(poll, 2500);
}

// ---------------- 文档 / 表格 预览与编辑 ----------------

const EDIT_TEXT_EXTS = ["txt", "log", "md", "yaml", "yml", "conf", "ini", "toml", "csv", "tsv", "json", "xml", "html", "htm", "css", "js", "mjs", "ts", "sh", "bash", "py", "sql", "properties", "env"];
const SHEET_EXTS = ["xlsx", "xlsm", "xls"];
const DOCX_EXTS = ["docx", "docm"];

async function openFile(name, mask) {
  const share = state.share;
  const path = childPath(name);
  let meta;
  try {
    meta = await api("GET", `/api/file/meta/${share.id}?path=${encodeURIComponent(path)}`);
  } catch (e) {
    if (mask) mask.remove();
    return toast(e.message);
  }
  mask = pvPrepare(mask, name);
  const sEl = mask.querySelector("#pv-size");
  if (sEl) sEl.textContent = `${formatSize(meta.size)} · ${esc(meta.mime)}`;
  mask.querySelector("[data-dl]").onclick = () => download(share.id, path, name);
  const sh = mask.querySelector("[data-share]");
  if (sh) sh.onclick = () => shareModal(share.id, path, name);
  pvNavButtons(mask);
  const body = mask.querySelector("#pv-body");
  pvCancel();
  const ext = meta.ext;
  if (ext === "pdf") return pvPdf(share, path, body);
  if (EDIT_TEXT_EXTS.includes(ext)) return pvText(share, path, body, meta.writable);
  if (SHEET_EXTS.includes(ext)) return pvSheet(share, path, body, ext, meta.writable);
  if (DOCX_EXTS.includes(ext)) return pvDocx(share, path, body);
  body.innerHTML = '<div class="pv-dl-only">该类型文件请下载后查看</div>';
}

function pvPdf(share, path, body) {
  body.classList.add("pv-fill");
  body.innerHTML = `<iframe class="pv-pdf" src="${esc(mediaUrl(share.id, path))}" title="PDF 预览"></iframe>`;
}

async function pvText(share, path, body, writable) {
  const editable = !!state.user && !!writable;
  let r;
  try {
    r = await api("GET", `/api/file/text/${share.id}?path=${encodeURIComponent(path)}`);
  } catch (e) {
    body.innerHTML = `<p class="pv-msg">${esc(e.message)}</p>`;
    return;
  }
  body.classList.add("pv-fill");
  const ta = document.createElement("textarea");
  ta.className = "pv-textarea" + (editable ? " editable" : "");
  ta.value = r.text;
  ta.readOnly = !editable;
  ta.spellcheck = false;
  body.appendChild(ta);
  if (!editable) return;
  const bar = document.createElement("div");
  bar.className = "pv-edit-bar";
  bar.innerHTML = `<button class="btn mini" id="pv-save">💾 保存 (Ctrl+S)</button><span id="pv-save-msg" class="pv-save-msg"></span>`;
  body.insertBefore(bar, body.firstChild);
  const saveBtn = bar.querySelector("#pv-save");
  const msgEl = bar.querySelector("#pv-save-msg");
  const doSave = async () => {
    saveBtn.disabled = true;
    msgEl.textContent = "保存中...";
    try {
      const rr = await api("POST", `/api/file/text/${share.id}?path=${encodeURIComponent(path)}`, { content: ta.value });
      msgEl.textContent = `✓ 已保存 (${formatSize(rr.size)})`;
      pvMarkSaved();
    } catch (e) {
      msgEl.textContent = "保存失败: " + e.message;
    }
    setTimeout(() => { saveBtn.disabled = false; }, 1800);
  };
  saveBtn.onclick = doSave;
  ta.addEventListener("keydown", (ev) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key === "s") {
      ev.preventDefault();
      doSave();
    }
  });
}

async function pvDocx(share, path, body) {
  if (typeof docx === "undefined" || !docx.renderAsync) {
    body.innerHTML = '<p class="pv-msg">文档渲染组件未加载</p>';
    return;
  }
  body.classList.add("pv-fill");
  body.innerHTML = '<div class="pv-loading"><div class="spinner"></div><span>文档加载中...</span></div>';
  try {
    const resp = await fetch(mediaUrl(share.id, path));
    if (!resp.ok) throw new Error("读取失败 (" + resp.status + ")");
    const blob = await resp.blob();
    body.innerHTML = "";
    const holder = document.createElement("div");
    holder.className = "pv-docx";
    body.appendChild(holder);
    await docx.renderAsync(blob, holder, null, { inWrapper: true, ignoreLastRenderedPageBreak: true });
  } catch (e) {
    if (body.isConnected) body.innerHTML = `<p class="pv-msg">文档无法解析: ${esc(e.message)}</p>`;
  }
}

let pvSheetWb = null;
let pvSheetName = "";
let pvSheetPath = "";
let pvSheetExtName = "";
let pvSheetEditable = false;

async function pvSheet(share, path, body, ext, writable) {
  if (typeof XLSX === "undefined") {
    body.innerHTML = '<p class="pv-msg">表格解析组件未加载</p>';
    return;
  }
  pvSheetPath = path;
  pvSheetExtName = ext;
  pvSheetEditable = !!state.user && !!writable && (ext === "xlsx" || ext === "xls");
  body.classList.add("pv-fill");
  body.innerHTML = '<div class="pv-loading"><div class="spinner"></div><span>表格加载中...</span></div>';
  try {
    const resp = await fetch(mediaUrl(share.id, path));
    if (!resp.ok) throw new Error("读取失败 (" + resp.status + ")");
    const buf = await resp.arrayBuffer();
    pvSheetWb = XLSX.read(buf);
  } catch (e) {
    if (body.isConnected) body.innerHTML = `<p class="pv-msg">${esc(e.message)}</p>`;
    return;
  }
  const sheets = pvSheetWb.SheetNames;
  if (!sheets.length) {
    body.innerHTML = '<p class="pv-msg">表格中没有工作表</p>';
    return;
  }
  let gridBox = null;
  const tabbar = document.createElement("div");
  tabbar.className = "pv-sheet-tabs";
  sheets.forEach((sn, i) => {
    const b = document.createElement("button");
    b.className = "btn mini sheet-tab" + (i === 0 ? " active" : "");
    b.textContent = sn;
    b.onclick = () => {
      tabbar.querySelectorAll(".sheet-tab").forEach((x) => x.classList.remove("active"));
      b.classList.add("active");
      renderSheetGrid(gridBox, sn);
    };
    tabbar.appendChild(b);
  });
  body.innerHTML = "";
  body.appendChild(tabbar);
  gridBox = document.createElement("div");
  gridBox.className = "pv-grid-wrap";
  body.appendChild(gridBox);
  renderSheetGrid(gridBox, sheets[0]);
}

function renderSheetGrid(box, sheetName) {
  if (!box) return;
  pvSheetName = sheetName;
  const ws = pvSheetWb.Sheets[sheetName];
  if (!ws) return;
  const aoa = XLSX.utils.sheet_to_json(ws, { header: 1, raw: true, defval: "" });
  const tab = document.createElement("table");
  tab.className = "pv-grid";
  const cols = Math.max(1, ...aoa.map((r) => (Array.isArray(r) ? r.length : 0)));
  const th = document.createElement("tr");
  th.innerHTML = `<th class="corner"></th>` + Array.from({ length: cols }, (_, i) => `<th class="colhead">${String.fromCharCode(65 + i)}</th>`).join("");
  tab.appendChild(th);
  aoa.forEach((row, ri) => {
    const tr = document.createElement("tr");
    tr.innerHTML = `<td class="rowhead">${ri + 1}</td>`;
    for (let ci = 0; ci < cols; ci++) {
      const td = document.createElement("td");
      const v = Array.isArray(row) && row[ci] !== undefined && row[ci] !== null ? String(row[ci]) : "";
      if (pvSheetEditable) td.contentEditable = "true";
      td.textContent = v;
      tr.appendChild(td);
    }
    tab.appendChild(tr);
  });
  box.innerHTML = "";
  box.appendChild(tab);
  if (!pvSheetEditable) return;
  const bar = document.createElement("div");
  bar.className = "pv-edit-bar";
  bar.innerHTML = `<button class="btn mini" id="pv-save">💾 保存表格</button><span id="pv-save-msg" class="pv-save-msg"></span>`;
  box.insertBefore(bar, box.firstChild);
  const saveBtn = bar.querySelector("#pv-save");
  const msgEl = bar.querySelector("#pv-save-msg");
  saveBtn.onclick = async () => {
    saveBtn.disabled = true;
    msgEl.textContent = "保存中...";
    try {
      const aoa2 = Array.from(tab.querySelectorAll("tr")).slice(1).map((tr) =>
        Array.from(tr.querySelectorAll("td:not(.rowhead)")).map((td) => td.textContent.trim())
      );
      const nws = XLSX.utils.aoa_to_sheet(aoa2);
      const nwb = XLSX.utils.book_new();
      XLSX.utils.book_append_sheet(nwb, nws, sheetName);
      const out = XLSX.write(nwb, { bookType: pvSheetExtName === "xls" ? "biff8" : "xlsx", type: "array" });
      const headers = {};
      if (state.token) headers["Authorization"] = "Bearer " + state.token;
      headers["Content-Type"] = "application/octet-stream";
      const resp = await fetch(`/api/file/binary/${state.share.id}?path=${encodeURIComponent(pvSheetPath)}`, { method: "POST", headers, body: out });
      const ct = resp.headers.get("content-type") || "";
      const j = ct.includes("json") ? await resp.json().catch(() => null) : null;
      if (!resp.ok) throw new Error(j && j.error ? j.error : "保存失败 (" + resp.status + ")");
      msgEl.textContent = "✓ 已保存";
      pvMarkSaved();
    } catch (e) {
      msgEl.textContent = "保存失败: " + e.message;
    }
    setTimeout(() => { saveBtn.disabled = false; }, 1800);
  };
}

function pvMarkSaved() {
  const mask = document.querySelector(".modal.pv");
  if (!mask) return;
  const dl = mask.querySelector("[data-dl]");
  if (dl) dl.textContent = "⬇ 已更新,重新下载";
}

async function shareModal(shareId, path, name) {
  const mask = openModal(`
    <div class="pf">
      <div class="pf-row" style="flex-wrap:wrap"><label>文件</label><b style="font-size:13px">${esc(name)}</b></div>
      <div class="pf-row" style="flex-wrap:wrap">
        <label>分享链接</label>
        <input id="fs-url" type="text" readonly placeholder="点击下方按钮生成链接">
        <button class="btn mini" id="fs-copy" disabled>复制</button>
      </div>
      <div id="fs-status" class="lbl" style="min-height:18px"></div>
      <div class="pf-actions">
        <button class="btn mini ghost" data-close="1">关闭</button>
        <button class="btn" id="fs-create">生成分享链接</button>
        <button class="btn mini danger hidden" id="fs-revoke">取消分享</button>
      </div>
    </div>`);
  mask.querySelector(".mhead .mt").textContent = "分享链接";
  const url = mask.querySelector("#fs-url");
  const status = mask.querySelector("#fs-status");
  const createBtn = mask.querySelector("#fs-create");
  const revokeBtn = mask.querySelector("#fs-revoke");
  const copyBtn = mask.querySelector("#fs-copy");
  const setLink = (tok) => {
    url.value = location.origin + "/s/" + tok;
    copyBtn.disabled = false;
    createBtn.classList.add("hidden");
    revokeBtn.classList.remove("hidden");
  };
  const clearLink = () => {
    url.value = "";
    copyBtn.disabled = true;
    createBtn.classList.remove("hidden");
    revokeBtn.classList.add("hidden");
  };
  copyBtn.onclick = () => copyText(url.value, "链接已复制");
  createBtn.onclick = async () => {
    status.textContent = "";
    try {
      const r = await api("POST", "/api/fileshares", { share_id: shareId, path });
      setLink(r.url.split("/").pop());
      status.textContent = "任何人打开此链接即可访问或下载该文件,无需登录";
    } catch (e) {
      status.textContent = e.message;
    }
  };
  revokeBtn.onclick = async () => {
    const tok = url.value.split("/").pop();
    if (!tok) return;
    try {
      await api("DELETE", "/api/fileshares/" + tok);
      status.textContent = "已取消分享,链接已失效";
      clearLink();
    } catch (e) {
      status.textContent = e.message;
    }
  };
  try {
    const r = await api("GET", `/api/fileshares?share_id=${shareId}`);
    const hit = (r.links || []).find((x) => x.path === path);
    if (hit) {
      setLink(hit.token);
      status.textContent = "该文件已有分享链接(已被访问 " + hit.hits + " 次)";
    }
  } catch (e) {}
}

// ---------------- 上传(分片 + 断点续传) ----------------

function onPickFiles(ev) {
  const files = Array.from(ev.target.files || []);
  if (!files.length) return;
  showUploadsPanel();
  (async () => {
    for (const f of files) {
      await uploadOne(f);
    }
    loadFiles();
  })();
}

function showUploadsPanel() {
  const p = document.getElementById("uploadPanel");
  p.classList.remove("hidden");
  if (p.dataset.bound !== "1") {
    p.dataset.bound = "1";
    p.innerHTML = `
      <div class="up-head">
        <span class="up-title">📤 上传任务 <span id="up-count" class="up-count-badge">0</span></span>
        <button class="btn mini ghost" id="up-clear" title="清除已完成">清除已完成</button>
        <button class="btn mini ghost" id="up-close" title="关闭" style="margin-left:auto">✕</button>
      </div>
      <div class="up-list"></div>`;
    document.getElementById("up-close").onclick = () => p.classList.add("hidden");
    document.getElementById("up-clear").onclick = () => {
      const list = p.querySelector(".up-list");
      list.querySelectorAll(".up-item.done").forEach((n) => n.remove());
      p.querySelector("#up-count").textContent = list.querySelectorAll(".up-item").length;
      if (!list.querySelector(".up-item")) p.classList.add("hidden");
    };
  }
}

function updateUploadCount() {
  const el = document.getElementById("up-count");
  if (!el) return;
  const list = document.querySelector("#uploadPanel .up-list");
  el.textContent = list ? list.querySelectorAll(".up-item").length : 0;
}

async function uploadOne(file) {
  const share = state.share;
  let init;
  try {
    init = await api("POST", "/api/upload/init", {
      share_id: share.id,
      path: state.path,
      filename: file.name,
      size: file.size,
    });
  } catch (e) {
    return toast(e.message);
  }
  const { token, chunk_size, num_parts } = init;
  const row = uploadRow(file.name, file.size);
  row.setText("准备中");
  if (!(await uploadParts(token, chunk_size, num_parts, file, row))) {
    row.markFail();
    row.showRetry(async () => {
      row.markResume();
      await resumeSession(token, chunk_size, num_parts, file, row);
    });
    return;
  }
  await uploadFinish(token, chunk_size, num_parts, file, row);
}

async function resumeSession(token, chunk_size, num_parts, file, row) {
  if (!(await uploadParts(token, chunk_size, num_parts, file, row))) {
    row.markFail();
    row.setText("网络中断,可稍后再试");
    return;
  }
  await uploadFinish(token, chunk_size, num_parts, file, row);
}

async function uploadParts(token, chunk_size, num_parts, file, row) {
  let doneParts = [];
  try {
    const p = await api("GET", `/api/upload/parts/${token}`);
    doneParts = p.parts || [];
    if (doneParts.length) row.setText("断点续传...");
  } catch (e) {}
  for (let i = 0; i < num_parts; i++) {
    if (doneParts.indexOf(i) >= 0) continue;
    const ok = await putChunkWithProgress(token, i, i * chunk_size, Math.min(file.size, (i + 1) * chunk_size), file, (loaded) => {
      row.setProgress(i * chunk_size + loaded, file.size);
    });
    if (!ok) return false;
  }
  return true;
}

async function uploadFinish(token, chunk_size, num_parts, file, row) {
  try {
    await api("POST", `/api/upload/complete/${token}`);
    row.markDone();
    row.setTextSize(file.size, file.size);
  } catch (e) {
    row.markFail();
    row.setText("合并失败: " + e.message);
    row.showRetry(async () => {
      row.markResume();
      await resumeSession(token, chunk_size, num_parts, file, row);
    });
  }
}

function putChunkWithProgress(token, part, start, end, file, onProg) {
  const slice = file.slice(start, end);
  return new Promise((resolve) => {
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", `/api/upload/part/${token}/${part}`, true);
    xhr.setRequestHeader("Authorization", "Bearer " + state.token);
    xhr.responseType = "json";
    xhr.upload.onprogress = (ev) => {
      if (ev.lengthComputable) onProg(ev.loaded);
    };
    xhr.onload = () => resolve(xhr.status === 200);
    xhr.onerror = () => resolve(false);
    xhr.send(slice);
  });
}

function uploadRow(name, size) {
  const panel = document.getElementById("uploadPanel");
  const list = panel.querySelector(".up-list");
  const item = document.createElement("div");
  item.className = "up-item";
  item.innerHTML = `
    <span class="up-name" title="${esc(name)}">${esc(name)}</span>
    <div class="up-bar"><div class="up-fill"></div></div>
    <span class="up-pct">0%</span>
    <button class="btn mini ghost up-retry hidden">重试</button>`;
  list.appendChild(item);
  updateUploadCount();
  const fill = item.querySelector(".up-fill");
  const pct = item.querySelector(".up-pct");
  const total = size || 1;
  const set = (done) => {
    const p = Math.min(100, Math.round((done / total) * 100));
    fill.style.width = p + "%";
    pct.textContent = p + "%";
  };
  return {
    setProgress: (done) => set(done || 0),
    setText: (txt) => {
      pct.textContent = txt;
    },
    setTextSize: (done, tot) => {
      pct.textContent = Math.round(done) + " / " + formatSize(tot);
    },
    markDone: () => {
      set(100);
      item.classList.add("done");
      item.querySelector(".up-retry").classList.add("hidden");
    },
    markFail: () => {
      item.querySelector(".up-retry").classList.remove("hidden");
      const p = document.getElementById("uploadPanel");
      if (p.classList.contains("hidden")) {
        p.classList.remove("hidden");
        toast("上传失败:" + item.querySelector(".up-name").textContent);
      }
    },
    markResume: () => {
      item.querySelector(".up-retry").classList.add("hidden");
    },
    showRetry: (cb) => {
      const btn = item.querySelector(".up-retry");
      btn.onclick = cb;
      btn.classList.remove("hidden");
    },
    item,
  };
}

// ---------------- 管理后台 ----------------

async function openHostDirPicker(onPick, initial, opts) {
  const pickFiles = !!(opts && opts.pickFiles);
  const mask = openModal(`
    <div class="pf">
      <div class="hb-cur"><span style="color:var(--muted)">当前目录</span><b id="hb-path">/</b></div>
      <div class="pf-row"><input id="hb-input" type="text" placeholder="输入关键字过滤;或输入完整路径后回车直接跳转" style="width:100%"></div>
      <div id="hb-list" class="hb-list"><div class="spinner"></div></div>
      <div class="pf-actions">
        <button class="btn mini ghost" data-close="1">取消</button>
        <button class="btn mini ghost" id="hb-up">⬆ 上一级</button>
        <button class="btn" id="hb-ok">${pickFiles ? "选择此文件" : "选择此目录"}</button>
      </div>
    </div>`);
  mask.querySelector(".mhead .mt").textContent = pickFiles ? "选择服务器证书文件" : "选择服务器目录";
  const cur = { path: initial || "/" };
  const pathEl = mask.querySelector("#hb-path");
  const listEl = mask.querySelector("#hb-list");
  const inputEl = mask.querySelector("#hb-input");
  const upBtn = mask.querySelector("#hb-up");
  const okBtn = mask.querySelector("#hb-ok");

  const filterInput = () => {
    const kw = inputEl.value.trim().toLowerCase();
    listEl.querySelectorAll(".hb-dir, .hb-file").forEach((el) => {
      el.style.display = !kw || el.dataset.dir.toLowerCase().includes(kw) ? "" : "none";
    });
  };
  inputEl.addEventListener("input", filterInput);
  inputEl.addEventListener("keydown", (ev) => {
    if (ev.key !== "Enter") return;
    const v = inputEl.value.trim();
    if (v.startsWith("/") || /^[a-zA-Z]:[\\/]/.test(v) || /^[a-zA-Z]:$/.test(v)) {
      cur.path = v;
      load();
    }
  });

  const renderDirs = (dirs, files) => {
    const rows = [];
    dirs.forEach((d) => rows.push(`<div class="hb-dir" data-dir="${esc(d)}"><span style="color:var(--warn)">📁</span><span>${esc(d)}</span></div>`));
    files.forEach((f) => rows.push(`<div class="hb-file" data-dir="${esc(f)}"><span style="color:var(--muted)">📄</span><span>${esc(f)}</span></div>`));
    if (!rows.length) {
      listEl.innerHTML = '<div style="padding:20px 0;color:var(--muted);text-align:center">${pickFiles ? "无子目录与证书文件" : "无子目录"}</div>';
      return;
    }
    listEl.innerHTML = rows.join("");
    listEl.querySelectorAll(".hb-dir").forEach((el) => {
      el.onclick = () => {
        cur.path = joinPath(cur.path, el.dataset.dir);
        load();
      };
    });
    listEl.querySelectorAll(".hb-file").forEach((el) => {
      el.onclick = () => {
        onPick(joinPath(cur.path, el.dataset.dir));
        mask.remove();
      };
    });
  };

  const load = async () => {
    const inp = inputEl;
    inp.disabled = true;
    upBtn.disabled = !cur.path || cur.path === "/";
    listEl.innerHTML = '<div class="spinner"></div>';
    try {
      const r = await api("GET", "/api/browse-host?path=" + encodeURIComponent(cur.path) + (pickFiles ? "&files=1" : ""));
      cur.path = r.path;
      pathEl.textContent = r.path;
      okBtn.style.display = pickFiles ? "none" : "";
      upBtn.disabled = !r.parent || r.path === "/";
      upBtn.onclick = () => {
        if (!r.parent) return;
        cur.path = r.parent;
        load();
      };
      renderDirs(r.dirs || [], pickFiles ? r.files || [] : []);
    } catch (e) {
      listEl.innerHTML = `<p style="color:var(--danger);padding:16px 0">${esc(e.message)}</p>`;
    } finally {
      inputEl.disabled = false;
      inputEl.value = "";
      filterInput();
      if (cur.path === "/") inputEl.placeholder = "输入路径回车直接跳转";
    }
  };
  okBtn.onclick = () => {
    onPick(cur.path);
    mask.remove();
  };
  await load();
}

function joinPath(a, b) {
  if (!b) return a;
  if (a === "/" && /^[a-zA-Z]:?$/.test(b)) return b.replace(/:$/, "") + ":\\";
  if (/^[a-zA-Z]:[\\/]/.test(a)) return a.replace(/[\\/]+$/, "") + "\\" + b;
  if (a === "/") return "/" + b.replace(/^\/+/, "");
  return a.replace(/\/+$/, "") + "/" + b.replace(/^\/+/, "");
}

function renderAdmin() {
  document.getElementById("app").innerHTML = `
  <div class="admin-wrap">
    <div class="admin-nav">
      <div class="nav-item active" data-pane="users">👥 用户管理</div>
      <div class="nav-item" data-pane="shares">📂 共享目录</div>
      <div class="nav-item" data-pane="tls">🔒 HTTPS 证书</div>
      <div class="nav-item" data-pane="account">🔑 我的密码</div>
      <div class="nav-item" data-pane="back">← 返回文件</div>
    </div>
    <div class="admin-pane" id="pane"></div>
  </div>`;
  document.querySelectorAll(".nav-item").forEach((el) => {
    el.onclick = () => {
      document.querySelectorAll(".nav-item").forEach((x) => x.classList.remove("active"));
      el.classList.add("active");
      if (el.dataset.pane === "back") return render();
      adminPane(el.dataset.pane);
    };
  });
  adminPane("users");
}

async function adminPane(p) {
  const el = document.getElementById("pane");
  if (p === "users") return adminUsers(el);
  if (p === "shares") return adminShares(el);
  if (p === "tls") return adminTls(el);
  if (p === "account") {
    el.innerHTML = `
      <div class="card">
        <h3>修改我的密码</h3>
        <div style="display:flex;gap:10px;flex-wrap:wrap">
          <input id="cp-old" type="password" placeholder="原密码">
          <input id="cp-new" type="password" placeholder="新密码">
          <button class="btn" id="cp-btn">保存</button>
        </div>
      </div>`;
    document.getElementById("cp-btn").onclick = async () => {
      try {
        await api("POST", "/api/auth/password", {
          old_password: document.getElementById("cp-old").value,
          password: document.getElementById("cp-new").value,
        });
        toast("密码已更新", true);
      } catch (e) {
        toast(e.message);
      }
    };
  }
}

async function adminUsers(el) {
  el.innerHTML = '<div class="center"><div class="spinner"></div></div>';
  let users;
  try {
    users = await api("GET", "/api/users");
  } catch (e) {
    return (el.innerHTML = `<p style="color:var(--danger)">${esc(e.message)}</p>`);
  }
  el.innerHTML = `
    <div class="card">
      <h3>创建用户</h3>
      <div style="display:flex;gap:10px;flex-wrap:wrap;align-items:center">
        <input id="nu-name" type="text" placeholder="用户名">
        <input id="nu-pass" type="password" placeholder="密码">
        <label class="check"><input type="checkbox" id="nu-admin"> 管理员</label>
        <button class="btn" id="nu-btn">创建</button>
      </div>
    </div>
    <div class="card">
      <h3>用户列表(勾选设置权限)</h3>
      <div style="font-size:12px;color:var(--muted);margin-bottom:10px">上传 / 下载 / 删除 / 建文件夹 权限</div>
      <table class="admin-table">
        <tr><th>用户名</th><th>类型</th><th style="min-width:360px">权限</th><th></th></tr>
        ${users
          .map(
            (u) => `
          <tr>
            <td>${esc(u.username)}</td>
            <td>${u.is_admin ? '<span style="color:var(--primary-2);font-weight:600">管理员</span>' : "普通"}</td>
            <td>
              ${["can_upload", "can_download", "can_delete", "can_mkdir"]
                .map(
                  (f) => `<label class="check"><input type="checkbox" class="flag" data-u="${u.id}" data-f="${f}" ${u[f] ? "checked" : ""}>${
                    f === "can_upload" ? "上传" : f === "can_download" ? "下载" : f === "can_delete" ? "删除" : "建目录"
                  }</label>`
                )
                .join("")}
            </td>
            <td>
              <button class="btn mini ghost" data-reset="${u.id}">重置密码</button>
              <button class="btn mini danger" data-del="${u.id}">删除</button>
            </td>
          </tr>`
          )
          .join("")}
      </table>
    </div>`;
  el.querySelectorAll(".flag").forEach((c) => {
    c.onchange = async () => {
      const u = users.find((x) => x.id == c.dataset.u);
      const permit = {
        can_upload: u.can_upload,
        can_download: u.can_download,
        can_delete: u.can_delete,
        can_mkdir: u.can_mkdir,
      };
      permit[c.dataset.f] = c.checked;
      try {
        await api("PATCH", `/api/users/${c.dataset.u}`, { permit });
        u[c.dataset.f] = c.checked;
        toast("已保存", true);
      } catch (e) {
        c.checked = !c.checked;
        toast(e.message);
      }
    };
  });
  el.querySelectorAll("[data-reset]").forEach((b) => {
    b.onclick = async () => {
      const pw = window.prompt("为这个用户输入新密码:");
      if (!pw) return;
      try {
        await api("PATCH", `/api/users/${b.dataset.reset}`, { password: pw });
        toast("已重置", true);
      } catch (e) {
        toast(e.message);
      }
    };
  });
  el.querySelectorAll("[data-del]").forEach((b) => {
    b.onclick = async () => {
      if (!window.confirm("确定删除该用户及其个人空间?")) return;
      try {
        await api("DELETE", `/api/users/${b.dataset.del}`);
        toast("已删除", true);
        adminUsers(el);
      } catch (e) {
        toast(e.message);
      }
    };
  });
  document.getElementById("nu-btn").onclick = async () => {
    try {
      await api("POST", "/api/users", {
        username: document.getElementById("nu-name").value.trim(),
        password: document.getElementById("nu-pass").value,
        is_admin: document.getElementById("nu-admin").checked,
      });
      toast("用户已创建", true);
      adminUsers(el);
    } catch (e) {
      toast(e.message);
    }
  };
}

async function adminTls(el) {
  el.innerHTML = '<div class="card"><h3>HTTPS 证书</h3><div id="tls-status"><div class="spinner"></div></div></div>';
  let st;
  try {
    st = await api("GET", "/api/tls");
  } catch (e) {
    return (el.innerHTML = `<p style="color:var(--danger)">${esc(e.message)}</p>`);
  }
  const cert = st.cert;
  el.innerHTML = `
    <div class="card">
      <h3>当前状态</h3>
      ${
        st.enabled
          ? `<p style="color:var(--primary-2);font-weight:600">✔ HTTPS 已启用</p>
             <p>访问地址:<code>https://${esc(cert ? cert.domain : "你的域名")}${st.port ? ":" + st.port : ""}</code></p>
             <p style="color:var(--muted);font-size:13px">监听:${esc(st.tls_listen)} · 域名字段:${esc(cert ? cert.domain : "-")}</p>`
          : `<p style="color:var(--warn)">未启用 HTTPS。添加有效证书后立即生效,可随时删除。不影响原有 HTTP 访问。</p>`
      }
      ${
        cert
          ? `<button class="btn mini danger" id="tls-del">删除证书并停用 HTTPS</button>`
          : ""
      }
    </div>
    <div class="card">
      <h3>添加 / 更新证书</h3>
      <div class="tabs">
        <button class="tab on" data-tab="paste">粘贴证书内容</button>
        <button class="tab" data-tab="file">服务器证书文件</button>
      </div>
      <div id="tls-paste">
        <input id="tls-domain" type="text" placeholder="证书对应域名,如 file.example.com" style="width:100%;max-width:420px;margin-bottom:8px">
        <div class="lbl" style="font-size:12px;color:var(--muted)">证书 (-----BEGIN CERTIFICATE-----)</div>
        <textarea id="tls-cert" rows="6" style="width:100%;font-family:monospace;font-size:12px" placeholder="-----BEGIN CERTIFICATE-----..."></textarea>
        <div class="lbl" style="font-size:12px;color:var(--muted);margin-top:8px">私钥 (-----BEGIN PRIVATE KEY----- / RSA PRIVATE KEY / EC PRIVATE KEY)</div>
        <textarea id="tls-key" rows="6" style="width:100%;font-family:monospace;font-size:12px" placeholder="-----BEGIN PRIVATE KEY-----..."></textarea>
      </div>
      <div id="tls-file" style="display:none">
        <input id="tls-domain-f" type="text" placeholder="证书对应域名,如 file.example.com" style="width:100%;max-width:420px;margin-bottom:8px">
        <div style="display:flex;gap:8px;align-items:center;margin-bottom:8px">
          <input id="tls-cert-path" type="text" placeholder="证书文件路径,如 /etc/letsencrypt/live/example.com/fullchain.pem" style="flex:1;font-family:monospace;font-size:12px">
          <button class="btn mini ghost" id="tls-browse-cert">浏览…</button>
        </div>
        <div style="display:flex;gap:8px;align-items:center;margin-bottom:8px">
          <input id="tls-key-path" type="text" placeholder="私钥文件路径,如 /etc/letsencrypt/live/example.com/privkey.pem" style="flex:1;font-family:monospace;font-size:12px">
          <button class="btn mini ghost" id="tls-browse-key">浏览…</button>
        </div>
      </div>
      <div style="margin-top:10px"><button class="btn" id="tls-save">保存并启用 HTTPS</button></div>
      <div id="tls-err" style="color:var(--danger);min-height:18px;margin-top:4px"></div>
    </div>`;
  const err = document.getElementById("tls-err");
  const tabs = el.querySelectorAll(".tab");
  tabs.forEach((t) => {
    t.onclick = () => {
      tabs.forEach((x) => x.classList.toggle("on", x === t));
      document.getElementById("tls-paste").style.display = t.dataset.tab === "paste" ? "" : "none";
      document.getElementById("tls-file").style.display = t.dataset.tab === "file" ? "" : "none";
    };
  });
  const browse = (inputId) =>
    openHostDirPicker((p) => {
      document.getElementById(inputId).value = p;
    }, document.getElementById(inputId).value, { pickFiles: true });
  el.querySelector("#tls-browse-cert").onclick = () => browse("tls-cert-path");
  el.querySelector("#tls-browse-key").onclick = () => browse("tls-key-path");
  document.getElementById("tls-save").onclick = async () => {
    err.textContent = "";
    const fileMode = !document.getElementById("tls-file").style.display;
    try {
      const body = fileMode
        ? {
            domain: document.getElementById("tls-domain-f").value.trim(),
            cert_path: document.getElementById("tls-cert-path").value.trim(),
            key_path: document.getElementById("tls-key-path").value.trim(),
          }
        : {
            domain: document.getElementById("tls-domain").value.trim(),
            cert_pem: document.getElementById("tls-cert").value,
            key_pem: document.getElementById("tls-key").value,
          };
      await api("PUT", "/api/tls", body);
      toast("证书已保存,HTTPS 已生效", true);
      adminTls(el);
    } catch (e) {
      err.textContent = e.message;
    }
  };
  const del = document.getElementById("tls-del");
  if (del) {
    del.onclick = async () => {
      if (!window.confirm("确定删除证书并停用 HTTPS?")) return;
      try {
        await api("DELETE", "/api/tls");
        toast("已停用 HTTPS", true);
        adminTls(el);
      } catch (e) {
        toast(e.message);
      }
    };
  }
}

async function adminShares(pane) {
  pane.innerHTML = '<div class="card"><h3>共享目录管理</h3><div class="err"></div><div id="sh-list"><div class="spinner"></div></div></div>';
  let shares;
  try {
    shares = await api("GET", "/api/shares");
  } catch (e) {
    pane.querySelector("#sh-list").innerHTML = `<p style="color:var(--danger)">${esc(e.message)}</p>`;
    return;
  }
  const list = shares.shares || shares;
  pane.innerHTML = `
    <div class="card">
      <h3>新增共享目录(指向服务器主机上的已有目录)</h3>
      <div style="display:flex;gap:10px;flex-wrap:wrap;align-items:center">
        <input id="sh-name" type="text" placeholder="显示名称" style="min-width:180px">
        <div style="display:flex;gap:6px;flex:1;min-width:260px">
          <input id="sh-path" type="text" placeholder="绝对路径,如 /srv/media" style="flex:1;min-width:200px;max-width:400px">
          <button class="btn mini ghost" id="hb-browse">浏览…</button>
        </div>
        <button class="btn" id="sh-add">添加目录</button>
      </div>
      <div id="sh-err" style="color:var(--danger);min-height:18px"></div>
      <div class="lbl" style="margin-top:4px">根目录默认为"登录可访问",可在下方针对子目录设定游客/登录/管理员访问级别。</div>
    </div>
    <div class="card" id="sh-cards"></div>`;
  document.getElementById("sh-add").onclick = async () => {
    const name = document.getElementById("sh-name").value.trim();
    const path = document.getElementById("sh-path").value.trim();
    if (!name || !path) {
      document.getElementById("sh-err").textContent = "请填写名称与路径";
      return;
    }
    try {
      await api("POST", "/api/shares", { name, host_path: path });
      toast("已添加共享目录", true);
      await reloadShares();
      adminShares(pane);
    } catch (e) {
      document.getElementById("sh-err").textContent = e.message;
    }
  };
  const hb = document.getElementById("hb-browse");
  if (hb) {
    hb.onclick = () =>
      openHostDirPicker((p) => {
        document.getElementById("sh-path").value = p;
        document.getElementById("sh-err").textContent = "";
      }, document.getElementById("sh-path").value.trim());
  }
  const custom = list.filter((s) => !s.home);
  if (!custom.length) {
    document.getElementById("sh-cards").innerHTML = `<p style="color:var(--muted)">暂无自定义共享目录</p>`;
    return;
  }
  document.getElementById("sh-cards").innerHTML = custom
    .map(
      (s) => `
    <div class="card" style="margin-bottom:10px">
      <div style="display:flex;align-items:center;gap:10px;flex-wrap:wrap">
        <b>${esc(s.name)}</b>
        <span style="font-size:12px;color:var(--muted)">${esc(s.host_path || "")}</span>
        <span class="rule-access ${esc(s.access || "login")}">${accessName(s.access)}</span>
        <div style="margin-left:auto;display:flex;gap:6px">
          <button class="btn mini ghost" data-rules="${s.id}">访问规则</button>
          <button class="btn mini ghost" data-edit="${s.id}">改路径</button>
          <button class="btn mini danger" data-shdel="${s.id}">删除</button>
        </div>
      </div>
      <div id="rules-${s.id}" class="hidden" style="margin-top:10px"></div>
    </div>`
    )
    .join("");
  pane.querySelectorAll("[data-rules]").forEach((b) => (b.onclick = () => toggleRules(b.dataset.rules)));
  pane.querySelectorAll("[data-edit]").forEach((b) => {
    b.onclick = async () => {
      const s = custom.find((x) => x.id == b.dataset.edit);
      const p = window.prompt("新的目录路径(绝对路径)", s.host_path || "");
      if (!p) return;
      try {
        await api("PATCH", `/api/shares/${s.id}`, { host_path: p });
        toast("已更新", true);
        await reloadShares();
        adminShares(pane);
      } catch (e) {
        toast(e.message);
      }
    };
  });
  pane.querySelectorAll("[data-shdel]").forEach((b) => {
    b.onclick = async () => {
      if (!window.confirm("删除该共享配置?(不会删除服务器上的文件)")) return;
      try {
        await api("DELETE", `/api/shares/${b.dataset.shdel}`);
        toast("已删除", true);
        await reloadShares();
        adminShares(pane);
      } catch (e) {
        toast(e.message);
      }
    };
  });
}

function toggleRules(sid) {
  const box = document.getElementById("rules-" + sid);
  if (!box) return;
  box.classList.toggle("hidden");
  if (!box.classList.contains("hidden")) loadRules(sid);
}

async function loadRules(sid) {
  const box = document.getElementById("rules-" + sid);
  if (!box) return;
  box.innerHTML = '<div class="spinner"></div>';
  let rules = [];
  try {
    const r = await api("GET", `/api/shares/${sid}/rules`);
    rules = r.rules || [];
  } catch (e) {
    box.innerHTML = `<p style="color:var(--danger)">${esc(e.message)}</p>`;
    return;
  }
  box.innerHTML = `
    <div style="display:flex;gap:8px;align-items:center;margin-bottom:8px">
      <input id="nr-${sid}-path" type="text" placeholder="子目录路径,根目录留空" style="flex:1">
      <select id="nr-${sid}-access">
        <option value="guest">游客可访问</option>
        <option value="login">登录可访问</option>
        <option value="admin">仅管理员</option>
      </select>
      <button class="btn mini" data-addrule="${sid}">添加规则</button>
    </div>
    <div class="lbl" style="margin-bottom:6px">规则向下继承,路径越具体优先级越高。</div>
    <div>
      ${rules.length ? rules.map((x) => `
        <div class="rule-row">
          <span class="rule-access ${esc(x.access)}">${accessName(x.access)}</span>
          <span>/ ${esc(x.rel_path || "(根目录)")}</span>
          <span style="margin-left:auto"><button class="btn mini ghost" data-delrule="${x.id}">删除</button></span>
        </div>`).join("") : '<span class="lbl">暂无规则(默认仅管理员可访问)</span>'}
    </div>`;
  box.querySelector(`[data-addrule="${sid}"]`).onclick = async () => {
    try {
      await api("POST", `/api/shares/${sid}/rules`, {
        rel_path: document.getElementById(`nr-${sid}-path`).value || "",
        access: document.getElementById(`nr-${sid}-access`).value,
      });
      toast("规则已保存", true);
      loadRules(sid);
    } catch (e) {
      toast(e.message);
    }
  };
  box.querySelectorAll("[data-delrule]").forEach((b) => {
    b.onclick = async () => {
      try {
        await api("DELETE", `/api/shares/${sid}/rules/${b.dataset.delrule}`);
        loadRules(sid);
      } catch (e) {
        toast(e.message);
      }
    };
  });
}

// ---------------- 快捷键 ----------------

document.addEventListener("keydown", (ev) => {
  if (ev.target.closest("input, select, textarea")) return;
  if (ev.key === "Delete") {
    if (state.sel.size) {
      ev.preventDefault();
      deleteModal([...state.sel]);
    }
  } else if (ev.key === "F2") {
    ev.preventDefault();
    if (state.sel.size === 1) renameModal([...state.sel][0]);
  } else if (ev.key === "Escape") {
    closeCtxMenu();
  }
});

document.addEventListener("keydown", (ev) => {
  if (ev.key !== "Enter" || state.sel.size !== 1) return;
  const name = [...state.sel][0];
  openItem(name, state.selDir.has(name), "");
});

document.addEventListener("keydown", (ev) => {
  const mask = document.querySelector(".modal.pv") ? document.querySelector(".modal.pv").closest(".modal-mask") : null;
  if (!mask) return;
  if (ev.key === "Escape") return mask.remove();
  if (ev.key === "ArrowLeft") return pvNav(-1);
  if (ev.key === "ArrowRight") return pvNav(1);
});

// ---------------- 启动 ----------------

boot();