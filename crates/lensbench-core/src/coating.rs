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
//!   反射の位相は OpticStudio の「光線相当の位相への換算」に合わせる（鏡のみ。透過する薄膜は未確認なので止める）
//! - `TABLE`: 角度・波長ごとの Rs Rp Ts Tp と位相、線形補間
//! - 暗号化された膜（.ZEC）: 層は読めないので（復号もしない）、OpticStudio の偏光追跡で測った係数の表
//!   （[`Measured`]）を使う。表がなければ追跡を止める
//!
//! 境界の鍵は `膜|前の媒質|後の媒質（鏡は MIRROR）`（[`media_keys`]）。

use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::Path;

pub use num_complex::Complex64 as C64;

use crate::glass;
use crate::surface::SurfType;
use crate::system::System;
use crate::{invalid, notimpl, Error, Result};

/// 膜なしの鏡の基板
pub const MIRROR_N: C64 = C64::new(0.7, -7.0);

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub mat: String,
    pub d: f64,
    /// true: d は絶対値 [µm]、false: 基準波長での光学的な波長単位
    pub absolute: bool,
}

/// 膜の表: 角度 [deg] の昇順に、波長ごとの (波長, Rs, Rp, Ts, Tp, 位相 rs rp ts tp [deg])
pub type Table = Vec<(f64, Vec<[f64; 9]>)>;

/// 膜ファイル（COATING.DAT 形式）の内容
#[derive(Debug, Clone, Default)]
pub struct CoatingFile {
    /// 材料: 波長 [µm]、n、k
    pub mate: HashMap<String, Vec<[f64; 3]>>,
    pub coat: HashMap<String, Vec<Layer>>,
    pub table: HashMap<String, Table>,
}

fn num(s: &str, what: &str) -> Result<f64> {
    s.parse::<f64>()
        .map_err(|_| Error::Invalid(format!("膜ファイル: {what} の数値 {s}")))
}

impl CoatingFile {
    pub fn load(path: &Path) -> Result<CoatingFile> {
        let txt =
            glass::read_text(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        Self::parse(&txt)
    }

    pub fn parse(txt: &str) -> Result<CoatingFile> {
        enum Cur {
            None,
            Mate(String),
            Coat(String),
            Table(String, Option<f64>),
        }
        let mut f = CoatingFile::default();
        let mut tables: HashMap<String, Table> = HashMap::new();
        let mut cur = Cur::None;
        for line in txt.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            if t.is_empty() || t[0].starts_with('!') {
                continue;
            }
            match t[0] {
                // 理想膜・テーパ・暗号化の宣言は使わない（I.xx は組込み、暗号化は実測表）
                "IDEAL" | "IDEAL2" | "ENCRYPTED" | "TAPR" => cur = Cur::None,
                "MATE" | "COAT" | "TABLE" if t.len() >= 2 => {
                    let name = t[1].to_string();
                    cur = match t[0] {
                        "MATE" => {
                            f.mate.insert(name.clone(), Vec::new());
                            Cur::Mate(name)
                        }
                        "COAT" => {
                            f.coat.insert(name.clone(), Vec::new());
                            Cur::Coat(name)
                        }
                        _ => {
                            tables.insert(name.clone(), Vec::new());
                            Cur::Table(name, None)
                        }
                    };
                }
                _ => match &mut cur {
                    Cur::None => {}
                    Cur::Mate(name) => {
                        if t.len() < 2 {
                            invalid!("膜ファイル: 材料 {name} の行 {line}");
                        }
                        let k = if t.len() > 2 { num(t[2], name)? } else { 0.0 };
                        let row = [num(t[0], name)?, num(t[1], name)?, k];
                        f.mate.get_mut(name.as_str()).unwrap().push(row);
                    }
                    Cur::Coat(name) => {
                        if t.len() < 2 {
                            invalid!("膜ファイル: 膜 {name} の行 {line}");
                        }
                        let flag = if t.len() > 2 {
                            num(t[2], name)? as i64
                        } else {
                            0
                        };
                        let l = Layer {
                            mat: t[0].to_string(),
                            d: num(t[1], name)?,
                            absolute: flag == 1,
                        };
                        f.coat.get_mut(name.as_str()).unwrap().push(l);
                    }
                    Cur::Table(name, ang) => match t[0] {
                        "ANGL" if t.len() >= 2 => {
                            let a = num(t[1], name)?;
                            let v = tables.get_mut(name.as_str()).unwrap();
                            v.retain(|(b, _)| *b != a);
                            v.push((a, Vec::new()));
                            *ang = Some(a);
                        }
                        "WAVE" => {
                            let Some(a) = *ang else {
                                invalid!("膜ファイル: 表 {name} の最初の ANGL より前に WAVE");
                            };
                            // 波長 Rs Rp Ts Tp に位相 4 つが続く（位相は省略すると 0）
                            if t.len() < 6 {
                                invalid!("膜ファイル: 表 {name} の行 {line}");
                            }
                            let mut row = [0.0; 9];
                            for i in 0..(t.len() - 1).min(9) {
                                row[i] = num(t[i + 1], name)?;
                            }
                            let v = tables.get_mut(name.as_str()).unwrap();
                            v.iter_mut().find(|(b, _)| *b == a).unwrap().1.push(row);
                        }
                        _ => {}
                    },
                },
            }
        }
        for (k, mut v) in tables {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            f.table.insert(k, v);
        }
        Ok(f)
    }
}

/// 暗号化された膜の実測の係数: 波長ごと・入射角ごとの (s, p)（透過、鏡では反射）。
/// 全反射より先の角度は NaN にしておく（補間に使わない）。
#[derive(Debug, Clone)]
pub struct Measured {
    pub wavelengths: Vec<f64>,
    /// 入射角 [deg]、昇順
    pub angles_deg: Vec<f64>,
    /// [波長][角度] → (s, p)
    pub data: Vec<Vec<[C64; 2]>>,
}

/// 系の膜: 膜ファイルと実測表
#[derive(Debug, Clone, Default)]
pub struct Coatings {
    pub file: CoatingFile,
    /// 境界の鍵 → 実測表
    pub measured: HashMap<String, Measured>,
}

impl Coatings {
    /// 系の膜ファイル（zmx の COFN）。なければ空（膜なし・理想膜・実測表だけ）
    pub fn for_system(sys: &System) -> Result<Coatings> {
        let file = match &sys.coating_file {
            Some(p) => CoatingFile::load(p)?,
            None => CoatingFile::default(),
        };
        Ok(Coatings {
            file,
            measured: HashMap::new(),
        })
    }

    pub fn add_measured(&mut self, key: &str, m: Measured) -> Result<()> {
        if m.data.len() != m.wavelengths.len()
            || m.data.iter().any(|r| r.len() != m.angles_deg.len())
        {
            invalid!("実測表 {key} の大きさ");
        }
        if m.angles_deg.windows(2).any(|w| !(w[0] < w[1])) {
            invalid!("実測表 {key} の角度が昇順でない");
        }
        self.measured.insert(key.to_string(), m);
        Ok(())
    }

    /// 材料の複素屈折率（MATE の表、なければ系のガラス）
    fn mat_n(&self, sys: &System, name: &str, wl: f64) -> Result<C64> {
        if let Some(m) = self.file.mate.get(name) {
            if m.is_empty() {
                invalid!("膜の材料 {name} にデータがない");
            }
            let xs: Vec<f64> = m.iter().map(|r| r[0]).collect();
            let n: Vec<f64> = m.iter().map(|r| r[1]).collect();
            let k: Vec<f64> = m.iter().map(|r| r[2]).collect();
            return Ok(C64::new(interp(wl, &xs, &n), interp(wl, &xs, &k)));
        }
        match sys.glasses.get(name) {
            Some(g) => Ok(C64::new(
                g.index(wl, sys.temperature, sys.pressure, sys.adjust_index),
                0.0,
            )),
            None => invalid!("膜の材料 {name} が膜ファイルにもガラスカタログにもない"),
        }
    }

    /// 境界 key の波長 wl [µm] での係数を用意する（n1・n2 は前後の屈折率、鏡では n2 は使わない）
    pub fn prepare(&self, sys: &System, key: &str, wl: f64, n1: f64, n2: f64) -> Result<Iface> {
        let mut it = key.splitn(3, '|');
        let (coat, _m1, m2) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        let mirror = m2 == "MIRROR";
        let kind = if let Some(tab) = self.file.table.get(coat) {
            let angs: Vec<f64> = tab.iter().map(|a| a.0).collect();
            let mut vals: [Vec<f64>; 8] = Default::default();
            for (a, rows) in tab {
                if rows.is_empty() {
                    invalid!("膜の表 {coat} の角度 {a} に行がない");
                }
                let xs: Vec<f64> = rows.iter().map(|r| r[0]).collect();
                for c in 0..8 {
                    let ys: Vec<f64> = rows.iter().map(|r| r[c + 1]).collect();
                    vals[c].push(interp(wl, &xs, &ys));
                }
            }
            Kind::Table {
                name: coat.to_string(),
                angs,
                vals,
            }
        } else if coat == "-" {
            Kind::Fresnel {
                ns: if mirror { MIRROR_N } else { C64::new(n2, 0.0) },
            }
        } else if coat.starts_with("I.") {
            // I.xx: 透過率 .xx
            let t: f64 = coat[1..]
                .parse()
                .map_err(|_| Error::Invalid(format!("理想膜 {coat}")))?;
            let v = if mirror { -(1.0 - t).sqrt() } else { t.sqrt() };
            Kind::Ideal(C64::new(v, 0.0))
        } else if let Some(ls) = self.file.coat.get(coat) {
            if !mirror {
                notimpl!("透過する薄膜 {coat}（光線相当の位相が未確認）");
            }
            let wl_ref = sys.wavelengths[sys.primary];
            let mut layers = Vec::with_capacity(ls.len());
            for l in ls {
                let n = self.mat_n(sys, &l.mat, wl)?;
                let d = if l.absolute {
                    l.d
                } else {
                    l.d * wl_ref / self.mat_n(sys, &l.mat, wl_ref)?.re
                };
                layers.push((n, d));
            }
            // 反射の位相は誘電体層の底を基準にする（+2 k d cosθ、d は誘電体層の物理的な厚さの和。
            // 吸収のある金属層は数えない）
            let d_diel = layers.iter().filter(|l| l.0.im == 0.0).map(|l| l.1).sum();
            Kind::Stack {
                layers,
                ns: MIRROR_N,
                d_diel,
            }
        } else {
            let Some(m) = self.measured.get(key) else {
                notimpl!(
                    "膜 {coat}（境界 {key}）: 膜ファイルになく、実測表もない（暗号化された膜？）"
                );
            };
            let Some(w) = m.wavelengths.iter().position(|&v| (v - wl).abs() <= 1e-9) else {
                notimpl!(
                    "膜の実測表 {key} に波長 {wl} µm がない（表は {:?}）",
                    m.wavelengths
                );
            };
            // 屈折角の余弦のスプライン（0 付近で角度に偶、全反射の平方根の振舞いもこれに線形）。鏡では入射角の余弦
            let r = if mirror { 1.0 } else { n1 / n2 };
            let mut xs = Vec::new();
            let mut ys: [Vec<f64>; 4] = Default::default();
            for (a, v) in m.angles_deg.iter().zip(&m.data[w]) {
                if !(v[0].re.is_finite() && v[0].im.is_finite()) {
                    continue;
                }
                xs.push(xcos(r, a.to_radians()));
                ys[0].push(v[0].re);
                ys[1].push(v[0].im);
                ys[2].push(v[1].re);
                ys[3].push(v[1].im);
            }
            let sp = [0, 1, 2, 3].map(|c| Spline::not_a_knot(&xs, &ys[c]));
            let sp = match sp {
                [Some(a), Some(b), Some(c), Some(d)] => [a, b, c, d],
                _ => invalid!("膜の実測表 {key}: 補間できる点が足りない"),
            };
            Kind::Measured {
                key: key.to_string(),
                r,
                max_deg: *m.angles_deg.last().unwrap(),
                sp: Box::new(sp),
            }
        };
        Ok(Iface {
            kind,
            mirror,
            n1,
            wl,
        })
    }
}

/// 実測表の横軸: −cos(屈折角)（増加）
fn xcos(r: f64, th: f64) -> f64 {
    let s = r * th.sin();
    -(1.0 - s * s).max(0.0).sqrt()
}

/// 線形補間（xs は昇順。範囲外は端の値、numpy の interp と同じ）
pub fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    let n = xs.len();
    if x.is_nan() {
        return f64::NAN;
    }
    if n == 1 || x <= xs[0] {
        return ys[0];
    }
    if x >= xs[n - 1] {
        return ys[n - 1];
    }
    let i = xs.partition_point(|&v| v <= x) - 1;
    let t = (x - xs[i]) / (xs[i + 1] - xs[i]);
    ys[i] + t * (ys[i + 1] - ys[i])
}

/// 3 次スプライン（両端 not-a-knot、範囲外は端の多項式で外挿。scipy の CubicSpline と同じ）
#[derive(Debug, Clone)]
pub struct Spline {
    x: Vec<f64>,
    /// 区間ごとの係数（t = x − x_i の 3, 2, 1, 0 次）
    c: Vec<[f64; 4]>,
}

impl Spline {
    pub fn not_a_knot(x: &[f64], y: &[f64]) -> Option<Spline> {
        let n = x.len();
        if n < 2 || y.len() != n || x.windows(2).any(|w| !(w[0] < w[1])) {
            return None;
        }
        let dx: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
        let sl: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / dx[i]).collect();
        // 各点の傾き s
        let s: Vec<f64> = if n == 2 {
            vec![sl[0], sl[0]]
        } else if n == 3 {
            // 放物線
            let a = (sl[1] - sl[0]) / (x[2] - x[0]);
            (0..3)
                .map(|i| sl[0] + a * (2.0 * x[i] - x[0] - x[1]))
                .collect()
        } else {
            // 三重対角の連立（行 0 と n−1 が not-a-knot）
            let mut sub = vec![0.0; n];
            let mut dia = vec![0.0; n];
            let mut sup = vec![0.0; n];
            let mut b = vec![0.0; n];
            for i in 1..n - 1 {
                sub[i] = dx[i];
                dia[i] = 2.0 * (dx[i - 1] + dx[i]);
                sup[i] = dx[i - 1];
                b[i] = 3.0 * (dx[i] * sl[i - 1] + dx[i - 1] * sl[i]);
            }
            let d0 = x[2] - x[0];
            dia[0] = dx[1];
            sup[0] = d0;
            b[0] = ((dx[0] + 2.0 * d0) * dx[1] * sl[0] + dx[0] * dx[0] * sl[1]) / d0;
            let d1 = x[n - 1] - x[n - 3];
            dia[n - 1] = dx[n - 3];
            sub[n - 1] = d1;
            b[n - 1] = (dx[n - 2] * dx[n - 2] * sl[n - 3]
                + (2.0 * d1 + dx[n - 2]) * dx[n - 3] * sl[n - 2])
                / d1;
            solve_tridiag(&sub, &dia, &sup, &b)?
        };
        let c = (0..n - 1)
            .map(|i| {
                let t = (s[i] + s[i + 1] - 2.0 * sl[i]) / (dx[i] * dx[i]);
                let q = (3.0 * sl[i] - 2.0 * s[i] - s[i + 1]) / dx[i];
                [t, q, s[i], y[i]]
            })
            .collect();
        Some(Spline { x: x.to_vec(), c })
    }

    pub fn eval(&self, v: f64) -> f64 {
        let n = self.x.len();
        let i = if v.is_nan() {
            return f64::NAN;
        } else {
            self.x[1..n - 1].partition_point(|&a| a <= v)
        };
        let t = v - self.x[i];
        let [a, b, c, d] = self.c[i];
        ((a * t + b) * t + c) * t + d
    }
}

/// 三重対角の連立方程式（部分ピボット付きの消去。行 0 と n−1 の対角が小さくてもよいように）
fn solve_tridiag(sub: &[f64], dia: &[f64], sup: &[f64], b: &[f64]) -> Option<Vec<f64>> {
    // 一般の帯行列として小さな密行列で解く（点数は 100 程度まで）
    let n = dia.len();
    let mut a = vec![vec![0.0; n + 1]; n];
    for i in 0..n {
        a[i][i] = dia[i];
        if i > 0 {
            a[i][i - 1] = sub[i];
        }
        if i + 1 < n {
            a[i][i + 1] = sup[i];
        }
        a[i][n] = b[i];
    }
    // 行 0 は (dia0, sup0)、行 n−1 は (sub, dia) の 2 要素。ピボット付きガウス消去
    for col in 0..n {
        let lim = (col + 3).min(n);
        let p = (col..lim).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[p][col] == 0.0 {
            return None;
        }
        a.swap(col, p);
        for r in col + 1..lim {
            let f = a[r][col] / a[col][col];
            if f != 0.0 {
                for c in col..=n {
                    a[r][c] -= f * a[col][c];
                }
            }
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut v = a[i][n];
        for c in i + 1..n {
            v -= a[i][c] * x[c];
        }
        x[i] = v / a[i][i];
    }
    Some(x)
}

#[derive(Debug, Clone)]
enum Kind {
    Fresnel {
        ns: C64,
    },
    Ideal(C64),
    Stack {
        layers: Vec<(C64, f64)>,
        ns: C64,
        d_diel: f64,
    },
    Table {
        name: String,
        angs: Vec<f64>,
        /// 量ごと（Rs Rp Ts Tp と位相）の角度に対する値
        vals: [Vec<f64>; 8],
    },
    Measured {
        key: String,
        r: f64,
        max_deg: f64,
        sp: Box<[Spline; 4]>,
    },
}

/// ある波長で用意した境界の係数
#[derive(Debug, Clone)]
pub struct Iface {
    kind: Kind,
    mirror: bool,
    n1: f64,
    wl: f64,
}

impl Iface {
    /// 入射角 theta [rad] での (s, p): 透過、鏡では反射
    pub fn eval(&self, theta: f64) -> Result<[C64; 2]> {
        let mirror = self.mirror;
        Ok(match &self.kind {
            Kind::Ideal(v) => [*v, *v],
            Kind::Fresnel { ns } => {
                let (rs, rp, ts, tp) = stack(self.n1, &[], *ns, self.wl, theta);
                if mirror {
                    [rs, rp]
                } else {
                    [ts, tp]
                }
            }
            Kind::Stack { layers, ns, d_diel } => {
                let (rs, rp, _, _) = stack(self.n1, layers, *ns, self.wl, theta);
                let ph = C64::new(0.0, 2.0 * 2.0 * PI / self.wl * d_diel * theta.cos()).exp();
                [rs * ph, rp * ph]
            }
            Kind::Table { name, angs, vals } => {
                let deg = theta.to_degrees();
                let (lo, hi) = (angs[0], *angs.last().unwrap());
                if deg.is_finite() && (deg > hi + 1e-9 || deg < lo - 1e-9) {
                    notimpl!("膜の表 {name}: 入射角 {deg:.4} deg が表の範囲 {lo}〜{hi} の外");
                }
                let v = |k: usize| interp(deg, angs, &vals[k]);
                let (a, b) = if mirror { (0, 4) } else { (2, 6) };
                [
                    C64::from_polar(v(a).sqrt(), v(b).to_radians()),
                    C64::from_polar(v(a + 1).sqrt(), v(b + 1).to_radians()),
                ]
            }
            Kind::Measured {
                key,
                r,
                max_deg,
                sp,
            } => {
                let deg = theta.to_degrees();
                if deg.is_finite() && deg > max_deg + 1e-12_f64.to_degrees() {
                    notimpl!(
                        "膜 {key}: 入射角 {deg:.4} deg が実測表の範囲（0〜{max_deg} deg）の外"
                    );
                }
                let x = xcos(*r, theta);
                [
                    C64::new(sp[0].eval(x), sp[1].eval(x)),
                    C64::new(sp[2].eval(x), sp[3].eval(x)),
                ]
            }
        })
    }
}

/// 複素屈折率 n の媒質での角の余弦（n0 sinθ0 = n sinθ、Im(n cos) ≤ 0 の枝）
fn ccos(n0: f64, s0: f64, n: C64) -> C64 {
    let q = C64::new(n0 * s0, 0.0) / n;
    let c = (C64::new(1.0, 0.0) - q * q).sqrt();
    if (n * c).im > 0.0 {
        -c
    } else {
        c
    }
}

/// 媒質 n0（実数）から層 [(N, d [µm])] を経て基板 ns へ、n0 での入射角 theta の薄膜
/// → (r_s, r_p, t_s, t_p)、パワー規格化、OpticStudio の約束
pub fn stack(n0: f64, layers: &[(C64, f64)], ns: C64, wl: f64, theta: f64) -> (C64, C64, C64, C64) {
    let (s0, c0) = theta.sin_cos();
    let cs = ccos(n0, s0, ns);
    let one = C64::new(1.0, 0.0);
    let i = C64::new(0.0, 1.0);
    let mut out = [(one, one); 2];
    for (pi, o) in out.iter_mut().enumerate() {
        let eta = |n: C64, c: C64| if pi == 0 { n * c } else { n / c };
        let e0 = eta(C64::new(n0, 0.0), C64::new(c0, 0.0));
        let es = eta(ns, cs);
        let mut m = [[one, C64::new(0.0, 0.0)], [C64::new(0.0, 0.0), one]];
        for &(n, d) in layers {
            let c = ccos(n0, s0, n);
            let e = eta(n, c);
            let dl = n * c * (2.0 * PI * d / wl);
            let (cd, sd) = (dl.cos(), dl.sin());
            let l = [[cd, i * sd / e], [i * e * sd, cd]];
            m = [
                [
                    m[0][0] * l[0][0] + m[0][1] * l[1][0],
                    m[0][0] * l[0][1] + m[0][1] * l[1][1],
                ],
                [
                    m[1][0] * l[0][0] + m[1][1] * l[1][0],
                    m[1][0] * l[0][1] + m[1][1] * l[1][1],
                ],
            ];
        }
        let b = m[0][0] + m[0][1] * es;
        let c = m[1][0] + m[1][1] * es;
        let r = (e0 * b - c) / (e0 * b + c);
        let t = e0 * 2.0 / (e0 * b + c) * C64::new(es.re / e0.re, 0.0).sqrt();
        *o = (r, t);
    }
    let [(rs, ts), (rp, tp)] = out;
    (rs, rp, ts, tp)
}

/// 面ごとの境界の鍵（`膜|前の媒質|後の媒質（鏡は MIRROR）`、境界でなければ None）と、各面の前の媒質の名前
pub fn media_keys(sys: &System) -> (Vec<Option<String>>, Vec<String>) {
    let sf = &sys.surfaces;
    let mut keys = vec![None; sf.len()];
    let mut before = vec!["AIR".to_string(); sf.len()];
    let mut prev = "AIR".to_string();
    let mut nprev = sf[0].n[0];
    for (k, d) in sf.iter().enumerate() {
        before[k] = prev.clone();
        if d.ignore || d.typ == SurfType::CoordBrk {
            continue;
        }
        let after = if d.mirror {
            prev.clone()
        } else if d.mat.is_empty() {
            "AIR".to_string()
        } else {
            d.mat.clone()
        };
        if k > 0 && (!d.coat.is_empty() || d.mirror || (d.n[0] - nprev).abs() > 1e-12) {
            let c = if d.coat.is_empty() { "-" } else { &d.coat };
            let a = if d.mirror { "MIRROR" } else { &after };
            keys[k] = Some(format!("{c}|{prev}|{a}"));
        }
        if !d.mirror {
            prev = after;
        }
        nprev = d.n[0];
    }
    (keys, before)
}

/// 媒質の内部透過率（厚さ 1 mm あたりの強度。IT データがない・波長が範囲外なら None）
pub fn internal_t(g: &glass::Glass, wl: f64) -> Option<f64> {
    if g.it.is_empty() {
        return None;
    }
    let mut it = g.it.clone();
    it.sort_by(|a, b| a[0].total_cmp(&b[0]));
    if !(it[0][0] <= wl && wl <= it[it.len() - 1][0]) {
        return None;
    }
    let xs: Vec<f64> = it.iter().map(|r| r[0]).collect();
    let t: Vec<f64> = it.iter().map(|r| r[1]).collect();
    let th: Vec<f64> = it.iter().map(|r| r[2]).collect();
    Some(interp(wl, &xs, &t).powf(1.0 / interp(wl, &xs, &th)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresnel_normal_incidence() {
        let (rs, rp, ts, tp) = stack(1.0, &[], C64::new(1.5, 0.0), 0.55, 0.0);
        assert!((rs.re + 0.2).abs() < 1e-15 && (rp.re + 0.2).abs() < 1e-15);
        assert!((ts.norm_sqr() + rs.norm_sqr() - 1.0).abs() < 1e-14);
        assert!((tp.norm_sqr() + rp.norm_sqr() - 1.0).abs() < 1e-14);
    }

    #[test]
    fn energy_is_conserved_off_axis() {
        for deg in [10.0f64, 30.0, 55.0] {
            let th = deg.to_radians();
            let (rs, rp, ts, tp) = stack(1.0, &[], C64::new(1.7, 0.0), 0.55, th);
            assert!((ts.norm_sqr() + rs.norm_sqr() - 1.0).abs() < 1e-13);
            assert!((tp.norm_sqr() + rp.norm_sqr() - 1.0).abs() < 1e-13);
        }
        // ブルースター角で p の反射が消える
        let th = 1.7f64.atan();
        let (_, rp, _, _) = stack(1.0, &[], C64::new(1.7, 0.0), 0.55, th);
        assert!(rp.norm() < 1e-14);
    }

    #[test]
    fn quarter_wave_ar() {
        // n = √1.5 の 1/4 波長膜は垂直入射で反射ゼロ
        let n = 1.5f64.sqrt();
        let d = 0.55 / 4.0 / n;
        let (rs, _, ts, _) = stack(1.0, &[(C64::new(n, 0.0), d)], C64::new(1.5, 0.0), 0.55, 0.0);
        assert!(rs.norm() < 1e-14);
        assert!((ts.norm_sqr() - 1.0).abs() < 1e-14);
    }

    #[test]
    fn spline_matches_cubic_exactly() {
        // not-a-knot は 3 次多項式を正確に再現する
        let f = |x: f64| 0.3 * x * x * x - x * x + 2.0 * x - 0.5;
        let x: Vec<f64> = (0..9)
            .map(|i| -1.0 + 0.13 * i as f64 + 0.01 * (i * i) as f64)
            .collect();
        let y: Vec<f64> = x.iter().map(|&v| f(v)).collect();
        let s = Spline::not_a_knot(&x, &y).unwrap();
        for v in [-1.2, -0.95, -0.3, 0.1, 0.7, 1.3] {
            assert!((s.eval(v) - f(v)).abs() < 1e-12, "{v}");
        }
    }

    #[test]
    fn parse_file() {
        let txt = "! c\nMATE N15\n0.55 1.5 0\n\nCOAT AR\nN15 0.25\nN15 0.1 1\n\nTABLE T1\nANGL 0\n\
                   WAVE 0.5 0.1 0.1 0.9 0.9 0 0 0 0\nANGL 45\nWAVE 0.5 0.2 0.3 0.8 0.7 0 0 10 20\n\
                   ENCRYPTED X\n";
        let f = CoatingFile::parse(txt).unwrap();
        assert_eq!(f.mate["N15"], vec![[0.55, 1.5, 0.0]]);
        assert_eq!(f.coat["AR"].len(), 2);
        assert!(f.coat["AR"][1].absolute && !f.coat["AR"][0].absolute);
        assert_eq!(f.table["T1"].len(), 2);
        assert!(CoatingFile::parse("TABLE T\nWAVE 1 2 3 4 5 6 7 8 9\n").is_err());
        assert!(CoatingFile::parse("TABLE T\nANGL 0\nWAVE 1 2 3 4\n").is_err());
        let f = CoatingFile::parse("TABLE T\nANGL 0\nWAVE 1 0.1 0.1 0.9 0.9\n").unwrap();
        assert_eq!(
            f.table["T"][0].1[0],
            [1.0, 0.1, 0.1, 0.9, 0.9, 0.0, 0.0, 0.0, 0.0]
        );
    }
}
