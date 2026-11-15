//! lensbench のコア: 系（面・ガラス・膜）、順次光線追跡（偏光・GRIN・自由曲面）、.zmx の読み書き、ソルブ・ZPL、基本の解析（波面・PSF・POP・ゴースト）。
//!
//! 外部のレンズカタログ（ZMF など）の読込は [`catalog::LensCatalogSource`] を実装した
//! 別クレートから差し込む。公開版にはその実装を含めない。
// `!(x >= 0.0)` は NaN も弾くための意図的な書き方。
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::needless_range_loop)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod error;
pub mod math;

mod aim;
pub mod analysis;
pub mod batch;
pub mod catalog;
pub mod coating;
pub mod edit;
pub mod elements;
pub mod ext;
pub mod fermat;
pub mod fiber;
pub mod freeform;
pub mod ghost;
pub mod glass;
pub mod grin;
pub mod i18n;
pub mod interference;
mod interp2d;
pub mod landing;
pub mod mech;
pub mod par;
pub mod param;
pub mod plugin;
pub mod polgrid;
pub mod pop;
pub mod psf;
pub mod raygrid;
pub mod report;
pub mod semi;
pub mod solve;
pub mod surface;
pub mod sweep;
pub mod sync;
pub mod system;
pub mod tansag;
pub mod trace;
pub mod trig;
pub mod units;
pub mod usersurf;
pub mod vfs;
pub mod waves;
pub mod zmx;
pub mod zmx_write;
pub mod zpl;
pub mod zpl_solve;

pub use error::{Error, Result};
pub use par::with_threads;

/// 環境変数から決める既定（CLI の起動・Python の import で呼ぶ）: スレッド数（LENSBENCH_THREADS、
/// [`par::default_threads_from_env`]）。消した環境変数が設定されていれば標準エラーに 1 回注意を出す
pub fn init_from_env() {
    par::default_threads_from_env();
    static ONCE: std::sync::Once = std::sync::Once::new();
    if std::env::var_os("LENSBENCH_FLOATING_CLIP").is_some() {
        ONCE.call_once(|| {
            eprintln!(
                "warning: the environment variable LENSBENCH_FLOATING_CLIP is no longer used (ignored). To keep floating apertures from clipping, set the system setting floating_clip to false"
            )
        });
    }
}
pub use sync::{LockExt, RwLockExt};
pub use system::{FieldSel, RecSel, System, TraceOpts};
pub use tansag::TanSag;
pub use waves::Waves;

/// ライブラリの版。ビルド時に環境変数 LENSBENCH_VERSION があればその値（配布物のタグ名など）、
/// 無ければ Cargo.toml の版。GUI の版の欄・CLI の --version・serve の ping などに出る。
pub const VERSION: &str = match option_env!("LENSBENCH_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => env!("CARGO_PKG_VERSION"),
};
