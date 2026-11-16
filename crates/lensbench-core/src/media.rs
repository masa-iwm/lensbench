//! 媒質の屈折率を材料から作る（dn・disp をかける前の値）。
//!
//! 材料の名前の読み方は 1 か所（[`System::material_index_at`]）: 空・AIR は 1、系のガラス（温度・気圧・
//! Adjust Index は系の設定）、モデルガラス "nd,vd[,dpgf]"（vd = 0 は分散なし）。GRIN の面は頂点の屈折率、
//! 鏡・座標ブレークは前の媒質を引き継ぐ。材料を設定したとき（[`System::refresh_media`]）も、系の波長を変える・足す
//! とき（[`System::set_wavelengths`]・[`System::with_wavelength`]）も、これで作る。

use crate::error::{Error, Result};
use crate::glass;
use crate::invalid;
use crate::surface::Bire;
use crate::system::{Prescription, System};

/// 面の後の媒質の、材料からの作り方
enum Medium {
    /// 前の媒質を引き継ぐ（鏡・座標ブレーク）
    Previous,
    /// GRIN の頂点の屈折率
    Grin,
    /// 分散のない媒質（空気、vd = 0 のモデルガラス）: 今の値をどの波長でも使う。値は名前でなく面の値
    /// （プロジェクトの屈折率だけのガラスは名前を丸めた "1.52,0.0" で、値は丸めない）
    Flat,
    /// 材料の名前から
    Material,
}

fn medium_of(sys: &System, j: usize) -> Medium {
    let d = &sys.surfaces[j];
    if j > 0 && (d.mirror || d.is_coord_break()) {
        return Medium::Previous;
    }
    if d.grin.is_some() {
        return Medium::Grin;
    }
    if d.n.is_empty() {
        return Medium::Material;
    }
    let m = d.mat.trim();
    let flat = match model_glass(m) {
        Some((_, vd, _)) => vd == 0.0,
        // 名前から作れない材料（カタログに無いガラス）で、値がどの波長でも同じもの（プロジェクトで屈折率だけ
        // 決めた媒質）も分散なし
        None => {
            !(m.is_empty() || m.eq_ignore_ascii_case("AIR") || sys.glasses.contains_key(m)) && {
                let n0 = sys.base_index(j, 0);
                (0..sys.wavelengths.len()).all(|w| sys.base_index(j, w) == n0)
            }
        }
    };
    if flat {
        Medium::Flat
    } else {
        Medium::Material
    }
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
    /// 面 j の後の媒質の、波長 lams [µm] での屈折率（dn・disp をかける前）。鏡・座標ブレークは None（前の媒質）
    pub(crate) fn medium_index_at(&self, j: usize, lams: &[f64]) -> Result<Option<Vec<f64>>> {
        let d = &self.surfaces[j];
        Ok(Some(match medium_of(self, j) {
            Medium::Previous => return Ok(None),
            Medium::Grin => {
                let g = d.grin.as_deref().expect("GRIN");
                lams.iter()
                    .map(|&l| {
                        g.vertex_index(l).ok_or_else(|| {
                            Error::Invalid(format!(
                                "surface {j}: the GRIN vertex is outside the profile range"
                            ))
                        })
                    })
                    .collect::<Result<Vec<f64>>>()?
            }
            Medium::Flat => vec![self.base_index(j, self.primary); lams.len()],
            Medium::Material => self
                .material_index_at(&d.mat, lams)
                .map_err(|e| Error::Invalid(format!("surface {j}: {e}")))?,
        }))
    }

    /// 全面の媒質の、波長 lam での屈折率（dn・disp をかける前。鏡・座標ブレーク・無視した面の扱いは系の値と同じ）
    fn media_at(&self, lam: f64) -> Result<Vec<f64>> {
        let mut prev = 1.0;
        let mut col = Vec::with_capacity(self.surfaces.len());
        for j in 0..self.surfaces.len() {
            let n = match self.medium_index_at(j, &[lam])? {
                Some(v) => v[0],
                None => prev,
            };
            col.push(n);
            if !self.surfaces[j].ignore || j == 0 {
                prev = n;
            }
        }
        Ok(col)
    }

    /// 材料の印 `n` が空の面（挿入した面・材料を変えた面）の屈折率を材料から作る。
    /// 複屈折材の異常光の屈折率（材料の名前-E）も、数が波長の数と違えば作り直す
    pub(crate) fn refresh_media(&mut self) -> Result<()> {
        for j in 0..self.surfaces.len() {
            if !self.surfaces[j].n.is_empty() {
                continue;
            }
            let Some(base) = self.medium_index_at(j, &self.wavelengths)? else {
                continue;
            };
            let d = &mut self.rx_mut().surfaces[j];
            d.n_base.clear();
            d.n = base;
            if let Some(Bire::In { ne, .. }) = &mut d.bire {
                ne.clear();
            }
        }
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
        let mut bire = false;
        for d in self.rx_mut().surfaces.iter_mut() {
            if let Some(Bire::In { ne, .. }) = &mut d.bire {
                ne.clear();
                bire = true;
            }
        }
        if bire {
            self.refresh_media()?;
        }
        Ok(())
    }

    /// 波長 i を lam [µm] に変える（(i, lam) の並び）。全媒質の屈折率（dn・disp をかける前の値と、複屈折材の異常光）を
    /// 材料から新しい波長で作り直す。dn・disp の反映（[`System::update_indices`]）とキャッシュの扱いは呼ぶ側
    /// （[`System::set_params`]）で
    pub(crate) fn set_wavelengths(&mut self, waves: &[(usize, f64)]) -> Result<()> {
        let mut new: Vec<(usize, Vec<f64>)> = Vec::with_capacity(waves.len());
        for &(i, lam) in waves {
            if i >= self.wavelengths.len() || !(lam > 0.0) {
                invalid!(
                    "wavelength {} = {lam} µm (there are {} wavelengths)",
                    i + 1,
                    self.wavelengths.len()
                );
            }
            new.push((i, self.media_at(lam)?));
        }
        for (i, col) in new {
            for (j, n) in col.into_iter().enumerate() {
                let d = &mut self.rx_mut().surfaces[j];
                let v = if d.n_base.is_empty() {
                    &mut d.n
                } else {
                    &mut d.n_base
                };
                if let Some(x) = v.get_mut(i) {
                    *x = n;
                }
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
        let add = self.media_at(lam)?;
        let mut d = self.clone();
        for (j, n) in add.into_iter().enumerate() {
            let sf = &mut d.rx_mut().surfaces[j];
            if sf.n_base.is_empty() {
                sf.n.push(n);
            } else {
                sf.n_base.push(n);
                sf.n.push(f64::NAN);
            }
        }
        d.rx_mut().wavelengths.push(lam);
        d.rx_mut().weights.push(0.0);
        d.rebuild_extraordinary()?;
        d.update_indices();
        d.changed()?;
        Ok((d, self.wavelengths.len()))
    }
}
