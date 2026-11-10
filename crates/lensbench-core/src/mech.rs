//! 機構部品（レンズチューブ・ケージ板・マウントなど）の穴による光線のケラレ。
//!
//! 部品は、系の面 `surface` の局所座標から `offset` だけずらし、x → y の順に `tilt` だけ傾けた座標に置く
//! （光学系の位置は変えない）。部品の穴（[`Bore`]）は部品の z 軸のまわりの円柱で、z の範囲 `z0`〜`z1` の中で
//! 半径 `radius` の外に出た光線を切る。光線は面 1 から（物体面から面 1 までの区間は見ない）。
//!
//! 光線の面と面の間は直線なので、穴の座標で z の範囲に切り取った区間の端で半径を見ればよい
//! （直線の上の x² + y² は助変数の 2 次の凸関数で、区間の最大は端）。
use crate::error::{Error, Result};
use crate::math::{add, col, dot, mm, mtv, mv, norm, rot, sub, M3, V3};
use crate::system::{FieldSel, System};

/// 部品の穴（円柱）。z は部品の座標 [mm]
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Bore {
    pub z0: f64,
    pub z1: f64,
    /// 半径 [mm]
    pub radius: f64,
}

/// 系に置いた機構部品
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MechPart {
    pub name: String,
    /// 置く基準の面（その面の局所座標）
    pub surface: usize,
    /// 基準の面の局所座標でのずれ [dx, dy, dz] [mm]
    #[cfg_attr(feature = "serde", serde(default))]
    pub offset: [f64; 3],
    /// 傾き [tx, ty] [deg]（ずらした後、x → y の順）
    #[cfg_attr(feature = "serde", serde(default))]
    pub tilt: [f64; 2],
    pub bores: Vec<Bore>,
    /// 外形（描画・重なりの確認用）: 部品の z の範囲と外径の半径。0 なら穴から決める
    #[cfg_attr(feature = "serde", serde(default))]
    pub length: f64,
    #[cfg_attr(feature = "serde", serde(default))]
    pub outer_radius: f64,
    /// 素子を保持する内壁の半径（保持径の半分）。0 なら穴の半径が内壁
    #[cfg_attr(feature = "serde", serde(default))]
    pub hold_radius: f64,
    /// ケージのロッドの間隔 [mm]（0 はケージ板でない）
    #[cfg_attr(feature = "serde", serde(default))]
    pub rod_spacing: f64,
    /// 外形が角（`outer_radius` は正方形の一辺の半分、辺は部品の x・y 軸に沿う）。false は丸
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "std::ops::Not::not")
    )]
    pub square: bool,
    /// 保持する素子の面（系の面の番号）。はみ出し・保持径の確認に使う
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub holds: Vec<usize>,
}

impl MechPart {
    /// 部品の座標系（大域座標の原点と、列が軸の回転行列）
    pub fn frame(&self, s: &System) -> Result<(V3, M3)> {
        let fr = s.frames();
        let Some(f) = fr.surf.get(self.surface).copied().flatten() else {
            return Err(Error::Invalid(format!(
                "機構 {}: 面 {} の座標がない（物体面・無視する面・範囲外）",
                self.name, self.surface
            )));
        };
        let o = add(f.o, mv(&f.r, self.offset));
        let r = mm(&mm(&f.r, &rot(0, self.tilt[0])), &rot(1, self.tilt[1]));
        Ok((o, r))
    }

    /// 部品の z の範囲（外形の長さ、なければ穴の範囲）
    pub fn z_range(&self) -> [f64; 2] {
        if self.length > 0.0 {
            return [0.0, self.length];
        }
        let lo = self
            .bores
            .iter()
            .map(|b| b.z0.min(b.z1))
            .fold(f64::INFINITY, f64::min);
        let hi = self
            .bores
            .iter()
            .map(|b| b.z0.max(b.z1))
            .fold(f64::NEG_INFINITY, f64::max);
        if lo.is_finite() {
            [lo, hi]
        } else {
            [0.0, 0.0]
        }
    }
}

impl MechPart {
    /// 外形の半径（`outer_radius`、0 なら一番太い穴・保持径の半径 + 1.5 mm）
    pub fn outer(&self) -> f64 {
        if self.outer_radius > 0.0 {
            return self.outer_radius;
        }
        self.bores
            .iter()
            .map(|b| b.radius)
            .fold(self.hold_radius, f64::max)
            + 1.5
    }

    /// 断面の輪 [z0, z1, 内側の半径, 外側の半径]（部品の座標、z の小さい順）。z の範囲を穴の端で区切り、
    /// 区間の内側は覆う穴の一番太い半径（保持径があればそれと太い方）、穴のない区間は内側 0（中の詰まった円柱）。
    /// 長さ 0 の部品は空
    pub fn profile(&self) -> Vec<[f64; 4]> {
        let [lo, hi] = self.z_range();
        if hi - lo <= 0.0 {
            return Vec::new();
        }
        let mut zs = vec![lo, hi];
        for b in &self.bores {
            for z in [b.z0, b.z1] {
                if z > lo && z < hi {
                    zs.push(z);
                }
            }
        }
        zs.sort_by(|a, b| a.total_cmp(b));
        zs.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        let ro = self.outer();
        zs.windows(2)
            .map(|w| {
                let m = 0.5 * (w[0] + w[1]);
                let rb = self
                    .bores
                    .iter()
                    .filter(|b| b.z0.min(b.z1) <= m && m <= b.z0.max(b.z1))
                    .map(|b| b.radius)
                    .fold(f64::NAN, f64::max);
                let ri = if rb.is_nan() {
                    0.0
                } else {
                    rb.max(self.hold_radius)
                };
                [w[0], w[1], ri.min(ro), ro]
            })
            .collect()
    }

    /// 部品の座標の点が部品の壁（輪の内側の半径〜外側の半径）の中にあるか。`eps` だけ内側に縮めて見る
    pub fn inside_wall(&self, p: V3, eps: f64) -> bool {
        let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
        // 外側の境: 角なら正方形（|x|・|y| の大きい方）
        let ro = if self.square {
            p[0].abs().max(p[1].abs())
        } else {
            r
        };
        self.profile()
            .iter()
            .any(|q| p[2] > q[0] + eps && p[2] < q[1] - eps && r > q[2] + eps && ro < q[3] - eps)
    }
}

/// 機構の配置の確認の結果
#[derive(Debug, Clone, PartialEq)]
pub struct FitIssue {
    /// "collision"（素子の縁が部品の壁に当たる）・"protrude"（保持する素子が部品の外にはみ出す）・
    /// "loose"（保持径より素子が細い、注意）・"overlap"（部品どうしが重なる）・"rod"（ケージのロッドの長さ、注意）・
    /// "rod_mismatch"（同じ軸のケージ板でロッドの間隔が違う）
    pub kind: &'static str,
    /// 誤り（false は注意）
    pub error: bool,
    pub part: usize,
    /// 相手の部品（overlap・rod・rod_mismatch）
    pub other: Option<usize>,
    /// 素子の面（collision・protrude・loose）
    pub surface: Option<usize>,
    /// 量 [mm]: 当たる・はみ出す・すき間の大きさ、ロッドの要る長さ
    pub value: f64,
    pub message: String,
}

/// 素子の面（ガラス・鏡に面する、半径のある面）の、縁の上の点（大域座標）。物体面・像面は除く
fn element_rims(s: &System, n: usize) -> Vec<(usize, f64, Vec<V3>)> {
    let fr = s.frames();
    let img = s.image();
    let glass = |m: &str| !m.is_empty() && m != "-";
    let mut out = Vec::new();
    let mut prev_glass = false;
    for j in 1..img {
        let d = &s.surfaces[j];
        if d.ignore || d.typ == crate::surface::SurfType::CoordBrk {
            continue;
        }
        let g = glass(&d.mat);
        let solid = g || prev_glass || d.mirror;
        prev_glass = g;
        let Some(f) = fr.surf.get(j).copied().flatten() else {
            continue;
        };
        if !solid || d.semi <= 0.0 {
            continue;
        }
        let pts = (0..n)
            .filter_map(|k| {
                let a = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
                let (x, y) = (d.semi * a.cos(), d.semi * a.sin());
                let z = crate::freeform::sag_xy(d, x, y).0;
                z.is_finite().then(|| add(f.o, mv(&f.r, [x, y, z])))
            })
            .collect();
        out.push((j, d.semi, pts));
    }
    out
}

/// 置いた機構部品の確認: 素子と部品の壁の当たり、保持する素子のはみ出し・保持径、部品どうしの重なり、
/// ケージのロッドの長さ。素子は面の縁（半径 semi の円）の点で、部品は壁の面の上の点で見る
pub fn fit_check(s: &System, parts: &[MechPart]) -> Result<Vec<FitIssue>> {
    const TOL: f64 = 1e-6;
    let frames: Vec<(V3, M3)> = parts.iter().map(|p| p.frame(s)).collect::<Result<_>>()?;
    let local = |k: usize, p: V3| mtv(&frames[k].1, sub(p, frames[k].0));
    let rims = element_rims(s, 36);
    let mut out = Vec::new();
    for (k, part) in parts.iter().enumerate() {
        let prof = part.profile();
        let [lo, hi] = part.z_range();
        for (j, semi, pts) in &rims {
            let held = part.holds.contains(j);
            // 当たり: 部品の z の範囲の中で、縁が壁の内側の半径より外
            let mut worst = 0.0f64;
            for &g in pts {
                let q = local(k, g);
                let r = (q[0] * q[0] + q[1] * q[1]).sqrt();
                for ring in &prof {
                    if q[2] > ring[0] + TOL && q[2] < ring[1] - TOL && r > ring[2] + TOL {
                        worst = worst.max(r.min(ring[3]) - ring[2]);
                    }
                }
            }
            if worst > 0.0 {
                out.push(FitIssue {
                    kind: "collision",
                    error: true,
                    part: k,
                    other: None,
                    surface: Some(*j),
                    value: worst,
                    message: format!(
                        "面 {j} の縁（半径 {semi:.3}）が {} の壁に {worst:.3} mm 当たる",
                        part.name
                    ),
                });
            }
            if !held {
                continue;
            }
            // はみ出し: 保持する素子の縁・頂点が部品の z の範囲の外
            let fr = s.frames();
            let mut zs: Vec<f64> = pts.iter().map(|&g| local(k, g)[2]).collect();
            if let Some(f) = fr.surf.get(*j).copied().flatten() {
                zs.push(local(k, f.o)[2]);
            }
            let out_by = zs
                .iter()
                .map(|&z| (lo - z).max(z - hi))
                .fold(0.0f64, f64::max);
            if out_by > TOL {
                out.push(FitIssue {
                    kind: "protrude",
                    error: true,
                    part: k,
                    other: None,
                    surface: Some(*j),
                    value: out_by,
                    message: format!(
                        "保持する面 {j} が {} の外に {out_by:.3} mm はみ出す（部品の長さ {:.3}）",
                        part.name,
                        hi - lo
                    ),
                });
            }
            if part.hold_radius > 0.0 && part.hold_radius - semi > 0.25 {
                let gap = part.hold_radius - semi;
                out.push(FitIssue {
                    kind: "loose",
                    error: false,
                    part: k,
                    other: None,
                    surface: Some(*j),
                    value: gap,
                    message: format!(
                        "面 {j}（直径 {:.3}）は {} の保持径 {:.3} より細い（アダプタが要る）",
                        2.0 * semi,
                        part.name,
                        2.0 * part.hold_radius
                    ),
                });
            }
        }
    }
    // 部品どうしの重なり: 一方の壁の面の上の点（少し内側）が他方の壁の中
    let samples: Vec<Vec<V3>> = parts
        .iter()
        .enumerate()
        .map(|(k, p)| {
            wall_points(p, 36)
                .into_iter()
                .map(|q| to_global(&frames[k], q))
                .collect()
        })
        .collect();
    for a in 0..parts.len() {
        for b in a + 1..parts.len() {
            let hit = samples[a]
                .iter()
                .any(|&g| parts[b].inside_wall(local(b, g), TOL))
                || samples[b]
                    .iter()
                    .any(|&g| parts[a].inside_wall(local(a, g), TOL));
            if hit {
                out.push(FitIssue {
                    kind: "overlap",
                    error: true,
                    part: a,
                    other: Some(b),
                    surface: None,
                    value: 0.0,
                    message: format!("{} と {} が重なる", parts[a].name, parts[b].name),
                });
            }
        }
    }
    // ケージ: 同じ軸のケージ板の組ごとに、ロッドの間隔と要る長さ
    let cages: Vec<usize> = (0..parts.len())
        .filter(|&k| parts[k].rod_spacing > 0.0)
        .collect();
    let mut used = vec![false; parts.len()];
    for &a in &cages {
        if used[a] {
            continue;
        }
        let (oa, ma) = frames[a];
        let za = col(&ma, 2);
        let group: Vec<usize> = cages
            .iter()
            .copied()
            .filter(|&b| {
                let (ob, mb) = frames[b];
                let d = sub(ob, oa);
                let lat = sub(d, crate::math::scale(za, dot(d, za)));
                dot(col(&mb, 2), za).abs() > 1.0 - 1e-9 && norm(lat) < 1e-6
            })
            .collect();
        for &b in &group {
            used[b] = true;
        }
        if group.len() < 2 {
            continue;
        }
        let (mut zmin, mut zmax) = (f64::INFINITY, f64::NEG_INFINITY);
        for &b in &group {
            let [l, h] = parts[b].z_range();
            for z in [l, h] {
                let g = to_global(&frames[b], [0.0, 0.0, z]);
                let t = dot(sub(g, oa), za);
                zmin = zmin.min(t);
                zmax = zmax.max(t);
            }
        }
        let last = *group.last().unwrap();
        for &b in &group[1..] {
            if (parts[b].rod_spacing - parts[a].rod_spacing).abs() > 1e-9 {
                out.push(FitIssue {
                    kind: "rod_mismatch",
                    error: true,
                    part: a,
                    other: Some(b),
                    surface: None,
                    value: (parts[b].rod_spacing - parts[a].rod_spacing).abs(),
                    message: format!(
                        "{} と {} のロッドの間隔が違う（{:.3} と {:.3}）",
                        parts[a].name, parts[b].name, parts[a].rod_spacing, parts[b].rod_spacing
                    ),
                });
            }
        }
        out.push(FitIssue {
            kind: "rod",
            error: false,
            part: a,
            other: Some(last),
            surface: None,
            value: zmax - zmin,
            message: format!(
                "ケージ（{}）のロッドは {:.3} mm 以上（間隔 {:.3}）",
                group
                    .iter()
                    .map(|&b| parts[b].name.as_str())
                    .collect::<Vec<_>>()
                    .join("・"),
                zmax - zmin,
                parts[a].rod_spacing
            ),
        });
    }
    Ok(out)
}

/// 部品の壁の面の上の点（部品の座標）: 輪ごとに内外の円柱の面と両端の輪の面
fn wall_points(p: &MechPart, n: usize) -> Vec<V3> {
    let mut v = Vec::new();
    for q in p.profile() {
        let zs = [q[0], 0.5 * (q[0] + q[1]), q[1]];
        let rs = [q[2], 0.5 * (q[2] + q[3]), q[3]];
        for k in 0..n {
            let a = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
            let (c, s) = (a.cos(), a.sin());
            for &z in &zs {
                for &r in &rs {
                    v.push([r * c, r * s, z]);
                }
            }
        }
    }
    v
}

/// 部品の座標の 2 点 a・b を結ぶ線分が穴の外に出るか
pub fn segment_clipped(a: V3, b: V3, bore: &Bore) -> bool {
    let (z0, z1) = (bore.z0.min(bore.z1), bore.z0.max(bore.z1));
    let dz = b[2] - a[2];
    // z の範囲に入る助変数の区間 [t0, t1]
    let (t0, t1) = if dz.abs() < 1e-300 {
        if a[2] < z0 || a[2] > z1 {
            return false;
        }
        (0.0, 1.0)
    } else {
        let (ta, tb) = ((z0 - a[2]) / dz, (z1 - a[2]) / dz);
        let (lo, hi) = (ta.min(tb).max(0.0), ta.max(tb).min(1.0));
        if lo > hi {
            return false;
        }
        (lo, hi)
    };
    let r2 = bore.radius * bore.radius;
    [t0, t1].iter().any(|&t| {
        let x = a[0] + t * (b[0] - a[0]);
        let y = a[1] + t * (b[1] - a[1]);
        x * x + y * y > r2 * (1.0 + 1e-12)
    })
}

/// 視野・波長ごとの集計
#[derive(Debug, Clone, PartialEq)]
pub struct MechVig {
    pub field: usize,
    pub wave: usize,
    /// 追った光線の数（瞳の点）
    pub launched: usize,
    /// 追跡が止まった光線（全反射・当たらない）
    pub failed: usize,
    /// 系の開口で切れた光線（どの部品より先に）
    pub optical: usize,
    /// 部品ごとの、その部品の穴の外に出た光線の数（ほかの部品・開口で先に切れたものも数える）
    pub clipped: Vec<usize>,
    /// 部品ごとの、最初にその部品で切れた光線の数（光の通る順で最初。系の開口より先のものだけ）
    pub first: Vec<usize>,
    /// どこでも切れずに像面まで届いた光線
    pub passed: usize,
}

/// 部品 `parts` のケラレを、視野 `fields`・波長 `waves` の瞳の点 (px, py) の光線で数える
pub fn vignetting(
    s: &System,
    parts: &[MechPart],
    fields: &[usize],
    waves: &[usize],
    px: &[f64],
    py: &[f64],
) -> Result<Vec<MechVig>> {
    let img = s.image();
    let frames: Vec<(V3, M3)> = parts.iter().map(|p| p.frame(s)).collect::<Result<_>>()?;
    let mut out = Vec::new();
    for &fi in fields {
        for &w in waves {
            let t = s.try_trace(FieldSel::Index(fi), px, py, Some(w), Some(img), true)?;
            let mut v = MechVig {
                field: fi,
                wave: w,
                launched: px.len(),
                failed: 0,
                optical: 0,
                clipped: vec![0; parts.len()],
                first: vec![0; parts.len()],
                passed: 0,
            };
            for (i, r) in t.rays.iter().enumerate() {
                if r.err != 0 {
                    v.failed += 1;
                    continue;
                }
                let last = if r.vig != 0 { r.vig } else { img };
                // 光の通る順の大域座標の点（無視する面は記録がない）
                let pts: Vec<(usize, V3)> = (1..=last)
                    .filter_map(|j| t.rec_at(j).map(|q| (j, q.entry(i).p)))
                    .collect();
                // 部品ごとの、最初に切れた区間（終わりの面の番号）
                let mut hit: Vec<Option<usize>> = vec![None; parts.len()];
                for (k, (p, (o, m))) in parts.iter().zip(&frames).enumerate() {
                    for seg in pts.windows(2) {
                        let a = mtv(m, sub(seg[0].1, *o));
                        let b = mtv(m, sub(seg[1].1, *o));
                        if p.bores.iter().any(|bo| segment_clipped(a, b, bo)) {
                            hit[k] = Some(seg[1].0);
                            break;
                        }
                    }
                }
                for (k, h) in hit.iter().enumerate() {
                    if h.is_some() {
                        v.clipped[k] += 1;
                    }
                }
                // 最初に切った部品: 区間の番号が一番小さいもの（同じ区間なら、区間の中で先に切ったもの）
                let first = first_clip(&pts, parts, &frames, &hit);
                // 区間は開口で切れた面までなので、部品で切れたなら開口より先
                match (first, r.vig != 0) {
                    (Some((k, _)), _) => v.first[k] += 1,
                    (None, true) => v.optical += 1,
                    (None, false) => v.passed += 1,
                }
            }
            out.push(v);
        }
    }
    Ok(out)
}

/// 最初に切った部品と、その区間の終わりの面
fn first_clip(
    pts: &[(usize, V3)],
    parts: &[MechPart],
    frames: &[(V3, M3)],
    hit: &[Option<usize>],
) -> Option<(usize, usize)> {
    let j = hit.iter().flatten().min().copied()?;
    let cands: Vec<usize> = (0..parts.len()).filter(|&k| hit[k] == Some(j)).collect();
    if cands.len() == 1 {
        return Some((cands[0], j));
    }
    // 同じ区間で複数: 区間を細かく刻んで、先に穴の外に出る部品
    let s = pts.iter().position(|x| x.0 == j)?;
    let (a, b) = (pts[s - 1].1, pts[s].1);
    let n = 256;
    for q in 0..n {
        let (t0, t1) = (q as f64 / n as f64, (q + 1) as f64 / n as f64);
        let p0 = add(a, crate::math::scale(sub(b, a), t0));
        let p1 = add(a, crate::math::scale(sub(b, a), t1));
        for &k in &cands {
            let (o, m) = frames[k];
            let (u, v) = (mtv(&m, sub(p0, o)), mtv(&m, sub(p1, o)));
            if parts[k].bores.iter().any(|bo| segment_clipped(u, v, bo)) {
                return Some((k, j));
            }
        }
    }
    Some((cands[0], j))
}

/// 瞳の正方格子（一辺 `n` 本、-1〜1）のうち単位円の中の点 (px, py)。`n` < 2 なら空
pub fn pupil_grid(n: usize) -> (Vec<f64>, Vec<f64>) {
    let (mut px, mut py) = (Vec::new(), Vec::new());
    if n < 2 {
        return (px, py);
    }
    for iy in 0..n {
        for ix in 0..n {
            let x = -1.0 + 2.0 * ix as f64 / (n - 1) as f64;
            let y = -1.0 + 2.0 * iy as f64 / (n - 1) as f64;
            if x * x + y * y <= 1.0 + 1e-12 {
                px.push(x);
                py.push(y);
            }
        }
    }
    (px, py)
}

/// 部品の座標の点を大域座標へ
pub fn to_global(frame: &(V3, M3), p: V3) -> V3 {
    add(frame.0, mv(&frame.1, p))
}

/// 部品の z 軸の向き（大域座標）
pub fn axis(frame: &(V3, M3)) -> V3 {
    col(&frame.1, 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_clipped_checks_the_ends_inside_the_range() {
        let b = Bore {
            z0: 0.0,
            z1: 10.0,
            radius: 5.0,
        };
        // 範囲の中で半径 4 → 通る
        assert!(!segment_clipped([0.0, 4.0, -5.0], [0.0, 4.0, 15.0], &b));
        // 範囲の外では半径 6 でも、範囲の中では 4〜5 → 通る
        assert!(!segment_clipped([0.0, 6.0, -10.0], [0.0, 4.0, 10.0], &b));
        // 範囲の中で半径 5.5 → 切れる
        assert!(segment_clipped([0.0, 4.0, 0.0], [0.0, 5.5, 10.0], &b));
        // 範囲に入らない
        assert!(!segment_clipped([0.0, 9.0, 11.0], [0.0, 9.0, 20.0], &b));
        // 斜めに横切る（端の 2 点は範囲の外、範囲の端で半径 > 5）
        assert!(segment_clipped([0.0, -20.0, -10.0], [0.0, 20.0, 20.0], &b));
        assert!(!segment_clipped([0.0, -10.0, -10.0], [0.0, 10.0, 20.0], &b));
        // 軸に垂直な区間
        assert!(segment_clipped([0.0, 0.0, 5.0], [6.0, 0.0, 5.0], &b));
    }
}
