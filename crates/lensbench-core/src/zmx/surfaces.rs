//! .zmx の面: 面の行（RawSurf）を面（Surface）にする。面タイプ・材料・面タイプごとの形・傾き偏心・開口・屈折率

use super::values::Glasses;
use super::*;
use crate::material::Material as M;
use std::sync::Arc;

/// 面を作るのに要る、系の値と解いた値
pub(super) struct Ctx<'a> {
    pub raw: &'a Raw,
    pub opt: &'a LoadOptions,
    pub ops: &'a Ops,
    /// 面ごとの厚さ・PARM・材料（ピックアップと多重構成を当てたもの）
    pub th: &'a [f64],
    pub parm: &'a [BTreeMap<i64, f64>],
    pub mats: &'a [Material],
    pub stop: usize,
    /// GRIN の材料（GRIN の面があるときだけ）
    pub grin: Option<&'a crate::grin::GrinData>,
    pub wl: &'a [f64],
    pub glasses: &'a Glasses,
    /// 温度 [°C]・気圧 [atm]・屈折率を空気に合わせるか（ENVD）
    pub env: (f64, f64, bool),
}

/// 面の後の傾き偏心のピックアップ（元の面, 反転）
pub(super) type AfterPick = Option<(usize, bool)>;

/// 面の並びと、面ごとの面の後の傾き偏心のピックアップ
pub(super) type Surfaces = (Vec<Surface>, Vec<AfterPick>);

impl Ctx<'_> {
    pub(super) fn surfaces(&self) -> Result<Surfaces> {
        let ns = self.raw.surfs.len();
        let mut sf: Vec<Surface> = Vec::with_capacity(ns);
        let mut after: Vec<AfterPick> = Vec::with_capacity(ns);
        let mut nprev = vec![1.0; self.wl.len()];
        for (k, s) in self.raw.surfs.iter().enumerate() {
            let (d, a, n) = self.surface(k, s, &nprev)?;
            if !d.ignore {
                nprev = n;
            }
            after.push(a);
            sf.push(d);
        }
        Ok((sf, after))
    }

    /// 面 k。面の後の傾き偏心のピックアップと、面の後の媒質の屈折率（波長ごと、nprev は前の媒質）も返す
    fn surface(
        &self,
        k: usize,
        s: &RawSurf,
        nprev: &[f64],
    ) -> Result<(Surface, AfterPick, Vec<f64>)> {
        let ki = k as i64;
        let fop = |op: &str, p1: i64, default: f64| fop(self.ops, op, p1, default);
        let name = s.typ.clone().unwrap_or_else(|| "STANDARD".into());
        let typ = surf_type(&name, k)?;
        let (mut mat, model) = self.mats[k].clone();
        if let Some((nd, vd, dp)) = model {
            mat = if vd == 0.0 {
                format!("{nd:.2},{vd:.1}")
            } else if dp == 0.0 {
                // 分散あり: 書き出しで値を戻せるように丸めない
                format!("{nd},{vd}")
            } else {
                format!("{nd},{vd},{dp}")
            };
        }
        let ign = k > 0 && fop("IGNR", ki, if s.ignore { 1.0 } else { 0.0 }) != 0.0;
        let p = |i: i64| self.parm[k].get(&i).copied().unwrap_or(0.0);
        let mut d = Surface {
            form: Form::of(typ),
            ignore: ign,
            t: self.th[k],
            comment: s.comment.clone(),
            // モデルガラスは値で持つ（分散のないものも丸めない）
            mat: match model {
                _ if typ == SurfType::CoordBrk => M::Inherit,
                Some((nd, 0.0, _)) => M::Index(nd),
                Some((nd, vd, dpgf)) => M::Model { nd, vd, dpgf },
                None => M::parse(&mat),
            },
            coat: s.coat.clone().unwrap_or_default(),
            stop: k == self.stop,
            semi: fop("SDIA", ki, s.diam.unwrap_or(0.0)),
            // 自動（DIAM … 0、欄がなければ自動）。多重構成の SDIA で値を決める面は固定
            semi_auto: !self.opt.fixed_semi
                && s.diam_mode.unwrap_or(0) == 0
                && !self.ops.contains_key(&("SDIA".to_string(), ki)),
            ..Default::default()
        };
        birefringent(&name, s, &p, k, &mut d)?;
        // 面タイプの PARM の値（番号と値の対応は Surface::set_parm の 1 か所）
        let set_parms = |d: &mut Surface| {
            for i in 1..=8 {
                d.set_parm(i, p(i as i64));
            }
        };
        if matches!(
            typ,
            SurfType::Jones | SurfType::CoordBrk | SurfType::Paraxial
        ) {
            set_parms(&mut d);
        }
        self.shape(typ, s, &mut d, &mat, &p, k)?;
        // 係数の並び（Tilted・Irregular は決まった数だけ）とトロイダルの回転半径（0 は無限 = 円柱面）
        if crate::form::asph_parm_base(typ).is_some() {
            set_parms(&mut d);
        }
        self.profile(typ, s, &mut d, k)?;
        let after = tilts(s, &mut d, k)?;
        aperture(s, k, &mut d, self.ops);
        let n = self.index(&d, typ, &mat, model, nprev, k)?;
        if let Some(crate::surface::Bire::In { ne, .. }) = d.bire.as_mut() {
            let en = crate::surface::Bire::e_name(&mat);
            let Some(g) = self.glasses.map.get(&en) else {
                let why = &self.glasses.why;
                invalid!(code = crate::error::code::GLASS_NOT_FOUND; "extraordinary glass {en} of birefringent material {mat} (surface {k}) is not in the catalogs{why}");
            };
            let (temp, pres, adjust) = self.env;
            *ne = self
                .wl
                .iter()
                .map(|&l| g.index(l, temp, pres, adjust))
                .collect();
        }
        Ok((d, after, n))
    }

    /// 面タイプごとの形（近軸レンズの確かめ、Zernike・Grid Sag、Binary 2、GRIN、円柱フレネル）
    fn shape(
        &self,
        typ: SurfType,
        s: &RawSurf,
        d: &mut Surface,
        mat: &str,
        p: &dyn Fn(i64) -> f64,
        k: usize,
    ) -> Result<()> {
        match typ {
            SurfType::Paraxial => {
                // 0・1 は基準 D0〜D3 で OpticStudio と一致を確認済み。それ以外は未確認
                if !(0..=1).contains(&d.opd_mode()) {
                    notimpl!("paraxial lens (surface {k}) OPD mode {}", d.opd_mode());
                }
                if d.focal() == 0.0 {
                    invalid!("paraxial lens (surface {k}) focal length is 0");
                }
            }
            SurfType::ZernikeStd | SurfType::ZernikeFringe | SurfType::GridSag => {
                freeform_surface(s, d, typ, p, k)?;
            }
            SurfType::Binary2 => {
                // PARM 0 は回折次数。XDAT 1 は項の数、2 は正規化半径、k+2 は ρ^{2k} の係数 [rad]
                let nt = s.xdat.get(&1).copied().unwrap_or(0.0);
                if nt < 0.0 || nt.fract() != 0.0 || nt > 240.0 {
                    invalid!("surface {k}: {nt} Binary 2 terms");
                }
                let norm = s.xdat.get(&2).copied().unwrap_or(1.0);
                if norm <= 0.0 {
                    invalid!("surface {k}: Binary 2 normalization radius {norm}");
                }
                let coef = (0..nt as i64)
                    .map(|j| s.xdat.get(&(j + 3)).copied().unwrap_or(0.0))
                    .collect();
                d.form = Form::Binary2 {
                    asph: Vec::new(),
                    bin2: Some(Arc::new(crate::surface::Binary2 {
                        order: p(0),
                        norm,
                        coef,
                    })),
                };
            }
            SurfType::Gradium | SurfType::Gradient9 => {
                if mat.eq_ignore_ascii_case("MIRROR") {
                    notimpl!("surface {k}: mirror on a GRIN surface");
                }
                let gd = self.grin.expect("GRIN のデータは読込済み");
                let g = if typ == SurfType::Gradium {
                    // PARM 0 素材の厚さ（分布のファイルの値）、1 Δt、2 Δz、3 頂点の基準の屈折率（表示用）、
                    // 4〜7 dx dy tx ty、8 端の扱い
                    let cap = p(8);
                    if !(0.0..=3.0).contains(&cap) || cap.fract() != 0.0 {
                        invalid!("surface {k}: Gradium end handling {cap}");
                    }
                    gd.gradium(mat, p(2), [p(4), p(5), p(6), p(7)], cap as u8, p(1))?
                } else {
                    gd.grad9(mat, [p(2), p(3)], p(1))?
                };
                let grin = Some(Arc::new(g));
                d.form = if typ == SurfType::Gradium {
                    Form::Gradium { grin }
                } else {
                    Form::Gradient9 { grin }
                };
            }
            SurfType::CylFresnel => {
                // XDAT 1 は項の数、2・3 は屈折の面の曲率・円錐係数、k+3 は y^{2k} の係数
                let nt = s.xdat.get(&1).copied().unwrap_or(0.0);
                if nt < 0.0 || nt.fract() != 0.0 || nt > 240.0 {
                    invalid!("surface {k}: {nt} Cylinder Fresnel terms");
                }
                let coef = (0..nt as i64)
                    .map(|j| s.xdat.get(&(j + 4)).copied().unwrap_or(0.0))
                    .collect();
                d.form = Form::CylFresnel {
                    asph: Vec::new(),
                    cylf: Some(Arc::new(crate::surface::CylFresnel {
                        c: s.xdat.get(&2).copied().unwrap_or(0.0),
                        k: s.xdat.get(&3).copied().unwrap_or(0.0),
                        coef,
                    })),
                };
            }
            _ => {}
        }
        Ok(())
    }

    /// 曲率・円錐係数（曲率を持つ面タイプだけ）と、面タイプに合わない値の確かめ
    fn profile(&self, typ: SurfType, s: &RawSurf, d: &mut Surface, k: usize) -> Result<()> {
        if !matches!(
            typ,
            SurfType::Standard
                | SurfType::Fresnel
                | SurfType::CylFresnel
                | SurfType::Binary2
                | SurfType::EvenAsph
                | SurfType::Jones
                | SurfType::Toroidal
                | SurfType::ZernikeStd
                | SurfType::ZernikeFringe
                | SurfType::GridSag
                | SurfType::Gradium
                | SurfType::Gradient9
                | SurfType::OddAsph
                | SurfType::TiltSurf
                | SurfType::Irregular
        ) {
            return Ok(());
        }
        let ki = k as i64;
        d.c = fop(self.ops, "CRVT", ki, s.curv.unwrap_or(0.0));
        d.k = fop(self.ops, "CONN", ki, s.coni.unwrap_or(0.0));
        if typ == SurfType::Toroidal {
            let nz = s.xdat.get(&1).copied().unwrap_or(0.0);
            if nz != 0.0 {
                notimpl!("surface {k}: Zernike terms on a toroidal surface ({nz} terms)");
            }
        } else if typ == SurfType::Standard
            && d.bire.is_none()
            && self.parm[k].values().any(|&v| v != 0.0)
        {
            notimpl!("surface {k}: parameters on a standard surface");
        }
        if typ == SurfType::Jones && d.c != 0.0 {
            notimpl!("Jones matrix surface {k} on a curved surface");
        }
        Ok(())
    }

    /// 面の後の媒質の屈折率（波長ごと）。鏡と座標ブレークは前の媒質のまま、GRIN は頂点の屈折率
    fn index(
        &self,
        d: &Surface,
        typ: SurfType,
        mat: &str,
        model: Option<(f64, f64, f64)>,
        nprev: &[f64],
        k: usize,
    ) -> Result<Vec<f64>> {
        let wl = self.wl;
        Ok(if d.is_mirror() || typ == SurfType::CoordBrk {
            nprev.to_vec()
        } else if let Some(g) = &d.grin() {
            // GRIN: 頂点の屈折率
            wl.iter()
                .map(|&l| {
                    g.vertex_index(l).ok_or_else(|| {
                        Error::Invalid(format!("surface {k}: the GRIN vertex is out of range"))
                    })
                })
                .collect::<Result<Vec<f64>>>()?
        } else if mat.is_empty() || mat == "AIR" {
            vec![1.0; wl.len()]
        } else if let Some((nd, vd, dp)) = model {
            if vd == 0.0 {
                vec![nd; wl.len()]
            } else {
                wl.iter()
                    .map(|&l| glass::model_index(nd, vd, dp, l))
                    .collect()
            }
        } else {
            let Some(g) = self.glasses.map.get(mat) else {
                let why = &self.glasses.why;
                invalid!(code = crate::error::code::GLASS_NOT_FOUND; "glass {mat} (surface {k}) is not in the catalogs{why}");
            };
            let (temp, pres, adjust) = self.env;
            wl.iter().map(|&l| g.index(l, temp, pres, adjust)).collect()
        })
    }
}

/// .zmx の面タイプの名前から面タイプ
fn surf_type(name: &str, k: usize) -> Result<SurfType> {
    Ok(match name {
        "STANDARD" => SurfType::Standard,
        "EVENASPH" => SurfType::EvenAsph,
        "COORDBRK" => SurfType::CoordBrk,
        "JONESMAT" => SurfType::Jones,
        "PARAXIAL" => SurfType::Paraxial,
        "TOROIDAL" => SurfType::Toroidal,
        "SZERNSAG" => SurfType::ZernikeStd,
        "FZERNSAG" => SurfType::ZernikeFringe,
        "GRID_SAG" => SurfType::GridSag,
        "FRESNELS" => SurfType::Fresnel,
        "CFRESNEL" => SurfType::CylFresnel,
        "BINARY_2" => SurfType::Binary2,
        "GRINSUR8" => SurfType::Gradium,
        "GRINSUR9" => SurfType::Gradient9,
        "ODDASPHE" => SurfType::OddAsph,
        "TILTSURF" => SurfType::TiltSurf,
        "IRREGULA" => SurfType::Irregular,
        // 複屈折材の板の入口・出口は形は標準面（偏光の位相は Surface::bire）
        "BIRE__IN" | "BIRE_OUT" => SurfType::Standard,
        _ => notimpl!("surface {k}: surface type {name}"),
    })
}

/// 複屈折材の板の入口・出口（BIRE__IN・BIRE_OUT）の偏光の設定
fn birefringent(
    name: &str,
    s: &RawSurf,
    p: &dyn Fn(i64) -> f64,
    k: usize,
    d: &mut Surface,
) -> Result<()> {
    if name == "BIRE__IN" {
        let mode = p(1);
        // Mode 1・3 は異常光の道（屈折が違う）で未対応
        if !(mode == 0.0 || mode == 2.0) {
            notimpl!("surface {k}: Birefringent In Mode {mode} (only 0 and 2)");
        }
        if s.xdat.get(&1).copied().unwrap_or(0.0) != 0.0 {
            notimpl!("surface {k}: Birefringent In Gradient Mode");
        }
        let axis = [p(2), p(3), p(4)];
        if !(axis.iter().map(|v| v * v).sum::<f64>() > 0.0) {
            invalid!("surface {k}: Birefringent In crystal axis is 0");
        }
        d.bire = Some(crate::surface::Bire::In {
            mode: mode as u8,
            axis,
            ne: Vec::new(),
        });
    } else if name == "BIRE_OUT" {
        d.bire = Some(crate::surface::Bire::Out);
    }
    Ok(())
}

/// 面ごとの材料（名前, モデルガラス）。多重構成の GLSS を当て、材料のピックアップ（GLAS … 2 元の面）は元の面の
/// 材料にする（連なっていても、元の面が後ろにあってもよい）
pub(super) type Material = (String, Option<(f64, f64, f64)>);

pub(super) fn materials(raw: &Raw, ops: &Ops) -> Result<Vec<Material>> {
    let ns = raw.surfs.len();
    let own = |k: usize| -> Material {
        let s = &raw.surfs[k];
        match ops.get(&("GLSS".to_string(), k as i64)) {
            Some(v) => (v.trim_matches('"').to_string(), None),
            None => (s.glas.clone().unwrap_or_default(), s.model_glass),
        }
    };
    (0..ns)
        .map(|k| {
            let mut j = k;
            for _ in 0..=ns {
                // 多重構成で材料を与えた面はピックアップより多重構成が勝つ
                let picked = raw.surfs[j]
                    .glas_pickup
                    .filter(|_| !ops.contains_key(&("GLSS".to_string(), j as i64)));
                match picked {
                    None => return Ok(own(j)),
                    Some(src) if src >= ns => {
                        invalid!("surface {j}: no material pickup source surface {src}")
                    }
                    Some(src) => j = src,
                }
            }
            invalid!("surface {k}: material pickups go round in a loop")
        })
        .collect()
}

fn tilts(s: &RawSurf, d: &mut Surface, k: usize) -> Result<AfterPick> {
    let mut pick = None;
    for v in &s.scbd {
        let g = |i: usize| -> f64 { v.get(i).and_then(|x| fv(x).ok()).unwrap_or(0.0) };
        let (which, order, mode) = (g(0) as i64, g(1) as i64, g(2) as i64);
        let vals = [g(3), g(4), g(5), g(6), g(7)];
        let any = vals.iter().any(|&x| x != 0.0);
        if d.typ() == SurfType::CoordBrk && any {
            notimpl!("surface {k}: surface tilt or decenter on a coordinate break");
        }
        if order != 0 && order != 1 {
            notimpl!("surface {k}: tilt and decenter order {order}");
        }
        let t = Tilt {
            d: vals,
            order: order as u8,
        };
        match which {
            1 => {
                if any {
                    d.before = Some(t);
                }
            }
            2 => match mode {
                2 => {
                    if d.before.is_some() {
                        d.after = After::Reverse;
                    }
                }
                0 => {
                    if any {
                        d.after = After::Tilt(t);
                    }
                }
                // 1 = この面の前と同じ、2·(この面 − 元の面) + 1 は元の面の前の値、+2 は反転（OpticStudio 2026 R1.01
                // で実測。元の面の後の値は使わない。後ろの面からは取れない）
                m if m >= 1 => {
                    let off = ((m - 1) / 2) as usize;
                    if off > k {
                        invalid!("surface {k}: pickup source surface of the tilt and decenter after the surface (mode {m})");
                    }
                    pick = Some((k - off, m % 2 == 0));
                }
                _ => notimpl!("surface {k}: tilt and decenter after the surface, mode {mode}"),
            },
            _ => notimpl!("surface {k}: SCBD {which}"),
        }
    }
    Ok(pick)
}

/// k は面の並びの添字（多重構成の行の面の番号。SURF の行の番号 s.n ではない）
fn aperture(s: &RawSurf, k: usize, d: &mut Surface, ops: &Ops) {
    let dec = s.obdc.clone().unwrap_or_default();
    let (dx, dy) = (
        dec.first().copied().unwrap_or(0.0),
        dec.get(1).copied().unwrap_or(0.0),
    );
    let k = k as i64;
    let g = |v: &Vec<f64>, i: usize| v.get(i).copied().unwrap_or(0.0);
    d.ap = if let Some(c) = &s.clap {
        let max = ops
            .get(&("APMX".to_string(), k))
            .and_then(|v| fv(v).ok())
            .unwrap_or(g(c, 1));
        Aperture::Circular {
            min: g(c, 0),
            max,
            dx,
            dy,
        }
    } else if let Some(c) = &s.sqap {
        Aperture::Rect {
            hx: g(c, 0),
            hy: g(c, 1),
            dx,
            dy,
        }
    } else if let Some(c) = &s.elap {
        Aperture::Ellipse {
            hx: g(c, 0),
            hy: g(c, 1),
            dx,
            dy,
        }
    } else if s.flap.is_some() {
        Aperture::Floating { semi: d.semi }
    } else if let Some(c) = &s.obsc {
        Aperture::Obsc {
            min: g(c, 0),
            max: g(c, 1),
            dx,
            dy,
        }
    } else if let Some(c) = &s.spid {
        Aperture::Spider {
            width: g(c, 0),
            arms: g(c, 1) as u32,
            dx,
            dy,
        }
    } else {
        Aperture::None
    };
}

/// Zernike Sag・Grid Sag の面の読込（OpticStudio 26.1 の書き方、OpticStudio で書き出した .zmx で確認）:
/// PARM 0 は外挿（Zernike、1 = する）か補間（Grid、0 = 双三次・1 = 双一次）、PARM 1〜8 は r^2〜r^16、
/// PARM 9・10 は Zernike の項の偏心、XDAT 1 は項の数、XDAT 2 は正規化半径、XDAT k+2 は項 k の係数。
/// Grid Sag は GDAT（nx ny dx dy xdec ydec unit …）と GARR（.DAT の行の順: +y の行から、行の中は −x から）
fn freeform_surface(
    s: &RawSurf,
    d: &mut Surface,
    typ: SurfType,
    p: &dyn Fn(i64) -> f64,
    k: usize,
) -> Result<()> {
    use crate::freeform::{GridSag, ZernSag};
    use std::sync::Arc;
    let fringe = typ == SurfType::ZernikeFringe;
    let nterm = s.xdat.get(&1).copied().unwrap_or(0.0);
    if nterm < 0.0 || nterm.fract() != 0.0 || nterm as usize > ZernSag::max_terms(fringe) {
        invalid!("surface {k}: {nterm} Zernike terms");
    }
    let nterm = nterm as usize;
    if nterm > 0 {
        let norm = s.xdat.get(&2).copied().unwrap_or(100.0);
        if !(norm > 0.0) {
            invalid!("surface {k}: Zernike normalization radius {norm}");
        }
        let coef = (1..=nterm)
            .map(|j| s.xdat.get(&(j as i64 + 2)).copied().unwrap_or(0.0))
            .collect();
        // Grid Sag には外挿の列がない（常に外挿する）
        let extrapolate = typ == SurfType::GridSag || s.parm.get(&0).copied().unwrap_or(1.0) != 0.0;
        let mut z = ZernSag::new(fringe, norm, coef, extrapolate);
        z.dx = p(9);
        z.dy = p(10);
        d.set_type(typ);
        *d.zern_mut().expect("Zernike・Grid Sag の面") = Some(Arc::new(z));
    } else if typ != SurfType::GridSag {
        d.set_type(typ);
        *d.zern_mut().expect("Zernike の面") =
            Some(Arc::new(ZernSag::new(fringe, 100.0, Vec::new(), true)));
    }
    if typ == SurfType::GridSag {
        let Some(h) = &s.gdat else {
            invalid!("surface {k}: no Grid Sag grid (GDAT)");
        };
        let g = |i: usize| h.get(i).copied().unwrap_or(0.0);
        let (nx, ny) = (
            count_in(
                g(0),
                &format!("surface {k}: Grid Sag grid columns"),
                2,
                MAX_GRID,
            )?,
            count_in(
                g(1),
                &format!("surface {k}: Grid Sag grid rows"),
                2,
                MAX_GRID,
            )?,
        );
        if nx.saturating_mul(ny) > MAX_GRID {
            invalid!("surface {k}: Grid Sag grid {nx}×{ny} (up to {MAX_GRID} points)");
        }
        if !(g(2) > 0.0 && g(3) > 0.0) {
            let (dx, dy) = (g(2), g(3));
            invalid!("surface {k}: Grid Sag grid {nx}×{ny}, spacing {dx} {dy}");
        }
        if g(6) != 0.0 {
            notimpl!("surface {k}: Grid Sag grid unit {} (not mm)", g(6));
        }
        let mut z = vec![[0.0; 4]; nx * ny];
        let mut nodata = vec![false; nx * ny];
        let mut seen = vec![false; nx * ny];
        for &(i, v) in &s.garr {
            if i >= nx * ny {
                invalid!("surface {k}: GARR number {i} (the grid is {nx}×{ny})");
            }
            let kk = (ny - 1 - i / nx) * nx + i % nx;
            z[kk] = [v[0], v[1], v[2], v[3]];
            nodata[kk] = v[4] != 0.0;
            seen[kk] = true;
        }
        if seen.iter().any(|&b| !b) {
            invalid!(
                "surface {k}: too few GARR points ({} / {})",
                s.garr.len(),
                nx * ny
            );
        }
        let interp = p(0);
        if interp != 0.0 && interp != 1.0 {
            notimpl!("surface {k}: Grid Sag interpolation {interp}");
        }
        let grid = Some(Arc::new(GridSag {
            nx,
            ny,
            dx: g(2),
            dy: g(3),
            xdec: g(4),
            ydec: g(5),
            z,
            nodata,
            linear: interp == 1.0,
        }));
        let zern = d.zern_mut().and_then(|z| z.take());
        d.form = Form::GridSag {
            asph: d.asph().to_vec(),
            grid,
            grid_file: s.fimp.clone().unwrap_or_default(),
            zern,
        };
    } else {
        let z = d
            .zern_mut()
            .and_then(|z| z.as_mut())
            .map(Arc::make_mut)
            .expect("Zernike data");
        z.dx = p(9);
        z.dy = p(10);
    }
    Ok(())
}
