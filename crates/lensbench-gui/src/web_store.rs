//! Web 版: 読み込んだファイル・ユーザーカタログをブラウザ（IndexedDB）に残す。
//!
//! 置き場（[`lensbench_core::vfs`]）のうち [`KEEP`] の下のファイルを、書いた・消したらすぐ IndexedDB に写し
//! （[`flush`]、毎フレーム）、ページを開き直したら起動の前に置き場へ戻す（[`restore`]）。
//! 設定（フォルダ・最近のファイル・並び）は eframe がブラウザの localStorage に残す。

use lensbench_core::vfs::sync;
use std::cell::Cell;
use wasm_bindgen::prelude::*;

/// 残すフォルダ（読み込んだファイルとユーザーカタログ）
pub const KEEP: [&str; 2] = ["/uploads/", "/user/"];

/// ユーザーカタログを置くフォルダ（レンズのフォルダにも入れる）
pub const USER_CATALOGS: &str = "/user/catalogs";

#[wasm_bindgen(inline_js = r#"
const DB = "lensbench", ST = "files";
let db = null;
function open() {
  if (db) return db;
  db = new Promise((res, rej) => {
    const q = indexedDB.open(DB, 1);
    q.onupgradeneeded = () => q.result.createObjectStore(ST);
    q.onsuccess = () => res(q.result);
    q.onerror = () => rej(q.error);
  });
  return db;
}
function done(t) {
  return new Promise((res, rej) => { t.oncomplete = res; t.onerror = t.onabort = () => rej(t.error); });
}
export async function store_load() {
  // 消されにくくする（ブラウザが認めたときだけ）
  try { await navigator.storage?.persist?.(); } catch (_) {}
  const d = await open();
  const t = d.transaction(ST, "readonly");
  const out = [];
  await new Promise((res, rej) => {
    const c = t.objectStore(ST).openCursor();
    c.onsuccess = () => {
      const x = c.result;
      if (!x) return res();
      out.push([x.key, new Uint8Array(x.value)]);
      x.continue();
    };
    c.onerror = () => rej(c.error);
  });
  return out;
}
export function store_put(changes) {
  open().then(d => {
    const t = d.transaction(ST, "readwrite");
    const s = t.objectStore(ST);
    for (const [k, v] of changes) v ? s.put(v, k) : s.delete(k);
    return done(t);
  }).catch(e => console.warn("lensbench: ブラウザに残せない", e));
}
export async function store_clear() {
  const d = await open();
  const t = d.transaction(ST, "readwrite");
  t.objectStore(ST).clear();
  await done(t);
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn store_load() -> Result<JsValue, JsValue>;
    fn store_put(changes: js_sys::Array);
    #[wasm_bindgen(catch)]
    async fn store_clear() -> Result<(), JsValue>;
}

thread_local! {
    /// この版より後の置き場の変更を、次の [`flush`] で写す
    static SEQ: Cell<Option<u64>> = const { Cell::new(None) };
}

fn kept(p: &str) -> bool {
    KEEP.iter().any(|k| p.starts_with(k))
}

/// ブラウザに残したファイルを置き場に戻す（起動の前に 1 回）。戻したファイルの数
pub async fn restore() -> usize {
    let n = match store_load().await {
        Ok(v) => {
            let a: js_sys::Array = v.unchecked_into();
            let mut n = 0;
            for e in a.iter() {
                let e: js_sys::Array = e.unchecked_into();
                let (Some(k), b) = (
                    e.get(0).as_string(),
                    e.get(1).unchecked_into::<js_sys::Uint8Array>(),
                ) else {
                    continue;
                };
                if kept(&k) && lensbench_core::vfs::write(&k, b.to_vec()).is_ok() {
                    n += 1;
                }
            }
            n
        }
        Err(e) => {
            web_sys::console::warn_2(&"lensbench: ブラウザに残したファイルを読めない".into(), &e);
            0
        }
    };
    SEQ.with(|s| s.set(Some(sync::seq())));
    n
}

/// 前の呼び出しから後に書いた・消した [`KEEP`] の下のファイルを IndexedDB に写す（毎フレーム呼ぶ）
pub fn flush() {
    let Some(last) = SEQ.with(Cell::get) else {
        return; // restore の前（通し確認など）
    };
    let now = sync::seq();
    if now == last {
        return;
    }
    SEQ.with(|s| s.set(Some(now)));
    let changes = js_sys::Array::new();
    for (p, b, _) in sync::changes_since(last) {
        if !kept(&p) {
            continue;
        }
        let v: JsValue = match b {
            Some(b) => js_sys::Uint8Array::from(&b[..]).into(),
            None => JsValue::NULL,
        };
        changes.push(&js_sys::Array::of2(&p.into(), &v));
    }
    if changes.length() > 0 {
        store_put(changes);
    }
}

/// ブラウザに残したファイルを全部消す（置き場からも）
pub fn clear() {
    for (p, _) in lensbench_core::vfs::files() {
        let s = p.to_string_lossy();
        if kept(&s) {
            let _ = lensbench_core::vfs::remove_file(&p);
        }
    }
    SEQ.with(|s| s.set(Some(sync::seq())));
    wasm_bindgen_futures::spawn_local(async {
        if let Err(e) = store_clear().await {
            web_sys::console::warn_2(&"lensbench: ブラウザに残したファイルを消せない".into(), &e);
        }
    });
}

/// ブラウザに残しているファイルの数と大きさの合計 [バイト]
pub fn usage() -> (usize, u64) {
    lensbench_core::vfs::files()
        .into_iter()
        .filter(|(p, _)| kept(&p.to_string_lossy()))
        .fold((0, 0), |(n, s), (_, b)| (n + 1, s + b))
}

/// 硝材・レンズのフォルダのうち、[`KEEP`] の下で置き場に無いものを外す
pub fn prune_dirs(glass: &mut Vec<String>, lens: &mut Vec<String>) {
    let gone = |d: &String| kept(d) && !lensbench_core::vfs::exists(d);
    glass.retain(|d| !gone(d));
    lens.retain(|d| !gone(d));
}
