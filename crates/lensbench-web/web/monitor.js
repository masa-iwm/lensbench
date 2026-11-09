// 計算の Web Worker（worker.js、スレッドありの版）の見張り。同じ wasm を同じ共有メモリで動かし、
// 計算のスレッドが塞がっている間も、画面からの頼み（MessagePort）を受ける:
//   {cancel: id}            要求 id を回していれば、解析に途中でやめるよう頼む
//   {stop: what, id}        要求 id を回していれば、最適化（optimize）・公差（tolerance）・差し替え探索（swap）を止める
//   {progress: 1}           進み具合（JSON の文字列）を返す
self.onmessage = async ({ data }) => {
  const { module, memory, port } = data;
  const lb = await import("./mt/lensbench_web.js");
  await lb.default({ module_or_path: module, memory });
  port.onmessage = ({ data: m }) => {
    try {
      if (m.cancel) lb.monitor_cancel(m.cancel);
      else if (m.stop) lb.monitor_stop(m.id, m.stop);
      else if (m.progress) port.postMessage(lb.monitor_progress());
    } catch (e) {
      console.warn("lensbench の見張り", e);
    }
  };
};
