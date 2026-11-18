//! lensbench のコア: 系（面・ガラス・膜）、順次光線追跡（偏光・GRIN・自由曲面）、.zmx の読み書き、ソルブ・ZPL、基本の解析（波面・PSF・POP）。.zmx の書き出しは lensbench-formats、ゴースト・干渉縞・偏光の格子は lensbench-analysis。
//!
//! 外部のレンズカタログ（ZMF など）の読込は [`catalog::LensCatalogSource`] を実装した
//! 別クレートから差し込む。公開版にはその実装を含めない。
//!
//! ライブラリとして使うときの入口は [`prelude`]（系の読込・追跡・主な解析・結果の型）。それ以外の `pub` のモジュールは
//! 上の層のクレート（analysis・project・engine など）と拡張のための口で、安定の度合いは低い。

#![warn(unreachable_pub)]
// 入力次第で panic しないように、ライブラリでは unwrap・panic を使わない。前に確かめた形は expect に理由を書く（試験は対象外）
// `!(x >= 0.0)` は NaN も弾くための意図的な書き方。
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::needless_range_loop)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod error;
pub mod material;
pub mod math;

pub mod action;
mod aim;
pub mod analysis;
pub mod batch;
pub mod catalog;
pub mod coating;
pub mod configs;
pub mod edit;
pub mod elements;
pub mod ext;
pub mod fermat;
pub mod fiber;
mod focus;
pub mod form;
pub mod freeform;
pub mod glass;
pub mod grid;
pub mod grin;
pub mod history;
pub mod i18n;
pub mod import;
pub mod interp;
mod interp2d;
pub mod landing;
pub mod linalg;
pub mod mech;
mod media;
mod newton2;
pub mod par;
pub mod param;
pub mod phase;
pub mod pickup;
pub mod plugin;
pub mod pop;
pub mod psf;
pub mod pupil;
pub mod quantity;
pub mod raygrid;
pub mod report;
pub mod semi;
pub mod shape;
mod shape_param;
pub mod solve;
pub mod solves;
pub mod surface;
pub mod sweep;
pub mod sync;
pub mod system;
pub mod tansag;
pub mod tol;
pub mod trace;
pub mod trig;
pub mod units;
pub mod usersurf;
pub mod vfs;
pub mod waves;
pub mod zmx;
pub mod zpl;
pub mod zpl_solve;

/// ライブラリとして使うときの主な口（`use lensbench_core::prelude::*;`）。系を読む（[`zmx::load_with`]）→
/// 追跡する（[`System::try_trace`]・[`TraceOpts`]）→ 解析する（[`analysis`]・[`psf`]）の流れに要るもの
pub mod prelude {
    pub use crate::analysis::{self, Mtf, Wavefront};
    pub use crate::error::{Error, Result};
    pub use crate::ext::{self, Ext};
    pub use crate::grid::Grid2;
    pub use crate::par::{with_threads, CancelToken};
    pub use crate::psf::{self, Psf, PsfOpts, PsfPlane};
    pub use crate::system::{FieldSel, Prescription, RecSel, System, TraceOpts};
    pub use crate::tansag::TanSag;
    pub use crate::vfs;
    pub use crate::waves::Waves;
    pub use crate::zmx::{self, LoadOptions};
    pub use crate::{init_from_env, report};
}

pub use error::{Error, Result};
pub use par::with_threads;

/// 環境変数から決める既定（CLI・GUI の起動・Python の import で呼ぶ。core のほかの所は環境変数を読まない）:
/// スレッド数（LENSBENCH_THREADS、[`par::default_threads_from_env`]）、ガラスのフォルダ（LENSBENCH_GLASSCAT、
/// [`glass::set_default_dirs`]）、マクロのフォルダ（LENSBENCH_MACROS、[`zpl_solve::set_default_dirs`]）。フォルダは OS の
/// パス区切りで並べる。消した環境変数が設定されていれば標準エラーに 1 回注意を出す
pub fn init_from_env() {
    par::default_threads_from_env();
    let dirs = |k: &str| {
        std::env::var_os(k)
            .map(|e| std::env::split_paths(&e).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    glass::set_default_dirs(dirs("LENSBENCH_GLASSCAT"));
    zpl_solve::set_default_dirs(dirs("LENSBENCH_MACROS"));
}
pub use sync::{LockExt, RwLockExt};
pub use system::{FieldSel, Prescription, RecSel, System, TraceOpts};
pub use tansag::TanSag;
pub use waves::Waves;

/// ライブラリの版。ビルド時に環境変数 LENSBENCH_VERSION があればその値（配布物のタグ名など）、
/// 無ければ Cargo.toml の版。GUI の版の欄・CLI の --version・serve の ping などに出る。
pub const VERSION: &str = match option_env!("LENSBENCH_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => env!("CARGO_PKG_VERSION"),
};
