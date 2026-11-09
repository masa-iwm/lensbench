//! lensbench の GUI。インストール不要の単一 exe。計算は serve と同じ API（[`lensbench_cli::serve::call`]）を
//! 同じプロセスの中で呼ぶ。
//!
//! ライブラリとしても使える: 別の exe（カタログの形式・プラグインを足した版）は、起動の最初に
//! [`lensbench_core::catalog::register_source`]・[`lensbench_core::plugin::global`] へ登録してから [`main`] を呼ぶ。

mod app;
mod argform;
mod catalog;
mod catsave;
mod combo;
mod combo_tree;
mod config_panel;
mod dpi;
pub mod ext;
mod figures;
mod files;
mod fonts;
mod i18n;
mod json_view;
mod layout;
mod lens_data;
mod open_sysset;
mod optim_panel;
mod palette;
mod plots;
mod portable;
mod surf_edit;
mod swap_panel;
mod sysset;
mod theme;
mod tol_panel;
mod tour;
mod view3d;
#[cfg(any(target_arch = "wasm32", test))]
mod wire;
mod worker;

pub use fonts::WEB_FONT;
pub use tour::Args;
/// Web Worker の中の入口（lensbench-web が使う）
#[cfg(target_arch = "wasm32")]
pub use worker::entry as web_worker;

/// GUI を起動する（引数はプロセスの引数から読む）
#[cfg(not(target_arch = "wasm32"))]
pub fn main() -> std::process::ExitCode {
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
/// [`lensbench_core::vfs::insert`] で置き場に入れておき、そのパスを args に書く
#[cfg(target_arch = "wasm32")]
pub async fn start_web(
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
