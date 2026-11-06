//! lensbench のコア: 順次光線追跡、.zmx 読込、解析。
//!
//! 外部のレンズカタログ（ZMF など）の読込は [`catalog::LensCatalogSource`] を実装した
//! 別クレートから差し込む。公開版にはその実装を含めない。
// `!(x >= 0.0)` は NaN も弾くための意図的な書き方。
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::needless_range_loop)]

pub mod error;
pub mod math;

mod aim;
pub mod analysis;
pub mod batch;
pub mod catalog;
pub mod coat_fit;
pub mod coating;
pub mod edit;
pub mod elements;
pub mod fermat;
pub mod fiber;
pub mod freeform;
pub mod fringe;
pub mod ghost;
pub mod glass;
pub mod grin;
pub mod huygens;
pub mod landing;
pub mod mech;
pub mod optim;
pub mod par;
pub mod param;
pub mod plugin;
pub mod polgrid;
pub mod pop;
pub mod psf;
pub mod raygrid;
pub mod report;
pub mod runs;
pub mod semi;
pub mod solve;
pub mod surface;
pub mod sweep;
pub mod system;
pub mod tolerance;
pub mod trace;
pub mod trig;
pub mod units;
pub mod usersurf;
pub mod vfs;
pub mod zmx;
pub mod zmx_write;
pub mod zoom;
pub mod zpl;
pub mod zpl_solve;
pub mod zplm;

pub use error::{Error, Result};
pub use par::with_threads;
pub use system::{FieldSel, RecSel, System};

/// ライブラリの版。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
