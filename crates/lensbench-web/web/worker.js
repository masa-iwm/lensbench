// lensbench の計算を回す Web Worker。中は画面と同じ wasm で、受け渡しはバイト列
// （lensbench-gui の wire.rs）。最初のメッセージ（数）で何を受けるかを決める。
// URL に ?mt があれば、スレッドありの版（mt/）を読み、serve と同じ要求を受ける Worker（0）は
// rayon のスレッドを作る（ページが cross-origin isolated のときだけ index.html が付ける）。
// mt/ を読めないか、スレッドを作れなければ 1 スレッドの版に戻す
let mt = new URL(self.location.href).searchParams.has("mt");
let lb = null;
let failed = null;

async function load() {
  if (mt) {
    try {
      lb = await import("./mt/lensbench_web.js");
      await lb.default();
      return;
    } catch (e) {
      console.warn("スレッドありの版を読めないので 1 スレッドで回す", e);
      mt = false;
    }
  }
  lb = await import("./lensbench_web.js");
  await lb.default();
}

// メッセージは来た順に 1 つずつ（スレッドを作り終えてから次へ）
let queue = load().catch(e => { failed = String(e); });

// 見張りのスレッド（monitor.js、同じ wasm・同じメモリ）を作り、画面にその口（MessagePort）を渡す。
// このスレッドが計算で塞がっている間も、画面は見張りを通して中断・停止・進み具合を頼める
let monitor = null;
function startMonitor() {
  try {
    const [module, memory] = lb.worker_shared();
    monitor = new Worker(new URL("monitor.js", import.meta.url), { type: "module" });
    const ch = new MessageChannel();
    monitor.postMessage({ module, memory, port: ch.port1 }, [ch.port1]);
    self.postMessage({ monitor: ch.port2 }, [ch.port2]);
  } catch (e) {
    console.warn("見張りのスレッドを作れない（中断・進み具合は使えない）", e);
  }
}

async function handle(data) {
  try {
    if (failed) throw new Error("wasm を読み込めない: " + failed);
    if (typeof data === "number") {
      await lb.worker_init(data);
      if (mt && data === 0) {
        try {
          await lb.initThreadPool(Math.max(1, Math.min(navigator.hardwareConcurrency || 4, 16)));
          lb.threads_ready();
          startMonitor();
        } catch (e) {
          console.warn("スレッドを作れないので 1 スレッドの版に戻す", e);
          mt = false;
          lb = await import("./lensbench_web.js");
          await lb.default();
          await lb.worker_init(data);
        }
      }
      return;
    }
    const out = lb.worker_call(data);
    self.postMessage(out, [out.buffer]);
  } catch (err) {
    self.postMessage({ error: String((err && err.stack) || err) });
  }
}

self.onmessage = (e) => {
  queue = queue.then(() => handle(e.data));
};
