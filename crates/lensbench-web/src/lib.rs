//! lensbench の Web 版の入口。GUI（lensbench-gui）をブラウザのキャンバスで動かす。
//!
//! ファイルシステムが無いので、見本の系・公開用のガラス・例題のカタログを wasm に同梱し、起動時に
//! [`lensbench_core::vfs`] の置き場（/lensbench/...）へ入れる。日本語のフォントはページ（index.html）が
//! 読み込んで [`start`] に渡す。
//!
//! 計算は Web Worker（`worker.js`）の中の同じ wasm で回す（[`worker_init`]・[`worker_call`]）。
//!
//! 別の入口（非公開版など）は、この crate を依存にした wasm の crate を作り、`#[wasm_bindgen(start)]` で
//! カタログの形式・プラグインを登録して [`set_extra`] でファイルを足す（JS の口・ページはこの crate のまま）。
//! スレッドありの版では、同じ wasm・同じメモリの見張りのスレッド（`monitor.js`）が、計算の途中で
//! 中断・停止・進み具合の問い合わせを受ける（`monitor_*`）。

/// 並列: rayon のスレッドを Web Worker で作る（JS の initThreadPool(n)）。作った後に threads_ready
#[cfg(all(target_arch = "wasm32", feature = "threads"))]
pub use wasm_bindgen_rayon::init_thread_pool;

/// 別の入口（非公開版など）が足すファイルとカタログのフォルダ。[`set_extra`]
#[derive(Default, Clone)]
pub struct Extra {
    /// 置き場に入れるファイル（パス、中身）。画面と計算の Web Worker の両方に入れる
    pub files: Vec<(&'static str, &'static [u8])>,
    /// 起動のときに読んで置き場に入れる、ファイルの束（ページからの相対の URL）。大きいデータを wasm に
    /// 入れずに配る（Cloudflare Pages は 1 ファイル 25 MiB まで）。画面と計算の Web Worker がそれぞれ読む
    /// （2 回目からはブラウザのキャッシュ）。束の形は [`unpack`]
    pub packs: Vec<&'static str>,
    /// 硝材のフォルダ（置き場のパス）
    pub glass_dirs: Vec<String>,
    /// レンズカタログのフォルダ（置き場のパス）
    pub lens_dirs: Vec<String>,
    /// 前もって作ったカタログの一覧（置き場のパス、`.json` か `.json.gz`。束に入れておく）。既定のフォルダの並び
    /// （見本の LB・LBP と lens_dirs、ガラスは見本と glass_dirs）で一覧を頼まれたら、読む代わりにこれを返す
    /// （`lensbench_cli::set_prepared_lens_list`）。ビルドのときに `lensbench catalog-search --json` で作る
    pub lens_index: Option<&'static str>,
    /// 品の STEP の共有の置き場の URL（例 `/cad`。同じサイトの口なら Access のログインがそのまま効く）。
    /// あれば、カタログの詳細から品の STEP を取り、3D・書き出しで機構部品を詳しい形で描ける（#158）
    pub cad_store: Option<&'static str>,
}

static EXTRA: std::sync::Mutex<Option<Extra>> = std::sync::Mutex::new(None);

/// 別の入口（この crate を依存にした wasm の crate）が、起動より前（`#[wasm_bindgen(start)]`）に、同梱する
/// ファイルとフォルダを足す。カタログの形式・プラグインの登録も同じ所で行う
pub fn set_extra(e: Extra) {
    *EXTRA.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn extra() -> Extra {
    EXTRA
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// ファイルの束を解く: u32 の数、続けてファイルごとに（u32 の長さ＋パス（UTF-8）、u32 の長さ＋中身）。
/// 数はすべてリトルエンディアン
/// ファイルの束が gzip なら展開する（先頭が gzip の印でなければそのまま。置き場が `Content-Encoding: gzip` で
/// 配って、ブラウザが先に展開した場合も読める）
pub fn ungzip(b: Vec<u8>) -> Result<Vec<u8>, String> {
    if !b.starts_with(&[0x1f, 0x8b]) {
        return Ok(b);
    }
    let mut out = Vec::with_capacity(b.len() * 4);
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&b[..]), &mut out)
        .map_err(|e| format!("gzip を展開できない: {e}"))?;
    Ok(out)
}

pub fn unpack(b: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
        if b.len() < n {
            return Err("ファイルの束が途中で切れている".into());
        }
        let (x, rest) = b.split_at(n);
        *b = rest;
        Ok(x)
    }
    fn u32(b: &mut &[u8]) -> Result<usize, String> {
        let x = take(b, 4)?;
        Ok(u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize)
    }
    let mut b = b;
    let n = u32(&mut b)?;
    let mut out = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let l = u32(&mut b)?;
        let p = String::from_utf8(take(&mut b, l)?.to_vec()).map_err(|e| e.to_string())?;
        let l = u32(&mut b)?;
        out.push((p, take(&mut b, l)?));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn unpack_reads_the_python_layout() {
        let mut b = Vec::new();
        b.extend(2u32.to_le_bytes());
        for (p, d) in [("/vendor/a.agf", &b"xy"[..]), ("/vendor/b", &b""[..])] {
            b.extend((p.len() as u32).to_le_bytes());
            b.extend(p.as_bytes());
            b.extend((d.len() as u32).to_le_bytes());
            b.extend(d);
        }
        let v = super::unpack(&b).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].0.as_str(), v[0].1), ("/vendor/a.agf", &b"xy"[..]));
        assert_eq!((v[1].0.as_str(), v[1].1), ("/vendor/b", &b""[..]));
        for n in 0..b.len() {
            assert!(super::unpack(&b[..n]).is_err());
        }
        // gzip の束も、gzip でない束も同じ中身になる
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &b).unwrap();
        assert_eq!(super::ungzip(gz.finish().unwrap()).unwrap(), b);
        assert_eq!(super::ungzip(b.clone()).unwrap(), b);
        assert!(super::ungzip(vec![0x1f, 0x8b, 0, 1]).is_err());
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    include!(concat!(env!("OUT_DIR"), "/samples.rs"));

    /// URL の中身（画面でも Web Worker でも、その場所の fetch で）
    async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
        use js_sys::{Function, Promise, Reflect};
        use wasm_bindgen_futures::JsFuture;
        let g = js_sys::global();
        let f: Function = Reflect::get(&g, &"fetch".into())?.dyn_into()?;
        let r = JsFuture::from(f.call1(&g, &url.into())?.dyn_into::<Promise>()?).await?;
        if !Reflect::get(&r, &"ok".into())?.as_bool().unwrap_or(false) {
            return Err(JsValue::from_str(&format!("{url} を読めない")));
        }
        let ab: Function = Reflect::get(&r, &"arrayBuffer".into())?.dyn_into()?;
        let buf = JsFuture::from(ab.call0(&r)?.dyn_into::<Promise>()?).await?;
        Ok(js_sys::Uint8Array::new(&buf).to_vec())
    }

    /// 同梱のファイル・足したファイル・ファイルの束を置き場に入れる
    async fn insert_all(extra: &super::Extra) -> Result<(), JsValue> {
        let err = |e: std::io::Error| JsValue::from_str(&e.to_string());
        for (p, b) in FILES.iter().chain(extra.files.iter()) {
            lensbench_core::vfs::insert(p, b).map_err(err)?;
        }
        for url in &extra.packs {
            let b = super::ungzip(fetch_bytes(url).await?)
                .map_err(|e| JsValue::from_str(&format!("{url}: {e}")))?;
            for (p, d) in
                super::unpack(&b).map_err(|e| JsValue::from_str(&format!("{url}: {e}")))?
            {
                lensbench_core::vfs::insert(p, d).map_err(err)?;
            }
        }
        Ok(())
    }

    /// 起動時に開く見本
    const FIRST: &str = "/lensbench/tests/refs/A0/system.zmx";

    /// 既定のフォルダの並び（ガラス、レンズカタログ）。前もって作った一覧があれば、この並びで登録する
    fn default_dirs(extra: &super::Extra) -> (Vec<String>, Vec<String>) {
        let mut glass_dirs = vec!["/lensbench/tests/glass".to_string()];
        glass_dirs.extend(extra.glass_dirs.iter().cloned());
        let mut lens_dirs = vec![
            "/lensbench/examples/lenses/LB".to_string(),
            "/lensbench/examples/lenses/LBP".to_string(),
        ];
        lens_dirs.extend(extra.lens_dirs.iter().cloned());
        if let Some(f) = extra.lens_index {
            let pb = |v: &[String]| v.iter().map(std::path::PathBuf::from).collect();
            lensbench_cli::set_prepared_lens_list(pb(&lens_dirs), pb(&glass_dirs), f.into());
        }
        (glass_dirs, lens_dirs)
    }

    /// Web Worker の中で始める（`worker.js`）。kind は受ける要求（0: serve と同じ、1: 下書き）
    #[wasm_bindgen]
    pub async fn worker_init(kind: u8) -> Result<(), JsValue> {
        let extra = super::extra();
        insert_all(&extra).await?;
        default_dirs(&extra);
        // 機構部品の詳しい形（置き場から取って vfs の /cad に入れた STEP）を描けるように
        lensbench_cad::detail::install_detail_loader();
        lensbench_gui::web_worker::init(kind);
        Ok(())
    }

    /// 並列: rayon のプールを作り終えた（これより後の計算は並列に回す）
    #[cfg(feature = "threads")]
    #[wasm_bindgen]
    pub fn threads_ready() {
        lensbench_core::par::set_pool_ready();
    }

    /// Web Worker の中で要求を回す（受け渡しの形は lensbench-gui の wire.rs）
    #[wasm_bindgen]
    pub fn worker_call(req: &[u8]) -> Vec<u8> {
        lensbench_gui::web_worker::call(req)
    }

    /// 並列: 見張りのスレッド（`monitor.js`）を同じ wasm・同じメモリで始めるための [module, memory]
    #[cfg(feature = "threads")]
    #[wasm_bindgen]
    pub fn worker_shared() -> js_sys::Array {
        js_sys::Array::of2(&wasm_bindgen::module(), &wasm_bindgen::memory())
    }

    /// 見張りのスレッドから: 要求 id を回していれば、解析に途中でやめるよう頼む
    #[wasm_bindgen]
    pub fn monitor_cancel(id: u32) {
        lensbench_gui::web_worker::cancel(id);
    }

    /// 見張りのスレッドから: 要求 id を回していれば、最適化・公差・差し替え探索を止める
    #[wasm_bindgen]
    pub fn monitor_stop(id: u32, what: &str) {
        lensbench_gui::web_worker::stop(id, what);
    }

    /// 見張りのスレッドから: 最適化・公差・差し替え探索の進み具合（JSON）
    #[wasm_bindgen]
    pub fn monitor_progress() -> String {
        lensbench_gui::web_worker::progress()
    }

    /// キャンバス（id）で GUI を始める。font は日本語のフォント（.ttf・.otf、無ければ欧文だけ）
    #[wasm_bindgen]
    pub async fn start(canvas_id: String, font: Option<Vec<u8>>) -> Result<(), JsValue> {
        let extra = super::extra();
        // 別のタブのパネル（計算しない）には同梱のデータは要らない
        if !lensbench_gui::web_panel() {
            insert_all(&extra).await?;
        }
        if let Some(f) = font {
            lensbench_core::vfs::insert(lensbench_gui::WEB_FONT, f)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
        }
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(&canvas_id))
            .ok_or_else(|| JsValue::from_str(&format!("{canvas_id} が無い")))?
            .dyn_into::<web_sys::HtmlCanvasElement>()?;
        // 見本: 例題の組合せ（開くと組合せに戻る）、プロジェクト、tests/refs の系の順
        let samples: Vec<String> = FILES
            .iter()
            .map(|f| f.0)
            .filter(|p| {
                ((p.starts_with("/lensbench/examples/combos/")
                    || p.starts_with("/lensbench/examples/projects/"))
                    && p.ends_with(".json"))
                    || p.ends_with("/system.zmx")
            })
            .map(String::from)
            .collect();
        let (glass_dirs, lens_dirs) = default_dirs(&extra);
        lensbench_gui::set_web_cad_store(extra.cad_store.map(String::from));
        lensbench_cad::detail::install_detail_loader();
        let args = lensbench_gui::Args {
            open: Some(FIRST.into()),
            glass_dirs,
            lens_dirs,
            samples,
            ..Default::default()
        };
        lensbench_gui::start_web(canvas, args).await
    }
}
