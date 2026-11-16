//! 品の CAD データ（メーカーの STEP）を取る・置き場に上げる・測る・詳しい形（三角形）にする。
//!
//! 以前は CLI クレートの中にあり、GUI・Web 版が CLI ごと（clap など）依存していた（見直し BLD-08・BLD-10）。
//! - [`detail`] は STEP を測る・三角形にする・gzip（ネイティブと Web 版の両方）。三角形にする部分（truck）は
//!   feature `mesh`（既定で有効）。外すと [`detail::details`] は None（詳しい形は描かない）
//! - 取る・置き場（[`Store`]）・アップロード（ネイティブだけ）
//! - [`cache`] は取った STEP のキャッシュのフォルダ・ファイルの名前（ネイティブと Web 版）
//! - [`coat_remote`] は膜のモデルを置き場から取る口（ネイティブだけ）
pub mod cache;
pub mod detail;
pub use cache::{cached_step, cad_cache_dir, cad_cache_find, cad_cache_path, set_cad_cache_dir};
#[cfg(not(target_arch = "wasm32"))]
mod fetch;
#[cfg(not(target_arch = "wasm32"))]
pub use fetch::*;
#[cfg(not(target_arch = "wasm32"))]
pub mod coat_remote;
