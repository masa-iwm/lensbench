// Web 版の起動確認: 配る形のフォルダ（scripts/build_web.sh の出力）を配って Chromium で開き、
// 起動できること・JavaScript のエラーと Rust の panic が出ないことを確かめ、画面を撮る。
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
  args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
});
const page = await browser.newPage({ viewport: { width: 1400, height: 860 }, locale: "ja-JP" });
const errors = [];
page.on("pageerror", e => errors.push(`pageerror: ${e}`));
page.on("console", m => {
  if (m.type() === "error" || /panicked/.test(m.text())) errors.push(`console: ${m.text()}`);
});
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
// 見本の系を読んで解析が回るのを待つ
await page.waitForTimeout(10000);
// 開く: Ctrl+O → ブラウザのファイル選択（rfd の窓の Ok）で例題のプロジェクトを読み込む。
// CI ではときどきファイル選択が出ない（キーが画面に届かない、または rfd の input.click() がユーザー操作の扱いに
// ならない）。rfd の窓が出ていればその input に直にファイルを入れ、窓も出ていなければもう 1 回 Ctrl+O を押す。
// どちらも ::warning:: を残す（起きる頻度を見るため）
const file = path.resolve("examples/projects/P8_catalog_two.json");
const okButton = page.getByRole("button", { name: "Ok" });
let opened = false;
for (let n = 1; n <= 2 && !opened; n++) {
  await page.mouse.click(700, 400);
  const chooser = page.waitForEvent("filechooser", { timeout: 15000 }).catch(() => null);
  await page.keyboard.press("Control+o");
  const fc = await chooser;
  if (fc) {
    await fc.setFiles([file]);
    opened = true;
  } else if (await okButton.isVisible()) {
    console.log(`::warning::rfd の窓は出たがファイル選択が出ない（${n} 回目）。input に直に入れる`);
    await page.locator("input[type=file]").last().setInputFiles(file);
    opened = true;
  } else {
    console.log(`::warning::Ctrl+O でファイル選択も rfd の窓も出ない（${n} 回目）`);
  }
}
if (opened) {
  await okButton.click().catch(e => errors.push(`Ok を押せない: ${e}`));
  // 開いたファイルの名前がタブの題に出るまで待つ（固定の待ち時間でなく、読み込めたことを確かめる）
  const name = path.basename(file);
  await page.waitForFunction(n => document.title.includes(n), name, { timeout: 60000 })
    .catch(async () => errors.push(`開いたのに題が「${name}」にならない（題: ${await page.title()}）`));
  // 読み込んだ系の解析が回る間も画面が止まらないかを見る
  await page.waitForTimeout(3000);
} else {
  errors.push("Ctrl+O でファイル選択が出ない");
}
const gap = await page.evaluate(() => window.__gap);
console.log(`画面が止まった最長 ${gap.toFixed(0)} ms`);
await page.screenshot({ path: shot });
await browser.close();
server.close();
if (errors.length) {
  console.error(errors.join("\n"));
  process.exit(1);
}
console.log("ok");
