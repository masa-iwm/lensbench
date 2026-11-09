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

async function handle(data) {
  try {
    if (failed) throw new Error("wasm を読み込めない: " + failed);
    if (typeof data === "number") {
      lb.worker_init(data);
      if (mt && data === 0) {
        try {
          await lb.initThreadPool(Math.max(1, Math.min(navigator.hardwareConcurrency || 4, 16)));
          lb.threads_ready();
        } catch (e) {
          console.warn("スレッドを作れないので 1 スレッドの版に戻す", e);
          mt = false;
          lb = await import("./lensbench_web.js");
          await lb.default();
          lb.worker_init(data);
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
