//! 面の膜と境界の偏光係数。
//!
//! OpticStudio の偏光追跡の約束に合わせる: s・p の複素振幅係数で、パワーで規格化（|t|² = T）、
//! 各側で p = s × k、伝搬の位相は含まない。
//!
//! - 膜なしの誘電体境界: フレネル
//! - 膜なしの鏡: OpticStudio の金属基板 N = 0.7 − 7i
//! - `I.xx`: 理想膜（強度透過率 xx、反射率 1 − xx、位相なし。鏡では s・p とも −√(1 − xx)）
//! - `COAT`: 薄膜（特性行列、傾いた入射のアドミタンス）。材料は `MATE` の表（n, k。k < 0 で吸収）か
//!   ガラスカタログ。厚さは基準波長での光学的な波長単位、またはフラグ 1 で絶対値 [µm]。
//!   係数の位相は OpticStudio の約束に合わせる（[`Coatings::prepare`] の注記）
//! - `TABLE`: 角度・波長ごとの Rs Rp Ts Tp と位相、線形補間（角度は cosθ について。垂直入射は [`TableNormal`]）
//! - 暗号化された膜（.ZEC）: 層は読めないので（復号もしない）、OpticStudio の偏光追跡で測った係数の表
//!   （[`Measured`]）か、膜ごとの特性行列モデル（[`CoatModel`]、任意の硝材・向き・波長）を使う。
//!   どちらもなければ追跡を止める
//!
//! 境界の鍵は `膜|前の媒質|後の媒質（鏡は MIRROR）`（[`media_keys`]）。屈折面での反射（ゴーストの経路、
//! lensbench-analysis の ghost）は `膜|入射側の媒質|反対側の媒質|R` で、係数は境界の反射（[`Coatings::prepare`] の注記）。

use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};

mod format;
mod iface;
mod interpolate;
mod library;
mod remote;
#[cfg(test)]
mod tests;
mod thin_film;

pub(crate) use crate::interp::Spline;
pub use format::dat::*;
pub use format::lbcb::*;
pub use format::lbcm::*;
pub use format::lbct::*;
pub use iface::*;
pub use interpolate::*;
pub use library::*;
pub use remote::{set_remote_loader, RemoteCoatings, RemoteLoader};
pub use thin_film::*;

pub use num_complex::Complex64 as C64;

use crate::glass;
use crate::system::System;
use crate::{invalid, notimpl, Error, Result};

/// TABLE 膜で垂直入射とみなす範囲（1 − cosθ。OpticStudio 26.1 で境界が 0.008102°〜0.008104° の間、
/// 1 − cosθ = 1e-8 の 0.0081029° と合う）
pub(crate) const TABLE_NORMAL: f64 = 1e-8;

/// 膜の材料の表の k の、誤りにしない正の値の上限（標準の膜ファイルの数値の誤差を許す）
const MATE_K_POS_TOL: f64 = 1e-6;

/// 膜なしの鏡の基板
pub(crate) const MIRROR_N: C64 = C64::new(0.7, -7.0);
