//! 面のデータ（OpticStudio のレンズデータエディタの 1 行に当たる）。

use crate::freeform::{GridSag, ZernSag};
use crate::grin::Grin;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SurfType {
    Standard,
    EvenAsph,
    CoordBrk,
    Jones,
    Paraxial,
    /// トロイダル面: YZ 面の曲線（c・k・asph を y で）を Y 軸に平行な軸の周りに回した面。
    /// x 方向の曲率は `cx`（回転半径の逆数、0 なら円柱面）
    Toroidal,
    /// Zernike Standard Sag（基準の面 c・k・asph ＋ Noll の順の Zernike の項、[`crate::freeform`]）
    ZernikeStd,
    /// Zernike Fringe Sag（Fringe の順の 37 項まで）
    ZernikeFringe,
    /// Grid Sag（基準の面 ＋ 格子の補間）
    GridSag,
    /// Fresnel（OpticStudio の FRESNELS）: 交点は頂点の平面、屈折は偶数次非球面（c・k・asph）の傾きで
    Fresnel,
    /// Cylinder Fresnel（CFRESNEL）: 交点は基板（c・k・asph、y だけの関数）、屈折は [`CylFresnel`] の面（y だけ）の傾きで
    CylFresnel,
    /// Binary Optic 2（BINARY_2）: 形は偶数次非球面、位相 [`Binary2`] で光路長と方向が変わる
    Binary2,
    /// Gradium（GRINSUR8）: 形は標準面、面の後ろは軸方向の屈折率分布 [`crate::grin`]
    Gradium,
    /// Gradient 9（GRINSUR9）: 形は標準面 + x tan α + y tan β、面の後ろは径方向の屈折率分布
    Gradient9,
    /// Odd Asphere（ODDASPHE）: 標準面（c・k）+ Σ β_i r^i（i = 1..8、`asph` に β_1..β_8）
    OddAsph,
    /// Tilted（TILTSURF）: 平面 z = x tan α + y tan β（`asph` に tan α・tan β）。局所座標は傾けない
    TiltSurf,
    /// Irregular（IRREGULA）: 偏心・傾けた標準面（c・k）に Zs ρ⁴ + Za ρ_y² + Zc ρ² ρ_y（ρ は半直径で正規化、
    /// 角度で回した座標）。`asph` に [偏心 x, 偏心 y, 傾き x°, 傾き y°, Zs, Za, Zc, 角度°]（[`crate::freeform`]）
    Irregular,
    /// 利用者定義の面（[`crate::usersurf`]）: 形は頂点の平面、曲げ方と位相は登録のモデル（`Surface::user`）
    User,
}

/// Binary 2 の位相 Φ = order · Σ coef[i] ρ^{2(i+1)}（ρ = r / norm、係数の単位は rad）
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Binary2 {
    /// 回折次数 M
    pub order: f64,
    /// 正規化半径
    pub norm: f64,
    pub coef: Vec<f64>,
}

impl Binary2 {
    /// 位相 [rad] と (∂Φ/∂x, ∂Φ/∂y)
    pub fn phase(&self, x: f64, y: f64) -> (f64, f64, f64) {
        let inv = 1.0 / (self.norm * self.norm);
        let p2 = (x * x + y * y) * inv;
        // Σ a_i p^i と d/dp
        let (mut f, mut df, mut pw) = (0.0, 0.0, 1.0);
        for (i, &a) in self.coef.iter().enumerate() {
            let e = (i + 1) as f64;
            df += a * e * pw;
            pw *= p2;
            f += a * pw;
        }
        let g = self.order * df * 2.0 * inv;
        (self.order * f, g * x, g * y)
    }
}

/// Cylinder Fresnel の屈折に使う面: z = c y² / (1 + √(1 − (1 + k) c² y²)) + Σ coef[i] y^{2(i+1)}
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CylFresnel {
    pub c: f64,
    pub k: f64,
    pub coef: Vec<f64>,
}

/// 傾き・偏心（dx dy tx ty tz [deg]）と順序（0: 偏心→X→Y→Z、1: Z→Y→X→偏心）
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Tilt {
    pub d: [f64; 5],
    pub order: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum After {
    None,
    /// 「前」の変換を戻す
    Reverse,
    Tilt(Tilt),
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Aperture {
    None,
    /// 最小・最大半径、偏心 x y
    Circular {
        min: f64,
        max: f64,
        dx: f64,
        dy: f64,
    },
    /// x・y の半幅、偏心
    Rect {
        hx: f64,
        hy: f64,
        dx: f64,
        dy: f64,
    },
    /// 自動の半径（既定では切らない）
    Floating {
        semi: f64,
    },
    /// 楕円（x・y の半軸、偏心）
    Ellipse {
        hx: f64,
        hy: f64,
        dx: f64,
        dy: f64,
    },
    /// 円形遮蔽（最小〜最大半径の間を遮る）
    Obsc {
        min: f64,
        max: f64,
        dx: f64,
        dy: f64,
    },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Surface {
    pub typ: SurfType,
    pub ignore: bool,
    /// 次の面までの厚さ [mm]（物体面は無限遠なら inf）
    pub t: f64,
    pub mirror: bool,
    pub comment: String,
    /// 材料名（空なら空気、座標ブレークは "-"）
    pub mat: String,
    pub coat: String,
    pub stop: bool,
    /// 半直径 [mm]。`semi_auto` なら光線から求めた値（[`crate::System::update_semi`]）
    pub semi: f64,
    /// 半直径を光線から求める（OpticStudio の Automatic、.zmx の DIAM … 0、系ファイルの "semi": "auto"）
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "std::ops::Not::not")
    )]
    pub semi_auto: bool,
    pub c: f64,
    pub k: f64,
    /// 偶数次非球面の係数 r^2, r^4, …（トロイダル面では y^2, y^4, …）
    pub asph: Vec<f64>,
    /// トロイダル面の回転の曲率（1 / 回転半径、0 は回転半径が無限 = 円柱面）
    pub cx: f64,
    /// Zernike Sag の項（[`SurfType::ZernikeStd`]・[`SurfType::ZernikeFringe`]）
    pub zern: Option<Arc<ZernSag>>,
    /// Grid Sag の格子（[`SurfType::GridSag`]）と、その .DAT のファイル名（書き出し用）
    pub grid: Option<Arc<GridSag>>,
    pub grid_file: String,
    /// Binary 2 の位相（[`SurfType::Binary2`]）
    pub bin2: Option<Arc<Binary2>>,
    /// Cylinder Fresnel の屈折の面（[`SurfType::CylFresnel`]）
    pub cylf: Option<Arc<CylFresnel>>,
    /// 面の後ろの屈折率分布（[`SurfType::Gradium`]・[`SurfType::Gradient9`]）。`n` は頂点の屈折率
    pub grin: Option<Arc<Grin>>,
    pub cb: [f64; 5],
    pub cb_order: u8,
    pub focal: f64,
    pub opd_mode: i32,
    pub jones: [f64; 8],
    pub before: Option<Tilt>,
    pub after: After,
    pub ap: Aperture,
    /// 面の後の媒質の屈折率（波長ごと、下の dn・disp を反映した値）
    pub n: Vec<f64>,
    /// 媒質の屈折率のずれ（全波長に足す）。[`crate::param`] で変える
    pub dn: f64,
    /// 媒質の分散の相対変化: n = n0(λ) + dn + disp·(n0(λ) − n0(主波長))
    pub disp: f64,
    /// dn・disp をかける前の屈折率 n0（どちらも 0 なら空）
    pub n_base: Vec<f64>,
    /// 屈折面での反射（ゴーストの経路、[`crate::ghost`]）。`mirror` と一緒に立て、光線は鏡として曲げ、
    /// 偏光の係数は入射側の媒質と反対側の媒質 [`Reflect::mat`] の境界（膜込み）の反射にする
    pub reflect: Option<Reflect>,
    /// Jones 面（[`SurfType::Jones`]）を波長ごとに作る偏光子・波長板。あれば偏光の追跡は `jones` でなく
    /// これを各波長で使う（`jones` は設計波長 `wl0` の値で、.zmx の JONESMAT に書く）
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub retarder: Option<Retarder>,
    /// 複屈折材の板の入口・出口（OpticStudio の Birefringent In / Out）。形は標準面のまま
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub bire: Option<Bire>,
    /// 利用者定義の面（[`SurfType::User`]）のモデルと設定
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub user: Option<crate::usersurf::UserSurf>,
}

/// 偏光子・波長板の Jones 行列の元: 軸（`axis_deg`、x から y へ）の向きの振幅透過 `tx`、直交する向き `ty`、
/// 軸の向きを速い軸とする遅延 φ = 2π·(`order` + `retardance`)·`wl0`/λ（ゼロ次は order 0）。
/// 行列は R(−θ)·diag(tx·e^{−iφ/2}, ty·e^{+iφ/2})·R(θ)（光路の長い側が位相 +、[`crate::fringe`] の e^{+ikL} と同じ向き）。
/// `plates` があれば（複屈折材の板）遅延は板ごとの Δn(λ)·d/λ で、行列は板の行列の積（光の通る順）
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq)]
pub struct Retarder {
    pub axis_deg: f64,
    pub tx: f64,
    pub ty: f64,
    /// 設計波長での遅延 [波長]（λ/4 板は 0.25、偏光子は 0）
    pub retardance: f64,
    /// 多次の波長板の整数の次数（遅延は order + retardance 波長）
    pub order: u32,
    /// 設計波長 [µm]
    pub wl0: f64,
    /// 複屈折材の板（空なら理想の遅延 order + retardance）
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub plates: Vec<Plate>,
}

/// 波長板の複屈折材の板 1 枚: 常光の屈折率はガラス `material`、異常光は `material`-E（OpticStudio の
/// 複屈折材の名前の約束）。板の光学軸は板の面内で、速い軸の向きは `axis_deg`（波長板の `axis_deg` からの角度）。
/// 遅延は φ(λ) = 2π·s·(ne − no)(λ)·d/λ（s は設計波長での ne − no の符号で、設計波長で速い軸が `axis_deg` になる）。
/// 斜めに入る光での遅延の変化は見ない（垂直入射の値）
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq)]
pub struct Plate {
    pub material: String,
    /// 厚さ [mm]
    pub thickness: f64,
    pub axis_deg: f64,
    /// 常光・異常光のガラス（[`Retarder::resolve`] で引く）
    #[cfg_attr(feature = "serde", serde(default))]
    pub o: Option<crate::glass::Glass>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub e: Option<crate::glass::Glass>,
}

impl Plate {
    /// 波長 `wl` [µm] の ne − no（ガラスを引いていなければ 0）
    pub fn dn(&self, wl: f64) -> f64 {
        match (&self.o, &self.e) {
            (Some(o), Some(e)) => e.n_cat(wl) - o.n_cat(wl),
            _ => 0.0,
        }
    }
}

/// 軸 `axis_deg` の向きの振幅 a、直交する向き b の Jones 行列
fn axis_matrix(
    a: num_complex::Complex64,
    b: num_complex::Complex64,
    axis_deg: f64,
) -> [num_complex::Complex64; 4] {
    let (s, c) = axis_deg.to_radians().sin_cos();
    // R(θ) = [[c, s], [−s, c]] で軸の座標へ、diag(a, b)、R(−θ) で戻す
    [
        a * c * c + b * s * s,
        (a - b) * c * s,
        (a - b) * c * s,
        a * s * s + b * c * c,
    ]
}

fn mul2(
    m: &[num_complex::Complex64; 4],
    n: &[num_complex::Complex64; 4],
) -> [num_complex::Complex64; 4] {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
    ]
}

impl Retarder {
    /// 板の常光・異常光のガラスを `glasses` から引く。厚さ 0 の板（1 枚だけのとき）は、設計波長の遅延
    /// order + retardance から厚さを決める
    pub fn resolve(
        &mut self,
        glasses: &std::collections::HashMap<String, crate::glass::Glass>,
    ) -> Result<(), String> {
        let n = self.plates.len();
        for p in &mut self.plates {
            let ename = format!("{}-E", p.material);
            let (Some(o), Some(e)) = (glasses.get(&p.material), glasses.get(&ename)) else {
                return Err(format!(
                    "複屈折材 {} の常光 {} と異常光 {ename} のガラスがカタログにない",
                    p.material, p.material
                ));
            };
            p.o = Some(o.clone());
            p.e = Some(e.clone());
            let dn = p.dn(self.wl0);
            if !(dn.is_finite() && dn != 0.0) {
                return Err(format!(
                    "複屈折材 {} の設計波長 {} µm の ne − no が {dn}",
                    p.material, self.wl0
                ));
            }
            if p.thickness == 0.0 {
                if n != 1 {
                    return Err("板が 2 枚以上の波長板は板ごとに厚さを書く".into());
                }
                // d [mm] = (order + retardance)·λ0 / |Δn(λ0)|
                p.thickness = (self.order as f64 + self.retardance) * self.wl0 * 1e-3 / dn.abs();
            }
            if !(p.thickness > 0.0 && p.thickness.is_finite()) {
                return Err(format!(
                    "複屈折材 {} の板の厚さ {}",
                    p.material, p.thickness
                ));
            }
        }
        Ok(())
    }

    /// 設計波長での板ごとの遅延 [波長]（速い軸から見た遅い軸の遅れ）
    pub fn plate_retardance(&self, wl: f64) -> Vec<f64> {
        self.plates
            .iter()
            .map(|p| p.dn(self.wl0).signum() * p.dn(wl) * p.thickness * 1e3 / wl)
            .collect()
    }

    /// 波長 `wl` [µm] の Jones 行列 [A, B, C, D]（Ex' = A·Ex + B·Ey、Ey' = C·Ex + D·Ey）
    pub fn jones_at(&self, wl: f64) -> [num_complex::Complex64; 4] {
        use num_complex::Complex64 as C;
        if !self.plates.is_empty() {
            let tau = 2.0 * std::f64::consts::PI;
            let mut m = axis_matrix(C::new(self.tx, 0.0), C::new(self.ty, 0.0), self.axis_deg);
            for (p, r) in self.plates.iter().zip(self.plate_retardance(wl)) {
                let q = axis_matrix(
                    C::from_polar(1.0, -tau * r / 2.0),
                    C::from_polar(1.0, tau * r / 2.0),
                    self.axis_deg + p.axis_deg,
                );
                m = mul2(&q, &m);
            }
            return m;
        }
        let phi = if self.retardance == 0.0 && self.order == 0 {
            0.0
        } else {
            2.0 * std::f64::consts::PI * (self.order as f64 + self.retardance) * self.wl0 / wl
        };
        axis_matrix(
            C::from_polar(self.tx, -phi / 2.0),
            C::from_polar(self.ty, phi / 2.0),
            self.axis_deg,
        )
    }
    /// 波長 `wl` の行列を面の `jones`（[Ar, Ai, Br, Bi, Cr, Ci, Dr, Di]）の並びで
    pub fn jones8(&self, wl: f64) -> [f64; 8] {
        let m = self.jones_at(wl);
        [
            m[0].re, m[0].im, m[1].re, m[1].im, m[2].re, m[2].im, m[3].re, m[3].im,
        ]
    }
}

/// 複屈折材の板（OpticStudio の Birefringent In / Out の組）。入口の面の後の媒質（`mat`）が常光の材料、
/// `mat`-E が異常光の材料。光線は常光の道を進み（屈折率 no）、出口の面で、板の中の光線の向き k と
/// 結晶軸 a から決まる異常光の成分（a の k に垂直な向き）に、常光の成分に対して
/// exp(−i·2π·(n(θ) − no)·L/λ) の位相を付ける（OpticStudio の Mode 2、波長板モード）。
/// n(θ) = 1/√(cos²θ/no² + sin²θ/ne²)、θ は k と a の角、L は板の中の光線の長さ。Mode 0 は常光の成分だけを通す
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq)]
pub enum Bire {
    In {
        /// 0: 常光の成分だけ、2: 波長板（両方の位相）。1・3（異常光の道）は未対応
        mode: u8,
        /// 結晶軸の向き（入口の面の局所座標、規格化しなくてよい）
        axis: [f64; 3],
        /// 異常光の屈折率（波長ごと）
        #[cfg_attr(feature = "serde", serde(default))]
        ne: Vec<f64>,
    },
    Out,
}

impl Bire {
    /// 異常光の材料の名前（OpticStudio の約束）
    pub fn e_name(mat: &str) -> String {
        format!("{}-E", mat.trim())
    }
}

/// 屈折面での反射の、反対側（光が入らない側）の媒質
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq)]
pub struct Reflect {
    /// 媒質の名前（空気は "AIR"）
    pub mat: String,
    /// 屈折率（波長ごと）
    pub n: Vec<f64>,
}

impl Default for Surface {
    fn default() -> Self {
        Surface {
            typ: SurfType::Standard,
            ignore: false,
            t: 0.0,
            mirror: false,
            comment: String::new(),
            mat: String::new(),
            coat: String::new(),
            stop: false,
            semi: 0.0,
            semi_auto: false,
            c: 0.0,
            k: 0.0,
            asph: Vec::new(),
            cx: 0.0,
            zern: None,
            grid: None,
            grid_file: String::new(),
            bin2: None,
            cylf: None,
            grin: None,
            cb: [0.0; 5],
            cb_order: 0,
            focal: 0.0,
            opd_mode: 0,
            jones: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            before: None,
            after: After::None,
            ap: Aperture::None,
            n: Vec::new(),
            dn: 0.0,
            disp: 0.0,
            n_base: Vec::new(),
            reflect: None,
            retarder: None,
            bire: None,
            user: None,
        }
    }
}

impl Surface {
    pub fn has_asph(&self) -> bool {
        self.asph.iter().any(|&a| a != 0.0)
    }
}
