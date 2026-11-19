//! ザイデル収差係数（3 次の収差、面ごと）と、最大の視野の実光線の歪曲

use crate::error::Result;
use crate::invalid;
use crate::shape::{Base, Shape};
use crate::system::{FieldSel, FieldType, System};

/// 面ごとのザイデル和 S_I〜S_V [レンズ単位]（Welford の定義。近軸周辺光線と、最大の視野へ合わせた近軸主光線から）。
/// `surf[j]` は面 j の寄与（物体面・像面・無視する面・座標ブレークは 0）
#[derive(Debug, Clone, PartialEq)]
pub struct Seidel {
    pub surf: Vec<[f64; 5]>,
    /// 波長 [レンズ単位]（波の数に直すのに使う）
    pub wavelength: f64,
}

impl Seidel {
    /// 面 j の寄与（0 なら全部の面の和）
    pub fn at(&self, j: usize) -> [f64; 5] {
        if j == 0 {
            self.surf.iter().fold([0.0; 5], |mut a, v| {
                for i in 0..5 {
                    a[i] += v[i];
                }
                a
            })
        } else {
            self.surf.get(j).copied().unwrap_or([0.0; 5])
        }
    }

    /// 面 j の寄与を波面収差の係数 [波] で: W040 = S_I/8、W131 = S_II/2、W222 = S_III/2、W220 = S_IV/4、W311 = S_V/2
    /// （OpticStudio の SPHA・COMA・ASTI・FCUR・DIST と同じ。26.1 で実測）
    pub fn waves(&self, j: usize) -> [f64; 5] {
        let s = self.at(j);
        let l = self.wavelength;
        [
            s[0] / (8.0 * l),
            s[1] / (2.0 * l),
            s[2] / (2.0 * l),
            s[3] / (4.0 * l),
            s[4] / (2.0 * l),
        ]
    }
}

/// 近軸主光線（物体空間の傾き 1 か物体の高さ 1）を最大の視野に合わせる倍率
pub fn chief_scale(s: &System) -> f64 {
    let hmax = s.fields.iter().map(|f| f.x.hypot(f.y)).fold(0.0, f64::max);
    match s.field_type {
        FieldType::Angle => hmax.to_radians().tan(),
        FieldType::ObjH => hmax,
        // 像高の視野は近軸像高から
        _ => {
            let p = s.paraxial();
            let pimh = crate::report::system_data(s).pimh;
            (pimh / p.chief_parimage_per_unit).abs()
        }
    }
}

/// 面の形の r⁴ の係数のうち、近軸の曲率の球からずれる分（二次曲面・偶数次非球面だけ）
fn quartic_excess(sh: Shape, c0: f64) -> Option<f64> {
    let (c, k, a4) = match sh {
        Shape::Plane => return Some(0.0),
        Shape::Conic { c, k } => (c, k, 0.0),
        Shape::EvenAsphere(Base { c, k, a }) => (c, k, a.get(1).copied().unwrap_or(0.0)),
        _ => return None,
    };
    Some((1.0 + k) * c.powi(3) / 8.0 + a4 - c0.powi(3) / 8.0)
}

/// 波長 w のザイデル和。ミラー・二次曲面と偶数次非球面でない面のある系は誤り
pub fn seidel(s: &System, w: usize) -> Result<Seidel> {
    let sf = &s.surfaces;
    let img = s.image();
    let p = s.paraxial();
    let k = chief_scale(s);
    let (ym, um) = s.ptrace(p.marginal.0[1], p.marginal.1[0], w, None);
    let (yc, uc) = s.ptrace(p.chief.0[1] * k, p.chief.1[0] * k, w, None);
    // ラグランジュ不変量（物体空間の n (ū y − u ȳ)）。OpticStudio 26.1 は絞りの n ū y（LINV）を使い、主光線を主波長の
    // 入射瞳の中心に通すので、主波長でない波長では絞りの中心を外れて不変量と少し違う（FCUR で 0.12%、DIST で最大 0.9%）
    let h = s.n(0)[w] * (uc[0] * ym[1] - um[0] * yc[1]);
    let mut out = vec![[0.0; 5]; sf.len()];
    for j in 1..img {
        let f = &sf[j];
        if f.ignore || f.is_coord_break() {
            continue;
        }
        if !matches!(
            f.action(),
            crate::action::Action::Refract | crate::action::Action::Reflect { .. }
        ) {
            invalid!("Seidel coefficients need refracting or reflecting surfaces (surface {j})");
        }
        if f.is_mirror() {
            invalid!(
                "Seidel coefficients of systems with mirrors are not supported yet (surface {j})"
            );
        }
        let sh = f.shape();
        let c = sh.paraxial_curv();
        let Some(d4) = quartic_excess(sh, c) else {
            invalid!("Seidel coefficients need standard or even asphere surfaces (surface {j})");
        };
        let (n1, n2) = (s.index_before(j, w), s.n(j)[w]);
        let a = n1 * (um[j - 1] + ym[j] * c);
        let ab = n1 * (uc[j - 1] + yc[j] * c);
        let y = ym[j];
        let du = um[j] / n2 - um[j - 1] / n1;
        let mut v = [
            -a * a * y * du,
            -a * ab * y * du,
            -ab * ab * y * du,
            -h * h * c * (1.0 / n2 - 1.0 / n1),
            0.0,
        ];
        // S_V = Ā (−Ā² y Δ(1/n²) + ȳ c Δ(1/n) (Ā y + H))。Ā/A · (S_III + S_IV) を割り算なしに書いた形で、
        // 周辺光線が面に垂直に入る（A = 0、同心の面）ときも平面でも有限
        v[4] = ab
            * (-ab * ab * y * (1.0 / (n2 * n2) - 1.0 / (n1 * n1))
                + yc[j] * c * (1.0 / n2 - 1.0 / n1) * (ab * y + h));
        // 非球面の分: ΔS_I = 8 Δ₄ (n′ − n) y⁴、ΔS_II〜ΔS_V は ȳ/y の 1〜3 乗を掛ける
        if d4 != 0.0 {
            let ds = 8.0 * d4 * (n2 - n1) * y.powi(4);
            let r = if y != 0.0 { yc[j] / y } else { 0.0 };
            v[0] += ds;
            v[1] += ds * r;
            v[2] += ds * r * r;
            v[4] += ds * r * r * r;
        }
        if v.iter().any(|x| !x.is_finite()) {
            invalid!("Seidel coefficients of surface {j} are not finite");
        }
        out[j] = v;
    }
    Ok(Seidel {
        surf: out,
        wavelength: s.wave(w).mm(),
    })
}

/// 最大の視野（正規化視野 (0, 1)）の主光線の像面での高さと近軸像高の差 [レンズ単位] と、近軸像高に対する % （波長 w）。
/// OpticStudio の DIST の Surf 0（%）と Absolute（差）
pub fn real_distortion(s: &System, w: usize) -> Result<(f64, f64)> {
    let img = s.image();
    let p = s.paraxial();
    let k = chief_scale(s);
    let (yc, _) = s.ptrace(p.chief.0[1] * k, p.chief.1[0] * k, w, None);
    let ybar = yc[img];
    let t = s.try_trace(
        FieldSel::Norm(0.0, 1.0),
        &[0.0],
        &[0.0],
        Some(w),
        None,
        false,
    )?;
    let r = &t.rays[0];
    if let Some(j) = r.status.surf() {
        invalid!("the chief ray of the largest field stopped (surface {j})");
    }
    let d = r.xyz[1] - ybar;
    let pct = 100.0 * d / ybar;
    if !d.is_finite() || !pct.is_finite() {
        invalid!("the distortion is not finite");
    }
    Ok((d, pct))
}

/// 軸上の光線が光軸を横切る点の、像面の頂点からの距離 [レンズ単位]（波長 w）。zone 0 は近軸周辺光線（主波長の入射瞳の縁を
/// 通す）、zone > 0 は正規化瞳座標 (0, zone) の実光線。OpticStudio の LONA（26.1 で実測）
pub fn longitudinal(s: &System, w: usize, zone: f64) -> Result<f64> {
    let img = s.image();
    let v = if zone == 0.0 {
        let p = s.paraxial();
        let (ym, um) = s.ptrace(p.marginal.0[1], p.marginal.1[0], w, None);
        -ym[img] / um[img - 1]
    } else {
        let t = s.try_trace(
            FieldSel::Norm(0.0, 0.0),
            &[0.0],
            &[zone],
            Some(w),
            None,
            false,
        )?;
        let r = &t.rays[0];
        if let Some(j) = r.status.surf() {
            invalid!("the ray of zone {zone} stopped (surface {j})");
        }
        // 像面の局所座標で、交点から軸と交わる点まで（曲面の像面では交点の z = 矢高を足し、頂点から測る）
        r.xyz[2] - r.xyz[1] * r.lmn[2] / r.lmn[1]
    };
    if !v.is_finite() {
        invalid!("the longitudinal aberration of zone {zone} is not finite");
    }
    Ok(v)
}

/// 最大の視野へ合わせた近軸主光線の像面での高さ [レンズ単位]（波長 w。主光線は主波長の入射瞳の中心を通す）。
/// 2 つの波長の差が OpticStudio の LACL（26.1 で実測）
pub fn chief_height(s: &System, w: usize) -> f64 {
    let p = s.paraxial();
    let k = chief_scale(s);
    s.ptrace(p.chief.0[1] * k, p.chief.1[0] * k, w, None).0[s.image()]
}
