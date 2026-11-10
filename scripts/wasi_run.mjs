// cargo の wasm32-wasip1 の試験を Node の WASI で回す（CARGO_TARGET_WASM32_WASIP1_RUNNER に置く。公開 #28）。
// 使い方: node scripts/wasi_run.mjs <試験の .wasm> [試験の引数...]。終了コードは試験のもの。
// 試験は env!("CARGO_MANIFEST_DIR") からの絶対パスで tests/ を読むので、リポジトリの根も同じパスで見せる
//
// GitHub Actions の Node 22 はときどき同じ .wasm の途中で SIGSEGV で落ちる（試験の失敗ではなく Node 自体が落ちる。
// 手元では再現しない）。そのときだけ子の Node を 1 回だけ回し直し、注記（::warning::）を残す。
// 試験が失敗した（終了コード 0 以外）ときは回し直さない
import { readFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { WASI } from "node:wasi";
import { argv, env, exit, execArgv, execPath } from "node:process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const self = fileURLToPath(import.meta.url);
const [file, ...args] = argv.slice(2);

if (env.LB_WASI_CHILD !== "1") {
  const MAX = 2;
  for (let n = 1; ; n++) {
    const r = spawnSync(execPath, [...execArgv, self, file, ...args], {
      stdio: "inherit",
      env: { ...env, LB_WASI_CHILD: "1" },
    });
    if (r.error) throw r.error;
    if (r.signal === null) exit(r.status ?? 1);
    const msg = `Node が ${r.signal} で落ちた（${path.basename(file)}、${n} 回目）`;
    if (n >= MAX || r.signal !== "SIGSEGV") {
      console.error(`::error::${msg}`);
      exit(1);
    }
    console.error(`::warning::${msg}。回し直す`);
  }
}

const root = path.resolve(path.dirname(self), "..");
const wasi = new WASI({ version: "preview1", args: [file, ...args], env, preopens: { ".": ".", [root]: root } });
const wasm = await WebAssembly.compile(await readFile(file));
const inst = await WebAssembly.instantiate(wasm, wasi.getImportObject());
exit(wasi.start(inst) ?? 0);
