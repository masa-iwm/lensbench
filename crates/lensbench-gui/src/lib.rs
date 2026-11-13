//! lensbench の GUI。インストール不要の単一 exe。計算は serve と同じ API（[`lensbench_cli::serve::call`]）を
//! 同じプロセスの中で呼ぶ。
//!
//! ライブラリとしても使える: 別の exe（カタログの形式・プラグインを足した版）は、起動の最初に
//! [`lensbench_core::catalog::register_source`]・[`lensbench_core::plugin::global`] へ登録してから [`main`] を呼ぶ。

mod app;
mod argform;
mod cad_fetch;
mod cad_store;
mod catalog;
mod catsave;
mod chrome;
mod coat_defs;
mod combo;
mod combo_tree;
mod config_panel;
mod crash;
mod export_table;
pub mod ext;
mod figures;
mod files;
mod fill;
mod fonts;
mod i18n;
mod json_view;
mod layout;
mod layout_rays;
mod lens_data;
mod open_sysset;
mod optim_panel;
mod palette;
mod param_pick;
mod plot_tools;
mod plots;
mod portable;
#[cfg(any(target_arch = "wasm32", test))]
mod remote_frame;
mod split;
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
mod web_store;
#[cfg(target_arch = "wasm32")]
mod web_tabs;
#[cfg(all(windows, feature = "webview"))]
mod webview;
#[cfg(any(target_arch = "wasm32", test))]
mod wire;
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
    // 引数: 系のファイル（エクスプローラーで .zmx をこの exe に落としたときも）、--glass-dir・--lens-catalog、
    // 画面の通し確認 --tour DIR（tour.rs）
    let args = match tour::Args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lensbench-gui: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // 設定の保存先
    let mut args = args;
    let store = portable::choose(
        args.settings.clone(),
        portable::exe_dir().as_deref(),
        portable::os_file().as_deref(),
    );
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
    lensbench_cli::cad::install_detail_loader();
    lensbench_cli::coat_remote::install();
    lensbench_cli::set_lens_list_cache_dir(settings_dir);
    let options = eframe::NativeOptions {
        persistence_path: store.file.clone(),
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
/// 決めてあれば、カタログの詳細の「詳しい形（STEP）を取る」で置き場から取る（#158）
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
            Box::new(move |cc| Ok(Box::new(app::App::new(cc, args)))),
        )
        .await
}
