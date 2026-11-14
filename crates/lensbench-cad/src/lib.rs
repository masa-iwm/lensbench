//! 品の CAD データ（メーカーの STEP）を取る・置き場に上げる・測る・詳しい形（三角形）にする。
//!
//! 以前は CLI クレートの中にあり、GUI・Web 版が CLI ごと（clap など）依存していた（見直し BLD-08・BLD-10）。
//! - [`detail`] は STEP を測る・三角形にする・gzip（ネイティブと Web 版の両方）。三角形にする部分（truck）は
//!   feature `mesh`（既定で有効）。外すと [`detail::install_detail_loader`] は何もしない（詳しい形は描かない）
//! - 取る・置き場（[`Store`]）・アップロード（ネイティブだけ）
//! - [`coat_remote`] は膜のモデルを置き場から取る口（ネイティブだけ）
pub mod detail;
#[cfg(not(target_arch = "wasm32"))]
mod fetch;
#[cfg(not(target_arch = "wasm32"))]
pub use fetch::*;
#[cfg(not(target_arch = "wasm32"))]
pub mod coat_remote;
