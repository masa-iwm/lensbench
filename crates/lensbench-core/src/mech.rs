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
#[derive(
    Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Bore {
    pub z0: f64,
    pub z1: f64,
    /// 半径 [mm]
    pub radius: f64,
}

/// 系に置いた機構部品
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct MechPart {
    pub name: String,
    /// 置く基準の面（その面の局所座標）
    pub surface: usize,
    /// 基準の面の局所座標でのずれ [dx, dy, dz] [mm]
    #[serde(default)]
    pub offset: [f64; 3],
    /// 傾き [tx, ty] [deg]（ずらした後、x → y の順）
    #[serde(default)]
    pub tilt: [f64; 2],
    pub bores: Vec<Bore>,
    /// 外形（描画・重なりの確認用）: 部品の z の範囲と外径の半径。0 なら穴から決める
    #[serde(default)]
    pub length: f64,
    #[serde(default)]
    pub outer_radius: f64,
    /// 素子を保持する内壁の半径（保持径の半分）。0 なら穴の半径が内壁
    #[serde(default)]
    pub hold_radius: f64,
    /// ケージのロッドの間隔 [mm]（0 はケージ板でない）
    #[serde(default)]
    pub rod_spacing: f64,
    /// 外形が角（`outer_radius` は正方形の一辺の半分、辺は部品の x・y 軸に沿う）。false は丸
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub square: bool,
    /// 部品の詳しい形の STEP（品の公式の STEP を取ったキャッシュのファイル）。描画で詳しい形を選ぶと使う
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub step: String,
    /// STEP の形を裏返して置く（部品を `reversed` で置いた。z → 長さ − z、y → −y）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub step_flip: bool,
    /// 保持する素子の面（系の面の番号）。はみ出し・保持径の確認に使う
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<usize>,
}

impl MechPart {
    /// 部品の座標系（大域座標の原点と、列が軸の回転行列）
    pub fn frame(&self, s: &System) -> Result<(V3, M3)> {
        self.frame_in(s.frames())
    }

    /// 座標系 `fr` での部品の座標系（[`MechPart::frame`]）
    pub fn frame_in(&self, fr: &crate::system::Frames) -> Result<(V3, M3)> {
        let Some(f) = fr.surf.get(self.surface).copied().flatten() else {
            return Err(Error::Invalid(format!(
                "mechanism {}: no coordinates of surface {} (object, ignored or out of range)",
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
    let glass = |m: &crate::surface::Material| {
        !matches!(
            m,
            crate::surface::Material::Air | crate::surface::Material::Inherit
        )
    };
    let mut out = Vec::new();
    let mut prev_glass = false;
    for j in 1..img {
        let d = &s.surfaces[j];
        if d.ignore || d.is_coord_break() {
            continue;
        }
        let g = glass(&d.mat);
        let solid = g || prev_glass || d.is_mirror();
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
                        "the edge of surface {j} (radius {semi:.3}) hits the wall of {} by {worst:.3} mm",
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
                        "held surface {j} sticks out of {} by {out_by:.3} mm (part length {:.3})",
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
                        "surface {j} (diameter {:.3}) is smaller than {}, whose holding diameter is {:.3} (needs an adapter)",
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
                    message: format!("{} and {} overlap", parts[a].name, parts[b].name),
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
        let last = *group.last().expect("non-empty group");
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
                        "the rod spacing of {} and {} differs ({:.3} and {:.3})",
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
                "the rods of the cage ({}) need {:.3} mm or more (spacing {:.3})",
                group
                    .iter()
                    .map(|&b| parts[b].name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
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
pub(crate) fn segment_clipped(a: V3, b: V3, bore: &Bore) -> bool {
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
    /// 瞳の点ごとの結果（[`STATE_PASSED`]・[`STATE_OPTICAL`]・[`STATE_FAILED`]、1 以上は最初に切った部品の番号 + 1）
    pub state: Vec<i32>,
}

/// [`MechVig::state`]: どこでも切れずに像面まで届いた
pub const STATE_PASSED: i32 = 0;
/// [`MechVig::state`]: 系の開口で切れた（どの部品より先に）
pub const STATE_OPTICAL: i32 = -1;
/// [`MechVig::state`]: 追跡が止まった（全反射・当たらない）
pub const STATE_FAILED: i32 = -2;

/// 部品 `parts` のケラレを、視野 `fields`・波長 `waves` の瞳の点 (px, py) の光線で数える
pub fn vignetting(
    s: &System,
    parts: &[MechPart],
    fields: &[usize],
    waves: &[usize],
    px: &[f64],
    py: &[f64],
) -> Result<Vec<MechVig>> {
    // 部品で切る設定でも、部品の判定はここで数える（追跡では切らない）
    let off;
    let s = if s.mech_clip {
        let mut c = s.clone();
        c.edit(|p| {
            p.mech_clip = false;
            Ok(())
        })?;
        off = c;
        &off
    } else {
        s
    };
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
                state: Vec::with_capacity(px.len()),
            };
            for (i, r) in t.rays.iter().enumerate() {
                if r.status.surf().is_some() {
                    v.failed += 1;
                    v.state.push(STATE_FAILED);
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
                    (Some((k, _)), _) => {
                        v.first[k] += 1;
                        v.state.push(k as i32 + 1);
                    }
                    (None, true) => {
                        v.optical += 1;
                        v.state.push(STATE_OPTICAL);
                    }
                    (None, false) => {
                        v.passed += 1;
                        v.state.push(STATE_PASSED);
                    }
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
    _hit: &[Option<usize>],
) -> Option<(usize, usize)> {
    clip_point(pts, parts, frames).map(|c| (c.part, c.surface))
}

/// 光線が最初に部品の穴の外に出る所（[`clip_point`]）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipPoint {
    /// 部品の番号
    pub part: usize,
    /// その区間の終わりの面
    pub surface: usize,
    /// 区間の終わりの点の、点の並びでの位置
    pub index: usize,
    /// 穴の外に出る点（大域座標）。穴の端の面で外にいればその面の上、途中で出れば穴の壁の上
    pub point: V3,
}

/// 部品の座標の線分 a → b が穴の外に出る最初の助変数（0〜1）。出なければ None
fn exit_param(a: V3, b: V3, bore: &Bore) -> Option<f64> {
    if !segment_clipped(a, b, bore) {
        return None;
    }
    let (z0, z1) = (bore.z0.min(bore.z1), bore.z0.max(bore.z1));
    let dz = b[2] - a[2];
    let (t0, t1) = if dz.abs() < 1e-300 {
        (0.0, 1.0)
    } else {
        let (ta, tb) = ((z0 - a[2]) / dz, (z1 - a[2]) / dz);
        (ta.min(tb).max(0.0), ta.max(tb).min(1.0))
    };
    let r2 = bore.radius * bore.radius;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let at = |t: f64| (a[0] + t * dx).powi(2) + (a[1] + t * dy).powi(2);
    if at(t0) > r2 * (1.0 + 1e-12) {
        return Some(t0);
    }
    // |a + t d|² = r² の大きい方の根（区間の始まりは内側なので、外へ出るのは大きい根）
    let qa = dx * dx + dy * dy;
    let qb = 2.0 * (a[0] * dx + a[1] * dy);
    let qc = a[0] * a[0] + a[1] * a[1] - r2;
    let disc = qb * qb - 4.0 * qa * qc;
    if qa > 0.0 && disc >= 0.0 {
        let t = (-qb + disc.sqrt()) / (2.0 * qa);
        if t.is_finite() {
            return Some(t.clamp(t0, t1));
        }
    }
    Some(t1)
}

/// 光の通る順の点の並び `pts`（面の番号, 大域座標）で、光線が最初に部品の穴の外に出る所。
/// 同じ区間で複数の部品が切るなら、区間の中で先に出る部品
pub fn clip_point(
    pts: &[(usize, V3)],
    parts: &[MechPart],
    frames: &[(V3, M3)],
) -> Option<ClipPoint> {
    for (s, seg) in pts.windows(2).enumerate() {
        let mut best: Option<(f64, usize)> = None;
        for (k, (p, (o, m))) in parts.iter().zip(frames).enumerate() {
            let u = mtv(m, sub(seg[0].1, *o));
            let v = mtv(m, sub(seg[1].1, *o));
            for bo in &p.bores {
                if let Some(t) = exit_param(u, v, bo) {
                    if best.is_none_or(|(bt, _)| t < bt) {
                        best = Some((t, k));
                    }
                }
            }
        }
        if let Some((t, k)) = best {
            let (a, b) = (seg[0].1, seg[1].1);
            return Some(ClipPoint {
                part: k,
                surface: seg[1].0,
                index: s + 1,
                point: add(a, crate::math::scale(sub(b, a), t)),
            });
        }
    }
    None
}

/// 系の部品の座標系（座標の決まらない部品はエラー）
pub fn frames(s: &System, parts: &[MechPart]) -> Result<Vec<(V3, M3)>> {
    parts.iter().map(|p| p.frame(s)).collect()
}

/// 瞳の正方格子（一辺 `n` 本、-1〜1）のうち単位円の中の点 (px, py)（[`crate::pupil::PupilGrid::Linear`] の
/// [`crate::pupil::Rim::Padded`]）。`n` < 2 なら空
pub fn pupil_grid(n: usize) -> (Vec<f64>, Vec<f64>) {
    if n < 2 {
        return (Vec::new(), Vec::new());
    }
    let s = crate::pupil::PupilGrid::Linear {
        n,
        rim: crate::pupil::Rim::Padded,
    }
    .samples();
    (s.px, s.py)
}

/// 部品の座標の点を大域座標へ
pub fn to_global(frame: &(V3, M3), p: V3) -> V3 {
    add(frame.0, mv(&frame.1, p))
}

/// 部品の z 軸の向き（大域座標）
pub fn axis(frame: &(V3, M3)) -> V3 {
    col(&frame.1, 2)
}

impl System {
    /// 面 j の玉の半径（OpticStudio の Mech Semi-Dia）: ガラスでつながる面（無視した面は飛ばしてつなぐ）の、縁の半径
    /// （円形の開口の面は開口の最大の半径、ほかは半直径）+ チップゾーンと、固定した機械半径の最大（OpticStudio 26.1 で確認。
    /// 固定した値が自動より小さければ自動のまま）
    pub fn lens_radius(&self, j: usize) -> f64 {
        self.lens_radius_but(j, None)
    }

    /// [`System::lens_radius`] で、面 skip の固定した機械半径だけは数えないもの（機械半径のピックアップの先が元と同じ玉に
    /// あるとき、先の値が元に戻ってこないように）
    pub(crate) fn lens_radius_but(&self, j: usize, skip: Option<usize>) -> f64 {
        self.lens_surfaces(j)
            .map(|i| {
                let d = &self.surfaces[i];
                let edge = match d.ap {
                    crate::surface::Aperture::Circular { max, .. } => max,
                    _ => d.semi,
                };
                let fixed = if Some(i) == skip { None } else { d.mech_semi };
                (edge + d.chip_zone).max(fixed.unwrap_or(0.0))
            })
            .fold(0.0, f64::max)
    }

    /// 面 j の玉の、固定した機械半径の最大（無ければ 0）
    pub fn fixed_mech(&self, j: usize) -> f64 {
        self.lens_surfaces(j)
            .filter_map(|i| self.surfaces[i].mech_semi)
            .fold(0.0, f64::max)
    }

    /// 面 j とガラスでつながる面（無視した面は飛ばしてつなぎ、返さない）
    fn lens_surfaces(&self, j: usize) -> impl Iterator<Item = usize> + '_ {
        let glass = |i: usize| {
            let m = &self.surfaces[i].mat;
            m.is_medium() && !m.is_air()
        };
        let ign = |i: usize| self.surfaces[i].ignore;
        let (mut a, mut b) = (j, j);
        while a > 1 && (glass(a - 1) || ign(a - 1)) {
            a -= 1;
        }
        while b < self.image() && (glass(b) || ign(b)) {
            b += 1;
        }
        (a..=b).filter(move |&i| !self.surfaces[i].ignore)
    }

    /// 面 j の形が終わって平らになる半径（クリア半径 + チップゾーン）
    pub fn flat_from(&self, j: usize) -> f64 {
        let d = &self.surfaces[j];
        d.semi + d.chip_zone
    }
}

/// 穴の端の面を置く所（[`as_apertures`]）
struct EndPlane {
    /// 前の面（この面の後ろの隙間に置く）
    gap: usize,
    /// 前の面の頂点から軸に沿った距離（前の面の厚さと同じ向き）
    d: f64,
    radius: f64,
    /// 前の面の局所座標での偏心
    dx: f64,
    dy: f64,
    label: String,
}

/// 穴の端の平面が面の頂点の上にあって前後 2 つの隙間に入るとき、どちらに置くかを決める。lensbench の判定
/// （[`segment_clipped`]）で頂点の平面を横切るのは、穴の向きに依らず、穴の縁での面の矢高が光の進む向き
/// （前の隙間の厚さの向き）に出ていれば面の前の線分、逆なら後ろの線分なので、前の隙間の終わりか後ろの隙間の
/// 始まりの一方を残す（平らな面は後ろの隙間の始まり）。縁の矢高の向きがそろわない面は決めない（印として
/// gap = usize::MAX の端を 2 つ返す）。鏡と隣り合わない 2 つの隙間はそのまま返す
fn on_vertex(
    s: &System,
    gaps: &[(usize, V3, [V3; 3], f64, usize)],
    mut hits: Vec<EndPlane>,
) -> Vec<EndPlane> {
    if hits.len() != 2 {
        return hits;
    }
    hits.sort_by_key(|e| e.gap);
    let (a, b) = (&hits[0], &hits[1]);
    let ga = gaps.iter().find(|g| g.0 == a.gap);
    let d = &s.surfaces[b.gap];
    let undecided = |mut h: Vec<EndPlane>| {
        for e in &mut h {
            e.gap = usize::MAX;
        }
        h
    };
    if ga.map(|g| g.4) != Some(b.gap) || d.is_mirror() {
        return hits;
    }
    let (Some(ga), Some(axk)) = (ga, gaps.iter().find(|g| g.0 == b.gap).map(|g| g.2[2])) else {
        return undecided(hits);
    };
    if ga.3 == 0.0 {
        return undecided(hits);
    }
    // 面の局所の z を光の進む向き（前の隙間の軸 × 厚さの符号）に直す
    let k = dot(axk, ga.2[2]) * ga.3.signum();
    let tol = 1e-12 * (1.0 + b.radius);
    let h: Vec<f64> = (0..16)
        .map(|i| {
            let th = i as f64 * std::f64::consts::TAU / 16.0;
            let (x, y) = (b.dx + b.radius * th.cos(), b.dy + b.radius * th.sin());
            crate::freeform::sag_xy(d, x, y).0 * k
        })
        .collect();
    if h.iter().any(|v| !v.is_finite()) {
        undecided(hits)
    } else if h.iter().all(|&v| v <= tol) {
        hits.remove(0);
        hits
    } else if h.iter().all(|&v| v >= -tol) {
        hits.truncate(1);
        hits
    } else {
        undecided(hits)
    }
}

/// 機構部品の穴を、穴の両端（z0・z1）の平面に置いた円形開口（半径 = 穴の半径）のダミー面にした系と注意
/// （OpticStudio には機構部品が無いので、.zmx に書くときに使う）。面は端の平面が通る隙間（前の面の軸に沿う隙間で、
/// 部品の軸が隙間の軸と平行なもの）に入れて隙間の厚さを分ける。偏心は開口の偏心にする。返す系に機構部品は無い。
///
/// 面と面の間の直線の上の x² + y² の最大は z の範囲の端なので、穴の中に光学面が無い隙間では端の 2 面の判定は
/// [`segment_clipped`] と同じ。穴の中に光学面があると（玉を保持する部品）、その面の交点での判定は落ちる。
/// 端が面の頂点の上にあれば前後の隙間の一方に置く（`on_vertex`）。置けない端（部品が隙間の軸に対して
/// 傾いている・どの隙間にも無い・頂点の上で面の矢高の向きがそろわない・鏡の頂点の上などで 2 つ以上の隙間に
/// ある・厚さがソルブかピックアップで決まる隙間・多重構成の系）は書かずに注意を返す
pub fn as_apertures(s: &System) -> Result<(System, Vec<String>)> {
    let mut notes = Vec::new();
    let mut out = s.clone();
    if s.mechanics.is_empty() {
        return Ok((out, notes));
    }
    if s.configs.is_some() {
        notes.push(format!(
            "{} mechanical parts were not written (a multi-configuration system)",
            s.mechanics.len()
        ));
    } else {
        let fr = s.frames();
        // 前の面と次の面が共軸につながる隙間（物体面の後ろは見ない）: (前の面, 原点, 局所の x・y・z 軸, 厚さ, 次の面)
        let mut gaps: Vec<(usize, V3, [V3; 3], f64, usize)> = Vec::new();
        let idx: Vec<usize> = (1..=s.image())
            .filter(|&j| !s.surfaces[j].ignore && fr.surf[j].is_some())
            .collect();
        for w in idx.windows(2) {
            let (j, k) = (w[0], w[1]);
            let (fj, fk) = (fr.surf[j].expect("frame"), fr.surf[k].expect("frame"));
            let ax = [col(&fj.r, 0), col(&fj.r, 1), col(&fj.r, 2)];
            let t = s.surfaces[j].t;
            let end = add(fj.o, crate::math::scale(ax[2], t));
            let fixed = !s
                .pickups
                .iter()
                .any(|q| q.dst == j && q.what == crate::param::Kind::Thick)
                && !s
                    .solves
                    .iter()
                    .any(|q| q.surf == j && q.kind.param_kind() == crate::param::Kind::Thick);
            if t.is_finite() && norm(sub(end, fk.o)) < 1e-9 * (1.0 + t.abs()) && fixed {
                gaps.push((j, fj.o, ax, t, k));
            }
        }
        let mut planes: Vec<EndPlane> = Vec::new();
        for p in &s.mechanics {
            let (o, m) = p.frame(s)?;
            let n = col(&m, 2);
            for b in &p.bores {
                for (z, end) in [(b.z0, "z0"), (b.z1, "z1")] {
                    let q = add(o, crate::math::scale(n, z));
                    let label = format!("MECH {} {end}", p.name);
                    // 部品の軸と平行で、平面が隙間の中にある隙間
                    let hits: Vec<EndPlane> = gaps
                        .iter()
                        .filter(|g| dot(n, g.2[2]).abs() > 1.0 - 1e-9)
                        .filter_map(|&(j, a, ax, t, _)| {
                            let v = sub(q, a);
                            let d = dot(v, ax[2]);
                            let inside = if t >= 0.0 {
                                (-1e-9..=t + 1e-9).contains(&d)
                            } else {
                                (t - 1e-9..=1e-9).contains(&d)
                            };
                            inside.then(|| EndPlane {
                                gap: j,
                                d: d.clamp(t.min(0.0), t.max(0.0)),
                                radius: b.radius,
                                dx: dot(v, ax[0]),
                                dy: dot(v, ax[1]),
                                label: label.clone(),
                            })
                        })
                        .collect();
                    let hits = on_vertex(s, &gaps, hits);
                    match hits.len() {
                        1 => planes.extend(hits),
                        0 => notes.push(format!(
                            "the bore end {} of mechanical part {} was not written (no surface gap along the part axis holds it)",
                            z, p.name
                        )),
                        _ if hits.iter().any(|e| e.gap == usize::MAX) => notes.push(format!(
                            "the bore end {} of mechanical part {} was not written (it lies on the vertex of a surface that crosses its plane)",
                            z, p.name
                        )),
                        _ => notes.push(format!(
                            "the bore end {} of mechanical part {} was not written (it lies in more than one surface gap)",
                            z, p.name
                        )),
                    }
                }
            }
        }
        // 後ろの隙間から入れる（前の面の番号が変わらない）。隙間の中は前の面から近い順
        planes.sort_by(|a, b| b.gap.cmp(&a.gap).then(a.d.abs().total_cmp(&b.d.abs())));
        let mut i = 0;
        while i < planes.len() {
            let j = planes[i].gap;
            let group: Vec<&EndPlane> = planes[i..].iter().take_while(|e| e.gap == j).collect();
            i += group.len();
            let t = s.surfaces[j].t;
            // 隙間の媒質（鏡・座標ブレークは前の媒質のまま）
            let mat = (1..=j)
                .rev()
                .map(|k| &s.surfaces[k])
                .find(|d| !d.is_mirror() && !d.is_coord_break())
                .map(|d| d.mat.clone())
                .unwrap_or(crate::surface::Material::Air);
            for (k, e) in group.iter().enumerate() {
                let next = group.get(k + 1).map_or(t, |x| x.d);
                let d = crate::surface::Surface {
                    t: next - e.d,
                    comment: e.label.clone(),
                    mat: mat.clone(),
                    semi: e.radius,
                    ap: crate::surface::Aperture::Circular {
                        min: 0.0,
                        max: e.radius,
                        dx: e.dx,
                        dy: e.dy,
                    },
                    ..Default::default()
                };
                out.insert_surface(j + 1 + k, d)?;
            }
            let first = group[0].d;
            out.edit(|r| {
                r.surfaces[j].t = first;
                Ok(())
            })?;
        }
    }
    out.edit(|r| {
        r.mechanics.clear();
        Ok(())
    })?;
    Ok((out, notes))
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
