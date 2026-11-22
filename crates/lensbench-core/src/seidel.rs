//! ザイデル収差係数（3 次の収差、面ごと）と、実光線の歪曲・縦収差

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
    if !s.symmetric() {
        invalid!("Seidel coefficients need a rotationally symmetric system");
    }
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

/// 面ごとの 1 次の色収差の和 C_L（軸上色）・C_T（倍率色）[レンズ単位]（Welford の C_I = A y Δ(δn/n)、C_II = Ā y Δ(δn/n)。
/// A・Ā・y は波長 w の近軸周辺光線と最大の視野へ合わせた近軸主光線（[`seidel`] と同じ）、δn は波長 long の屈折率 − short の屈折率。
/// 符号は OpticStudio の Seidel Coefficients の CLA・CTR に合わせた（26.1 で実測、Welford の δn = n_F − n_C と逆）。
/// `[j]` は面 j の寄与（物体面・像面・無視する面・座標ブレークは 0）
pub fn chromatic(s: &System, w: usize, short: usize, long: usize) -> Result<Vec<[f64; 2]>> {
    if !s.symmetric() {
        invalid!("Seidel coefficients need a rotationally symmetric system");
    }
    let sf = &s.surfaces;
    let img = s.image();
    let p = s.paraxial();
    let k = chief_scale(s);
    let (ym, um) = s.ptrace(p.marginal.0[1], p.marginal.1[0], w, None);
    let (yc, uc) = s.ptrace(p.chief.0[1] * k, p.chief.1[0] * k, w, None);
    let mut out = vec![[0.0; 2]; sf.len()];
    for j in 1..img {
        let f = &sf[j];
        if f.ignore || f.is_coord_break() {
            continue;
        }
        if f.is_mirror() {
            invalid!(
                "Seidel coefficients of systems with mirrors are not supported yet (surface {j})"
            );
        }
        let c = f.shape().paraxial_curv();
        let (n1, n2) = (s.index_before(j, w), s.n(j)[w]);
        let dn1 = s.index_before(j, long) - s.index_before(j, short);
        let dn2 = s.n(j)[long] - s.n(j)[short];
        let a = n1 * (um[j - 1] + ym[j] * c);
        let ab = n1 * (uc[j - 1] + yc[j] * c);
        let d = dn2 / n2 - dn1 / n1;
        let v = [a * ym[j] * d, ab * ym[j] * d];
        if v.iter().any(|x| !x.is_finite()) {
            invalid!("Seidel coefficients of surface {j} are not finite");
        }
        out[j] = v;
    }
    Ok(out)
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

/// 視野 (hx, hy) の、歪曲の線形の予測に使う座標 t（角度の視野は各向きの角の tan、ほかは正規化視野 × 正規化の値）
fn field_t(s: &System, hx: f64, hy: f64) -> Result<[f64; 2]> {
    let (nx, ny) = s.field_norm_values();
    if s.field_type == FieldType::Angle {
        let (ax, ay) = ((hx * nx).to_radians(), (hy * ny).to_radians());
        if ax.abs() >= std::f64::consts::FRAC_PI_2 || ay.abs() >= std::f64::consts::FRAC_PI_2 {
            invalid!("the distortion of field ({hx}, {hy}) needs field angles below 90 degrees");
        }
        return Ok([ax.tan(), ay.tan()]);
    }
    Ok([hx * nx, hy * ny])
}

/// [`field_t`] の逆
fn field_of_t(s: &System, t: [f64; 2]) -> (f64, f64) {
    let (nx, ny) = s.field_norm_values();
    if s.field_type == FieldType::Angle {
        return (t[0].atan().to_degrees() / nx, t[1].atan().to_degrees() / ny);
    }
    (t[0] / nx, t[1] / ny)
}

/// 視野 (hx, hy) の主光線の像面での (x, y)（波長 w）。アフォーカル像空間の系では向きの tan (l/n, m/n)（OpticStudio の
/// アフォーカルの歪曲・グリッド歪曲の Real の値。2026 R1.01 で実測）
fn chief_xy(s: &System, h: (f64, f64), w: usize) -> Result<[f64; 2]> {
    let t = s.try_trace(
        FieldSel::Norm(h.0, h.1),
        &[0.0],
        &[0.0],
        Some(w),
        None,
        false,
    )?;
    let r = &t.rays[0];
    if let Some(j) = r.status.surf() {
        invalid!(
            "the chief ray of field ({}, {}) stopped (surface {j})",
            h.0,
            h.1
        );
    }
    if s.afocal {
        return Ok([r.lmn[0] / r.lmn[2], r.lmn[1] / r.lmn[2]]);
    }
    Ok([r.xyz[0], r.xyz[1]])
}

/// 歪曲の線形の予測の基準: 基準の視野の主光線の像面の点 p、その視野の t（[`field_t`]）、そこでの ∂(x, y)/∂t の J
/// （グリッド歪曲の A・B・C・D = J[0][0]・J[0][1]・J[1][0]・J[1][1]）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistRef {
    pub p: [f64; 2],
    pub t: [f64; 2],
    pub j: [[f64; 2]; 2],
}

/// 歪曲の向き（OpticStudio の DISA の Data 0・1・2）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DistComp {
    /// 動径（ずれの長さ、符号は予測の変位との内積の符号）
    #[default]
    Radial,
    X,
    Y,
}

/// 基準の視野（None は軸上）の主光線の点と t（J は単位行列）
pub fn distortion_point(s: &System, w: usize, reference: Option<(f64, f64)>) -> Result<DistRef> {
    let (p, t) = match reference {
        None => ([0.0, 0.0], [0.0, 0.0]),
        Some(r) => (chief_xy(s, r, w)?, field_t(s, r.0, r.1)?),
    };
    Ok(DistRef {
        p,
        t,
        j: [[1.0, 0.0], [0.0, 1.0]],
    })
}

/// 基準の視野（None は軸上）での [`DistRef`]。軸上の J は近軸主光線の倍率（回転対称な系だけ。アフォーカル像空間では
/// 像空間の傾きの倍率）、ほかは主光線の点を t で中心差分した値
pub fn distortion_ref(s: &System, w: usize, reference: Option<(f64, f64)>) -> Result<DistRef> {
    let mut r = distortion_point(s, w, reference)?;
    r.j = match reference {
        None => {
            if !s.symmetric() {
                invalid!("the distortion against the axis needs a rotationally symmetric system");
            }
            // 近軸主光線の像面の高さは t に比例する（最大の半径の視野の t で割る）
            let img = s.image();
            let p = s.paraxial();
            let k = chief_scale(s);
            let (yc, uc) = s.ptrace(p.chief.0[1] * k, p.chief.1[0] * k, w, None);
            // t の大きさで最大の半径の視野（角度の視野は k が tan、ほかは視野の座標の最大の半径）
            let tm = if s.field_type == FieldType::Angle {
                k
            } else {
                s.fields.iter().map(|f| f.x.hypot(f.y)).fold(0.0, f64::max)
            };
            // アフォーカル像空間では像空間の近軸主光線の傾き（向きの tan の倍率）
            let m = if s.afocal { uc[img - 1] } else { yc[img] } / tm;
            [[m, 0.0], [0.0, m]]
        }
        Some(_) => {
            let tm = field_t(s, 1.0, 0.0)?[0]
                .abs()
                .max(field_t(s, 0.0, 1.0)?[1].abs());
            if tm == 0.0 {
                invalid!("the distortion needs a field other than the axis");
            }
            let d = 1e-5 * tm;
            let mut j = [[0.0; 2]; 2];
            for k in 0..2 {
                let (mut a, mut b) = (r.t, r.t);
                a[k] -= d;
                b[k] += d;
                let (pa, pb) = (
                    chief_xy(s, field_of_t(s, a), w)?,
                    chief_xy(s, field_of_t(s, b), w)?,
                );
                for i in 0..2 {
                    j[i][k] = (pb[i] - pa[i]) / (2.0 * d);
                }
            }
            j
        }
    };
    if r.j.iter().flatten().any(|v| !v.is_finite()) {
        invalid!("the distortion is not finite");
    }
    Ok(r)
}

/// グリッド歪曲の格子の点 (u, v)（-1〜1）の正規化視野。格子は最大の視野（各視野の (x, y) の長さの最大）の円に内接する正方形で、
/// 角度の視野は tan θ の、高さの視野は高さの等間隔（OpticStudio の Grid Distortion。26.1 で実測）
pub fn grid_field(s: &System, u: f64, v: f64) -> Result<(f64, f64)> {
    let r = s.fields.iter().map(|f| f.x.hypot(f.y)).fold(0.0, f64::max);
    if r == 0.0 {
        invalid!("the grid distortion needs a field off the axis");
    }
    let a = if s.field_type == FieldType::Angle {
        if r >= 90.0 {
            invalid!("the grid distortion needs field angles below 90 degrees");
        }
        r.to_radians().tan()
    } else {
        r
    } / std::f64::consts::SQRT_2;
    Ok(field_of_t(s, [a * u, a * v]))
}

/// 視野 h の主光線の像面の点 p と、基準 r からの線形の予測 p_r + J (t − t_r) の差 d を、長さと予測の変位
/// Δ = J (t − t_r) に対する % で返す。動径は |d|（符号は d·Δ の符号）と |d| / |Δ|、x・y はその成分 d_x と d_x / Δ_x
/// （Δ の成分が 0 なら % は 0）。OpticStudio の DISG・DISA（26.1 で実測）
pub fn distortion_against(
    s: &System,
    w: usize,
    h: (f64, f64),
    r: &DistRef,
    comp: DistComp,
) -> Result<(f64, f64)> {
    let t = field_t(s, h.0, h.1)?;
    let p = chief_xy(s, h, w)?;
    let (j, dt) = (r.j, [t[0] - r.t[0], t[1] - r.t[1]]);
    let del = [
        j[0][0] * dt[0] + j[0][1] * dt[1],
        j[1][0] * dt[0] + j[1][1] * dt[1],
    ];
    let d = [p[0] - r.p[0] - del[0], p[1] - r.p[1] - del[1]];
    let (v, n) = match comp {
        DistComp::Radial => (
            d[0].hypot(d[1]).copysign(d[0] * del[0] + d[1] * del[1]),
            del[0].hypot(del[1]),
        ),
        DistComp::X => (d[0], del[0]),
        DistComp::Y => (d[1], del[1]),
    };
    let pct = if n != 0.0 { 100.0 * v / n } else { 0.0 };
    if !v.is_finite() || !pct.is_finite() {
        invalid!("the distortion is not finite");
    }
    Ok((v, pct))
}

/// 視野 h の主光線の像面の点 p と、基準 r からの線形の予測 p_r + J (t − t_r)（グリッド歪曲の実際の点と理想の点）
pub fn distortion_points(
    s: &System,
    w: usize,
    h: (f64, f64),
    r: &DistRef,
) -> Result<([f64; 2], [f64; 2])> {
    let t = field_t(s, h.0, h.1)?;
    let p = chief_xy(s, h, w)?;
    let (j, dt) = (r.j, [t[0] - r.t[0], t[1] - r.t[1]]);
    let q = [
        r.p[0] + j[0][0] * dt[0] + j[0][1] * dt[1],
        r.p[1] + j[1][0] * dt[0] + j[1][1] * dt[1],
    ];
    Ok((p, q))
}

/// 一般の歪曲（OpticStudio の DISG）: 基準の視野 reference（None は軸上）からの [`distortion_against`] の動径。
/// OpticStudio 26.1 で実測（Double Gauss 28 の軸上・視野 2 の基準、斜めの視野で 1e-10）
pub fn distortion_at(
    s: &System,
    w: usize,
    h: (f64, f64),
    reference: Option<(f64, f64)>,
) -> Result<(f64, f64)> {
    let r = distortion_ref(s, w, reference)?;
    distortion_against(s, w, h, &r, DistComp::Radial)
}

/// 軸上から視野 h までの線の上（100 等分の点）の、軸上を基準にした [`distortion_at`] の絶対値の最大（absolute なら長さ、
/// でなければ %）。OpticStudio の DIMX
pub fn distortion_max(s: &System, w: usize, h: (f64, f64), absolute: bool) -> Result<f64> {
    let r = distortion_ref(s, w, None)?;
    let mut m: f64 = 0.0;
    for k in 1..=100 {
        let f = k as f64 / 100.0;
        let (v, pct) = distortion_against(s, w, (h.0 * f, h.1 * f), &r, DistComp::Radial)?;
        m = m.max(if absolute { v.abs() } else { pct.abs() });
    }
    Ok(m)
}

/// 校正した f-θ 歪曲（OpticStudio の DISC）: y の向きの視野を 100 等分した角 θ の主光線の高さ y に、原点を通る直線 y = cθ を
/// 最小二乗で当て、ずれ |y − cθ| の最大と、|y − cθ| / |cθ| の最大 [%] を返す（角度の視野だけ。OpticStudio 26.1 で実測、
/// Double Gauss 28 で 1e-13）
pub fn ftheta_distortion(s: &System, w: usize) -> Result<(f64, f64)> {
    if s.field_type != FieldType::Angle {
        invalid!("the f-theta distortion needs angle fields");
    }
    let (_, ny) = s.field_norm_values();
    let mut pts = Vec::with_capacity(101);
    for k in 0..=100 {
        let h = k as f64 / 100.0;
        pts.push(((h * ny).to_radians(), chief_xy(s, (0.0, h), w)?[1]));
    }
    let c =
        pts.iter().map(|(a, y)| a * y).sum::<f64>() / pts.iter().map(|(a, _)| a * a).sum::<f64>();
    let (mut da, mut dp): (f64, f64) = (0.0, 0.0);
    for &(a, y) in &pts {
        let d = (y - c * a).abs();
        da = da.max(d);
        if a != 0.0 {
            dp = dp.max(100.0 * d / (c * a).abs());
        }
    }
    if !da.is_finite() || !dp.is_finite() {
        invalid!("the f-theta distortion is not finite");
    }
    Ok((da, dp))
}
