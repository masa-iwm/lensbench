//! フィルタ・ダイクロイックの透過特性: プロジェクト・ユーザーカタログの `coatings`（名前 → 膜の書き方）を
//! TABLE 膜（角度・波長ごとの Rs Rp Ts Tp、[`lensbench_core::coating`]）にする。面の `coat` に名前を書いて使う。
//!
//! - `table`: 測った・メーカーの表をそのまま（行 [入射角 deg, 波長 µm, Rs, Rp, Ts, Tp]）
//! - `curve`: 波長ごとの透過（と反射）。角度・偏光によらない（色ガラス・吸収型 ND を 1 面で表すときなど）
//! - `longpass`・`shortpass`・`bandpass`・`notch`・`nd`: 仕様からの理想の形。帯域内の透過 `t`、帯域外は
//!   OD（T = 10^−od）、エッジは 10〜90 % の幅 `width_nm` のロジスティック。斜め入射ではエッジが
//!   λ(θ) = λ0·√(1 − (sinθ / neff)²) へ短波長側にずれる（s・p は同じ）。反射は 1 − T（`absorptive` なら 0）
//!
//! 位相は 0（TABLE 膜の位相を書かない）。

use lensbench_core::coating::Table;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn d_t() -> f64 {
    0.95
}
fn d_od() -> f64 {
    4.0
}
fn d_neff() -> f64 {
    2.0
}

/// 理想フィルタの共通の項目
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ideal {
    /// 帯域内の透過率（0〜1）
    #[serde(default = "d_t")]
    pub t: f64,
    /// 帯域外（阻止域）の光学濃度（T = 10^−od）
    #[serde(default = "d_od")]
    pub od: f64,
    /// エッジの幅 [nm]（透過が 10 % から 90 % まで。省略は 1 % のエッジ波長、帯域フィルタは FWHM の 10 %）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_nm: Option<f64>,
    /// 斜め入射でのエッジのずれの実効屈折率
    #[serde(default = "d_neff")]
    pub neff: f64,
    /// 反射を 0 にする（吸収型）。省略は反射 = 1 − 透過（干渉型・ダイクロイック）
    #[serde(default)]
    pub absorptive: bool,
}

/// プロジェクト・カタログの `coatings` の 1 つ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoatSpec {
    /// 表: 行 [入射角 deg, 波長 µm, Rs, Rp, Ts, Tp]
    Table { rows: Vec<[f64; 6]> },
    /// 波長 [nm] ごとの透過 `t`（と反射 `r`、省略は 0）。角度・偏光によらない
    Curve {
        wavelengths_nm: Vec<f64>,
        t: Vec<f64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        r: Vec<f64>,
    },
    /// エッジ `edge_nm`（透過 50 %）より長い波長を通す
    Longpass {
        edge_nm: f64,
        #[serde(flatten)]
        ideal: Ideal,
    },
    /// エッジより短い波長を通す
    Shortpass {
        edge_nm: f64,
        #[serde(flatten)]
        ideal: Ideal,
    },
    /// 中心 `center_nm`・半値全幅 `fwhm_nm` を通す
    Bandpass {
        center_nm: f64,
        fwhm_nm: f64,
        #[serde(flatten)]
        ideal: Ideal,
    },
    /// 中心・幅の帯域を od で止める（帯域外は t）
    Notch {
        center_nm: f64,
        fwhm_nm: f64,
        #[serde(flatten)]
        ideal: Ideal,
    },
    /// 波長によらない ND（T = 10^−od）
    Nd {
        od: f64,
        #[serde(default)]
        absorptive: bool,
    },
}

/// 表の入射角 [deg]
const ANGLES: [f64; 13] = [
    0.0, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 60.0, 75.0,
];
/// 表の波長の範囲 [µm] と粗い刻み
const WL: (f64, f64, f64) = (0.2, 2.5, 0.005);

fn logistic(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// 10〜90 % の幅 w のロジスティックの尺度（σ(x/s) が 0.1→0.9 になる x の幅が w）
fn scale(w: f64) -> f64 {
    w / (2.0 * 9f64.ln())
}

fn check01(v: f64, what: &str) -> Result<(), String> {
    if (0.0..=1.0).contains(&v) {
        Ok(())
    } else {
        Err(format!("{what} {v}（0〜1）"))
    }
}

impl Ideal {
    fn check(&self) -> Result<(), String> {
        check01(self.t, "フィルタの透過率")?;
        if !(self.od >= 0.0 && self.od.is_finite()) {
            return Err(format!("フィルタの OD {}", self.od));
        }
        if !(self.neff >= 1.0 && self.neff.is_finite()) {
            return Err(format!("フィルタの neff {}（1 以上）", self.neff));
        }
        if let Some(w) = self.width_nm {
            if !(w > 0.0 && w.is_finite()) {
                return Err(format!("フィルタのエッジの幅 {w} nm"));
            }
        }
        Ok(())
    }
    fn tmin(&self) -> f64 {
        10f64.powf(-self.od).min(self.t)
    }
}

fn pos(v: f64, what: &str) -> Result<(), String> {
    if v > 0.0 && v.is_finite() {
        Ok(())
    } else {
        Err(format!("フィルタの {what} {v}"))
    }
}

impl CoatSpec {
    /// TABLE 膜の表（角度ごとの [波長, Rs, Rp, Ts, Tp, 位相 4 つ] の並び）
    pub fn table(&self) -> Result<Table, String> {
        match self {
            CoatSpec::Table { rows } => table_rows(rows),
            CoatSpec::Curve {
                wavelengths_nm,
                t,
                r,
            } => {
                if wavelengths_nm.len() < 2 || t.len() != wavelengths_nm.len() {
                    return Err("curve は 2 点以上で、wavelengths_nm と t は同じ数".into());
                }
                if !r.is_empty() && r.len() != t.len() {
                    return Err("curve の r は t と同じ数".into());
                }
                if wavelengths_nm
                    .windows(2)
                    .any(|w| w[0].partial_cmp(&w[1]) != Some(std::cmp::Ordering::Less))
                {
                    return Err("curve の波長が昇順でない".into());
                }
                for (i, &x) in t.iter().enumerate() {
                    check01(x, "curve の t")?;
                    let rr = r.get(i).copied().unwrap_or(0.0);
                    check01(rr, "curve の r")?;
                    if x + rr > 1.0 + 1e-9 {
                        return Err(format!(
                            "curve の t + r が 1 を超える（{} nm）",
                            wavelengths_nm[i]
                        ));
                    }
                }
                let rows: Vec<[f64; 9]> = wavelengths_nm
                    .iter()
                    .enumerate()
                    .map(|(i, &l)| {
                        let (tt, rr) = (t[i], r.get(i).copied().unwrap_or(0.0));
                        [l * 1e-3, rr, rr, tt, tt, 0.0, 0.0, 0.0, 0.0]
                    })
                    .collect();
                Ok(vec![(0.0, rows.clone()), (89.0, rows)])
            }
            CoatSpec::Nd { od, absorptive } => {
                if !(*od >= 0.0 && od.is_finite()) {
                    return Err(format!("ND の OD {od}"));
                }
                let t = 10f64.powf(-od);
                let r = if *absorptive { 0.0 } else { 1.0 - t };
                let rows = vec![
                    [WL.0, r, r, t, t, 0.0, 0.0, 0.0, 0.0],
                    [WL.1, r, r, t, t, 0.0, 0.0, 0.0, 0.0],
                ];
                Ok(vec![(0.0, rows.clone()), (89.0, rows)])
            }
            CoatSpec::Longpass { edge_nm, ideal } | CoatSpec::Shortpass { edge_nm, ideal } => {
                ideal.check()?;
                pos(*edge_nm, "エッジ波長")?;
                let e = edge_nm * 1e-3;
                let w = ideal.width_nm.map_or(0.01 * e, |w| w * 1e-3);
                let long = matches!(self, CoatSpec::Longpass { .. });
                Ok(sample(ideal, &[e], w, |l, sh| {
                    let x = (l - e * sh) / scale(w);
                    if long {
                        logistic(x)
                    } else {
                        logistic(-x)
                    }
                }))
            }
            CoatSpec::Bandpass {
                center_nm,
                fwhm_nm,
                ideal,
            }
            | CoatSpec::Notch {
                center_nm,
                fwhm_nm,
                ideal,
            } => {
                ideal.check()?;
                pos(*center_nm, "中心波長")?;
                pos(*fwhm_nm, "半値全幅")?;
                if fwhm_nm >= center_nm {
                    return Err("フィルタの半値全幅が中心波長以上".into());
                }
                let (c, f) = (center_nm * 1e-3, fwhm_nm * 1e-3);
                let w = ideal.width_nm.map_or(0.1 * f, |w| w * 1e-3);
                let (lo, hi) = (c - f / 2.0, c + f / 2.0);
                let band = matches!(self, CoatSpec::Bandpass { .. });
                Ok(sample(ideal, &[lo, hi, c], w, |l, sh| {
                    let s = scale(w);
                    // 帯域の中（両側のエッジのロジスティックの積、帯域が狭くても中心で 1 になるよう正規化）
                    let inside = |l: f64| logistic((l - lo * sh) / s) * logistic((hi * sh - l) / s);
                    let v = inside(l) / inside(c * sh);
                    if band {
                        v
                    } else {
                        1.0 - v
                    }
                }))
            }
        }
    }
}

/// 理想フィルタの表: 角度ごとに、粗い刻みとエッジの近く（ずれた位置の ±4 幅を幅の 1/10 で）の波長で
/// 形 f(λ, ずれの比) ∈ [0, 1] を透過 tmin〜t に写す
fn sample(ideal: &Ideal, edges: &[f64], w: f64, f: impl Fn(f64, f64) -> f64) -> Table {
    let (t0, t1) = (ideal.tmin(), ideal.t);
    ANGLES
        .iter()
        .map(|&a| {
            let s = a.to_radians().sin() / ideal.neff;
            let sh = (1.0 - s * s).sqrt();
            let mut wl: Vec<f64> = Vec::new();
            let n = ((WL.1 - WL.0) / WL.2).round() as usize;
            wl.extend((0..=n).map(|i| WL.0 + i as f64 * WL.2));
            let step = w / 10.0;
            for &e in edges {
                let e = e * sh;
                wl.extend((-40..=40).map(|i| e + i as f64 * step));
            }
            wl.retain(|&l| l > 0.0);
            wl.sort_by(f64::total_cmp);
            wl.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
            let rows = wl
                .into_iter()
                .map(|l| {
                    let t = t0 + (t1 - t0) * f(l, sh).clamp(0.0, 1.0);
                    let r = if ideal.absorptive { 0.0 } else { 1.0 - t };
                    [l, r, r, t, t, 0.0, 0.0, 0.0, 0.0]
                })
                .collect();
            (a, rows)
        })
        .collect()
}

fn table_rows(rows: &[[f64; 6]]) -> Result<Table, String> {
    let mut out: Table = Vec::new();
    for r in rows {
        if !r.iter().all(|v| v.is_finite()) {
            return Err("表の値が数でない".into());
        }
        if !(0.0..90.0).contains(&r[0]) || r[1] <= 0.0 {
            return Err(format!("表の行の角度 {}・波長 {}", r[0], r[1]));
        }
        for &v in &r[2..] {
            check01(v, "表の R・T")?;
        }
        let row = [r[1], r[2], r[3], r[4], r[5], 0.0, 0.0, 0.0, 0.0];
        match out.iter_mut().find(|(a, _)| *a == r[0]) {
            Some((_, v)) => v.push(row),
            None => out.push((r[0], vec![row])),
        }
    }
    if out.is_empty() {
        return Err("表の行が無い".into());
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (a, v) in &mut out {
        v.sort_by(|x, y| x[0].total_cmp(&y[0]));
        if v.windows(2).any(|w| w[0][0] == w[1][0]) {
            return Err(format!("表の角度 {a} に同じ波長の行が 2 つ"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表の (角度, 波長) の透過（角度は表にある値、波長は線形補間）
    fn t_at(t: &Table, a: f64, l: f64) -> f64 {
        let rows = &t.iter().find(|x| x.0 == a).unwrap().1;
        let xs: Vec<f64> = rows.iter().map(|r| r[0]).collect();
        let ys: Vec<f64> = rows.iter().map(|r| r[3]).collect();
        lensbench_core::coating::interp(l, &xs, &ys)
    }

    fn ideal() -> Ideal {
        serde_json::from_str("{}").unwrap()
    }

    #[test]
    fn longpass_edge_od_and_shift() {
        let c = CoatSpec::Longpass {
            edge_nm: 600.0,
            ideal: Ideal {
                width_nm: Some(10.0),
                ..ideal()
            },
        };
        let t = c.table().unwrap();
        assert!((t_at(&t, 0.0, 0.6) - (1e-4 + (0.95 - 1e-4) * 0.5)).abs() < 1e-9);
        assert!((t_at(&t, 0.0, 0.8) - 0.95).abs() < 1e-9);
        assert!((t_at(&t, 0.0, 0.4) - 1e-4).abs() < 1e-9);
        // 10〜90 % の幅
        let f = |l: f64| (t_at(&t, 0.0, l) - 1e-4) / (0.95 - 1e-4);
        assert!((f(0.605) - 0.9).abs() < 1e-3 && (f(0.595) - 0.1).abs() < 1e-3);
        // 45° ではエッジが 600·√(1 − (sin45°/2)²) nm へ
        let e45 = 0.6 * (1.0 - (45f64.to_radians().sin() / 2.0).powi(2)).sqrt();
        assert!((f_at(&t, 45.0, e45) - 0.5).abs() < 1e-6, "{e45}");
        // 反射は 1 − T
        let r = &t[0].1[0];
        assert!((r[1] + r[3] - 1.0).abs() < 1e-12);
    }

    fn f_at(t: &Table, a: f64, l: f64) -> f64 {
        (t_at(t, a, l) - 1e-4) / (0.95 - 1e-4)
    }

    #[test]
    fn bandpass_notch_nd_curve() {
        let b = CoatSpec::Bandpass {
            center_nm: 532.0,
            fwhm_nm: 10.0,
            ideal: ideal(),
        };
        let t = b.table().unwrap();
        assert!((t_at(&t, 0.0, 0.532) - 0.95).abs() < 1e-6);
        assert!((f_at(&t, 0.0, 0.527) - 0.5).abs() < 0.02);
        assert!((t_at(&t, 0.0, 0.6) - 1e-4).abs() < 1e-9);
        let n = CoatSpec::Notch {
            center_nm: 532.0,
            fwhm_nm: 10.0,
            ideal: Ideal { od: 6.0, ..ideal() },
        };
        let t = n.table().unwrap();
        assert!((t_at(&t, 0.0, 0.532) - 1e-6).abs() < 1e-8);
        assert!((t_at(&t, 0.0, 0.7) - 0.95).abs() < 1e-9);
        let nd = CoatSpec::Nd {
            od: 1.0,
            absorptive: true,
        };
        let t = nd.table().unwrap();
        assert!((t_at(&t, 0.0, 1.0) - 0.1).abs() < 1e-12);
        assert_eq!(t[0].1[0][1], 0.0);
        let c: CoatSpec = serde_json::from_value(serde_json::json!({
            "type": "curve", "wavelengths_nm": [400, 500, 600], "t": [0.1, 0.5, 0.9]
        }))
        .unwrap();
        let t = c.table().unwrap();
        assert!((t_at(&t, 0.0, 0.45) - 0.3).abs() < 1e-12);
        assert!((t_at(&t, 89.0, 0.55) - 0.7).abs() < 1e-12);
    }

    #[test]
    fn table_rows_and_errors() {
        let c: CoatSpec = serde_json::from_value(serde_json::json!({
            "type": "table", "rows": [[0, 0.5, 0.1, 0.1, 0.9, 0.9], [0, 0.6, 0.2, 0.2, 0.8, 0.8], [45, 0.5, 0.3, 0.2, 0.7, 0.8]]
        }))
        .unwrap();
        let t = c.table().unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].1.len(), 2);
        assert_eq!(t[1].1[0][2], 0.2);
        for bad in [
            serde_json::json!({"type": "table", "rows": []}),
            serde_json::json!({"type": "table", "rows": [[0, 0.5, 0.1, 0.1, 1.5, 0.9]]}),
            serde_json::json!({"type": "table", "rows": [[0, 0.5, 0, 0, 1, 1], [0, 0.5, 0, 0, 1, 1]]}),
            serde_json::json!({"type": "curve", "wavelengths_nm": [500], "t": [0.5]}),
            serde_json::json!({"type": "curve", "wavelengths_nm": [500, 400], "t": [0.5, 0.5]}),
            serde_json::json!({"type": "curve", "wavelengths_nm": [400, 500], "t": [0.5, 0.6], "r": [0.6, 0]}),
            serde_json::json!({"type": "longpass", "edge_nm": -1}),
            serde_json::json!({"type": "longpass", "edge_nm": 500, "t": 2}),
            serde_json::json!({"type": "bandpass", "center_nm": 500, "fwhm_nm": 600}),
            serde_json::json!({"type": "nd", "od": -1}),
        ] {
            let c: CoatSpec = serde_json::from_value(bad.clone()).unwrap();
            assert!(c.table().is_err(), "{bad}");
        }
    }
}
