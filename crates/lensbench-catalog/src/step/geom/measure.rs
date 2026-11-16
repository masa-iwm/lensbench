//! 立体の形から品の寸法を測る（[`measure`]）。順に「光軸を決める」（[`find_axis`]）、「立体を分ける」
//! （[`solids`]: ガラスと枠）、「枠の寸法を読む」（[`glass_mount`]・[`mech_part`]）、「ねじ・ロッドを当てる」
//! （[`rod_spacing`]・[`end_thread`]・[`pick_hold`]）。規格の表（[`THREADS`]・[`ROD_SPACINGS`]）と
//! 許容差（`*_COS`・`*_REL`・`*_MM`）は名前のある定数にまとめる（PRJ-26）

use super::{
    add, boxed, cross, dot, notes, perp, scale, sub, unit, Geo, Measure, Res, RetainingRing, Step,
    SubSq, V3,
};
use lensbench_step::StepFile;
use serde_json::json;
use std::collections::{HashMap, HashSet};

/// 向きが平行とみなす余弦の絶対値の下限
const PARALLEL_COS: f64 = 0.999;
/// 光軸の上・同じ位置とみなす距離（最大の円の半径に対する比）
const COAX_REL: f64 = 1e-4;
/// 同じ大きさ・正方形の角とみなす差（半径・距離に対する比）
const SAME_REL: f64 = 1e-3;
/// 同じ半径とみなす差 [mm]（ロッドの穴の 1 つだけ逃げの大きい穴、正方形に並ぶ円）
const SAME_R_MM: f64 = 0.2;
/// 端の面取りとみなす長さ [mm]（端のねじ）
const END_TOL_MM: f64 = 1.0;

/// ケージのロッドの間隔の規格 [mm]（16・30・60 は Thorlabs、26.87 は Edmund の 25・30 mm のケージ（ロッドの円の
/// 直径 38 mm））
const ROD_SPACINGS: [f64; 4] = [16.0, 26.87, 30.0, 60.0];

/// 端のねじとする最短の長さ [mm]（ねじの径の浅い座ぐり・刻印などを除く）
const MIN_THREAD_MM: f64 = 0.5;

/// ケージのロッドの間隔の規格に合わない組を使うときの、ロッドの穴の数
const ROD_HOLES: usize = 4;

/// 光学部品のねじ（名前、ねじの外径（メスの谷の径）[mm]、ピッチ [mm]、入る素子の径 [mm]（0 は素子を入れない
/// ねじ）、メスを外径の円柱でも判定するか（C マウントの外径 25.4 mm は 1 インチの素子の穴と区別できないので、
/// 端のねじで、奥にねじの逃げの溝（外径より太い短い穴）のあるときだけ。[`relief_next_to`]））
struct Thread {
    name: &'static str,
    major: f64,
    pitch: f64,
    holds: f64,
    female_major: bool,
}

const THREADS: [Thread; 5] = [
    Thread {
        name: "SM05",
        major: 0.535 * 25.4,
        pitch: 25.4 / 40.0,
        holds: 12.7,
        female_major: true,
    },
    Thread {
        name: "SM1",
        major: 1.035 * 25.4,
        pitch: 25.4 / 40.0,
        holds: 25.4,
        female_major: true,
    },
    Thread {
        name: "SM2",
        major: 2.035 * 25.4,
        pitch: 25.4 / 40.0,
        holds: 50.8,
        female_major: true,
    },
    Thread {
        name: "SM3",
        major: 3.035 * 25.4,
        pitch: 25.4 / 40.0,
        holds: 76.2,
        female_major: true,
    },
    Thread {
        name: "C-Mount",
        major: 25.4,
        pitch: 25.4 / 32.0,
        holds: 0.0,
        female_major: false,
    },
];

impl Thread {
    /// 円柱の直径 `d` [mm] がこのねじか。STEP のねじは外径の円柱で描かれる（Thorlabs）か、メスは山の径（内径）の
    /// 円柱で描かれる（Edmund。外径 − 0.96 ピッチほど）。メスの内径は基準の内径（外径 − 1.0825 ピッチ）から
    /// 外径 − 0.9 ピッチまで
    fn fits(&self, d: f64, female: bool, relief: bool) -> bool {
        ((!female || self.female_major || relief) && (self.major - d).abs() <= 0.05)
            || (female
                && d >= self.major - 1.0825 * self.pitch - 0.05
                && d <= self.major - 0.9 * self.pitch)
    }
}

/// 直径 `d` [mm] のねじ。`relief` は穴の並びがねじの逃げの溝を持つか（[`relief_next_to`]）
fn thread_fits(d: f64, female: bool, relief: &dyn Fn(f64) -> bool) -> Option<&'static Thread> {
    THREADS.iter().find(|t| t.fits(d, female, relief(t.major)))
}

/// 穴の並び（mm）で、直径 `major` の穴のとなりに、それより太い（1.1 mm まで）短い（2 mm まで）穴があるか
/// （メスのねじの奥の逃げの溝。Thorlabs の C マウントのメスは外径 25.4 の円柱と逃げの溝 26.416 で描かれる）
fn relief_next_to(bores: &[lensbench_project::BoreSpec], major: f64) -> bool {
    bores.windows(2).any(|w| {
        let groove = |g: &lensbench_project::BoreSpec| {
            g.diameter > major + 0.05 && g.diameter <= major + 1.1 && g.z1 - g.z0 <= 2.0
        };
        let thread = |t: &lensbench_project::BoreSpec| (t.diameter - major).abs() <= 0.05;
        (thread(&w[0]) && groove(&w[1])) || (groove(&w[0]) && thread(&w[1]))
    })
}

/// 保持する素子の径（mm）と、そのねじの名前（ねじでなければ空）。穴の並び `bores`（mm、止め輪を除いた全立体）から
/// 次の順で選ぶ（組立のマウントで、素子の入る立体と最も長い立体が違っても読めるように。#182）:
/// 1. 長さ 0.5 mm 以上の、素子を入れるメスねじ（SM05〜SM3）のうち最も長いもの。径はねじの呼び
/// 2. 座: 長さ 0.5 mm 以上で、となりにそれより細い穴（径の 0.75 倍以上で、0.2〜6 mm 細い。素子が当たる肩）がある
///    区間。複数なら最も細いもの。両隣より太い 1 mm 未満の溝（止め輪の溝など）は除いてから探す
/// 3. 最も長い穴（同じ径の区間の長さの和）
///
/// 2・3 で選んだ穴が素子を入れないねじ（C マウントなど）なら、そのねじの名前も返す
pub(super) fn pick_hold(bores: &[lensbench_project::BoreSpec]) -> (String, f64) {
    let len = |b: &lensbench_project::BoreSpec| b.z1 - b.z0;
    let by_len = |a: &&(f64, f64), b: &&(f64, f64)| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal))
    };
    let mut threads: Vec<(f64, f64)> = Vec::new();
    for b in bores.iter().filter(|b| len(b) >= MIN_THREAD_MM) {
        if thread_fits(b.diameter, true, &|_| false).is_some_and(|t| t.holds > 0.0) {
            match threads.iter_mut().find(|r| (r.0 - b.diameter).abs() < 1e-9) {
                Some(r) => r.1 += len(b),
                None => threads.push((b.diameter, len(b))),
            }
        }
    }
    if let Some(&(d, _)) = threads.iter().max_by(by_len) {
        let t = thread_fits(d, true, &|_| false).expect("ねじ");
        return (t.name.to_string(), t.holds);
    }
    let n = bores.len();
    let groove = |i: usize| {
        i > 0
            && i + 1 < n
            && len(&bores[i]) < 1.0
            && bores[i - 1].diameter < bores[i].diameter
            && bores[i + 1].diameter < bores[i].diameter
    };
    let bs: Vec<&lensbench_project::BoreSpec> =
        (0..n).filter(|&i| !groove(i)).map(|i| &bores[i]).collect();
    let seat = (0..bs.len())
        .filter(|&i| len(bs[i]) >= 0.5)
        .filter(|&i| {
            let d = bs[i].diameter;
            [i.wrapping_sub(1), i + 1]
                .iter()
                .filter_map(|&j| bs.get(j))
                .any(|nb| {
                    nb.diameter >= 0.75 * d && nb.diameter < d - 0.2 && nb.diameter >= d - 6.0
                })
        })
        .map(|i| bs[i].diameter)
        .fold(f64::INFINITY, f64::min);
    // 素子を入れないねじ（C マウントなど）の穴なら、径はそのままでねじの名前を付ける
    let named = |d: f64| {
        let t = thread_fits(d, true, &|_| false).filter(|t| t.holds <= 0.0);
        (t.map_or_else(String::new, |t| t.name.to_string()), d)
    };
    if seat.is_finite() {
        return named(seat);
    }
    let mut runs: Vec<(f64, f64)> = Vec::new();
    for b in bores {
        match runs.iter_mut().find(|r| (r.0 - b.diameter).abs() < 1e-9) {
            Some(r) => r.1 += len(b),
            None => runs.push((b.diameter, len(b))),
        }
    }
    named(runs.iter().max_by(by_len).map_or(0.0, |r| r.0))
}

/// 光軸と単位（測る途中の共通の値）
struct Ctx {
    /// 光軸の上の点・向き（STEP の座標）
    o: V3,
    axis: V3,
    /// 光軸に取った円の半径（許容差の基準）
    rmax: f64,
    /// 長さの倍率（mm へ）と丸め
    k: f64,
    round: fn(f64) -> f64,
    /// 同じ z とみなす差
    eps: f64,
}

impl Ctx {
    /// mm にして丸めた長さ
    fn sc(&self, x: f64) -> f64 {
        (self.round)(x * self.k)
    }

    fn z_of(&self, p: V3) -> f64 {
        dot(sub(p, self.o), self.axis)
    }

    fn rho(&self, p: V3) -> f64 {
        let d = sub(p, self.o);
        let z = dot(d, self.axis);
        dot(d, d).sub_sq(z * z).sqrt()
    }

    /// 中心 c・向き a の円・円柱が光軸と同じ軸か
    fn coax(&self, c: V3, a: V3) -> bool {
        dot(a, self.axis).abs() > PARALLEL_COS && self.rho(c) <= COAX_REL * self.rmax + 1e-9
    }

    /// 光軸に直交する 2 方向
    fn plane_axes(&self) -> (V3, V3) {
        let ex = perp(self.axis);
        (ex, cross(self.axis, ex))
    }
}

/// 測る。長さは `k` 倍し `round` で丸める（mm にする）
pub(crate) fn measure(f: &StepFile, k: f64, round: fn(f64) -> f64) -> Res<Measure> {
    let st = Step::new(f);
    let inst = st.instances();
    // 立体ごとの形（共通の座標）
    let mut cache: HashMap<u64, Geo> = HashMap::new();
    let geos: Vec<Geo> = inst
        .iter()
        .map(|(s, m)| cache.entry(*s).or_insert_with(|| st.solid_geo(*s)).moved(m))
        .collect();
    // 全体の円と頂点（立体が無ければ全部の実体から）
    let all: Geo = if geos.is_empty() {
        let ids: HashSet<u64> = st.f.ids().collect();
        st.geo_of(&ids)
    } else {
        let mut g = Geo::default();
        for x in &geos {
            g.circles.extend(x.circles.iter().copied());
            g.arcs.extend(x.arcs.iter().copied());
            g.verts.extend(x.verts.iter().copied());
        }
        g
    };
    let Some((o, axis, rmax)) = find_axis(&all, k) else {
        return boxed(&st, &all, |x| round(x * k));
    };
    let cx = Ctx {
        o,
        axis,
        rmax,
        k,
        round,
        eps: 1e-6 * rmax,
    };
    let (sols, merged) = solids(&geos, &cx);
    let rings: Vec<&Sol> = sols.iter().filter(|s| s.ring).collect();
    let glass: Vec<&Sol> = sols.iter().filter(|s| !s.ring && s.r > 0.0).collect();
    if !rings.is_empty() && !glass.is_empty() {
        return Ok(glass_mount(&cx, &sols, &rings, &glass, merged));
    }
    // ガラスの無い機構部品（チューブ・ケージ板・マウント）: 全体を枠として測る
    if !rings.is_empty() {
        if let Some(m) = mech_part(&cx, &geos, &sols, &rings) {
            return Ok(m);
        }
    }
    unknown(&cx, &geos, &all)
}

/// 光軸（中心・向き・円の半径）。半周以上の弧の円のうち最大のもの（無ければ円の最大）。ロッドの穴の周りの溝など
/// の、半径の大きい短い弧を光軸に取らないように。円が無ければ None
fn find_axis(all: &Geo, k: f64) -> Option<(V3, V3, f64)> {
    let biggest_of = |cs: &[(V3, V3, f64)]| {
        cs.iter()
            .copied()
            .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    };
    let (o, axis, rmax) = biggest_of(&all.arcs).or_else(|| biggest_of(&all.circles))?;
    // 同じ大きさの円が複数（穴の無い板のロッドの穴など）なら、頂点の重心に近いもの（同じなら座標の小さいもの）。
    // 円の並びの順によらない
    let cands: Vec<(V3, V3, f64)> = if all.arcs.is_empty() {
        &all.circles
    } else {
        &all.arcs
    }
    .iter()
    .copied()
    .filter(|c| (c.2 - rmax).abs() <= 1e-9 * rmax)
    .collect();
    let mid = {
        let n = all.verts.len().max(1) as f64;
        let s = all.verts.iter().fold([0.0; 3], |a, p| add(a, *p));
        scale(s, 1.0 / n)
    };
    let (o, axis, rmax) = cands
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = dot(sub(a.0, mid), sub(a.0, mid));
            let db = dot(sub(b.0, mid), sub(b.0, mid));
            if (da - db).abs() > 1e-9 * rmax * rmax {
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
            }
        })
        .unwrap_or((o, axis, rmax));
    // 光軸に取った円が、同じ大きさ（0.2 mm まで）の平行な円（円柱の面を含む）4 つの正方形の 1 つなら（穴の無い板の
    // ロッドの穴）、正方形の中心を光軸にする
    let o = {
        let tol = SAME_R_MM / k;
        let mut sq: Vec<V3> = Vec::new();
        for c in all.circles.iter().filter(|c| {
            (c.2 - rmax).abs() <= tol && dot(unit(c.1).unwrap_or(c.1), axis).abs() > PARALLEL_COS
        }) {
            // 軸の向きに重なる円（穴の両端）は 1 つに数える
            let d = sub(c.0, o);
            let p = sub(d, scale(axis, dot(d, axis)));
            if !sq
                .iter()
                .any(|q| dot(sub(*q, p), sub(*q, p)).sqrt() <= SAME_REL * rmax)
            {
                sq.push(p);
            }
        }
        let cen = scale(sq.iter().fold([0.0; 3], |a, p| add(a, *p)), 0.25);
        let rr: Vec<f64> = sq
            .iter()
            .map(|p| dot(sub(*p, cen), sub(*p, cen)).sqrt())
            .collect();
        let square = sq.len() == 4
            && rr
                .iter()
                .all(|r| (r - rr[0]).abs() <= SAME_REL * rr[0] && *r > 2.0 * rmax)
            && sq
                .iter()
                .skip(1)
                .any(|p| dot(sub(*p, cen), sub(sq[0], cen)).abs() <= SAME_REL * rr[0] * rr[0]);
        if square {
            add(o, cen)
        } else {
            o
        }
    };
    // 軸の向きは STEP の座標の向きにそろえる（絶対値の最大の成分を正に）。光の進む向きは STEP から分からない
    let m = (0..3)
        .max_by(|&a, &b| {
            axis[a]
                .abs()
                .partial_cmp(&axis[b].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(2);
    let axis = if axis[m] < 0.0 {
        scale(axis, -1.0)
    } else {
        axis
    };
    Some((o, axis, rmax))
}

/// 立体 1 つの光軸に沿った形: z の範囲、軸と同じ軸の円の最大の半径、穴の内壁の面
struct Sol {
    z: [f64; 2],
    r: f64,
    /// 穴の内壁を持つ（枠）
    ring: bool,
    walls: Vec<(f64, [f64; 2])>,
    /// 外向きの円柱の面（外径・外側のねじ）の半径と z の範囲
    outs: Vec<(f64, [f64; 2])>,
    /// 光軸をふさぐ帽子（ガラスの面）の z と半径
    caps: Vec<(f64, f64)>,
}

/// 光軸と同じ軸の円柱の面（内壁か外向き）の半径と z の範囲
fn cyl_z(cx: &Ctx, g: &Geo, inward: bool) -> Vec<(f64, [f64; 2])> {
    g.cyls
        .iter()
        .filter(|f| f.inward == inward && cx.coax(f.c, f.axis))
        .filter_map(|f| {
            let (a, b) = f
                .pts
                .iter()
                .map(|p| cx.z_of(*p))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), z| {
                    (a.min(z), b.max(z))
                });
            (b > a).then_some((f.r, [a, b]))
        })
        .collect()
}

/// 立体を光軸に沿った形にし、ガラスと枠が 1 つの立体なら光軸をふさぐ面の間をガラスとして足す。足した数も返す
fn solids(geos: &[Geo], cx: &Ctx) -> (Vec<Sol>, usize) {
    let mut sols: Vec<Sol> = geos
        .iter()
        .filter_map(|g| {
            // z の範囲は頂点から（円の中心は、円錐・トーラスの面の基準の円など立体の外にあることがある）。
            // 頂点の無い立体だけ円の中心から
            let zs: Vec<f64> = if g.verts.len() >= 2 {
                g.verts.iter().map(|p| cx.z_of(*p)).collect()
            } else {
                g.verts
                    .iter()
                    .copied()
                    .chain(g.circles.iter().map(|c| c.0))
                    .map(|p| cx.z_of(p))
                    .collect()
            };
            let (lo, hi) = zs
                .into_iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| {
                    (lo.min(z), hi.max(z))
                });
            if hi.is_nan() || lo.is_nan() || hi < lo {
                return None;
            }
            let r = g
                .circles
                .iter()
                .filter(|c| cx.coax(c.0, c.1))
                .map(|c| c.2)
                .fold(0.0, f64::max);
            let walls = cyl_z(cx, g, true);
            let caps: Vec<(f64, f64)> = g
                .caps
                .iter()
                .filter(|c| cx.coax(c.0, c.1))
                .map(|c| (cx.z_of(c.0), c.2))
                .collect();
            Some(Sol {
                z: [lo, hi],
                r,
                ring: !walls.is_empty(),
                walls,
                outs: cyl_z(cx, g, false),
                caps,
            })
        })
        .collect();
    // ガラスと枠が 1 つの立体（穴の内壁と、光軸をふさぐ面の両方を持つ）: 光軸をふさぐ面の間をガラスとみなす。
    // ガラスの径は見えている径（帽子の縁）で、枠に隠れた部分は分からない
    let virt: Vec<Sol> = sols
        .iter()
        .filter(|s| s.ring && s.caps.len() >= 2)
        .filter_map(|s| {
            let (lo, hi) = s
                .caps
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), c| {
                    (lo.min(c.0), hi.max(c.0))
                });
            let r = s.caps.iter().map(|c| c.1).fold(0.0, f64::max);
            (hi - lo > cx.eps && r > 0.0).then(|| Sol {
                z: [lo, hi],
                r,
                ring: false,
                walls: Vec::new(),
                outs: Vec::new(),
                caps: Vec::new(),
            })
        })
        .collect();
    let merged = virt.len();
    sols.extend(virt);
    (sols, merged)
}

/// 立体の並びの z の範囲
fn span(v: &[&Sol]) -> (f64, f64) {
    v.iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), s| {
            (lo.min(s.z[0]), hi.max(s.z[1]))
        })
}

/// z での内壁の最小の半径（内壁が無ければ無限大）
fn wall_min(cx: &Ctx, walls: &[(f64, [f64; 2])], z: f64) -> f64 {
    walls
        .iter()
        .filter(|w| w.1[0] - cx.eps <= z && z <= w.1[1] + cx.eps)
        .map(|w| w.0)
        .fold(f64::INFINITY, f64::min)
}

/// 穴: 内壁の面の端で z を区切り、区間ごとに区間をまたぐ内壁の最小の半径。z は all0 から、長さ length（mm）
fn bores_of(
    cx: &Ctx,
    walls: &[(f64, [f64; 2])],
    all0: f64,
    length: f64,
) -> Vec<lensbench_project::BoreSpec> {
    let eps = cx.eps;
    let mut zs: Vec<f64> = walls.iter().flat_map(|w| [w.1[0], w.1[1]]).collect();
    zs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    zs.dedup_by(|a, b| (*a - *b).abs() <= eps);
    let mut bores: Vec<lensbench_project::BoreSpec> = Vec::new();
    for w in zs.windows(2) {
        if w[1] - w[0] <= eps {
            continue;
        }
        let r = wall_min(cx, walls, 0.5 * (w[0] + w[1]));
        if !r.is_finite() {
            continue;
        }
        let (z0, z1, d) = (cx.sc(w[0] - all0), cx.sc(w[1] - all0), cx.sc(2.0 * r));
        match bores.last_mut() {
            Some(b) if b.diameter == d && (b.z1 - z0).abs() < 1e-9 => b.z1 = z1,
            _ => bores.push(lensbench_project::BoreSpec {
                z0,
                z1,
                diameter: d,
            }),
        }
    }
    // 内壁の円柱の無い区間（面取りの円錐など）は、となりの穴の太い方でうめる（穴の無い区間は詰まった
    // 円柱として描かれ、確かめられるので）
    let mut filled: Vec<lensbench_project::BoreSpec> = Vec::new();
    let mut z = 0.0;
    for (i, b) in bores.iter().enumerate() {
        if b.z0 > z + 1e-9 {
            let prev = filled
                .last()
                .map_or(0.0, |x: &lensbench_project::BoreSpec| x.diameter);
            filled.push(lensbench_project::BoreSpec {
                z0: z,
                z1: b.z0,
                diameter: prev.max(b.diameter),
            });
        }
        filled.push(*b);
        z = b.z1;
        if i + 1 == bores.len() && z < length - 1e-9 {
            filled.push(lensbench_project::BoreSpec {
                z0: z,
                z1: length,
                diameter: b.diameter,
            });
        }
    }
    let mut bores: Vec<lensbench_project::BoreSpec> = Vec::new();
    for b in filled {
        match bores.last_mut() {
            Some(x) if x.diameter == b.diameter && (x.z1 - b.z0).abs() < 1e-9 => x.z1 = b.z1,
            _ => bores.push(b),
        }
    }
    bores
}

/// ガラスと枠の組立: ガラスの厚さ・径と、枠の穴・長さ・外径・座
fn glass_mount(cx: &Ctx, sols: &[Sol], rings: &[&Sol], glass: &[&Sol], merged: usize) -> Measure {
    let (g0, g1) = span(glass);
    let gr = glass.iter().map(|s| s.r).fold(0.0, f64::max);
    let (all0, all1) = span(&sols.iter().collect::<Vec<_>>());
    let length = cx.sc(all1 - all0);
    let walls: Vec<(f64, [f64; 2])> = rings.iter().flat_map(|s| s.walls.iter().copied()).collect();
    let bores = bores_of(cx, &walls, all0, length);
    let outer = rings.iter().map(|s| s.r).fold(0.0, f64::max);
    let hold_r = wall_min(cx, &walls, 0.5 * (g0 + g1));
    let hold_d = if hold_r.is_finite() {
        2.0 * hold_r
    } else {
        2.0 * gr
    };
    let clear = bores
        .iter()
        .map(|b| b.diameter)
        .fold(f64::INFINITY, f64::min);
    let frame = lensbench_project::MechSpec {
        bores,
        length,
        outer_diameter: cx.sc(2.0 * outer),
        holds: Some(lensbench_project::HoldSpec {
            diameter: cx.sc(hold_d),
            seat_z: cx.sc(g0 - all0),
        }),
        ..Default::default()
    };
    Measure {
        thickness: cx.sc(g1 - g0),
        diameter: json!(cx.sc(2.0 * gr)),
        note: notes::glass_mount(
            [sols.len() - merged, glass.len(), rings.len()],
            [
                cx.sc(g1 - g0),
                cx.sc(2.0 * gr),
                frame.length,
                frame.outer_diameter,
                clear,
                cx.sc(g0 - all0),
            ],
            merged > 0,
        ),
        frame: Some(frame),
        merged: merged > 0,
        mech: false,
        retaining_rings: Vec::new(),
        axes: Some((cx.o, cx.axis, perp(cx.axis), all0)),
    }
}

/// ガラスの無い機構部品（チューブ・ケージ板・マウント）: 止め輪を除いた全体を枠として測る。穴が無ければ None
fn mech_part(cx: &Ctx, geos: &[Geo], sols: &[Sol], rings: &[&Sol]) -> Option<Measure> {
    let (rmax, eps) = (cx.rmax, cx.eps);
    // 本体（z の最も長い穴のある立体）と、付属の止め輪（本体より短く、外径が本体の穴に入る輪）。止め輪は
    // 素子に合わせて動かすので枠に含めず、別に返す
    let body = rings.iter().copied().max_by(|a, b| {
        (a.z[1] - a.z[0])
            .partial_cmp(&(b.z[1] - b.z[0]))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let body_wall = body.map_or(0.0, |b| b.walls.iter().map(|w| w.0).fold(0.0, f64::max));
    let is_rr = |x: &Sol| {
        body.is_some_and(|b| {
            x.ring
                && !std::ptr::eq(x, b)
                && x.z[1] - x.z[0] < 0.5 * (b.z[1] - b.z[0])
                && x.r <= body_wall + SAME_REL * rmax
        })
    };
    let keep: Vec<&Sol> = sols.iter().filter(|x| !is_rr(x)).collect();
    let rrs: Vec<&Sol> = rings.iter().copied().filter(|x| is_rr(x)).collect();
    let (all0, all1) = span(&keep);
    let length = cx.sc(all1 - all0);
    let walls: Vec<(f64, [f64; 2])> = keep
        .iter()
        .filter(|s| s.ring)
        .flat_map(|s| s.walls.iter().copied())
        .collect();
    let bores = bores_of(cx, &walls, all0, length);
    if bores.is_empty() {
        return None;
    }
    let retaining: Vec<RetainingRing> = rrs
        .iter()
        .map(|x| RetainingRing {
            z0: cx.sc(x.z[0] - all0),
            z1: cx.sc(x.z[1] - all0),
            inner_diameter: cx.sc(2.0 * x.walls.iter().map(|w| w.0).fold(f64::INFINITY, f64::min)),
            outer_diameter: cx.sc(2.0 * x.r),
        })
        .collect();
    let bore_r = walls.iter().map(|w| w.0).fold(0.0, f64::max);
    let rods = rod_spacing(cx, geos, bore_r);
    let (ex, ey) = cx.plane_axes();
    let (min_width, min_t) = min_width(cx, geos.iter().flat_map(|g| g.verts.iter().copied()));
    // 局所の x 軸: 幅の最小の向き（角の部品の辺に直交）
    let xdir = {
        let (c, s) = (min_t.cos(), min_t.sin());
        [
            ex[0] * c + ey[0] * s,
            ex[1] * c + ey[1] * s,
            ex[2] * c + ey[2] * s,
        ]
    };
    // 外形: 光軸に直交する向きの幅の最小（ケージ板・キューブは一辺。取付けの出っ張りを除く）と、
    // 光軸と同じ軸の円の直径の大きい方（円柱は頂点が少ないので）
    let circ = 2.0 * sols.iter().map(|s| s.r).fold(0.0, f64::max);
    let outer = if min_width.is_finite() {
        min_width.max(circ)
    } else {
        circ
    };
    // 簡易外形: 幅の最小が軸の円より太ければ角（ケージ板・キューブ）。辺の向きは 0°・90° に揃っているとする
    let square = min_width.is_finite() && min_width > circ + SAME_REL * rmax;
    // 保持する素子（止め輪を除いた全立体の穴から。[`pick_hold`]）
    let (thread, hold_d) = pick_hold(&bores);
    // 座: 本体と別の止め輪（本体より z の短い輪）が前半にあれば、その奥の面。枠の端から保持する素子の径より
    // 細い穴が続けば、その段（素子が当たる肩）。どちらも無ければ枠の端
    let ring_seat = rrs
        .iter()
        .filter(|s| s.z[0] >= all0 - eps)
        .map(|s| s.z[1] - all0)
        .filter(|z| *z < 0.5 * (all1 - all0))
        .fold(0.0, f64::max);
    let shoulder_mm = bores
        .iter()
        .take_while(|b| b.diameter < hold_d - 1e-6)
        .map(|b| b.z1)
        .fold(0.0, f64::max);
    let shoulder = if shoulder_mm < length - 1e-9 {
        shoulder_mm
    } else {
        0.0
    };
    let seat = cx.sc(ring_seat).max(shoulder);
    let ends = Ends {
        keep: &keep,
        bores: &bores,
        outer,
        z: [all0, all1],
    };
    let front_thread = end_thread(cx, &ends, all0);
    let back_thread = end_thread(cx, &ends, all1);
    let rings_n = keep.iter().filter(|x| x.ring).count();
    let frame = lensbench_project::MechSpec {
        bores,
        length,
        outer_diameter: cx.sc(outer),
        thread: thread.clone(),
        rod_spacing: rods.unwrap_or(0.0),
        holds: Some(lensbench_project::HoldSpec {
            diameter: hold_d,
            seat_z: seat,
        }),
        front_thread,
        back_thread,
        outline: if square {
            lensbench_project::MechOutline::Square
        } else {
            lensbench_project::MechOutline::Round
        },
    };
    let clear = frame
        .bores
        .iter()
        .map(|b| b.diameter)
        .fold(f64::INFINITY, f64::min);
    Some(Measure {
        thickness: frame.length,
        diameter: json!(frame.outer_diameter),
        note: notes::mech(
            (keep.len(), rings_n),
            &frame,
            rods.is_some(),
            clear,
            &thread,
            if seat <= 0.0 {
                notes::Seat::End
            } else if seat == shoulder {
                notes::Seat::Step
            } else {
                notes::Seat::Ring
            },
            &retaining,
        ),
        frame: Some(frame),
        merged: false,
        mech: true,
        retaining_rings: retaining,
        axes: Some((cx.o, cx.axis, xdir, all0)),
    })
}

/// 光軸に直交する向きの、点の幅の最小とその向き [rad]（1° おき）
fn min_width(cx: &Ctx, pts: impl Iterator<Item = V3>) -> (f64, f64) {
    let (ex, ey) = cx.plane_axes();
    let q: Vec<[f64; 2]> = pts
        .map(|p| {
            let d = sub(p, cx.o);
            [dot(d, ex), dot(d, ey)]
        })
        .collect();
    (0..180)
        .map(|i| {
            let t = (i as f64).to_radians();
            let (c, s) = (t.cos(), t.sin());
            let (lo, hi) = q
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| {
                    let x = q[0] * c + q[1] * s;
                    (lo.min(x), hi.max(x))
                });
            (hi - lo, t)
        })
        .fold((f64::INFINITY, 0.0), |a, b| if b.0 < a.0 { b } else { a })
}

/// ケージのロッドの間隔 [mm]（規格の間隔に 0.05 mm 以内なら規格の値）。ロッドの穴は、軸に平行で光軸に無い内壁の
/// 円柱のうち、同じ半径・同じ光軸からの距離に 4 つ以上（90° おき）で、光軸の穴（半径 `bore_r`）の外のもの。
/// 規格に合う組が複数（ロッドの間隔を変える板など）なら間隔の大きい方。規格に合わなければ、ちょうど 4 つの
/// 穴の組（絞りの羽根のピンなど、数の多い穴の組を除く）の外側。穴の並びの順によらない。無ければ None
fn rod_spacing(cx: &Ctx, geos: &[Geo], bore_r: f64) -> Option<f64> {
    let rmax = cx.rmax;
    let (ex, ey) = cx.plane_axes();
    let plane = |p: V3| {
        let d = sub(p, cx.o);
        [dot(d, ex), dot(d, ey)]
    };
    let mut holes: Vec<(f64, [f64; 2])> = Vec::new();
    for g in geos {
        for f in &g.cyls {
            if f.inward && dot(f.axis, cx.axis).abs() > PARALLEL_COS && !cx.coax(f.c, f.axis) {
                let q = plane(f.c);
                if !holes.iter().any(|h| {
                    (h.0 - f.r).abs() <= COAX_REL * rmax
                        && (h.1[0] - q[0]).hypot(h.1[1] - q[1]) <= COAX_REL * rmax
                }) {
                    holes.push((f.r, q));
                }
            }
        }
    }
    // 半径は 0.2 mm まで違ってよい（1 つだけ逃げの大きい穴）
    let same_r = SAME_R_MM / cx.k;
    let cands: Vec<(f64, usize)> = holes
        .iter()
        .filter_map(|h| {
            let rho = h.1[0].hypot(h.1[1]);
            let mut same: Vec<&(f64, [f64; 2])> = Vec::new();
            for x in holes.iter().filter(|x| {
                (x.0 - h.0).abs() <= same_r && (x.1[0].hypot(x.1[1]) - rho).abs() <= COAX_REL * rmax
            }) {
                // 同じ位置の穴は 1 つに数える
                if !same
                    .iter()
                    .any(|y| (y.1[0] - x.1[0]).hypot(y.1[1] - x.1[1]) <= COAX_REL * rmax)
                {
                    same.push(x);
                }
            }
            // 4 つの穴が光軸の周りに 90° おき（正方形の角）
            let perp = same
                .iter()
                .find(|x| (x.1[0] * h.1[0] + x.1[1] * h.1[1]).abs() <= SAME_REL * rho * rho)?;
            // 穴は光軸の穴（内壁）の外
            (same.len() >= 4 && rho - h.0 > bore_r && perp.0 > 0.0).then_some((rho, same.len()))
        })
        .collect();
    let spacing_mm = |rho: f64| rho * std::f64::consts::SQRT_2 * cx.k;
    let standard = |mm: f64| {
        ROD_SPACINGS
            .iter()
            .copied()
            .find(|s| (s - mm).abs() <= 0.05)
    };
    let outer_most = |a: &&(f64, usize), b: &&(f64, usize)| {
        a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
    };
    let rho = cands
        .iter()
        .filter(|r| standard(spacing_mm(r.0)).is_some())
        .max_by(outer_most)
        .or_else(|| cands.iter().filter(|r| r.1 == ROD_HOLES).max_by(outer_most))?
        .0;
    let mm = spacing_mm(rho);
    Some(standard(mm).unwrap_or_else(|| (cx.round)(mm)))
}

/// 端のねじを探す材料: 止め輪を除いた立体、穴の並び [mm]、外形の幅、z の範囲
struct Ends<'a> {
    keep: &'a [&'a Sol],
    bores: &'a [lensbench_project::BoreSpec],
    outer: f64,
    z: [f64; 2],
}

/// 端 `end`（z）のねじ: 端に接する、ねじの谷の径の外向きの円柱（オス、外径より細い）か内壁（メス）
fn end_thread(cx: &Ctx, e: &Ends, end: f64) -> Option<lensbench_project::ThreadEnd> {
    let (rmax, eps) = (cx.rmax, cx.eps);
    let relief = |major: f64| relief_next_to(e.bores, major);
    let thread_of = |r: f64, female: bool| thread_fits(cx.sc(2.0 * r), female, &relief);
    // 端から 1 mm 以内（面取りの分）
    let tol = END_TOL_MM / cx.k;
    let dist = |zz: [f64; 2]| (zz[0] - end).abs().min((zz[1] - end).abs());
    let far = |zz: [f64; 2]| (zz[0] - end).abs().max((zz[1] - end).abs());
    // オス: 外径より細い外向きの円柱が端から続く範囲（ねじと逃げ）がねじ込む長さ
    let narrow: Vec<(f64, [f64; 2])> = e
        .keep
        .iter()
        .flat_map(|x| x.outs.iter().copied())
        .filter(|o| 2.0 * o.0 < e.outer - SAME_REL * rmax)
        .collect();
    if let Some(t) = narrow
        .iter()
        .filter(|o| dist(o.1) <= tol)
        .find_map(|o| thread_of(o.0, false))
    {
        let mut reach = 0.0f64;
        loop {
            let next = narrow
                .iter()
                .filter(|o| dist(o.1) <= reach + tol && far(o.1) > reach + 1e-9)
                .map(|o| far(o.1))
                .fold(reach, f64::max);
            if next <= reach + 1e-9 {
                break;
            }
            reach = next;
        }
        if cx.sc(reach) < MIN_THREAD_MM {
            return None;
        }
        return Some(lensbench_project::ThreadEnd {
            thread: t.name.to_string(),
            male: true,
            length: cx.sc(reach),
            recess: 0.0,
        });
    }
    // メス: 端から最も近い内壁がねじの径（ケージ板のように、ねじの穴の端が外形の端から引っ込んでいても。
    // 引っ込みは recess）
    // 同じ近さの内壁が複数なら、ねじの径のもの、次に細いもの（面をたどる順によらない）
    let nearest = e.keep.iter().flat_map(|x| x.walls.iter()).min_by(|a, b| {
        let (da, db) = (dist(a.1), dist(b.1));
        if (da - db).abs() > eps {
            return da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal);
        }
        let ta = thread_of(a.0, true).is_none();
        let tb = thread_of(b.0, true).is_none();
        ta.cmp(&tb)
            .then(a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    })?;
    let t = thread_of(nearest.0, true)?;
    // ねじの奥: 同じねじの径の内壁が続く範囲（別の立体の短い輪が同じ径の内壁を持っていても、本体の
    // 長い内壁まで。面をたどる順によらない）
    let mut reach = far(nearest.1);
    loop {
        let next = e
            .keep
            .iter()
            .flat_map(|x| x.walls.iter())
            .filter(|w| {
                thread_of(w.0, true).is_some_and(|u| u.name == t.name) && dist(w.1) <= reach + eps
            })
            .map(|w| far(w.1))
            .fold(reach, f64::max);
        if next <= reach + eps {
            break;
        }
        reach = next;
    }
    // ねじの長さは面取りも含める（端から 1 mm 以内の面取りは端まで）
    let total = e.z[1] - e.z[0];
    let start = if dist(nearest.1) <= tol {
        0.0
    } else {
        dist(nearest.1)
    };
    let stop = if total - reach <= tol { total } else { reach };
    (dist(nearest.1) < 0.5 * total && cx.sc(stop - start) >= MIN_THREAD_MM).then(|| {
        lensbench_project::ThreadEnd {
            thread: t.name.to_string(),
            male: false,
            length: cx.sc(stop - start),
            // 1 mm 以内は面取り（オスの肩は端の面に当たる）
            recess: if dist(nearest.1) <= tol {
                0.0
            } else {
                cx.sc(dist(nearest.1))
            },
        }
    })
}

/// 枠の分からない品: 軸に平行な円の最大の直径と、軸の向きの広がり
fn unknown(cx: &Ctx, geos: &[Geo], all: &Geo) -> Res<Measure> {
    let par: Vec<&(V3, V3, f64)> = all
        .circles
        .iter()
        .filter(|c| dot(c.1, cx.axis).abs() > PARALLEL_COS)
        .collect();
    // 光軸と同じ軸の円の最大。無ければ（ロッドの穴の組の中心を光軸にした穴の無い板）頂点の幅の最小の半分
    let r = par
        .iter()
        .filter(|c| cx.coax(c.0, c.1))
        .map(|c| c.2)
        .fold(0.0, f64::max);
    let r = if r > 0.0 {
        r
    } else {
        let w = min_width(cx, all.verts.iter().copied()).0;
        if w.is_finite() {
            0.5 * w
        } else {
            par.iter().map(|c| c.2).fold(0.0, f64::max)
        }
    };
    let (lo, hi) = all
        .verts
        .iter()
        .copied()
        .chain(par.iter().map(|c| c.0))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            let z = cx.z_of(p);
            (lo.min(z), hi.max(z))
        });
    let t = hi - lo;
    if t.is_nan() || t <= 0.0 {
        return Err("no thickness along the circle axis in the STEP".into());
    }
    Ok(Measure {
        thickness: cx.sc(t),
        diameter: json!(cx.sc(2.0 * r)),
        note: if geos.is_empty() {
            format!(
                "largest diameter of {} circles ({} parallel to the axis) and extent of {} vertices along the axis",
                all.circles.len(),
                par.len(),
                all.verts.len()
            )
        } else {
            format!(
                "{} solids, largest diameter of {} circles ({} parallel to the axis) and extent of {} vertices along the axis",
                geos.len(),
                all.circles.len(),
                par.len(),
                all.verts.len()
            )
        },
        frame: None,
        merged: false,
        mech: false,
        retaining_rings: Vec::new(),
        axes: None,
    })
}
