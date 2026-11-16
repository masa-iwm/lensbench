//! 媒質の屈折率を材料から作る（dn・disp をかける前の値）。
//!
//! 材料の名前の読み方は 1 か所（[`Prescription::material_index_at`]）: 空・AIR は 1、系のガラス（温度・気圧・
//! Adjust Index は系の設定）、モデルガラス "nd,vd[,dpgf]"（vd = 0 は分散なし）。GRIN の面は頂点の屈折率、
//! 鏡・座標ブレークは前の媒質を引き継ぐ。屈折率は文書から [`resolve`] で作り、[`crate::System`] が
//! 持つ（文書には屈折率を置かない。屈折率だけの媒質は面の `index`）。

use crate::error::{Error, Result};
use crate::glass;
use crate::invalid;
use crate::surface::Bire;
use crate::system::{Prescription, System};

/// 文書から求めた媒質の屈折率（面ごと・波長ごと）。[`crate::System`] が持ち、文書を直すたびに作り直す
#[derive(Debug, Clone, Default)]
pub(crate) struct Media {
    /// 面の後の媒質の屈折率（dn・disp 込み。鏡・座標ブレークは前の媒質）
    pub n: Vec<Vec<f64>>,
    /// dn・disp をかける前の屈折率
    pub base: Vec<Vec<f64>>,
}

/// 面 j の後の媒質の、波長 lams [µm] での屈折率（dn・disp をかける前）。鏡・座標ブレークは None（前の媒質）
fn medium_index_at(p: &Prescription, j: usize, lams: &[f64]) -> Result<Option<Vec<f64>>> {
    let d = &p.surfaces[j];
    if j > 0 && (d.mirror || d.is_coord_break()) {
        return Ok(None);
    }
    if let Some(g) = d.grin.as_deref() {
        return lams
            .iter()
            .map(|&l| {
                g.vertex_index(l).ok_or_else(|| {
                    Error::Invalid(format!(
                        "surface {j}: the GRIN vertex is outside the profile range"
                    ))
                })
            })
            .collect::<Result<Vec<f64>>>()
            .map(Some);
    }
    if let Some(x) = d.index {
        return Ok(Some(vec![x; lams.len()]));
    }
    p.material_index_at(&d.mat, lams)
        .map(Some)
        .map_err(|e| Error::Invalid(format!("surface {j}: {e}")))
}

/// 文書の全面の媒質の屈折率を作る（屈折率を求めるのはここだけ）。無視した面は次の面の「前の媒質」にならない
pub(crate) fn resolve(p: &Prescription) -> Result<Media> {
    let ns = p.surfaces.len();
    let wp = p.primary;
    let mut m = Media {
        n: Vec::with_capacity(ns),
        base: Vec::with_capacity(ns),
    };
    let mut prev = vec![1.0; p.wavelengths.len()];
    for j in 0..ns {
        let d = &p.surfaces[j];
        let (n, base) = match medium_index_at(p, j, &p.wavelengths)? {
            // 鏡・座標ブレーク: 前の媒質（ずれをかける前の値も前の媒質の値）
            None => (prev.clone(), prev.clone()),
            Some(b) => {
                let n = if (d.dn != 0.0 || d.disp != 0.0) && wp < b.len() {
                    let bp = b[wp];
                    b.iter().map(|&x| x + d.dn + d.disp * (x - bp)).collect()
                } else {
                    b.clone()
                };
                (n, b)
            }
        };
        if !d.ignore || j == 0 {
            prev = n.clone();
        }
        m.n.push(n);
        m.base.push(base);
    }
    Ok(m)
}

/// モデルガラスの名前 "nd,vd[,dpgf]" の (nd, vd, dpgf)
fn model_glass(mat: &str) -> Option<(f64, f64, f64)> {
    let v: Vec<f64> = mat
        .trim()
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    ((2..=3).contains(&v.len()) && v.iter().all(|x| x.is_finite()) && v[0] > 0.0)
        .then(|| (v[0], v[1], v.get(2).copied().unwrap_or(0.0)))
}

impl Prescription {
    /// 材料の屈折率（系の波長ごと、dn・disp をかける前）。空・AIR は 1、"nd,vd[,dpgf]" はモデルガラス、
    /// ほかは系のガラス（[`System::add_glasses`] で足せる）
    pub fn material_index(&self, mat: &str) -> Result<Vec<f64>> {
        self.material_index_at(mat, &self.wavelengths)
    }

    /// [`System::material_index`] を波長 lams [µm] で
    pub fn material_index_at(&self, mat: &str, lams: &[f64]) -> Result<Vec<f64>> {
        let m = mat.trim();
        if m.is_empty() || m.eq_ignore_ascii_case("AIR") {
            return Ok(vec![1.0; lams.len()]);
        }
        if m.eq_ignore_ascii_case("MIRROR") {
            invalid!("MIRROR is not a medium");
        }
        if let Some(g) = self.glasses.get(m) {
            let (t, p, a) = (self.temperature, self.pressure, self.adjust_index);
            return Ok(lams.iter().map(|&l| g.index(l, t, p, a)).collect());
        }
        if let Some((nd, vd, dp)) = model_glass(m) {
            return Ok(lams
                .iter()
                .map(|&l| {
                    if vd == 0.0 {
                        nd
                    } else {
                        glass::model_index(nd, vd, dp, l)
                    }
                })
                .collect());
        }
        invalid!("no glass {m} (add catalogs: add_glasses, add_glass_catalogs)")
    }
}

impl System {
    /// 複屈折材の異常光の屈折率（材料の名前-E）を、数が波長の数と違えば材料から作り直す
    pub(crate) fn refresh_media(&mut self) -> Result<()> {
        let nw = self.wavelengths.len();
        for j in 0..self.surfaces.len() {
            let d = &self.surfaces[j];
            let Some(Bire::In { ne, .. }) = &d.bire else {
                continue;
            };
            if ne.len() == nw {
                continue;
            }
            let en = Bire::e_name(&d.mat);
            let v = self.material_index(&en).map_err(|e| {
                Error::Invalid(format!(
                    "surface {j}: extraordinary material {en} of the birefringent material: {e}"
                ))
            })?;
            if let Some(Bire::In { ne, .. }) = &mut self.rx_mut().surfaces[j].bire {
                *ne = v;
            }
        }
        Ok(())
    }

    /// 複屈折材の異常光の屈折率を材料から作り直す（波長を変えた・足したとき）
    fn rebuild_extraordinary(&mut self) -> Result<()> {
        if !self
            .surfaces
            .iter()
            .any(|d| matches!(d.bire, Some(Bire::In { .. })))
        {
            return Ok(());
        }
        for d in self.rx_mut().surfaces.iter_mut() {
            if let Some(Bire::In { ne, .. }) = &mut d.bire {
                ne.clear();
            }
        }
        self.refresh_media()
    }

    /// 波長 i を lam [µm] に変える（(i, lam) の並び）。複屈折材の異常光も作り直す。媒質の屈折率と
    /// キャッシュの扱いは呼ぶ側（[`System::set_params`]）で
    pub(crate) fn set_wavelengths(&mut self, waves: &[(usize, f64)]) -> Result<()> {
        for &(i, lam) in waves {
            if i >= self.wavelengths.len() || !(lam > 0.0) {
                invalid!(
                    "wavelength {} = {lam} µm (there are {} wavelengths)",
                    i + 1,
                    self.wavelengths.len()
                );
            }
        }
        for &(i, lam) in waves {
            self.rx_mut().wavelengths[i] = lam;
        }
        self.rebuild_extraordinary()
    }

    /// 波長 lam [µm] を末尾に重み 0 で足した系と、その波長の番号（0 始まり）。主波長・視野・エイミングは変えない。
    /// 媒質の屈折率は材料から作る（[`System::set_wavelengths`] と同じ）
    pub fn with_wavelength(&self, lam: f64) -> Result<(System, usize)> {
        if !(lam > 0.0) {
            invalid!("wavelength {lam} µm");
        }
        let mut d = self.clone();
        d.rx_mut().wavelengths.push(lam);
        d.rx_mut().weights.push(0.0);
        d.rebuild_extraordinary()?;
        d.changed()?;
        Ok((d, self.wavelengths.len()))
    }
}
