//! lensbench の GUI。インストール不要の単一 exe。計算は serve と同じ API（[`lensbench_engine::call`]）を
//! 同じプロセスの中で呼ぶ。
//!
//! ライブラリとしても使える: 別の exe（カタログの形式・プラグインを足した版）は、起動の最初に
//! `lensbench_engine::Extensions` の install（拡張の表 [`lensbench_core::ext`] の既定に足す）をしてから [`main`] を呼ぶ。
#![allow(clippy::unwrap_used, clippy::panic)] // 入口の exe（計算の中身のクレートは検査の内）

mod app;
mod argform;
#[cfg(not(target_arch = "wasm32"))]
mod builtin_glass;
mod cad_fetch;
mod cad_store;
mod catalog;
mod catsave;
mod chrome;
mod coat_defs;
mod combo;
mod combo_doc;
mod combo_from;
mod combo_reshape;
mod combo_split;
mod combo_tree;
mod config_panel;
mod crash;
mod export_table;
pub mod ext;
mod figures;
mod files;
mod fill;
mod fonts;
mod hist_label;
mod i18n;
mod image_util;
mod json_view;
mod layout;
mod layout_rays;
mod lens_data;
mod missing_glass;
mod open_sysset;
mod optim_panel;
mod palette;
#[cfg(test)]
mod panel_ui_tests;
mod param_pick;
mod plot_tools;
mod plots;
mod portable;
#[cfg(any(target_arch = "wasm32", test))]
mod remote_frame;
#[cfg(not(target_arch = "wasm32"))]
mod samples;
mod split;
mod state_table;
mod surf_edit;
mod swap_panel;
mod sysset;
mod theme;
mod tol_catalog;
mod tol_panel;
mod tour;
mod unit_mech;
mod view3d;
#[cfg(target_arch = "wasm32")]
mod web_bg;
#[cfg(target_arch = "wasm32")]
mod web_store;
#[cfg(target_arch = "wasm32")]
mod web_tabs;
#[cfg(all(windows, feature = "webview"))]
mod webview;
mod widgets;
#[cfg(any(target_arch = "wasm32", test))]
mod wire;
mod work_fs;
mod worker;
mod zoom_panel;

pub use fonts::WEB_FONT;
pub use tour::Args;
/// Web Worker の中の入口（lensbench-web が使う）
#[cfg(target_arch = "wasm32")]
pub use worker::entry as web_worker;

/// GUI を起動する（引数はプロセスの引数から読む）
#[cfg(not(target_arch = "wasm32"))]
pub fn main() -> std::process::ExitCode {
    // 品ページを開くアプリ内のブラウザ（未完のカタログ品を補う窓が、この exe を --webview で起こす）
    #[cfg(all(windows, feature = "webview"))]
    {
        let a: Vec<String> = std::env::args().collect();
        if a.get(1).map(String::as_str) == Some("--webview") {
            return webview::run(&a);
        }
    }
    // 前の版の組合せの保存（gui.combo）を今の形に直す、一度きりの変換（次の版で消す）
    {
        let a: Vec<String> = std::env::args().collect();
        if a.get(1).map(String::as_str) == Some("--convert-combo") {
            return app::convert_old(&a[2..]);
        }
    }
    // 既定のスレッド数（LENSBENCH_THREADS）など、環境変数から決める既定
    lensbench_core::init_from_env();
    // 引数: 系のファイル（エクスプローラーで .zmx をこの exe に落としたときも）、--glass-dir・--lens-catalog、
    // 画面の通し確認 --tour DIR（tour.rs）
    let args = match tour::Args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lensbench-gui: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if args.help {
        println!("{}", tour::USAGE);
        return std::process::ExitCode::SUCCESS;
    }
    // 設定の保存先
    let mut args = args;
    let store = portable::choose(args.settings.clone(), portable::exe_dir().as_deref());
    portable::set(&store);
    args.notice = store.notice.clone();
    // カタログの一覧を設定の隣に覚えておく（次の起動で読み直さない）。補ったカタログ品も設定の隣に
    let settings_dir: Option<std::path::PathBuf> = store
        .file
        .clone()
        .or_else(portable::os_file)
        .and_then(|f| f.parent().map(Into::into));
    crash::install(settings_dir.clone());
    fill::set_fill_dir(settings_dir.clone());
    cad_fetch::set_cad_dir(settings_dir.clone());
    samples::install(settings_dir.clone());
    lensbench_cad::coat_remote::install();
    lensbench_engine::set_lens_list_cache_dir(settings_dir);
    let options = eframe::NativeOptions {
        // 通し確認は利用者の設定（窓の配置・残した状態）を読まず、書き換えもしない
        persistence_path: if args.tour.is_some() {
            None
        } else {
            store.file.clone()
        },
        viewport: egui::ViewportBuilder::default()
            .with_title("lensbench")
            .with_icon(app::icon())
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 400.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let r = eframe::run_native(
        "lensbench",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, args)))),
    );
    if let Err(e) = r {
        eprintln!("lensbench-gui: {e}");
        return std::process::ExitCode::FAILURE;
    }
    if tour::FAILED.load(std::sync::atomic::Ordering::SeqCst) {
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

/// ブラウザで GUI を起動する（Web 版）。系・ガラス・カタログのファイルは先に
/// [`lensbench_core::vfs::insert`] で置き場に入れておき、そのパスを args に書く。
/// 前に読み込んだファイル・ユーザーカタログ（ブラウザの IndexedDB に残したもの）はここで置き場に戻し、
/// ユーザーカタログのフォルダ（`/user/catalogs`）があればレンズのフォルダに足す
#[cfg(target_arch = "wasm32")]
pub async fn start_web(
    canvas: web_sys::HtmlCanvasElement,
    mut args: Args,
) -> Result<(), wasm_bindgen::JsValue> {
    crash::install();
    // 別のタブのパネルは計算しない（元のタブの結果を描くだけ）。ブラウザに残したファイルも要らない
    if web_panel() {
        return run_web(canvas, args).await;
    }
    // ここまでに入れた見本は計算の Web Worker も持っている。戻したファイルは最初の要求と一緒に渡す
    worker::set_sync_base(lensbench_core::vfs::sync::seq());
    web_store::restore().await;
    // フォルダはファイルがあるときだけ（無いフォルダは読めないカタログになる。最初に保存したときに足す）
    let d = web_store::USER_CATALOGS.to_string();
    if lensbench_core::vfs::is_dir(&d) && !args.lens_dirs.contains(&d) {
        args.lens_dirs.push(d);
    }
    run_web(canvas, args).await
}

/// Web 版: 品の STEP の共有の置き場の URL（例 `/cad`）。別の入口（`lensbench-web` の `Extra::cad_store`）が起動の前に決める。
/// 決めてあれば、カタログの詳細の「詳しい形（STEP）を取る」で置き場から取る
#[cfg(target_arch = "wasm32")]
pub fn set_web_cad_store(url: Option<String>) {
    cad_fetch::set_web_store(url);
}

/// Web 版: このページが別のタブのパネル（`?panel=`）か。そのときは同梱のデータも要らない
#[cfg(target_arch = "wasm32")]
pub fn web_panel() -> bool {
    web_tabs::panel_of_location().is_some()
}

#[cfg(target_arch = "wasm32")]
async fn run_web(
    canvas: web_sys::HtmlCanvasElement,
    args: Args,
) -> Result<(), wasm_bindgen::JsValue> {
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |cc| {
                let panel = web_panel();
                let a = std::rc::Rc::new(std::cell::RefCell::new(app::App::new(cc, args)));
                TEST_APP.with(|t| *t.borrow_mut() = std::rc::Rc::downgrade(&a));
                // 元のタブ: 裏に回っている間も別の窓のパネルを動かす
                if !panel {
                    web_bg::install(std::rc::Rc::downgrade(&a), cc.egui_ctx.clone());
                }
                Ok(Box::new(Shell(a)))
            }),
        )
        .await
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// 試験用の口 [`web_test_state`] が見る画面（起動した後）
    static TEST_APP: std::cell::RefCell<std::rc::Weak<std::cell::RefCell<app::App>>> =
        Default::default();
}

/// Web 版の試験用の口: 画面の今の様子の JSON（待っている要求の数・開いている系・解析パネルの結果の有無）。
/// 起動前は `{"started": false}`、画面を描いている最中は `{"busy": true}`（scripts/web_smoke.mjs）
#[cfg(target_arch = "wasm32")]
pub fn web_test_state() -> String {
    let v = TEST_APP.with(|t| match t.borrow().upgrade() {
        None => serde_json::json!({ "started": false }),
        Some(a) => match a.try_borrow() {
            Ok(a) => {
                let mut v = a.test_state();
                v["started"] = true.into();
                v
            }
            Err(_) => serde_json::json!({ "started": true, "busy": true }),
        },
    });
    v.to_string()
}

/// eframe に渡す画面。中身は [`web_bg`] とも分け合う（裏に回ったタブで別の窓の分を描く）
#[cfg(target_arch = "wasm32")]
struct Shell(std::rc::Rc<std::cell::RefCell<app::App>>);

#[cfg(target_arch = "wasm32")]
impl eframe::App for Shell {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.0.borrow_mut().update(ctx, frame);
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        self.0.borrow().auto_save_interval()
    }

    fn persist_egui_memory(&self) -> bool {
        self.0.borrow().persist_egui_memory()
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.0.borrow_mut().save(storage);
    }
}

/// serve の系（lensbench_engine::session::global）はプロセスで 1 つなので、load で系を差し替えるテスト
/// （App を回す画面のテスト・serve を直に呼ぶテスト）はこの鍵を持って 1 つずつ回す（cargo test は 1 プロセスで並行）
#[cfg(test)]
pub(crate) fn serve_lock() -> std::sync::MutexGuard<'static, ()> {
    static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
    M.lock().unwrap_or_else(|e| e.into_inner())
}
