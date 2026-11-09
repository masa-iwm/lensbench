// COOP/COEP のヘッダーを付け直す Service Worker。GitHub Pages はヘッダーを付けられないので、
// これでページを cross-origin isolated にして、スレッドありの版（mt/）を使えるようにする。
// 置き場はページと同じオリジンだけなので require-corp で困らない。キャッシュはしない
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));
self.addEventListener("fetch", (e) => {
  const r = e.request;
  if (r.cache === "only-if-cached" && r.mode !== "same-origin") return;
  e.respondWith(
    fetch(r).then((res) => {
      if (res.status === 0) return res;
      const h = new Headers(res.headers);
      h.set("Cross-Origin-Opener-Policy", "same-origin");
      h.set("Cross-Origin-Embedder-Policy", "require-corp");
      return new Response(res.body, { status: res.status, statusText: res.statusText, headers: h });
    })
  );
});
