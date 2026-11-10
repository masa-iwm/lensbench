// Web 版とネイティブの解析の速さを同じマシンで比べる。docs/web.md の「速さ」
//
//   node scripts/web_bench.mjs [target/web] [--exe target/release/lensbench] [--systems A0,C0,A2,C1]
//        [--cmds spot,wavefront,mtf,psf] [--rounds 3] [--json out.json]
//
// 解析に引数を付けるときは「解析:名前=値:…」（例 spot:rings=60 は光線の多いスポット。値は数のまま渡す）
//
// ネイティブは `lensbench serve`（--threads 1 と全コア）、Web 版は配る形のフォルダ（scripts/build_web.sh の出力）を
// Chromium で開き、計算の Web Worker（worker.js。1 スレッドの版と mt/ の版）に同じ要求（load・analyze）を渡す。
// 時間はどちらも計算の側で測った 1 回の解析の時間（serve の elapsed_ms、Worker の応答の ms）の中央値で、
// 受け渡しと描画は含まない。最初の 1 回は捨てる（ガラスの読み込みなど）。
//
// 要るもの: npm の playwright（Chromium）、cargo build --release -p lensbench-cli
import http from "node:http";
import net from "node:net";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { spawn } from "node:child_process";
import { chromium } from "playwright";

const argv = process.argv.slice(2);
const opt = (name, def) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 ? argv.splice(i, 2)[1] : def;
};
const exe = opt("exe", "target/release/lensbench" + (process.platform === "win32" ? ".exe" : ""));
const systems = opt("systems", "A0,C0,A2,C1").split(",");
const cmds = opt("cmds", "spot,wavefront,mtf,psf").split(",");
const rounds = Number(opt("rounds", "3"));
const jsonOut = opt("json", null);
const [dir = "target/web"] = argv;
const cores = os.availableParallelism ? os.availableParallelism() : os.cpus().length;

const median = xs => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length ? (s[(s.length - 1) >> 1] + s[s.length >> 1]) / 2 : NaN;
};
// 系ごとに load してから、解析ごとに rounds + 1 回（最初は捨てる）
function plan(file, glassDir) {
  const reqs = [{ method: "load", params: { file, glass_dir: [glassDir] } }];
  for (const cmd of cmds) {
    const [name, ...kv] = cmd.split(":");
    const params = { cmd: name };
    for (const a of kv) {
      const [k, v] = a.split("=");
      params[k] = v === undefined ? true : isNaN(Number(v)) ? v : Number(v);
    }
    for (let r = 0; r <= rounds; r++) reqs.push({ method: "analyze", params, cmd, warm: r === 0 });
  }
  return reqs;
}
function collect(reqs, ms) {
  const out = {};
  reqs.forEach((q, i) => {
    if (q.cmd && !q.warm) (out[q.cmd] ??= []).push(ms[i]);
  });
  return Object.fromEntries(Object.entries(out).map(([k, v]) => [k, median(v)]));
}

// ---- ネイティブ（serve に JSON-RPC）
async function native(threads) {
  const p = spawn(exe, ["--threads", String(threads), "serve", "--port", "0"], { stdio: ["ignore", "pipe", "ignore"] });
  const port = await new Promise((res, rej) => {
    let buf = "";
    p.stdout.on("data", d => {
      buf += d;
      const i = buf.indexOf("\n");
      if (i >= 0) res(JSON.parse(buf.slice(0, i)).port);
    });
    p.on("exit", c => rej(new Error(`serve が終わった (${c})`)));
  });
  const sock = net.connect(port, "127.0.0.1");
  await new Promise(r => sock.once("connect", r));
  let buf = "";
  const waiting = [];
  sock.on("data", d => {
    buf += d;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, i);
      buf = buf.slice(i + 1);
      waiting.shift()(JSON.parse(line));
    }
  });
  let id = 0;
  const rpc = (method, params) => new Promise(r => {
    waiting.push(r);
    sock.write(JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }) + "\n");
  });
  const res = {};
  for (const s of systems) {
    const reqs = plan(path.resolve(`tests/refs/${s}/system.zmx`), path.resolve("tests/glass"));
    const ms = [];
    for (const q of reqs) {
      const a = await rpc(q.method, q.params);
      if (a.error) throw new Error(`ネイティブ ${s} ${q.method} ${q.cmd ?? ""}: ${a.error.message}`);
      ms.push(a.elapsed_ms);
    }
    res[s] = collect(reqs, ms);
  }
  sock.end();
  p.kill();
  return res;
}

// ---- Web 版（Chromium の中の worker.js）
const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm",
  ".png": "image/png", ".ttf": "font/ttf" };
const server = http.createServer((req, res) => {
  const u = new URL(req.url, "http://x").pathname;
  const h = { "Cross-Origin-Opener-Policy": "same-origin", "Cross-Origin-Embedder-Policy": "require-corp" };
  if (u === "/__bench") {
    res.writeHead(200, { ...h, "Content-Type": "text/html" }).end("<!doctype html><title>bench</title>");
    return;
  }
  const f = path.join(dir, decodeURIComponent(u));
  if (!fs.existsSync(f) || fs.statSync(f).isDirectory()) {
    res.writeHead(404).end();
    return;
  }
  res.writeHead(200, { ...h, "Content-Type": types[path.extname(f)] ?? "application/octet-stream" });
  fs.createReadStream(f).pipe(res);
});

async function web(page, mt) {
  const res = {};
  for (const s of systems) {
    const reqs = plan(`/lensbench/tests/refs/${s}/system.zmx`, "/lensbench/tests/glass");
    const ms = await page.evaluate(async ({ reqs, mt }) => {
      // 受け渡しの形は lensbench-gui の wire.rs（u32 の長さ＋中身、リトルエンディアン）
      const te = new TextEncoder(), td = new TextDecoder();
      const enc = (method, params) => {
        const m = te.encode(method), p = te.encode(JSON.stringify(params));
        const b = new Uint8Array(4 * 6 + m.length + p.length);
        const v = new DataView(b.buffer);
        v.setUint32(0, 1, true); // 要求の番号
        v.setUint32(4, 0, true); // スレッド数: 既定
        v.setUint32(8, 1, true); // 呼び出しの数
        v.setUint32(12, m.length, true);
        b.set(m, 16);
        v.setUint32(16 + m.length, p.length, true);
        b.set(p, 20 + m.length);
        v.setUint32(20 + m.length + p.length, 0, true); // ファイル: なし
        return b;
      };
      const dec = b => {
        const v = new DataView(b.buffer, b.byteOffset, b.byteLength);
        const ms = v.getFloat64(0, true);
        let err = null;
        if (v.getUint32(8, true) > 0 && b[12] === 0) {
          const n = v.getUint32(13, true);
          err = td.decode(b.subarray(17, 17 + n));
        }
        return { ms, err };
      };
      globalThis.__w ??= {};
      let w = globalThis.__w[mt];
      if (!w) {
        w = globalThis.__w[mt] = new Worker(mt ? "worker.js?mt" : "worker.js", { type: "module" });
        w.postMessage(0);
      }
      const ms = [];
      for (const q of reqs) {
        const out = await new Promise((r, j) => {
          w.onmessage = e => {
            if (e.data instanceof Uint8Array) r(e.data);
            else if (!e.data.monitor) j(new Error(e.data.error)); // 見張りの口は使わない
          };
          w.onerror = e => j(new Error(e.message));
          w.postMessage(enc(q.method, q.params));
        });
        const { ms: t, err } = dec(out);
        if (err) throw new Error(`${q.method} ${q.cmd ?? ""}: ${err}`);
        ms.push(t);
      }
      return ms;
    }, { reqs, mt });
    res[s] = collect(reqs, ms);
  }
  return res;
}

await new Promise(r => server.listen(0, "127.0.0.1", r));
const browser = await chromium.launch();
const page = await browser.newPage();
const warns = [];
page.on("console", m => { if (m.type() === "warning" || m.type() === "error") warns.push(m.text()); });
await page.goto(`http://127.0.0.1:${server.address().port}/__bench`);
const iso = await page.evaluate(() => crossOriginIsolated);
if (!iso) throw new Error("ページが cross-origin isolated にならない");

const cols = [
  ["native 1T", () => native(1)],
  [`native ${cores}T`, () => native(cores)],
  ["web 1T", () => web(page, false)],
  [`web mt`, () => web(page, true)],
];
const result = {};
for (const [name, f] of cols) {
  console.error(`… ${name}`);
  result[name] = await f();
}
await browser.close();
server.close();
if (warns.length) console.error(warns.join("\n"));

const fmt = x => (x >= 100 ? x.toFixed(0) : x >= 10 ? x.toFixed(1) : x.toFixed(2));
const names = cols.map(c => c[0]);
console.log(`${os.cpus()[0].model.trim()}、${cores} コア、解析 1 回の時間 [ms]（${rounds} 回の中央値）と比`);
console.log();
console.log(`| 系 | 解析 | ${names.join(" | ")} | web 1T ÷ native 1T | web mt ÷ native ${cores}T |`);
console.log(`| --- | --- | ${names.map(() => "---:").join(" | ")} | ---: | ---: |`);
for (const s of systems) {
  for (const c of cmds) {
    const v = names.map(n => result[n][s][c]);
    console.log(`| ${s} | ${c} | ${v.map(fmt).join(" | ")} | ${(v[2] / v[0]).toFixed(2)} | ${(v[3] / v[1]).toFixed(2)} |`);
  }
}
if (jsonOut) fs.writeFileSync(jsonOut, JSON.stringify({ cores, rounds, result }, null, 1));
