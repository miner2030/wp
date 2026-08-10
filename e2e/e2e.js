const { chromium } = require("playwright-core");
const fs = require("fs");
const assert = require("assert");

const BASE = process.env.WP_BASE || "http://127.0.0.1:9000";
const SHARE_DIR = process.env.WP_SHARE_DIR || "/tmp/hostshare";
const results = [];
function ok(name) { results.push(["PASS", name]); console.log("PASS:", name); }
function fail(name, e) { results.push(["FAIL", name]); console.error("FAIL:", name, "::", (e && e.message || e)); }

function pageErrors(p) {
  const out = [];
  p.on("pageerror", (e) => out.push("pageerror:" + e.message));
  p.on("console", (m) => { if (m.type() === "error") out.push("console:" + m.text().slice(0, 160)); });
  p.on("requestfailed", (r) => out.push("reqfail:" + r.url().split("?")[0]));
  return out;
}

async function login(page, u, pw) {
  await page.fill("#in-user", u);
  await page.fill("#in-pass", pw);
  const cap = await page.evaluate(async () => {
    const r = await fetch("/api/captcha").then((x) => x.json());
    const code = [...r.svg.matchAll(/<text[^>]*>([^<])<\/text>/g)].map((m) => m[1]).join("");
    return { id: r.id, code };
  });
  await page.evaluate((cap) => {
    document.getElementById("cap-id").value = cap.id;
  }, cap);
  await page.fill("#in-captcha", cap.code);
  await page.click("#btn-login");
  await page.waitForSelector(".shell", { timeout: 15000 });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function openTile(p, name, kind) {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      await closeModals(p);
      await p.locator(".tile", { hasText: name }).dblclick();
      const sel = kind === "image" ? "#pv-body img" : "#pv-body video";
      await p.waitForSelector(sel, { timeout: 4000 });
      return;
    } catch (e) {
      await sleep(900);
    }
  }
  throw new Error("could not open tile " + name);
}
async function closeModals(p) {
  for (let i = 0; i < 5; i++) {
    const m = p.locator(".modal-mask");
    if (!(await m.count())) return;
    await m.first().click({ position: { x: 5, y: 5 }, force: true }).catch(() => {});
    await sleep(300);
  }
}


(async () => {
  const browser = await chromium.launch({ headless: true, args: ["--no-sandbox"] });

  // ============ 1. admin boot / login ============
  const ctx = await browser.newContext({ acceptDownloads: true });
  const page = await ctx.newPage();
  pageErrors(page);
  await page.goto(BASE + "/");
  try {
    assert.strictEqual(await page.title(), "云盘 · 多用户网盘");
    assert(await page.locator(".login-card").isVisible(), "login card visible");
    await login(page, "admin", "admin123");
    assert((await page.evaluate(() => document.cookie)).includes("wp_token="), "cookie set");
    assert((await page.locator(".share-item").count()) === 0, "admin has no shares yet");
    ok("boot + login + cookie");
  } catch (e) { fail("boot/login", e); }

  // ============ 2. create user alice ============
  try {
    await page.click("#btn-admin");
    const padName = await page.evaluate(() => getComputedStyle(document.getElementById("nu-name")).paddingLeft);
    const padPass = await page.evaluate(() => getComputedStyle(document.getElementById("nu-pass")).paddingLeft);
    assert.strictEqual(padName, padPass, "username input styled identically to password input");
    await page.fill("#nu-name", "alice");
    await page.fill("#nu-pass", "pw123");
    await page.click("#nu-btn");
    await page.waitForFunction(() => [...document.querySelectorAll(".admin-table tr")].some((r) => r.textContent.includes("alice")), { timeout: 15000 });
    ok("create user alice");
  } catch (e) { fail("create user", e); }

  // ============ 3. create custom share ============
  try {
    await page.click('[data-pane="shares"]');
    await page.fill("#sh-name", "公开目录");
    await page.fill("#sh-path", SHARE_DIR);
    await page.click("#sh-add");
    await page.waitForSelector("#sh-cards .card", { timeout: 15000 });
    const pathW = await page.evaluate(() => document.getElementById("sh-path").getBoundingClientRect().width);
    assert(pathW <= 650, "absolute path input has reasonable width, got " + pathW);
    ok("create share");
  } catch (e) { fail("create share", e); }

  // ============ 3.5. admin server dir picker ============
  try {
    await page.waitForSelector("#hb-browse", { timeout: 10000 });
    await page.click("#hb-browse");
    await page.waitForSelector("#hb-path", { timeout: 10000 });
    await page.fill("#hb-input", "/tmp");
    await page.press("#hb-input", "Enter");
    await page.waitForFunction(() => document.getElementById("hb-path").textContent === "/tmp", { timeout: 10000 });
    assert(await page.locator("#hb-up").isEnabled(), "up button enabled after navigation");
    const dirs = await page.locator("#hb-list .hb-dir").count();
    assert(dirs > 0, "server dirs listed, got " + dirs);
    await page.fill("#hb-input", "/no-such-dir-xyz-123");
    await page.press("#hb-input", "Enter");
    await page.waitForFunction(() => (document.getElementById("hb-list").textContent || "").includes("路径不存在"), { timeout: 10000 });
    await page.fill("#hb-input", "/tmp");
    await page.press("#hb-input", "Enter");
    await page.waitForFunction(() => document.getElementById("hb-path").textContent === "/tmp", { timeout: 10000 });
    await page.click("#hb-ok");
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    assert.strictEqual((await page.inputValue("#sh-path")).trim(), "/tmp", "picker filled sh-path");
    await page.click('[data-pane="back"]');
    await page.waitForSelector(".share-item", { timeout: 10000 });
    ok("admin server dir picker");
  } catch (e) { fail("server dir picker", e); }

  // ============ 4. browse share ============
  if ((await page.locator(".admin-nav").count()) > 0) {
    await page.click('[data-pane="back"]');
  }
  await page.waitForSelector(".share-item", { timeout: 15000 });
  const shareId = await page.locator(".share-item", { hasText: "公开目录" }).getAttribute("data-share");
  assert.ok(shareId, "share id");
  await page.locator(".share-item", { hasText: "公开目录" }).click();
  await page.waitForSelector("#files .grid .tile", { timeout: 15000 });
  const tilesRoot = await page.locator(".tile").count();
  assert(tilesRoot >= 3, "root lists files, got " + tilesRoot);
  ok("browse share root lists files");

  // ============ 4.7. rules panel: add/remove reflect immediately ============
  try {
    await page.click("#btn-admin");
    await page.click('[data-pane="shares"]');
    await page.locator(`[data-rules="${shareId}"]`).click();
    await page.waitForFunction(
      (sid) => {
        const box = document.getElementById(`rules-${sid}`);
        return box && box.offsetParent !== null && !box.querySelector(".spinner");
      },
      shareId,
      { timeout: 10000 }
    );
    await page.fill(`#nr-${shareId}-path`, "sub");
    await page.selectOption(`#nr-${shareId}-access`, "login");
    await page.locator(`[data-addrule="${shareId}"]`).click();
    await page.waitForFunction(
      (sid) => {
        const box = document.getElementById(`rules-${sid}`);
        return box && box.contains(document.querySelector(`#rules-${sid} .rule-row`)) && box.textContent.includes("sub") && document.getElementById(`rules-${sid}`).offsetParent !== null;
      },
      shareId,
      { timeout: 10000 }
    );
    assert(await page.locator(`#rules-${shareId}`).isVisible(), "rules panel stays open after adding a rule");
    assert((await page.locator(`#rules-${shareId}`).innerText()).includes("sub"), "new rule shown without refresh");
    await page.locator(`#rules-${shareId} [data-delrule]`).first().click();
    await page.waitForFunction(
      (sid) => {
        const box = document.getElementById(`rules-${sid}`);
        return box && !box.textContent.includes("sub") && box.textContent.includes("(根目录)") && box.offsetParent !== null;
      },
      shareId,
      { timeout: 10000 }
    );
    await page.locator('[data-pane="back"]').click();
    await page.waitForSelector(".toolbar", { timeout: 10000 });
    ok("rules panel live-updates after add/delete");
  } catch (e) { fail("rules panel live-update", e); }

  // ============ 4.5. view toggle + sort ============
  try {
    await page.locator('.view-btn[data-view="list"]').click();
    await page.waitForSelector(".files-table .ftr", { timeout: 5000 });
    const rows = await page.locator(".files-table .ftr").count();
    assert(rows >= 3, "list view rows: " + rows);
    assert.strictEqual(await page.locator(".files-table .ftr", { hasText: "clip.mp4" }).count(), 1, "list shows clip.mp4");
    await page.selectOption("#sort-by", "mtime");
    await sleep(300);
    assert.strictEqual(await page.locator(".files-table .ftr").count(), rows, "sort keeps row count");
    await page.locator(".files-table .ftr", { hasText: "pic.png" }).dblclick();
    await page.waitForSelector("#pv-body img", { timeout: 10000 });
    await page.locator(".modal .close").click();
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    await page.locator('.view-btn[data-view="grid"]').click();
    await page.waitForSelector(".tile", { timeout: 5000 });
    ok("grid/list toggle + sort + list interaction");
  } catch (e) { fail("view toggle/sort", e); }

  // ============ 5. mkdir with permission ============
  await closeModals(page);
  try {
    await page.click("#btn-mkdir");
    await page.waitForSelector("#mkdir-name", { timeout: 10000 });
    await page.fill("#mkdir-name", "sub");
    await page.selectOption("#mkdir-access", "inherit");
    await page.click("#mkdir-ok");
    await page.waitForSelector(".tile", { hasText: "sub" }, { timeout: 10000 });
    ok("mkdir via UI modal");
  } catch (e) { fail("mkdir", e); }

  // ============ 5.5. create a guest-visible and an admin-only folder ============
  await closeModals(page);
  try {
    await page.click("#btn-mkdir");
    await page.waitForSelector("#mkdir-name", { timeout: 10000 });
    await page.fill("#mkdir-name", "guest_ok");
    await page.selectOption("#mkdir-access", "guest");
    await page.click("#mkdir-ok");
    await page.waitForSelector(".tile", { hasText: "guest_ok" }, { timeout: 10000 });

    await page.click("#btn-mkdir");
    await page.waitForSelector("#mkdir-name", { timeout: 10000 });
    await page.fill("#mkdir-name", "admin_ok");
    await page.selectOption("#mkdir-access", "admin");
    await page.click("#mkdir-ok");
    await page.waitForSelector(".tile", { hasText: "admin_ok" }, { timeout: 10000 });

    ok("created guest_ok (guest) + admin_ok (admin-only)");
  } catch (e) { fail("permission folders", e); }

  // ============ 5.6. change permission of an existing folder via UI ============
  try {
    await page.locator(".tile", { hasText: "guest_ok" }).dblclick();
    await page.waitForSelector("#btn-perm", { timeout: 10000 });
    await page.click("#btn-perm");
    await page.waitForSelector("#perm-access", { timeout: 10000 });
    await page.selectOption("#perm-access", "admin");
    await page.click("#perm-save");
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    const applied = await page.evaluate(async (sid) => {
      const res = await fetch(`/api/shares/${sid}/rules`, { headers: { Authorization: "Bearer " + localStorage.getItem("wp_token") } });
      const j = await res.json();
      const m = (j.rules || []).find((x) => x.rel_path === "guest_ok");
      return m ? m.access : null;
    }, shareId);
    assert.strictEqual(applied, "admin", "guest_ok rule now admin");
    await page.click("#btn-perm");
    await page.waitForSelector("#perm-access", { timeout: 10000 });
    await page.selectOption("#perm-access", "guest");
    await page.click("#perm-save");
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    ok("existing folder permission change via UI");
  } catch (e) { fail("folder perm UI", e); }

  // ============ 5.7. root folder -> guest (share visible to visitors) ============
  try {
    await page.locator('.crumbs[data-path=""]').click();
    await page.waitForSelector("#btn-perm", { timeout: 10000 });
    await page.click("#btn-perm");
    await page.waitForSelector("#perm-access", { timeout: 10000 });
    await page.selectOption("#perm-access", "guest");
    await page.click("#perm-save");
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    ok("root folder set guest via UI -> share visible when logged out");
  } catch (e) { fail("root perm UI", e); }

  // ============ 5.8. visitor (logged-out) sees only guest-visible folders ============
  try {
    const gctx = await browser.newContext();
    const gp = await gctx.newPage();
    pageErrors(gp);
    await gp.goto(BASE + "/");
    await gp.waitForSelector(".shell", { timeout: 15000 });
    assert((await gp.locator(".who").innerText()).includes("游客"), "guest label");
    await gp.locator(".share-item", { hasText: "公开目录" }).click();
    await gp.waitForSelector(".tile", { timeout: 15000 });
    const names = await gp.locator(".tile .tname").allTextContents();
    assert(names.includes("guest_ok"), "visitor sees guest_ok, got: " + names);
    assert(!names.includes("admin_ok"), "visitor must not see admin_ok");
    assert(!names.includes("secret"), "visitor must not see secret");
    await gp.locator(".tile", { hasText: "guest_ok" }).dblclick();
    await gp.waitForTimeout(800);
    assert((await gp.locator("#files").innerText()).includes("空文件夹"), "visitor opened guest_ok");
    await gctx.close();
    ok("visitor sees only guest-visible folders");
  } catch (e) { fail("visitor folder visibility", e); }

  // ============ 6. mp4 direct preview ============
  try {
    await openTile(page, "clip.mp4", "video");
    const dur = await (async () => {
      const t0 = Date.now();
      while (Date.now() - t0 < 30000) {
        const d = await page.evaluate(() => (document.querySelector("#pv-body video") || {}).duration || 0);
        if (d > 0) return d;
        await sleep(500);
      }
      return 0;
    })();
    assert(dur > 0, "mp4 duration " + dur);
    await page.locator(".modal .close").click();
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    ok("video direct preview + playback");
  } catch (e) { fail("mp4 preview", e); }

  // ============ 7. transcode preview (avi -> mp4) ============
  try {
    await openTile(page, "mov.avi", "video");
    const played = await (async () => {
      const t0 = Date.now();
      while (Date.now() - t0 < 120000) {
        const v = await page.$("#pv-body video");
        if (v) {
          const d = await page.evaluate(() => (document.querySelector("#pv-body video") || {}).duration || 0);
          if (d > 0) return true;
        }
        await sleep(1500);
      }
      return false;
    })();
    assert(played, "transcoded avi became playable");
    await page.locator(".modal .close").click();
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    ok("transcode playback (avi -> mp4)");
  } catch (e) { fail("transcode preview", e); }

  // ============ 8. image preview + keyboard nav ============
  try {
    await openTile(page, "pic.png", "image");
    const nw = await page.evaluate(() => document.querySelector("#pv-body img").naturalWidth);
    assert(nw > 0, "img naturalWidth " + nw);
    assert((await page.locator(".modal.pv #pv-nav .btn").count()) >= 1, "pv nav buttons rendered");
    const pollName = async (badName, ms) => {
      const t0 = Date.now();
      while (Date.now() - t0 < ms) {
        const n = await page.locator("#pv-name").innerText().catch(() => "");
        if (n && n !== badName) return n;
        await sleep(300);
      }
      return "";
    };
    await page.keyboard.press("ArrowLeft");
    let prevName = await pollName("pic.png", 4000);
    let navKey = "ArrowRight";
    if (!prevName) {
      await page.keyboard.press("ArrowRight");
      prevName = await pollName("pic.png", 4000);
      navKey = "ArrowLeft";
    }
    assert(prevName, "arrow nav moved away from pic.png");
    assert.strictEqual(await page.locator(".modal").count(), 1, "modal stays open after arrow nav");
    await page.keyboard.press(navKey);
    await page.waitForFunction(() => (document.querySelector("#pv-name") || {}).textContent === "pic.png", { timeout: 10000 });
    await page.keyboard.press("Escape");
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    await openTile(page, "pic.png", "image");
    await page.locator(".modal .close").click();
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    ok("image preview + nav keys");
  } catch (e) { fail("image preview", e); }

  // ============ 9. chunked upload with resume ============
  await closeModals(page);
  try {
    const big = Buffer.alloc(20 * 1024 * 1024);
    for (let i = 0; i < big.length; i++) big[i] = i % 251;
    let partReq = 0;
    await page.route("**/api/upload/part/**", (route) => {
      partReq++;
      if (partReq === 2) route.abort();
      else route.continue();
    });
    await page.setInputFiles("#file-input", [{ name: "big.bin", mimeType: "application/octet-stream", buffer: big }]);
    await page.waitForSelector(".up-item .up-retry", { state: "visible", timeout: 30000 });
    await page.route("**/api/upload/part/**", (route) => route.continue());
    await page.locator(".up-item .up-retry").click();
    const done = await (async () => {
      const t0 = Date.now();
      while (Date.now() - t0 < 90000) {
        if (await page.locator(".up-item.done").count()) return true;
        await sleep(1000);
      }
      return false;
    })();
    assert(done, "upload finished after resume");
    const upRect = await page.evaluate(() => {
      const p = document.getElementById("uploadPanel");
      const r = p.getBoundingClientRect();
      const cs = getComputedStyle(p);
      return { pos: cs.position, top: r.top, left: r.left, vh: innerHeight, vw: innerWidth, visible: !p.classList.contains("hidden") };
    });
    assert.strictEqual(upRect.pos, "fixed", "upload panel is fixed positioned");
    assert(upRect.visible, "upload panel visible while uploading");
    assert(upRect.top > upRect.vh * 0.55, "upload panel sits in lower half (top=" + upRect.top + " vh=" + upRect.vh + ")");
    assert(upRect.left > upRect.vw * 0.3, "upload panel sits on the right side (left=" + upRect.left + ")");
    assert.strictEqual(await page.locator(".upload-panel .up-item.done").count(), 1, "upload panel shows finished item");
    await page.locator("#up-close").click();
    assert.strictEqual(await page.locator("#uploadPanel.hidden").count(), 1, "upload panel can be closed manually");
    await page.setInputFiles("#file-input", [{ name: "tiny2.txt", mimeType: "text/plain", buffer: Buffer.from("x") }]);
    await page.waitForSelector(".upload-panel:not(.hidden)", { timeout: 10000 });
    assert.strictEqual(await page.locator("#uploadPanel").count(), 1, "panel reopens on new upload");
    await sleep(800);
    const served = fs.readFileSync(SHARE_DIR + "/big.bin");
    assert.strictEqual(served.length, big.length, "uploaded size");
    assert(served.equals(big), "uploaded content identical");
    ok("chunked upload + resume (byte-identical)");
  } catch (e) { fail("upload/resume", e); }

  // ============ 10. download ============
  await closeModals(page);
  try {
    await page.reload();
    await page.waitForSelector(".share-item");
    await page.locator(".share-item", { hasText: "公开目录" }).click();
    await page.waitForSelector(".tile", { hasText: "pic.png" }, { timeout: 15000 });
    await openTile(page, "pic.png", "image");
    const [download] = await Promise.all([
      page.waitForEvent("download", { timeout: 15000 }),
      page.locator(".modal button[data-dl]").first().click(),
    ]);
    assert.strictEqual(download.suggestedFilename(), "pic.png");
    const dl = await download.createReadStream ? (await readStream(download)).toString() : "";
    assert(dl.length >= 0);
    ok("download via ?dl=1 with filename");
  } catch (e) { fail("download", e); }

  // ============ 10.5. select + download / rename / delete ============
  await closeModals(page);
  try {
    await page.reload();
    await page.waitForSelector(".share-item");
    await page.locator(".share-item", { hasText: "公开目录" }).click();
    await page.waitForSelector(".tile", { hasText: "pic.png" }, { timeout: 15000 });
    // selection is ONLY via checkbox: clicking the tile body must do nothing
    await page.locator(".tile", { hasText: "pic.png" }).click();
    assert.strictEqual(await page.locator(".modal").count(), 0, "single click must not open preview");
    assert.strictEqual(await page.locator(".tile.sel").count(), 0, "single click must not select");
    assert.strictEqual(await page.locator(".selbar:not(.hidden)").count(), 0, "no selbar after plain click");
    // checkbox = select (must not open preview)
    await page.locator(".tile", { hasText: "pic.png" }).locator(".sel-cb").click();
    assert.strictEqual(await page.locator(".modal").count(), 0, "checkbox does not open preview");
    await page.waitForSelector(".selbar:not(.hidden)", { timeout: 5000 });
    assert((await page.locator("#selbar .sb-count").innerText()).includes("已选 1 项"), "selbar says 1 after checkbox");
    assert.strictEqual(await page.locator("#sb-share").count(), 1, "share button for single file selection");
    const sbBox = await page.locator("#selbar").boundingBox();
    const tbBox = await page.locator("#toolbar").boundingBox();
    assert(sbBox && tbBox && sbBox.y < tbBox.y + tbBox.height + 2, "selbar pinned below toolbar (top), y=" + (sbBox && sbBox.y));
    assert.strictEqual(await page.locator("#sb-link").count(), 1, "copy link action present for single selection");
    await page.locator("#sb-link").click();
    await page.waitForFunction(() => [...document.querySelectorAll(".toast")].some((t) => t.textContent.includes("已复制")), { timeout: 5000 });
    // second checkbox = multi-select
    await page.locator(".tile", { hasText: "clip.mp4" }).locator(".sel-cb").click();
    await page.waitForSelector(".selbar:not(.hidden)", { timeout: 5000 });
    assert((await page.locator("#selbar .sb-count").innerText()).includes("已选 2 项"), "two checkboxes select two");
    // uncheck pic -> back to 1
    await page.locator(".tile", { hasText: "pic.png" }).locator(".sel-cb").click();
    assert((await page.locator("#selbar .sb-count").innerText()).includes("已选 1 项"), "uncheck reduces to 1");
    // click empty area clears selection
    await page.locator("#files").click({ position: { x: 5, y: 5 } });
    await page.waitForSelector("#selbar.hidden", { state: "attached", timeout: 5000 });
    assert.strictEqual(await page.locator(".tile.sel").count(), 0, "click-away clears selection");
    await page.locator(".tile", { hasText: "pic.png" }).locator(".sel-cb").click();
    assert.strictEqual(await page.locator(".modal").count(), 0, "checkbox does not open preview");
    await page.locator(".tile", { hasText: "clip.mp4" }).locator(".sel-cb").click();
    await page.waitForSelector(".selbar:not(.hidden)", { timeout: 5000 });
    assert((await page.locator("#selbar .sb-count").innerText()).includes("已选 2 项"), "selbar says 2");
    assert.strictEqual(await page.locator("#sb-dl").count(), 1, "download action present");
    assert.strictEqual(await page.locator("#sb-del").count(), 1, "delete action present");
    assert.strictEqual(await page.locator("#sb-link").count(), 1, "copy link action present for multi selection");
    const [zipDl] = await Promise.all([
      page.waitForEvent("download", { timeout: 20000 }),
      page.locator("#sb-dl").click(),
    ]);
    assert.strictEqual(zipDl.suggestedFilename(), "批量下载.zip");
    const zipFail = await zipDl.failure();
    const s = await zipDl.createReadStream();
    const bufs = [];
    for await (const c of s) bufs.push(c);
    const zbin = Buffer.concat(bufs);
    assert(zbin.length > 22, "zip not empty");
    assert.strictEqual(zbin.subarray(0, 4).toString("latin1"), "PK\u0003\u0004", "zip local header magic");
    const znames = zipEntries(zbin);
    assert.deepStrictEqual(znames.sort(), ["clip.mp4", "pic.png"], "zip contains both selected files");
    const zipLink = await page.evaluate(async () => {
      const tok = localStorage.getItem("wp_token");
      const p = encodeURIComponent(state.path);
      const names = encodeURIComponent("clip.mp4") + "," + encodeURIComponent("pic.png");
      const r = await fetch(`/api/zip?share_id=${encodeURIComponent(state.share.id)}&path=${p}&names=${names}`, { headers: { Authorization: `Bearer ${tok}` } });
      const b = await r.arrayBuffer();
      return { ok: r.ok, len: b.byteLength, magic: String.fromCharCode(...new Uint8Array(b).subarray(0, 4)) };
    });
    assert.strictEqual(zipLink.ok, true, "GET zip link returns ok");
    assert(zipLink.len > 22 && zipLink.magic === "PK\u0003\u0004", "GET zip link yields valid zip");
    await page.locator(".tile", { hasText: "clip.mp4" }).locator(".sel-cb").click();
    assert((await page.locator("#selbar .sb-count").innerText()).includes("已选 1 项"), "selbar says 1");
    await page.locator("#sb-rn").click();
    await page.waitForSelector("#rn-name", { timeout: 5000 });
    await page.fill("#rn-name", "pic_renamed.png");
    await page.click("#rn-ok");
    await page.waitForSelector('.tile:has-text("pic_renamed.png")', { timeout: 10000 });
    await page.waitForSelector('.tile:has-text("pic.png")', { state: "detached", timeout: 10000 });
    await page.locator(".tile", { hasText: "pic_renamed.png" }).locator(".sel-cb").click();
    await page.waitForSelector("#sb-del", { timeout: 5000 });
    await page.click("#sb-del");
    await page.waitForSelector("#del-ok", { timeout: 5000 });
    await page.click("#del-ok");
    await page.waitForSelector('.tile:has-text("pic_renamed.png")', { state: "detached", timeout: 10000 });
    await page.waitForSelector("#selbar.hidden", { state: "attached", timeout: 5000 });
    ok("select → batch download / rename / delete");
  } catch (e) { fail("select actions", e); }

  // ============ 11.5 share link: restricted file via public link ============
  await closeModals(page);
  try {
    await page.evaluate(async (sid) => {
      const tok = localStorage.getItem("wp_token");
      const h = { "Content-Type": "application/json", Authorization: "Bearer " + tok };
      const mk = await fetch("/api/mkdir", { method: "POST", headers: h, body: JSON.stringify({ share_id: Number(sid), path: "", name: "secret" }) });
      const rl = await fetch(`/api/shares/${sid}/rules`, { method: "POST", headers: h, body: JSON.stringify({ rel_path: "secret", access: "admin" }) });
      return { mk: mk.status, rl: rl.status };
    }, shareId).then((st) => { assert(st.mk === 200 && st.rl === 200, "mkdir/rules status: " + JSON.stringify(st)); });
    await page.reload();
    await page.waitForSelector(".share-item", { timeout: 15000 });
    await page.locator(".share-item", { hasText: "公开目录" }).click();
    await page.waitForSelector(".tile", { hasText: "secret" }, { timeout: 15000 });
    await page.locator(".tile", { hasText: "secret" }).dblclick();
    await page.waitForSelector("#files .empty", { timeout: 10000 });
    await page.setInputFiles("#file-input", [{ name: "topsecret.txt", mimeType: "text/plain", buffer: Buffer.from("TOP-SECRET-CONTENT") }]);
    await page.waitForSelector(".up-item.done", { timeout: 30000 });
    await sleep(600);
    await page.locator(".tile", { hasText: "topsecret.txt" }).dblclick();
    await page.waitForSelector(".modal.pv [data-share]", { timeout: 10000 });
    await page.locator(".modal.pv [data-share]").click();
    await page.waitForSelector("#fs-url", { timeout: 10000 });
    await page.click("#fs-create");
    await page.waitForFunction(() => document.getElementById("fs-url").value.length > 10, { timeout: 10000 });
    const link = await page.evaluate(() => document.getElementById("fs-url").value);
    const stok = link.split("/").pop();
    assert(link.startsWith(BASE), "share link absolute: " + link);
    const anonCtx = await browser.newContext();
    const dlResp = await anonCtx.request.get(link + "/api/download?dl=1");
    assert.strictEqual(dlResp.status(), 200, "anonymous download via link: 200");
    assert.strictEqual((await dlResp.body()).toString(), "TOP-SECRET-CONTENT", "anonymous download content matches");
    const browseResp = await anonCtx.request.get(BASE + `/api/browse/${shareId}?path=secret`);
    assert.notStrictEqual(browseResp.status(), 200, "anonymous browse of admin-only dir stays blocked");
    const landPage = await anonCtx.newPage();
    await landPage.goto(link);
    assert((await landPage.title()).includes("分享文件"), "landing page title");
    assert.strictEqual(await landPage.locator('a[href$="/api/download?dl=1"]').count(), 1, "landing download button");
    await landPage.close();
    await anonCtx.close();
    await page.click("#fs-revoke");
    await page.waitForFunction(() => document.getElementById("fs-url").value === "", { timeout: 10000 });
    const goneCtx = await browser.newContext();
    const goneResp = await goneCtx.request.get(link + "/api/download?dl=1");
    assert.strictEqual(goneResp.status(), 404, "revoked link returns 404");
    await goneCtx.close();
    await page.locator(".modal:not(.pv) .mhead [data-close='1']").click();
    await page.waitForSelector("#fs-url", { state: "detached", timeout: 5000 });
    await page.locator(".modal.pv .close").click();
    await page.waitForSelector(".modal", { state: "detached", timeout: 5000 });
    await page.evaluate(async (arg) => {
      const h = { "Content-Type": "application/json", Authorization: "Bearer " + localStorage.getItem("wp_token") };
      await fetch("/api/delete", { method: "POST", headers: h, body: JSON.stringify({ share_id: Number(arg.sid), path: "secret", recursive: true }) });
      await fetch(`/api/fileshares/${arg.tk}`, { method: "DELETE", headers: h });
    }, { sid: shareId, tk: stok });
    ok("share link: restricted file downloadable anonymously + revoke");
  } catch (e) {
    const extra = await page.evaluate(() => ({ sel: !!document.querySelector("#selbar.hidden"), empty: (document.getElementById("files") || {}).innerText?.slice(0, 120) || "", modal: document.querySelectorAll(".modal").length, pv: !!document.querySelector(".modal.pv") })).catch(() => ({}));
    fail("share link feature", new Error(e.message + " | page:" + JSON.stringify(extra)));
  }

  // ============ 11. toggle user flags ============
  await closeModals(page);
  try {
    await page.click("#btn-admin");
    await page.click('[data-pane="users"]');
    const aliceRow = page.locator("tr", { hasText: "alice" });
    await aliceRow.locator('.flag[data-f="can_upload"]').check();
    await aliceRow.locator('.flag[data-f="can_mkdir"]').check();
    await sleep(700);
    const persisted = await page.evaluate(async () => {
      const res = await fetch("/api/users", { headers: { Authorization: "Bearer " + localStorage.getItem("wp_token") } });
      const list = await res.json();
      const a = list.find((u) => u.username === "alice");
      return a.can_upload && a.can_mkdir;
    });
    assert(persisted === true, "flags persisted");
    await page.click('[data-pane="back"]');
    await page.waitForSelector(".share-item", { timeout: 15000 });
    ok("toggle user flags persist");
  } catch (e) { fail("flag toggle", e); }

  // ============ 12. alice login / home / upload ============
  const actx = await browser.newContext({ acceptDownloads: true });
  const apage = await actx.newPage();
  pageErrors(apage);
  await apage.goto(BASE + "/");
  await apage.waitForTimeout(600);
  if (await apage.locator("#btn-login2").count()) await apage.click("#btn-login2");
  try {
    await login(apage, "alice", "pw123");
    await apage.waitForSelector(".share-item", { timeout: 15000 });
    const ashareCount = await apage.locator(".share-item").count();
    assert(ashareCount >= 1, "alice sees share(s): " + ashareCount);
    assert.strictEqual(await apage.locator(".share-item", { hasText: "个人空间" }).count(), 1, "alice has home share");
    await apage.locator(".share-item").first().click();
    await apage.waitForSelector("#files", { timeout: 15000 });
    await apage.setInputFiles("#file-input", [{ name: "hello.txt", mimeType: "text/plain", buffer: Buffer.from("hello world") }]);
    const hdone = await (async () => { const t0 = Date.now(); while (Date.now() - t0 < 30000) { if (await apage.locator(".up-item.done").count()) return true; await sleep(800); } return false; })();
    assert(hdone, "alice upload done");
    await sleep(500);
    ok("alice login + private home + upload");
  } catch (e) { fail("alice flow", e); }

  // ============ 13. admin-only rule blocks alice ============
  try {
    await page.evaluate(async (sid) => {
      const tok = localStorage.getItem("wp_token");
      await fetch("/api/mkdir", { method: "POST", headers: { "Content-Type": "application/json", Authorization: "Bearer " + tok }, body: JSON.stringify({ share_id: Number(sid), path: "", name: "secret" }) });
      await fetch(`/api/shares/${sid}/rules`, { method: "POST", headers: { "Content-Type": "application/json", Authorization: "Bearer " + tok }, body: JSON.stringify({ rel_path: "secret", access: "admin" }) });
    }, shareId);
    await apage.locator(".share-item", { hasText: "公开目录" }).click();
    await apage.waitForSelector(".tile", { timeout: 15000 });
    const aliceNames = await apage.locator(".tile .tname").allTextContents();
    assert(!aliceNames.includes("secret"), "admin-only folder hidden from alice, got: " + aliceNames);
    ok("admin-only subfolder hidden from normal user");
  } catch (e) { fail("admin-only rule", e); }

  // ============ 14. guest rule + anonymous guest ============
  try {
    await page.evaluate(async (sid) => {
      const tok = localStorage.getItem("wp_token");
      await fetch(`/api/shares/${sid}/rules`, { method: "POST", headers: { "Content-Type": "application/json", Authorization: "Bearer " + tok }, body: JSON.stringify({ rel_path: "", access: "guest" }) });
    }, shareId);
    ok("set guest rule");
  } catch (e) { fail("set guest rule", e); }

  const gctx = await browser.newContext();
  const gpage = await gctx.newPage();
  pageErrors(gpage);
  await gpage.goto(BASE + "/");
  try {
    await gpage.waitForSelector(".shell", { timeout: 15000 });
    assert((await gpage.locator(".who").innerText()).includes("游客"), "guest label");
    assert.strictEqual(await gpage.locator("#btn-mkdir").count(), 0, "guest no mkdir");
    assert.strictEqual(await gpage.locator("#file-input").count(), 0, "guest no upload");
    await gpage.locator(".share-item", { hasText: "公开目录" }).click();
    await gpage.waitForSelector(".tile", { hasText: "clip.mp4" }, { timeout: 15000 });
    await gpage.locator(".tile", { hasText: "clip.mp4" }).dblclick();
    await gpage.waitForSelector("#pv-body video", { timeout: 15000 });
    ok("anonymous guest browse + stream");
  } catch (e) { fail("guest mode", e); }

  // ============ 15. password change ============
  await closeModals(page);
  try {
    await page.click("#btn-admin");
    await page.click('[data-pane="account"]');
    await page.fill("#cp-old", "admin123");
    await page.fill("#cp-new", "newpass");
    await page.click("#cp-btn");
    await page.waitForFunction(() => [...document.querySelectorAll(".toast")].some((t) => t.textContent.includes("密码已更新")), { timeout: 10000 });
    await page.click('[data-pane="back"]');
    await page.waitForSelector("#btn-logout", { timeout: 10000 });
    await page.click("#btn-logout");
    await page.waitForSelector(".login-card");
    await page.fill("#in-user", "admin");
    await page.fill("#in-pass", "admin123");
    const old1 = await page.evaluate(async () => {
      const r = await fetch("/api/captcha").then((x) => x.json());
      return { id: r.id, code: [...r.svg.matchAll(/<text[^>]*>([^<])<\/text>/g)].map((m) => m[1]).join("") };
    });
    await page.evaluate((c) => (document.getElementById("cap-id").value = c.id), old1);
    await page.fill("#in-captcha", old1.code);
    await page.click("#btn-login");
    await page.waitForTimeout(1200);
    assert(await page.locator(".login-card").isVisible(), "old password rejected");

    await page.fill("#in-user", "admin");
    await page.fill("#in-pass", "admin123");
    await page.fill("#in-captcha", "XXXX");
    await page.click("#btn-login");
    await page.waitForTimeout(1200);
    assert(await page.locator(".login-card").isVisible(), "bad captcha rejected");

    await login(page, "admin", "newpass");
    ok("password change + relogin");
  } catch (e) { fail("password change", e); }

  const passed = results.filter((r) => r[0] === "PASS").length;
  console.log(`\n==== RESULT: ${passed}/${results.length} passed ====`);
  if (passed !== results.length) { console.error("FAILED:", results.filter(r=>r[0]==="FAIL").map(x=>x[1])); process.exit(1); }
  await browser.close();
})().catch((e) => { console.error("FATAL", e.stack); process.exit(2); });

async function readStream(dl) {
  const s = await dl.createReadStream();
  let buf = Buffer.alloc(0);
  for await (const c of s) buf = Buffer.concat([buf, c]);
  return buf.toString();
}

function zipEntries(buf) {
  const names = [];
  let eocd = -1;
  const from = Math.max(0, buf.length - 65557);
  for (let i = from; i <= buf.length - 22; i++) {
    if (buf.readUInt32LE(i) === 0x06054b50) { eocd = i; break; }
  }
  if (eocd < 0) throw new Error("no EOCD");
  const count = buf.readUInt16LE(eocd + 10);
  let off = buf.readUInt32LE(eocd + 16);
  for (let k = 0; k < count; k++) {
    if (buf.readUInt32LE(off) !== 0x02014b50) throw new Error("bad central dir at " + off);
    const nl = buf.readUInt16LE(off + 28);
    const name = buf.toString("utf8", off + 46, off + 46 + nl);
    names.push(name);
    off += 46 + nl + buf.readUInt16LE(off + 30) + buf.readUInt16LE(off + 32);
  }
  return names;
}