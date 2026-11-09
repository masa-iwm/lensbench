//! lensbench の Web 版の入口。GUI（lensbench-gui）をブラウザのキャンバスで動かす。
//!
//! ファイルシステムが無いので、見本の系・公開用のガラス・例題のカタログを wasm に同梱し、起動時に
//! [`lensbench_core::vfs`] の置き場（/lensbench/...）へ入れる。日本語のフォントはページ（index.html）が
//! 読み込んで [`start`] に渡す。
//!
//! 計算は Web Worker（`worker.js`）の中の同じ wasm で回す（[`worker_init`]・[`worker_call`]）。

/// 並列: rayon のスレッドを Web Worker で作る（JS の initThreadPool(n)）。作った後に threads_ready
#[cfg(all(target_arch = "wasm32", feature = "threads"))]
pub use wasm_bindgen_rayon::init_thread_pool;

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    include!(concat!(env!("OUT_DIR"), "/samples.rs"));

    /// 起動時に開く見本
    const FIRST: &str = "/lensbench/tests/refs/A0/system.zmx";

    /// Web Worker の中で始める（`worker.js`）。kind は受ける要求（0: serve と同じ、1: 下書き）
    #[wasm_bindgen]
    pub fn worker_init(kind: u8) -> Result<(), JsValue> {
        for (p, b) in FILES {
            lensbench_core::vfs::insert(p, b).map_err(|e| JsValue::from_str(&e.to_string()))?;
        }
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

    /// キャンバス（id）で GUI を始める。font は日本語のフォント（.ttf・.otf、無ければ欧文だけ）
    #[wasm_bindgen]
    pub async fn start(canvas_id: String, font: Option<Vec<u8>>) -> Result<(), JsValue> {
        for (p, b) in FILES {
            lensbench_core::vfs::insert(p, b).map_err(|e| JsValue::from_str(&e.to_string()))?;
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
        // 見本: 例題のプロジェクトと tests/refs の系
        let samples: Vec<String> = FILES
            .iter()
            .map(|f| f.0)
            .filter(|p| {
                (p.starts_with("/lensbench/examples/projects/") && p.ends_with(".json"))
                    || p.ends_with("/system.zmx")
            })
            .map(String::from)
            .collect();
        let args = lensbench_gui::Args {
            open: Some(FIRST.into()),
            glass_dirs: vec!["/lensbench/tests/glass".into()],
            lens_dirs: vec![
                "/lensbench/examples/lenses/LB".into(),
                "/lensbench/examples/lenses/LBP".into(),
            ],
            samples,
            ..Default::default()
        };
        lensbench_gui::start_web(canvas, args).await
    }
}
