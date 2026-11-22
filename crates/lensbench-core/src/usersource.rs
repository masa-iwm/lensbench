//! 利用者定義の光源: 設定から光線の始点・方向・重みの並びを作る。
//!
//! 外のクレートは [`SourceModel`] を実装して拡張の表に名前で足し（[`crate::ext::Ext::add_source_model`]）、使う側は
//! 名前と設定から [`SourceInstance`] を作って波長ごとに光線を受け取る。追跡は [`crate::system::Launch::Rays`] で
//! 行う（[`System::source_rays`] が物体の座標系から大域に直す）。
//!
//! 約束:
//! - 光線は物体の座標系（有限物体は物体面、無限遠物体は面 1 の変換の前の座標系。[`System::object_frame`]）で返す。
//!   方向は規格化しなくてよい（長さ 0 は誤り）。重みは 0 以上の有限の数（相対的な強さ。足して 1 でなくてよい）
//! - 同じ設定・波長なら同じ光線を返す（乱数を使うなら種を設定に持つ）
//! - 設定が合わなければ [`crate::Error::Invalid`] を返す（panic しない）

use crate::error::Result;
use crate::math::V3;
use crate::plugin::{Config, ParamSpec};
use crate::{invalid, System};
use std::sync::Arc;

/// 光源のモデル
pub trait SourceModel: Send + Sync {
    /// 登録の名前
    fn name(&self) -> &str;
    /// 1 行の説明
    fn doc(&self) -> String {
        String::new()
    }
    /// 設定の項目（一覧用）
    fn params(&self) -> Vec<ParamSpec> {
        Vec::new()
    }
    /// 設定を確かめて光源を作る
    fn prepare(&self, config: &Config) -> Result<Arc<dyn SourceInstance>>;
    /// 試しの設定（拡張の検査で [`check_source`] に渡す）。既定は [`SourceModel::params`] の既定値を並べたもの
    /// （既定値の無い項目があれば None。そのときは自分で返す）
    fn example_config(&self) -> Option<Config> {
        self.params()
            .into_iter()
            .map(|p| Some((p.name, p.default?)))
            .collect()
    }
}

/// 準備した光源
pub trait SourceInstance: Send + Sync {
    /// 波長 wl [µm] の光線（物体の座標系）
    fn rays(&self, wl: f64) -> Result<SourceRays>;
}

/// 光源の光線: 始点・方向・重み（同じ長さ）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceRays {
    pub p: Vec<V3>,
    pub k: Vec<V3>,
    pub w: Vec<f64>,
}

impl SourceRays {
    /// 本数
    pub fn len(&self) -> usize {
        self.p.len()
    }

    pub fn is_empty(&self) -> bool {
        self.p.is_empty()
    }

    /// 約束（長さがそろう・有限・方向が 0 でない・重みが 0 以上）を確かめる
    pub fn check(&self) -> Result<()> {
        let n = self.p.len();
        if self.k.len() != n || self.w.len() != n {
            invalid!(
                "the source gave different numbers of starts, directions and weights ({n}, {} and {})",
                self.k.len(),
                self.w.len()
            );
        }
        for i in 0..n {
            let (p, k, w) = (self.p[i], self.k[i], self.w[i]);
            if !p.iter().chain(&k).all(|x| x.is_finite()) || k == [0.0; 3] {
                invalid!("ray {i} of the source has a start {p:?} or direction {k:?} that is not finite or is zero");
            }
            if !(w.is_finite() && w >= 0.0) {
                invalid!(
                    "ray {i} of the source has the weight {w} (must be finite and not negative)"
                );
            }
        }
        Ok(())
    }
}

/// 光源のモデルの約束を確かめる: 設定 config で作れ、F・d・C 線の波長で約束どおりの光線を 1 本以上返し、2 回呼んで
/// 同じ光線を返す（雛形・拡張の検査で呼ぶ）
pub fn check_source(m: &dyn SourceModel, config: &Config) -> Result<()> {
    let inst = m.prepare(config)?;
    for wl in [0.4861327, 0.5875618, 0.6562725] {
        let r = inst.rays(wl)?;
        r.check()?;
        if r.is_empty() {
            invalid!("the source gave no rays at {wl} µm");
        }
        if inst.rays(wl)? != r {
            invalid!("the source gave different rays for the same setting at {wl} µm");
        }
    }
    Ok(())
}

impl System {
    /// 光源 src の波長番号 w（0 始まり）の光線を大域座標に直したもの（[`crate::system::Launch::Rays`] に渡せる）。
    /// 約束を守らない光線は誤り
    pub fn source_rays(&self, src: &dyn SourceInstance, w: usize) -> Result<SourceRays> {
        let Some(&wl) = self.wavelengths.get(w) else {
            invalid!(
                "no wavelength {} (the system has {})",
                w + 1,
                self.wavelengths.len()
            );
        };
        let mut r = src.rays(wl)?;
        r.check()?;
        let f = self.object_frame();
        let mv = |v: V3| [0, 1, 2].map(|c| f.r[c][0] * v[0] + f.r[c][1] * v[1] + f.r[c][2] * v[2]);
        for q in r.p.iter_mut() {
            let g = mv(*q);
            *q = [f.o[0] + g[0], f.o[1] + g[1], f.o[2] + g[2]];
        }
        for q in r.k.iter_mut() {
            *q = mv(*q);
        }
        Ok(r)
    }
}
