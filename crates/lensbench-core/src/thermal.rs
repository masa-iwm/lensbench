//! 熱の構成を作る（OpticStudio の Make Thermal に当たる）。
//!
//! 温度の一覧から、多重構成の表に温度・気圧の行と、各温度での曲率・厚さ・非球面の係数・半直径・機械半径・
//! チップゾーンの行を作る。構成 1 は元の系（今の温度・気圧）で、構成 2 以降が与えた温度。屈折率は構成の温度・気圧から
//! 材料の式で決まる（屈折率を環境に合わせる設定 [`System::adjust_index`] が切れていれば入れる。OpticStudio と同じ）。
//!
//! 規則（OpticStudio 2026 R1.01 の Make Thermal の熱のピックアップの値で確かめた）:
//! - 線膨張係数 α: ガラスの面はカタログの TCE（面の後ろがガラスならそのガラス、空気なら前のガラス）、鏡と前後とも空気の
//!   面はレンズデータの TCE の列。温度差 ΔT は元の系の温度から
//! - 曲率 c / (1 + α ΔT)、非球面（偶数次）の r^(2n) の係数 × (1 + α ΔT)^(1 − 2n)、円錐係数はそのまま。半直径・機械半径・
//!   チップゾーンは × (1 + α ΔT)。ガラスの厚さは × (1 + α ΔT)（そのガラスの α）
//! - 空気の間隔は、間隔の前後の面の半直径の縁を結ぶ薄い円筒のスペーサー（α は間隔の面のレンズデータの TCE）とみる。
//!   スペーサーの長さ（縁から縁）が × (1 + α_m ΔT)。縁の高さは、半直径の小さい方の面はスペーサーの α_m で、大きい方は
//!   その面のガラスの α で変わる。中心の厚さは縁の長さと、その高さでの温度の後の矢高から決める
//!
//! 熱のピックアップ（元の系を変えると追いかける）は作らず、作った時の値を表に書く。

use crate::error::Result;
use crate::invalid;
use crate::material::Material;
use crate::param::{Kind, Param};
use crate::shape::Shape;
use crate::system::System;

/// 熱の構成の設定
#[derive(Debug, Clone, Default)]
pub struct ThermalSpec {
    /// 足す構成の温度 [°C]
    pub temps: Vec<f64>,
    /// 足す構成の気圧 [atm]（空なら元の系の気圧、それ以外は temps と同じ数）
    pub pressures: Vec<f64>,
}

/// 熱の構成を作った結果
#[derive(Debug, Clone)]
pub struct Thermal {
    /// 構成 1 が元の系、構成 2 以降が与えた温度の系（今の構成は 1）
    pub system: System,
    pub warnings: Vec<String>,
}

/// 面 j の線膨張係数 [1/°C]（モジュールの説明の規則）
fn alpha(s: &System, j: usize, warn: &mut Vec<String>) -> f64 {
    let glass_tce = |m: &Material, warn: &mut Vec<String>| -> Option<f64> {
        match m {
            Material::Glass(name) => Some(match s.glasses.get(name).and_then(|g| g.props) {
                Some(p) => p.tce * 1e-6,
                None => {
                    warn.push(format!("surface {j}: no TCE for {name} (taken as 0)"));
                    0.0
                }
            }),
            Material::Model { .. } | Material::Index(_) => {
                warn.push(format!(
                    "surface {j}: a model glass has no TCE (taken as 0)"
                ));
                Some(0.0)
            }
            _ => None,
        }
    };
    let d = &s.surfaces[j];
    if let Some(a) = glass_tce(&d.mat, warn) {
        return a;
    }
    if !d.is_mirror() && j > 0 {
        if let Some(a) = glass_tce(&s.surfaces[j - 1].mat, warn) {
            return a;
        }
    }
    d.tce * 1e-6
}

/// 面 j がレンズの面（前か後ろがガラス、または鏡）か
fn lens_surface(s: &System, j: usize) -> bool {
    let glass = |m: &Material| m.is_medium() && !m.is_air();
    let d = &s.surfaces[j];
    d.is_mirror() || glass(&d.mat) || (j > 0 && glass(&s.surfaces[j - 1].mat))
}

/// 面 j の y = h の矢高（形の外は NaN）
fn sag(s: &System, j: usize, h: f64) -> f64 {
    s.surfaces[j].shape().sag(0.0, h).0
}

/// 熱の構成を作る（元の系が多重構成なら、構成 base の系を元にし、表はその系の値で作り直す）
pub fn make_thermal(s: &System, base: usize, spec: &ThermalSpec) -> Result<Thermal> {
    if spec.temps.is_empty() {
        invalid!("give one or more temperatures");
    }
    if !spec.pressures.is_empty() && spec.pressures.len() != spec.temps.len() {
        invalid!(
            "{} pressures for {} temperatures",
            spec.pressures.len(),
            spec.temps.len()
        );
    }
    if let Some(t) = spec
        .temps
        .iter()
        .find(|t| !(t.is_finite() && **t >= -273.15))
    {
        invalid!("temperature {t} °C");
    }
    if let Some(p) = spec
        .pressures
        .iter()
        .find(|p| !(p.is_finite() && **p >= 0.0))
    {
        invalid!("pressure {p} atm");
    }
    let mut warnings = Vec::new();
    let mut s0 = s.config(base)?;
    if s0.configs.is_some() {
        s0.set_config_count(1)?;
    }
    if !s0.adjust_index {
        // 屈折率を構成の温度・気圧に合わせる（OpticStudio の Make Thermal も入れる）
        s0.edit(|p| {
            p.adjust_index = true;
            Ok(())
        })?;
        warnings.push(
            "turned on adjust index to the environment (the glass indices follow the temperature and pressure)"
                .into(),
        );
    }
    let img = s0.image();
    let t_nom = s0.temperature;
    let al: Vec<f64> = (0..=img).map(|j| alpha(&s0, j, &mut warnings)).collect();
    let lens: Vec<bool> = (0..=img).map(|j| lens_surface(&s0, j)).collect();
    if s0.surfaces.iter().any(|d| d.is_coord_break() && !d.ignore) {
        warnings
            .push("coordinate breaks are left as they are (their thickness doesn't expand)".into());
    }
    let n = spec.temps.len() + 1;
    // 構成 k（0 始まり、0 が元の系）の温度差
    let dts: Vec<f64> = std::iter::once(0.0)
        .chain(spec.temps.iter().map(|t| t - t_nom))
        .collect();
    let mut rows: Vec<(Param, Vec<f64>)> = Vec::new();
    let temps: Vec<f64> = std::iter::once(t_nom)
        .chain(spec.temps.iter().copied())
        .collect();
    let pres: Vec<f64> = std::iter::once(s0.pressure)
        .chain((0..spec.temps.len()).map(|i| spec.pressures.get(i).copied().unwrap_or(s0.pressure)))
        .collect();
    rows.push((Param::system(Kind::Temp), temps));
    rows.push((Param::system(Kind::Pres), pres));
    let scaled = |v: f64, a: f64| -> Vec<f64> { dts.iter().map(|dt| v * (1.0 + a * dt)).collect() };
    // 曲率
    for j in 1..img {
        let d = &s0.surfaces[j];
        if d.ignore || d.is_coord_break() || !lens[j] {
            continue;
        }
        rows.push((
            Param::new(j, Kind::Curv),
            dts.iter().map(|dt| d.c / (1.0 + al[j] * dt)).collect(),
        ));
    }
    // 厚さ
    for j in 1..img {
        let d = &s0.surfaces[j];
        if d.ignore || d.is_coord_break() || !d.t.is_finite() {
            continue;
        }
        let glass = d.mat.is_medium() && !d.mat.is_air();
        let v = if glass {
            scaled(d.t, al[j])
        } else {
            // 次の有効な面（無視した面は飛ばす）
            let Some(b) = (j + 1..=img).find(|&i| !s0.surfaces[i].ignore) else {
                continue;
            };
            let am = d.tce * 1e-6;
            let (hf, hb) = (d.semi.abs(), s0.surfaces[b].semi.abs());
            let sf = |h: f64| if lens[j] { sag(&s0, j, h) } else { 0.0 };
            let sb = |h: f64| if lens[b] { sag(&s0, b, h) } else { 0.0 };
            let e0 = d.t - sf(hf) + sb(hb);
            // 縁の高さの α: 半直径の小さい方はスペーサーの α
            let (xf, xb) = if hf <= hb { (am, al[b]) } else { (al[j], am) };
            dts.iter()
                .map(|dt| {
                    let (gf, gb) = (1.0 + al[j] * dt, 1.0 + al[b] * dt);
                    // 温度の後の形は長さが全部 (1 + α ΔT) 倍: z_T(h) = g z(h / g)
                    let zf = gf * sf(hf * (1.0 + xf * dt) / gf);
                    let zb = gb * sb(hb * (1.0 + xb * dt) / gb);
                    e0 * (1.0 + am * dt) + zf - zb
                })
                .collect()
        };
        if v.iter().any(|x| !x.is_finite()) {
            invalid!("surface {j}: the thickness can't be expanded (the sag at the semi-diameter is undefined)");
        }
        rows.push((Param::new(j, Kind::Thick), v));
    }
    // 非球面の係数・半直径・機械半径・チップゾーン
    for j in 1..img {
        let d = &s0.surfaces[j];
        if d.ignore || d.is_coord_break() || !lens[j] {
            continue;
        }
        match d.shape() {
            Shape::EvenAsphere(_) => {
                // Asph(i) は r^(2(i + 1)) の係数（.zmx の PARM i + 1）
                for i in 0..8u8 {
                    let par = Param::new(j, Kind::Asph(i));
                    let Ok(v) = s0.get_param(par) else { continue };
                    let e = 1.0 - 2.0 * (i as f64 + 1.0);
                    rows.push((
                        par,
                        dts.iter()
                            .map(|dt| v * (1.0 + al[j] * dt).powf(e))
                            .collect(),
                    ));
                }
            }
            Shape::Conic { .. } | Shape::Plane => {}
            _ => warnings.push(format!(
                "surface {j}: only the curvature and size of this surface type are scaled"
            )),
        }
        rows.push((Param::new(j, Kind::Semi), scaled(d.semi.abs(), al[j])));
        rows.push((
            Param::new(j, Kind::MechSemi),
            scaled(s0.lens_radius(j), al[j]),
        ));
        rows.push((Param::new(j, Kind::ChipZone), scaled(d.chip_zone, al[j])));
    }
    let mut out = s0.clone();
    // 表の値を書く曲率・厚さのソルブは外す（値は元の系のまま。OpticStudio も熱の解析の前に外すよう求める）
    for (p, _) in &rows {
        let k = match p.kind {
            Kind::Curv | Kind::Thick => p.kind,
            _ => continue,
        };
        let j = p.index();
        if out
            .solves
            .iter()
            .any(|q| q.surf == j && q.kind.param_kind() == k)
        {
            out.remove_solve(j, k)?;
            warnings.push(format!(
                "removed the solve of surface {j} {} (the value of the original system is kept)",
                k.name()
            ));
        }
    }
    out.set_config_count(n)?;
    for (p, v) in rows {
        out.set_config_row(p, v)?;
    }
    let system = out.config(1)?;
    Ok(Thermal { system, warnings })
}
