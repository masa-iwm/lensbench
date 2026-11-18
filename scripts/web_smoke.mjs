// Web 版の起動確認: 配る形のフォルダ（scripts/build_web.sh の出力）を配って Chromium で開き、
// 起動できること・JavaScript のエラーと Rust の panic が出ないこと、見本の系と落としたプロジェクトの解析が
// 誤りなく結果を出すこと（wasm の試験用の口 test_state で要求が片付くのを待つ）、解析のパネルを別の窓に出すと
// その窓が自分で計算し、ブラウザ（IndexedDB）に何も書かないことを確かめ、画面を撮る。
//
//   node scripts/web_smoke.mjs target/web shot.png
//
// 要るもの: npm の playwright（Chromium）
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";

const [dir = "target/web", shot = "web-smoke.png"] = process.argv.slice(2);
const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm",
  ".png": "image/png", ".ttf": "font/ttf" };
const server = http.createServer((req, res) => {
  const p = path.join(dir, decodeURIComponent(new URL(req.url, "http://x").pathname));
  const f = fs.existsSync(p) && fs.statSync(p).isDirectory() ? path.join(p, "index.html") : p;
  if (!fs.existsSync(f)) {
    res.writeHead(404).end();
    return;
  }
  // _headers と同じ（スレッドに要る）。HEADERS=0 は GitHub Pages と同じく付けない（coi.js が付け直す）
  const h = { "Content-Type": types[path.extname(f)] ?? "application/octet-stream" };
  if (process.env.HEADERS !== "0") {
    h["Cross-Origin-Opener-Policy"] = "same-origin";
    h["Cross-Origin-Embedder-Policy"] = "require-corp";
  }
  res.writeHead(200, h);
  fs.createReadStream(f).pipe(res);
});
await new Promise(r => server.listen(0, "127.0.0.1", r));
// ST=1 は 1 スレッドの版（?st）
const url = `http://127.0.0.1:${server.address().port}/${process.env.ST ? "?st" : ""}`;

const browser = await chromium.launch({
  args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--disable-popup-blocking"],
});
const context = await browser.newContext({ viewport: { width: 1400, height: 860 }, locale: "ja-JP" });
// ページごとに IndexedDB へ書いた回数を数える（別の窓は書かないはず）
await context.addInitScript(() => {
  window.__idbWrites = 0;
  for (const m of ["put", "add", "delete", "clear"]) {
    const f = IDBObjectStore.prototype[m];
    IDBObjectStore.prototype[m] = function (...a) {
      window.__idbWrites++;
      return f.apply(this, a);
    };
  }
});
const page = await context.newPage();
const errors = [];
const watch = (p, tag) => {
  p.on("pageerror", e => errors.push(`${tag}pageerror: ${e}`));
  p.on("console", m => {
    if (m.type() === "error" || /panicked/.test(m.text())) errors.push(`${tag}console: ${m.text()}`);
  });
};
watch(page, "");
const t0 = Date.now();
await page.goto(url);
try {
  // 起動すると index.html の「読み込み中」の表示が消える
  await page.waitForFunction(() => !document.getElementById("msg"), null, { timeout: 120000 });
} catch {
  errors.push(`起動しない: ${await page.locator("#msg").textContent().catch(() => "")}`);
}
console.log(`起動 ${Date.now() - t0} ms`);
const iso = await page.evaluate(() => [crossOriginIsolated, globalThis.lensbenchWorker]);
console.log(`cross-origin isolated ${iso[0]}、計算の Worker ${iso[1]}`);
if (process.env.EXPECT_MT === "1" && iso[1] !== "worker.js?mt") errors.push("スレッドありの版になっていない");
// 画面のスレッドが止まった長さ（計算は Web Worker で回るので、解析中も短いはず）
await page.evaluate(() => {
  window.__gap = 0;
  let last = performance.now();
  const f = t => { window.__gap = Math.max(window.__gap, t - last); last = t; requestAnimationFrame(f); };
  requestAnimationFrame(f);
});
// 画面の今の様子（wasm の試験用の口 test_state、GAR-25）。画面は canvas なので DOM からは見えない。
// index.html が読んだのと同じモジュール（ES モジュールは 1 回だけ読まれる）
const state = (p = page) => p.evaluate(async () => JSON.parse((await import("./lensbench_web.js")).test_state()));
// 要求が片付き、cond を満たすまで待つ（固定の時間で待たない）。待ちきれなければ最後の様子を返して誤りに足す
async function settle(what, cond, timeout = 120000, p = page) {
  const t = Date.now();
  let s;
  // 片付いた様子が 2 回続けて見えたら（次の要求を出す前の 1 フレームを拾わないように）
  let calm = 0;
  while (Date.now() - t < timeout) {
    s = await state(p).catch(() => ({}));
    calm = s.started && !s.busy && cond(s) ? calm + 1 : 0;
    if (calm >= 2) {
      console.log(`${what}: ${Date.now() - t} ms`);
      return s;
    }
    await p.waitForTimeout(250);
  }
  errors.push(`${what}: ${timeout / 1000} 秒で片付かない（${JSON.stringify(s)}）`);
  return s;
}
// 解析パネルの結果: 1 つ以上あり、誤りのものが無い
const analysesOk = (what, s) => {
  const bad = (s.analyses ?? []).filter(a => a.error);
  for (const a of bad) errors.push(`${what}: 解析 ${a.cmd} が誤り: ${a.error}`);
  if (!(s.analyses ?? []).some(a => a.ok)) errors.push(`${what}: 結果の出た解析パネルが無い（${JSON.stringify(s)}）`);
  if (s.status_error) errors.push(`${what}: 下の行が誤り: ${s.status}`);
};
// 起動時に開く見本の系を読み、解析が回り終わるのを待つ
const s0 = await settle("見本の系の解析", s => s.system && (s.analyses ?? []).some(a => a.ok || a.error));
analysesOk("見本の系", s0);
// 開く: 例題のプロジェクトを canvas に落とす（窓にファイルを落とすのと同じ道。ファイル選択の窓は CI で出ないことがある）
const file = path.resolve("examples/projects/P8_catalog_two.json");
const name = path.basename(file);
await page.evaluate(([n, text]) => {
  const dt = new DataTransfer();
  dt.items.add(new File([text], n, { type: "application/json" }));
  const c = document.querySelector("canvas");
  for (const type of ["dragenter", "dragover", "drop"]) {
    c.dispatchEvent(new DragEvent(type, { dataTransfer: dt, bubbles: true, cancelable: true }));
  }
}, [name, fs.readFileSync(file, "utf8")]);
// 開いたファイルの名前が題に出て、その系の解析が回り終わるまで待つ
await page.waitForFunction(n => document.title.includes(n), name, { timeout: 60000 })
  .catch(async () => errors.push(`落としたのに題が「${name}」にならない（題: ${await page.title()}）`));
const s1 = await settle("開いたプロジェクトの解析", s => s.sys_gen > s0.sys_gen && (s.file === name || s.doc === name));
analysesOk("開いたプロジェクト", s1);
const gap = await page.evaluate(() => window.__gap);
// 別の窓: 解析のパネルを別の窓に出すと、その窓が自分の計算の Worker で解析を回し、ブラウザに何も残さない
if (!process.env.NO_POPUP) {
  const popupP = context.waitForEvent("page", { timeout: 60000 });
  const moved = await page.evaluate(async () => (await import("./lensbench_web.js")).test_detach_analysis());
  // 画面は入力が無いと描き直さないので、マウスを動かして次のフレームを回す
  for (let i = 0; i < 5; i++) await page.mouse.move(300 + 10 * i, 300);
  if (!moved) errors.push("別の窓: 出せる解析のパネルが無い");
  else {
    const popup = await popupP.catch(() => null);
    if (!popup) errors.push(`別の窓が開かない（${JSON.stringify(await state())}）`);
    else {
      watch(popup, "別の窓: ");
      await popup.waitForFunction(() => !document.getElementById("msg"), null, { timeout: 120000 }).catch(() => {});
      const s2 = await settle("別の窓の解析", s => s.calc_window && s.system && (s.analyses ?? []).some(a => a.ok || a.error),
        120000, popup);
      analysesOk("別の窓", s2);
      // 元のタブが変わったものを書き終えるまで少し待ってから数える
      await popup.waitForTimeout(3000);
      const w = await popup.evaluate(() => window.__idbWrites);
      const wm = await page.evaluate(() => window.__idbWrites);
      console.log(`IndexedDB へ書いた回数: 元のタブ ${wm}、別の窓 ${w}`);
      if (w !== 0) errors.push(`別の窓が IndexedDB に ${w} 回書いた（残すのは元のタブだけ）`);
      if (wm === 0) errors.push("元のタブが IndexedDB に書いていない（数え方が効いていない）");
    }
  }
}
console.log(`画面が止まった最長 ${gap.toFixed(0)} ms`);
await page.screenshot({ path: shot });
await browser.close();
server.close();
if (errors.length) {
  console.error(errors.join("\n"));
  process.exit(1);
}
console.log("ok");
