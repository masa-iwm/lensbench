// cargo の wasm32-wasip1 の試験を Node の WASI で回す（CARGO_TARGET_WASM32_WASIP1_RUNNER に置く。公開 #28）。
// 使い方: node scripts/wasi_run.mjs <試験の .wasm> [試験の引数...]。終了コードは試験のもの
import { readFile } from "node:fs/promises";
import { WASI } from "node:wasi";
import { argv, env, exit } from "node:process";

const [file, ...args] = argv.slice(2);
const wasi = new WASI({ version: "preview1", args: [file, ...args], env, preopens: { ".": "." } });
const wasm = await WebAssembly.compile(await readFile(file));
const inst = await WebAssembly.instantiate(wasm, wasi.getImportObject());
exit(wasi.start(inst) ?? 0);
