//! STEP（AP203/AP214/AP242）の B-rep を、立体ごとに組立の置き方を当てて読む（カタログの品の寸法を測る用の最小限）。
//!
//! 品ページの STEP は、枠付きの品（マウント済みのフィルタなど）ではガラスと枠が別の立体の組立になっている。
//! 立体はそれぞれの部品の座標で書かれ、組立の置き方（`REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION`）で
//! 組み合わさるので、置き方を当てずに全部の点を合わせると寸法を誤る。ここでは
//! - 立体（`MANIFOLD_SOLID_BREP`・`BREP_WITH_VOIDS`）ごとに、円・頂点・円柱の面を集め、
//! - 組立の置き方を当てて共通の座標にし、
//! - 光軸（最大の円の軸）に沿った z と軸からの距離で、ガラスと枠を見分ける。
//!
//! 枠（環）は、光軸と同じ軸の円柱の面で、外向きの法線が軸を向く（穴の内壁）ものを持つ立体。ガラスはそれを
//! 持たない立体（円板・玉）。

use lensbench_step::{Arg, StepFile};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(crate) type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// 単位の向き a に直交する単位の向き（決まった選び方）
fn perp(a: V3) -> V3 {
    let t = if a[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let c = cross(a, t);
    scale(c, 1.0 / dot(c, c).sqrt())
}

pub(crate) fn unit(a: V3) -> Option<V3> {
    let n = dot(a, a).sqrt();
    (n > 0.0 && n.is_finite()).then(|| [a[0] / n, a[1] / n, a[2] / n])
}

pub(crate) fn triple(a: &Arg) -> Option<V3> {
    let n: Vec<f64> = a.list()?.iter().filter_map(Arg::num).collect();
    match n.as_slice() {
        [x, y, z] => Some([*x, *y, *z]),
        [x, y] => Some([*x, *y, 0.0]),
        _ => None,
    }
}

/// 剛体の変換 p' = R p + t
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rigid {
    r: [[f64; 3]; 3],
    t: V3,
}

impl Rigid {
    const ID: Rigid = Rigid {
        r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0; 3],
    };
    fn rot(&self, v: V3) -> V3 {
        [dot(self.r[0], v), dot(self.r[1], v), dot(self.r[2], v)]
    }
    fn apply(&self, p: V3) -> V3 {
        let q = self.rot(p);
        [q[0] + self.t[0], q[1] + self.t[1], q[2] + self.t[2]]
    }
    fn inv(&self) -> Rigid {
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, x) in row.iter_mut().enumerate() {
                *x = self.r[j][i];
            }
        }
        let t = [-dot(r[0], self.t), -dot(r[1], self.t), -dot(r[2], self.t)];
        Rigid { r, t }
    }
    /// self ∘ b（b を先に当てる）
    fn then_after(&self, b: &Rigid) -> Rigid {
        let mut r = [[0.0; 3]; 3];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, x) in row.iter_mut().enumerate() {
                *x = (0..3).map(|k| self.r[i][k] * b.r[k][j]).sum();
            }
        }
        Rigid {
            r,
            t: self.apply(b.t),
        }
    }
}

/// 読んだ STEP（[`StepFile`]）と、測りに使う表
pub(crate) struct Step<'a> {
    f: &'a StepFile,
    /// 半周以上の弧の円（CIRCLE）の実体（[`Step::big_arcs`]）
    big: HashSet<u64>,
}

impl<'a> Step<'a> {
    pub(crate) fn new(f: &'a StepFile) -> Step<'a> {
        let mut st = Step {
            f,
            big: HashSet::new(),
        };
        st.big = st.big_arcs();
        st
    }

    /// 辺（EDGE_CURVE）で半周以上を回る円（CIRCLE）の実体。同じ円（中心・軸・半径）を分けて描いた辺は回る角を
    /// 足す。光軸を決めるのに使う（ロッドの穴の周りの溝のような、半径の大きい短い弧を除く）
    fn big_arcs(&self) -> HashSet<u64> {
        // 同じ円の弧の組（中心・軸・半径）。中心は半径の 1% まで、半径は 0.1% まで違ってよい（分けて描いた弧の
        // 中心が丸めでずれる STEP がある）
        let mut groups: Vec<(V3, V3, f64, f64)> = Vec::new();
        let mut ids: Vec<(u64, usize)> = Vec::new();
        let mut keys: Vec<u64> = self.f.ids().collect();
        keys.sort_unstable();
        for parts in keys.iter().filter_map(|k| self.f.parts(*k)) {
            let Some(a) = parts
                .iter()
                .find(|(n, _)| n == "EDGE_CURVE")
                .map(|(_, a)| a)
            else {
                continue;
            };
            let Some(cid) = a.get(3).and_then(Arg::r) else {
                continue;
            };
            let Some(ca) = self.part(cid, "CIRCLE") else {
                continue;
            };
            let Some((c, z, rx)) = ca.get(1).and_then(|x| self.placement(x)) else {
                continue;
            };
            let Some(r) = ca.get(2).and_then(Arg::num) else {
                continue;
            };
            let vert = |i: usize| {
                let v = a.get(i)?;
                self.point(self.get(v, "VERTEX_POINT")?.get(1)?)
            };
            let (Some(p1), Some(p2)) = (vert(1), vert(2)) else {
                continue;
            };
            let x0 = rx.unwrap_or_else(|| perp(z));
            let x = unit(sub(x0, scale(z, dot(x0, z)))).unwrap_or_else(|| perp(z));
            let y = cross(z, x);
            let ang = |p: V3| {
                let d = sub(p, c);
                dot(d, y).atan2(dot(d, x))
            };
            let tau = std::f64::consts::TAU;
            let same = !matches!(a.get(4), Some(Arg::Bool(false)));
            let d = dot(sub(p1, p2), sub(p1, p2)).sqrt();
            let w = if d <= 1e-6 * r {
                tau
            } else if same {
                (ang(p2) - ang(p1)).rem_euclid(tau)
            } else {
                (ang(p1) - ang(p2)).rem_euclid(tau)
            };
            let g = match groups.iter().position(|g| {
                dot(g.1, z).abs() > 0.999
                    && (g.2 - r).abs() <= 1e-3 * r
                    && dot(sub(g.0, c), sub(g.0, c)).sqrt() <= 1e-2 * r
            }) {
                Some(i) => i,
                None => {
                    groups.push((c, z, r, 0.0));
                    groups.len() - 1
                }
            };
            groups[g].3 += w;
            ids.push((cid, g));
        }
        ids.into_iter()
            .filter(|(_, g)| groups[*g].3 >= 0.99 * std::f64::consts::PI)
            .map(|(id, _)| id)
            .collect()
    }

    /// 実体 id の部分 ty の値
    fn part(&self, id: u64, ty: &str) -> Option<&'a Vec<Arg>> {
        self.f
            .parts(id)?
            .iter()
            .find(|(n, _)| n == ty)
            .map(|(_, a)| a)
    }
    fn get(&self, a: &Arg, ty: &str) -> Option<&Vec<Arg>> {
        self.part(a.r()?, ty)
    }
    fn point(&self, a: &Arg) -> Option<V3> {
        self.get(a, "CARTESIAN_POINT")?.get(1).and_then(triple)
    }
    fn dir(&self, a: &Arg) -> Option<V3> {
        self.get(a, "DIRECTION")?
            .get(1)
            .and_then(triple)
            .and_then(unit)
    }
    /// AXIS2_PLACEMENT_3D: (原点, 軸, 基準の向き)
    fn placement(&self, a: &Arg) -> Option<(V3, V3, Option<V3>)> {
        let v = self.get(a, "AXIS2_PLACEMENT_3D")?;
        let c = self.point(v.get(1)?)?;
        let z = v
            .get(2)
            .and_then(|x| self.dir(x))
            .unwrap_or([0.0, 0.0, 1.0]);
        Some((c, z, v.get(3).and_then(|x| self.dir(x))))
    }
    /// AXIS2_PLACEMENT_3D の座標（局所 → 外）
    fn frame(&self, a: &Arg) -> Option<Rigid> {
        let (o, z, rx) = self.placement(a)?;
        let x0 = rx.unwrap_or(if z[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        });
        let x = unit(sub(x0, scale(z, dot(x0, z))))?;
        let y = cross(z, x);
        Some(Rigid {
            r: [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]],
            t: o,
        })
    }

    /// 円（CIRCLE・CYLINDRICAL_SURFACE）: (中心, 軸, 半径)
    fn circle(&self, id: u64) -> Option<(V3, V3, f64)> {
        for ty in ["CIRCLE", "CYLINDRICAL_SURFACE"] {
            if let Some(a) = self.part(id, ty) {
                let (c, z, _) = a.get(1).and_then(|x| self.placement(x))?;
                let r = a.get(2)?.num()?;
                return (r > 0.0).then_some((c, z, r));
            }
        }
        None
    }

    /// id からたどれる実体
    fn reach(&self, start: &[u64]) -> HashSet<u64> {
        let mut seen = HashSet::new();
        let mut st: Vec<u64> = start.to_vec();
        while let Some(i) = st.pop() {
            if !seen.insert(i) {
                continue;
            }
            if let Some(ps) = self.f.parts(i) {
                for (_, args) in ps {
                    push_refs(args, &mut st);
                }
            }
        }
        seen
    }
}

fn push_refs(args: &[Arg], st: &mut Vec<u64>) {
    for a in args {
        match a {
            Arg::Ref(r) => st.push(*r),
            Arg::List(v) => push_refs(v, st),
            _ => {}
        }
    }
}

/// 円柱の面
#[derive(Debug, Clone)]
struct CylFace {
    c: V3,
    axis: V3,
    r: f64,
    /// 外向きの法線が軸を向く（穴の内壁）
    inward: bool,
    /// 面の頂点と縁の円の中心（z の範囲を見る）
    pts: Vec<V3>,
}

/// 立体 1 つの形（部品の座標か、置き方を当てた共通の座標）
#[derive(Debug, Clone, Default)]
struct Geo {
    circles: Vec<(V3, V3, f64)>,
    /// 半周以上の弧の円（[`Step::big_arcs`]）
    arcs: Vec<(V3, V3, f64)>,
    verts: Vec<V3>,
    cyls: Vec<CylFace>,
    /// 縁が 1 つの円だけの面（円板・球の帽子）: (円の中心, 軸, 半径)。光軸をふさぐ面（ガラスの面）
    caps: Vec<(V3, V3, f64)>,
}

impl Geo {
    fn moved(&self, m: &Rigid) -> Geo {
        Geo {
            circles: self
                .circles
                .iter()
                .map(|(c, a, r)| (m.apply(*c), m.rot(*a), *r))
                .collect(),
            arcs: self
                .arcs
                .iter()
                .map(|(c, a, r)| (m.apply(*c), m.rot(*a), *r))
                .collect(),
            verts: self.verts.iter().map(|p| m.apply(*p)).collect(),
            cyls: self
                .cyls
                .iter()
                .map(|f| CylFace {
                    c: m.apply(f.c),
                    axis: m.rot(f.axis),
                    r: f.r,
                    inward: f.inward,
                    pts: f.pts.iter().map(|p| m.apply(*p)).collect(),
                })
                .collect(),
            caps: self
                .caps
                .iter()
                .map(|(c, a, r)| (m.apply(*c), m.rot(*a), *r))
                .collect(),
        }
    }
}

/// 立体の名前
const SOLIDS: [&str; 3] = [
    "MANIFOLD_SOLID_BREP",
    "BREP_WITH_VOIDS",
    "SHELL_BASED_SURFACE_MODEL",
];

impl<'a> Step<'a> {
    /// 実体の集まりの円と頂点
    fn geo_of(&self, set: &HashSet<u64>) -> Geo {
        let mut g = Geo::default();
        for &j in set {
            if let Some(c) = self.circle(j) {
                g.circles.push(c);
                if self.big.contains(&j) {
                    g.arcs.push(c);
                }
            }
            if let Some(p) = self.part(j, "VERTEX_POINT").and_then(|a| a.get(1)) {
                if let Some(p) = self.point(p) {
                    g.verts.push(p);
                }
            }
        }
        g
    }

    /// 立体の形（部品の座標）
    fn solid_geo(&self, id: u64) -> Geo {
        let set = self.reach(&[id]);
        let mut g = self.geo_of(&set);
        for &j in &set {
            let Some(a) = self
                .part(j, "ADVANCED_FACE")
                .or_else(|| self.part(j, "FACE_SURFACE"))
            else {
                continue;
            };
            let Some(geom) = a.get(2).and_then(Arg::r) else {
                continue;
            };
            if self.part(geom, "CYLINDRICAL_SURFACE").is_none() {
                // 縁が 1 つの円だけの面（円柱・円錐でない）は帽子
                let one = a.get(1).and_then(Arg::list).is_some_and(|b| b.len() == 1);
                if one && self.part(geom, "CONICAL_SURFACE").is_none() {
                    let mut st = Vec::new();
                    push_refs(std::slice::from_ref(&a[1]), &mut st);
                    let fg = self.geo_of(&self.reach(&st));
                    if let Some(&(c, ax, r)) = fg.circles.first() {
                        let tol = 1e-6 * r + 1e-9;
                        let same = fg.circles.iter().all(|q| {
                            (q.2 - r).abs() <= tol && dot(sub(q.0, c), sub(q.0, c)).sqrt() <= tol
                        });
                        let on_rim = fg.verts.iter().all(|v| {
                            let d = sub(*v, c);
                            let z = dot(d, ax);
                            ((dot(d, d) - z * z).max(0.0).sqrt() - r).abs() <= tol
                        });
                        if same && on_rim {
                            g.caps.push((c, ax, r));
                        }
                    }
                }
                continue;
            }
            let Some((c, axis, r)) = self.circle(geom) else {
                continue;
            };
            let same = !matches!(a.get(3), Some(Arg::Bool(false)));
            let mut st = Vec::new();
            if let Some(b) = a.get(1) {
                push_refs(std::slice::from_ref(b), &mut st);
            }
            let fg = self.geo_of(&self.reach(&st));
            let pts = fg
                .verts
                .iter()
                .copied()
                .chain(fg.circles.iter().map(|c| c.0))
                .collect();
            g.cyls.push(CylFace {
                c,
                axis,
                r,
                inward: !same,
                pts,
            });
        }
        g
    }

    /// 立体を共通の座標に置く変換（組立で同じ部品を何度も使えば、その数だけ）
    fn instances(&self) -> Vec<(u64, Rigid)> {
        let solids: Vec<u64> = {
            let mut v: Vec<u64> = self
                .f
                .iter()
                .filter(|(_, ps)| ps.iter().any(|(n, _)| SOLIDS.contains(&n.as_str())))
                .map(|(i, _)| i)
                .collect();
            v.sort_unstable();
            v
        };
        if solids.is_empty() {
            return Vec::new();
        }
        // 表現（…REPRESENTATION）とその項目
        let mut rep_of_solid: HashMap<u64, u64> = HashMap::new();
        let mut reps: Vec<u64> = Vec::new();
        for (id, ps) in self.f.iter() {
            for (n, a) in ps {
                if n.ends_with("REPRESENTATION") && !n.contains("RELATIONSHIP") {
                    if let Some(items) = a.get(1).and_then(Arg::list) {
                        reps.push(id);
                        for it in items.iter().filter_map(Arg::r) {
                            if solids.contains(&it) {
                                rep_of_solid.insert(it, id);
                            }
                        }
                    }
                }
            }
        }
        // 置き方の無い関係（同じ座標）でつながる表現は同じ組
        let mut uf: HashMap<u64, u64> = reps.iter().map(|r| (*r, *r)).collect();
        fn find(uf: &mut HashMap<u64, u64>, x: u64) -> u64 {
            let mut r = x;
            let mut n = 0;
            while let Some(&p) = uf.get(&r) {
                if p == r || n > 10_000 {
                    break;
                }
                r = p;
                n += 1;
            }
            uf.insert(x, r);
            r
        }
        let mut rels: Vec<(u64, u64, u64, Option<u64>)> = Vec::new(); // (関係, rep1, rep2, 置き方)
        for (id, ps) in self.f.iter() {
            let rr = ps.iter().find(|(n, _)| {
                n == "REPRESENTATION_RELATIONSHIP" || n == "SHAPE_REPRESENTATION_RELATIONSHIP"
            });
            let Some((_, a)) = rr else { continue };
            let (Some(r1), Some(r2)) = (a.get(2).and_then(Arg::r), a.get(3).and_then(Arg::r))
            else {
                continue;
            };
            let tr = ps
                .iter()
                .find(|(n, _)| n == "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION")
                .and_then(|(_, a)| a.first().and_then(Arg::r));
            rels.push((id, r1, r2, tr));
        }
        for &(_, r1, r2, tr) in &rels {
            if tr.is_none() {
                let a = find(&mut uf, r1);
                let b = find(&mut uf, r2);
                if a != b {
                    uf.insert(a, b);
                }
            }
        }
        // 組立の親子: NEXT_ASSEMBLY_USAGE_OCCURRENCE の子の品の表現
        let mut rep_of_pd: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut child_of_rel: HashMap<u64, u64> = HashMap::new();
        let pds_def = |pds: &Arg| -> Option<u64> {
            self.get(pds, "PRODUCT_DEFINITION_SHAPE")?
                .get(2)
                .and_then(Arg::r)
        };
        for (_, ps) in self.f.iter() {
            for (n, a) in ps {
                if n == "SHAPE_DEFINITION_REPRESENTATION" {
                    if let (Some(d), Some(rep)) =
                        (a.first().and_then(pds_def), a.get(1).and_then(Arg::r))
                    {
                        rep_of_pd.entry(d).or_default().push(rep);
                    }
                }
            }
        }
        for (_, ps) in self.f.iter() {
            for (n, a) in ps {
                if n == "CONTEXT_DEPENDENT_SHAPE_REPRESENTATION" {
                    let (Some(rel), Some(nauo)) =
                        (a.first().and_then(Arg::r), a.get(1).and_then(pds_def))
                    else {
                        continue;
                    };
                    let Some(child_pd) = self
                        .part(nauo, "NEXT_ASSEMBLY_USAGE_OCCURRENCE")
                        .and_then(|v| v.get(4))
                        .and_then(Arg::r)
                    else {
                        continue;
                    };
                    if let Some(rep) = rep_of_pd.get(&child_pd).and_then(|v| v.first()) {
                        child_of_rel.insert(rel, *rep);
                    }
                }
            }
        }
        // 子の組 → (親の組, 子の座標から親の座標へ)
        let mut up: HashMap<u64, Vec<(u64, Rigid)>> = HashMap::new();
        for &(id, r1, r2, tr) in &rels {
            let Some(tr) = tr else { continue };
            let Some(t) = self.part(tr, "ITEM_DEFINED_TRANSFORMATION") else {
                continue;
            };
            let (Some(f1), Some(f2)) = (
                t.get(2).and_then(|x| self.frame(x)),
                t.get(3).and_then(|x| self.frame(x)),
            ) else {
                continue;
            };
            let c1 = find(&mut uf, r1);
            let c2 = find(&mut uf, r2);
            // 子が分からなければ rep_2 を子とする
            let child_is_1 = child_of_rel
                .get(&id)
                .is_some_and(|c| find(&mut uf, *c) == c1);
            let (child, parent, fc, fp) = if child_is_1 {
                (c1, c2, f1, f2)
            } else {
                (c2, c1, f2, f1)
            };
            if child == parent {
                continue;
            }
            up.entry(child)
                .or_default()
                .push((parent, fp.then_after(&fc.inv())));
        }
        // 組の、根までの変換
        fn to_root(
            c: u64,
            up: &HashMap<u64, Vec<(u64, Rigid)>>,
            depth: usize,
            out: &mut Vec<Rigid>,
        ) {
            if out.len() >= 256 {
                return;
            }
            match up.get(&c) {
                Some(es) if depth < 16 => {
                    for (p, m) in es {
                        let mut ps = Vec::new();
                        to_root(*p, up, depth + 1, &mut ps);
                        out.extend(ps.iter().map(|q| q.then_after(m)));
                    }
                }
                _ => out.push(Rigid::ID),
            }
        }
        let mut out = Vec::new();
        for s in solids {
            let ms = match rep_of_solid.get(&s) {
                Some(r) => {
                    let c = find(&mut uf, *r);
                    let mut v = Vec::new();
                    to_root(c, &up, 0, &mut v);
                    v
                }
                None => vec![Rigid::ID],
            };
            out.extend(ms.into_iter().map(|m| (s, m)));
        }
        out
    }
}

/// STEP を測った結果（長さは STEP の単位）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Measure {
    pub thickness: f64,
    /// 径（数か [幅, 高さ]）
    pub diameter: Value,
    pub note: String,
    /// 枠付きの品の枠（ガラスは thickness・diameter）
    pub frame: Option<lensbench_project::MechSpec>,
    /// ガラスと枠が 1 つの立体（ガラスの径は見えている径）
    pub merged: bool,
    /// ガラスの無い機構部品（厚さ・径は枠の長さ・外径）
    pub mech: bool,
    /// 機構部品の付属の止め輪（枠に含めない）
    pub retaining_rings: Vec<RetainingRing>,
    /// 枠・機構部品の座標（STEP の座標・単位）: 光軸の上の点・光軸の向き・局所の x 軸・枠の端の z（光軸の向きの
    /// `dot(p - o, axis)`）。`frame` の寸法はこの座標で測っている
    pub axes: Option<(V3, V3, V3, f64)>,
}

fn fmt(x: f64) -> String {
    let r = (x * 1e4).round() / 1e4;
    format!("{r}")
}

use super::{Res, RetainingRing};

mod notes;

mod measure;
pub(crate) use measure::measure;
#[cfg(test)]
use measure::pick_hold;

/// STEP の立体の殻（`CLOSED_SHELL` の実体の番号）ごとの、STEP の座標から部品の座標 [mm] への変換（3 行 4 列、
/// `[R | t]`）。部品の座標は [`measure`] で枠・機構部品として測った座標（z は枠の端から光軸の向き、x は角の部品の
/// 辺に直交）。同じ殻が組立で何度も置かれていれば、置き方ごとに 1 つ
pub(crate) fn shell_poses(f: &StepFile, k: f64) -> Res<Vec<super::ShellPose>> {
    let m = measure(f, k, |x| x)?;
    let Some((o, axis, x, z0)) = m.axes else {
        return Err("the mount or mechanical part coordinates can't be found from the STEP (no solid with a bore)".into());
    };
    let y = cross(axis, x);
    let st = Step::new(f);
    let mut out = Vec::new();
    for (solid, rg) in st.instances() {
        let Some(shell) = st.f.parts(solid).and_then(|ps| {
            ps.iter()
                .find(|(n, _)| SOLIDS.contains(&n.as_str()))
                .and_then(|(_, a)| a.get(1).and_then(Arg::r))
        }) else {
            continue;
        };
        // p → k·B·(R p + t − o) − k·z0·e_z（B の行は x・y・axis）
        let mut mtx = [[0.0; 4]; 3];
        for (i, b) in [x, y, axis].into_iter().enumerate() {
            for (j, v) in mtx[i].iter_mut().take(3).enumerate() {
                *v = k * (0..3).map(|l| b[l] * rg.r[l][j]).sum::<f64>();
            }
            mtx[i][3] = k * dot(b, sub(rg.t, o)) - if i == 2 { k * z0 } else { 0.0 };
        }
        out.push((shell, mtx));
    }
    Ok(out)
}

trait SubSq {
    fn sub_sq(self, b: f64) -> f64;
}
impl SubSq for f64 {
    /// 負にならない差（丸め誤差で負の平方根を取らない）
    fn sub_sq(self, b: f64) -> f64 {
        (self - b).max(0.0)
    }
}

/// 円の無い品: 頂点（無ければ全部の点）の箱
fn boxed(st: &Step, all: &Geo, sc: impl Fn(f64) -> f64) -> Res<Measure> {
    let pts: Vec<V3> = if all.verts.len() >= 2 {
        all.verts.clone()
    } else {
        st.f.iter()
            .filter_map(|(_, ps)| ps.iter().find(|(n, _)| n == "CARTESIAN_POINT"))
            .filter_map(|(_, a)| a.get(1).and_then(triple))
            .collect()
    };
    if pts.len() < 2 {
        return Err("no points in the STEP".into());
    }
    let mut ext = [0.0; 3];
    for (a, e) in ext.iter_mut().enumerate() {
        let (lo, hi) = pts
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p[a]), hi.max(p[a]))
            });
        *e = hi - lo;
    }
    ext.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if ext[0].is_nan() || ext[0] <= 0.0 {
        return Err("the STEP shape has no thickness".into());
    }
    let d = if (ext[1] - ext[2]).abs() < 1e-9 {
        json!(sc(ext[2]))
    } else {
        json!([sc(ext[1]), sc(ext[2])])
    };
    Ok(Measure {
        thickness: sc(ext[0]),
        diameter: d,
        note: format!(
            "no circles, so a box of {} vertices (along the coordinate axes)",
            pts.len()
        ),
        frame: None,
        merged: false,
        mech: false,
        retaining_rings: Vec::new(),
        axes: None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// 枠（外径 25、長さ 3.5、z 0〜0.4 は内径 21.2 の縁、その先は内径 24.08）と、厚さ 2・径 23.9 のガラスの組立。
    /// ガラスは部品の座標で z 0〜2 に書き、組立で z 0.4 へ置く。`nauo` なら組立の関係を子 → 親の順に書き、
    /// 親子は NEXT_ASSEMBLY_USAGE_OCCURRENCE で示す（無ければ rep_1 が親）
    pub(crate) fn assembly_step(nauo: bool) -> String {
        let mut d = String::from(
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));\nENDSEC;\nDATA;\n\
#1=CARTESIAN_POINT('',(0.,0.,0.));\n#2=DIRECTION('',(0.,0.,1.));\n#3=DIRECTION('',(1.,0.,0.));\n\
#4=AXIS2_PLACEMENT_3D('',#1,#2,#3);\n\
#10=CARTESIAN_POINT('',(0.,0.,2.));\n#11=AXIS2_PLACEMENT_3D('',#10,#2,#3);\n#12=CIRCLE('',#4,11.95);\n\
#13=CIRCLE('',#11,11.95);\n#14=CYLINDRICAL_SURFACE('',#4,11.95);\n#15=CARTESIAN_POINT('',(11.95,0.,0.));\n\
#16=VERTEX_POINT('',#15);\n#17=CARTESIAN_POINT('',(11.95,0.,2.));\n#18=VERTEX_POINT('',#17);\n\
#19=ADVANCED_FACE('',(#12,#13,#16,#18),#14,.T.);\n#20=CLOSED_SHELL('',(#19));\n\
#21=MANIFOLD_SOLID_BREP('glass',#20);\n\
#30=CARTESIAN_POINT('',(0.,0.,3.5));\n#31=AXIS2_PLACEMENT_3D('',#30,#2,#3);\n\
#32=CARTESIAN_POINT('',(0.,0.,0.4));\n#33=AXIS2_PLACEMENT_3D('',#32,#2,#3);\n\
#34=CIRCLE('',#4,12.5);\n#35=CIRCLE('',#31,12.5);\n#36=CYLINDRICAL_SURFACE('',#4,12.5);\n\
#38=CARTESIAN_POINT('',(12.5,0.,0.));\n#37=VERTEX_POINT('',#38);\n\
#40=CARTESIAN_POINT('',(12.5,0.,3.5));\n#39=VERTEX_POINT('',#40);\n\
#41=ADVANCED_FACE('',(#34,#35,#37,#39),#36,.T.);\n\
#42=CYLINDRICAL_SURFACE('',#4,10.6);\n#43=CIRCLE('',#4,10.6);\n#44=CIRCLE('',#33,10.6);\n\
#45=ADVANCED_FACE('',(#43,#44),#42,.F.);\n\
#46=CYLINDRICAL_SURFACE('',#4,12.04);\n#47=CIRCLE('',#33,12.04);\n#48=CIRCLE('',#31,12.04);\n\
#49=ADVANCED_FACE('',(#47,#48),#46,.F.);\n#50=CLOSED_SHELL('',(#41,#45,#49));\n\
#51=MANIFOLD_SOLID_BREP('frame',#50);\n\
#60=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#21,#4),#99);\n\
#61=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#51,#4),#99);\n\
#64=CARTESIAN_POINT('',(0.,0.,0.4));\n#63=AXIS2_PLACEMENT_3D('',#64,#2,#3);\n\
#62=SHAPE_REPRESENTATION('asm',(#4,#63),#99);\n\
#72=ITEM_DEFINED_TRANSFORMATION('','',#4,#4);\n",
        );
        if nauo {
            d += "#70=ITEM_DEFINED_TRANSFORMATION('','',#4,#63);\n\
#71=( REPRESENTATION_RELATIONSHIP('','',#60,#62) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#70) SHAPE_REPRESENTATION_RELATIONSHIP() );\n\
#73=( REPRESENTATION_RELATIONSHIP('','',#61,#62) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#72) SHAPE_REPRESENTATION_RELATIONSHIP() );\n\
#80=PRODUCT_DEFINITION('','',#98,#97);\n#81=PRODUCT_DEFINITION('','',#98,#97);\n#82=PRODUCT_DEFINITION('','',#98,#97);\n\
#83=PRODUCT_DEFINITION_SHAPE('','',#80);\n#84=SHAPE_DEFINITION_REPRESENTATION(#83,#62);\n\
#85=PRODUCT_DEFINITION_SHAPE('','',#81);\n#86=SHAPE_DEFINITION_REPRESENTATION(#85,#60);\n\
#87=PRODUCT_DEFINITION_SHAPE('','',#82);\n#88=SHAPE_DEFINITION_REPRESENTATION(#87,#61);\n\
#89=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#80,#81,$);\n#90=PRODUCT_DEFINITION_SHAPE('','',#89);\n\
#91=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#71,#90);\n\
#92=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#80,#82,$);\n#93=PRODUCT_DEFINITION_SHAPE('','',#92);\n\
#94=CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#73,#93);\n";
        } else {
            d += "#70=ITEM_DEFINED_TRANSFORMATION('','',#63,#4);\n\
#71=( REPRESENTATION_RELATIONSHIP('','',#62,#60) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#70) SHAPE_REPRESENTATION_RELATIONSHIP() );\n\
#73=( REPRESENTATION_RELATIONSHIP('','',#62,#61) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#72) SHAPE_REPRESENTATION_RELATIONSHIP() );\n";
        }
        d += "ENDSEC;\nEND-ISO-10303-21;\n";
        d
    }

    /// 組立の置き方を当てて、ガラスと枠を分けて測る（親子の書き方の 2 通り）
    #[test]
    fn framed_assembly_splits_glass_and_frame() {
        for nauo in [false, true] {
            let s = crate::step::read_step(&assembly_step(nauo)).unwrap();
            assert_eq!(s.thickness, 2.0, "{nauo}: {s:?}");
            assert_eq!(s.diameter, json!(23.9));
            let f = s.frame.expect("枠");
            assert_eq!(f.length, 3.5);
            assert_eq!(f.outer_diameter, 25.0);
            let b: Vec<[f64; 3]> = f.bores.iter().map(|b| [b.z0, b.z1, b.diameter]).collect();
            assert_eq!(b, vec![[0.0, 0.4, 21.2], [0.4, 3.5, 24.08]], "{nauo}");
            let h = f.holds.unwrap();
            assert_eq!((h.seat_z, h.diameter), (0.4, 24.08), "{nauo}");
        }
    }

    /// ガラスと枠が 1 つの立体（外径 25.4、長さ 6.3、両端の縁は内径 21、z ±2.65 にガラスの面）
    #[test]
    fn merged_solid_splits_between_caps() {
        let d = "ISO-10303-21;\nDATA;\n\
#1=DIRECTION('',(0.,0.,1.));\n#2=DIRECTION('',(1.,0.,0.));\n\
#3=CARTESIAN_POINT('',(0.,0.,-3.15));\n#4=AXIS2_PLACEMENT_3D('',#3,#1,#2);\n\
#5=CARTESIAN_POINT('',(0.,0.,3.15));\n#6=AXIS2_PLACEMENT_3D('',#5,#1,#2);\n\
#7=CARTESIAN_POINT('',(0.,0.,-2.65));\n#8=AXIS2_PLACEMENT_3D('',#7,#1,#2);\n\
#9=CARTESIAN_POINT('',(0.,0.,2.65));\n#10=AXIS2_PLACEMENT_3D('',#9,#1,#2);\n\
#11=CIRCLE('',#4,12.7);\n#12=CIRCLE('',#6,12.7);\n#13=CYLINDRICAL_SURFACE('',#4,12.7);\n\
#14=ADVANCED_FACE('',(#11,#12),#13,.T.);\n\
#15=CIRCLE('',#4,10.5);\n#16=CIRCLE('',#8,10.5);\n#17=CYLINDRICAL_SURFACE('',#4,10.5);\n\
#18=ADVANCED_FACE('',(#15,#16),#17,.F.);\n\
#19=CIRCLE('',#10,10.5);\n#20=CIRCLE('',#6,10.5);\n#21=ADVANCED_FACE('',(#19,#20),#17,.F.);\n\
#22=PLANE('',#8);\n#23=FACE_OUTER_BOUND('',#16,.T.);\n#24=ADVANCED_FACE('',(#23),#22,.F.);\n\
#25=PLANE('',#10);\n#26=FACE_OUTER_BOUND('',#19,.T.);\n#27=ADVANCED_FACE('',(#26),#25,.T.);\n\
#28=PLANE('',#4);\n#29=FACE_OUTER_BOUND('',#11,.T.);\n#30=FACE_BOUND('',#15,.T.);\n\
#31=ADVANCED_FACE('',(#29,#30),#28,.F.);\n\
#32=CLOSED_SHELL('',(#14,#18,#21,#24,#27,#31));\n#33=MANIFOLD_SOLID_BREP('one',#32);\nENDSEC;\n";
        let s = crate::step::read_step(d).unwrap();
        assert_eq!(
            (s.thickness, s.diameter.clone()),
            (5.3, json!(21.0)),
            "{s:?}"
        );
        let f = s.frame.expect("枠");
        assert_eq!((f.length, f.outer_diameter), (6.3, 25.4));
        let b: Vec<[f64; 3]> = f.bores.iter().map(|b| [b.z0, b.z1, b.diameter]).collect();
        assert_eq!(b, vec![[0.0, 6.3, 21.0]]);
        assert_eq!(f.holds.unwrap().seat_z, 0.5);
        assert!(s.note.contains("one solid"), "{}", s.note);
    }

    /// ガラスの無い機構部品: SM1 のねじ（内径 1.035 in）のチューブ（端に内径 24 の段、付属の止め輪つき）と、30 mm ケージ板（一辺 40.64、ロッドの穴 4 つ）
    #[test]
    fn mech_parts_are_measured_as_frames() {
        let tube = "ISO-10303-21;\nDATA;\n\
#1=DIRECTION('',(0.,0.,1.));\n#2=DIRECTION('',(1.,0.,0.));\n\
#3=CARTESIAN_POINT('',(0.,0.,0.));\n#4=AXIS2_PLACEMENT_3D('',#3,#1,#2);\n\
#5=CARTESIAN_POINT('',(0.,0.,10.));\n#6=AXIS2_PLACEMENT_3D('',#5,#1,#2);\n\
#11=CIRCLE('',#4,15.24);\n#12=CIRCLE('',#6,15.24);\n#13=CYLINDRICAL_SURFACE('',#4,15.24);\n\
#14=ADVANCED_FACE('',(#11,#12),#13,.T.);\n\
#7=CARTESIAN_POINT('',(0.,0.,2.));\n#8=AXIS2_PLACEMENT_3D('',#7,#1,#2);\n\
#15=CIRCLE('',#8,13.1445);\n#16=CIRCLE('',#6,13.1445);\n#17=CYLINDRICAL_SURFACE('',#8,13.1445);\n\
#18=ADVANCED_FACE('',(#15,#16),#17,.F.);\n\
#19=CIRCLE('',#4,12.);\n#20=CIRCLE('',#8,12.);\n#21=CYLINDRICAL_SURFACE('',#4,12.);\n\
#22=ADVANCED_FACE('',(#19,#20),#21,.F.);\n\
#32=CLOSED_SHELL('',(#14,#18,#22));\n#33=MANIFOLD_SOLID_BREP('tube',#32);\n\
#40=CARTESIAN_POINT('',(0.,0.,8.));\n#41=AXIS2_PLACEMENT_3D('',#40,#1,#2);\n\
#42=CIRCLE('',#41,13.1);\n#43=CIRCLE('',#6,13.1);\n#44=CYLINDRICAL_SURFACE('',#41,13.1);\n\
#45=ADVANCED_FACE('',(#42,#43),#44,.T.);\n\
#46=CIRCLE('',#41,11.4554);\n#47=CIRCLE('',#6,11.4554);\n#48=CYLINDRICAL_SURFACE('',#41,11.4554);\n\
#49=ADVANCED_FACE('',(#46,#47),#48,.F.);\n\
#50=CLOSED_SHELL('',(#45,#49));\n#51=MANIFOLD_SOLID_BREP('ring',#50);\nENDSEC;\n";
        let s = crate::step::read_step(tube).unwrap();
        assert!(s.mech, "{s:?}");
        let f = s.frame.clone().expect("枠");
        assert_eq!((f.length, f.outer_diameter), (10.0, 30.48));
        assert_eq!(f.outline, lensbench_project::MechOutline::Round);
        let b: Vec<[f64; 3]> = f.bores.iter().map(|b| [b.z0, b.z1, b.diameter]).collect();
        assert_eq!(b, vec![[0.0, 2.0, 24.0], [2.0, 10.0, 26.289]]);
        assert_eq!(f.thread, "SM1");
        // 座は、保持する素子（25.4）より細い z 0〜2 の穴の段
        assert_eq!(f.holds.map(|h| (h.diameter, h.seat_z)), Some((25.4, 2.0)));
        assert!(s.note.contains("bore step narrower"), "{}", s.note);
        // 後ろの端は SM1 のメス（z 2〜10 の内壁）、前の端はねじでない
        assert_eq!(f.front_thread, None);
        let b = f.back_thread.clone().unwrap();
        assert_eq!((b.thread.as_str(), b.male, b.length), ("SM1", false, 8.0));
        assert_eq!(f.rod_spacing, 0.0);
        assert_eq!((s.thickness, s.diameter.clone()), (10.0, json!(30.48)));
        // 付属の止め輪（z 8〜10、内径 22.91）は枠の穴に含めず、別に返す
        assert_eq!(s.retaining_rings.len(), 1, "{s:?}");
        let r = s.retaining_rings[0];
        assert_eq!(
            (r.z0, r.z1, r.inner_diameter, r.outer_diameter),
            (8.0, 10.0, 22.9108, 26.2)
        );
        assert!(
            s.note.contains("the supplied retaining ring ("),
            "{}",
            s.note
        );
        // ケージ板: 角の頂点 8 つ、光軸の穴（内径 22.9）、ロッドの穴（内径 6）が ±15 mm に 4 つ
        let mut d = String::from(
            "ISO-10303-21;\nDATA;\n#1=DIRECTION('',(0.,0.,1.));\n#2=DIRECTION('',(1.,0.,0.));\n\
#3=CARTESIAN_POINT('',(0.,0.,0.));\n#4=AXIS2_PLACEMENT_3D('',#3,#1,#2);\n\
#5=CARTESIAN_POINT('',(0.,0.,8.));\n#6=AXIS2_PLACEMENT_3D('',#5,#1,#2);\n\
#15=CIRCLE('',#4,11.45);\n#16=CIRCLE('',#6,11.45);\n#17=CYLINDRICAL_SURFACE('',#4,11.45);\n\
#18=ADVANCED_FACE('',(#15,#16),#17,.F.);\n",
        );
        let mut faces = vec!["#18".to_string()];
        let mut id = 100;
        for (x, y) in [(15.0, 15.0), (-15.0, 15.0), (-15.0, -15.0), (15.0, -15.0)] {
            d += &format!(
                "#{a}=CARTESIAN_POINT('',({x},{y},0.));\n#{b}=AXIS2_PLACEMENT_3D('',#{a},#1,#2);\n\
#{c}=CARTESIAN_POINT('',({x},{y},8.));\n#{e}=AXIS2_PLACEMENT_3D('',#{c},#1,#2);\n\
#{f}=CIRCLE('',#{b},3.);\n#{g}=CIRCLE('',#{e},3.);\n#{h}=CYLINDRICAL_SURFACE('',#{b},3.);\n\
#{i}=ADVANCED_FACE('',(#{f},#{g}),#{h},.F.);\n",
                a = id,
                b = id + 1,
                c = id + 2,
                e = id + 3,
                f = id + 4,
                g = id + 5,
                h = id + 6,
                i = id + 7
            );
            faces.push(format!("#{}", id + 7));
            id += 10;
        }
        let mut verts = Vec::new();
        for (x, y) in [
            (20.32, 20.32),
            (-20.32, 20.32),
            (-20.32, -20.32),
            (20.32, -20.32),
        ] {
            for z in [0.0, 8.0] {
                d += &format!(
                    "#{id}=CARTESIAN_POINT('',({x},{y},{z}));\n#{}=VERTEX_POINT('',#{id});\n",
                    id + 1
                );
                verts.push(format!("#{}", id + 1));
                id += 2;
            }
        }
        d += &format!(
            "#{id}=PLANE('',#4);\n#{}=ADVANCED_FACE('',({}),#{id},.T.);\n",
            id + 1,
            verts.join(",")
        );
        faces.push(format!("#{}", id + 1));
        d += &format!(
            "#900=CLOSED_SHELL('',({}));\n#901=MANIFOLD_SOLID_BREP('plate',#900);\nENDSEC;\n",
            faces.join(",")
        );
        let s = crate::step::read_step(&d).unwrap();
        let f = s.frame.clone().expect("枠");
        assert!(s.mech);
        assert_eq!(
            (f.length, f.outer_diameter, f.rod_spacing),
            (8.0, 40.64, 30.0),
            "{s:?}"
        );
        assert_eq!(f.outline, lensbench_project::MechOutline::Square);
        assert_eq!(f.thread, "");
        assert_eq!(f.holds.unwrap().diameter, 22.9);
        assert!(s.note.contains("cage plate"), "{}", s.note);
    }

    /// 手元の STEP を測って見る（`LB_STEP_FILES` に ; 区切りのパス）
    #[test]
    #[ignore]
    fn print_local_steps() {
        let Ok(v) = std::env::var("LB_STEP_FILES") else {
            return;
        };
        for p in v.split(';').filter(|x| !x.is_empty()) {
            let t = String::from_utf8_lossy(&std::fs::read(p).unwrap()).into_owned();
            println!("== {p}\n{:#?}", crate::step::read_step(&t));
        }
    }

    /// 円柱の面（`inward` なら内壁）を 1 つ書く。返すのは面の番号
    fn cyl_face(
        d: &mut String,
        id: &mut u32,
        c: (f64, f64),
        z: (f64, f64),
        r: f64,
        inward: bool,
    ) -> String {
        arc_face(d, id, c, z, r, inward, None)
    }

    /// 円柱の面。縁は辺（EDGE_CURVE）で、`span` が無ければ全周、あれば角度 [rad] の範囲の弧
    fn arc_face(
        d: &mut String,
        id: &mut u32,
        c: (f64, f64),
        z: (f64, f64),
        r: f64,
        inward: bool,
        span: Option<(f64, f64)>,
    ) -> String {
        let n = *id;
        let (x, y) = c;
        let (t0, t1) = span.unwrap_or((0.0, 0.0));
        let p = |t: f64| (x + r * t.cos(), y + r * t.sin());
        let (p0, p1) = (p(t0), p(t1));
        *d += &format!(
            "#{a}=CARTESIAN_POINT('',({x},{y},{z0}));\n#{b}=AXIS2_PLACEMENT_3D('',#{a},#1,#2);\n\
#{c}=CARTESIAN_POINT('',({x},{y},{z1}));\n#{e}=AXIS2_PLACEMENT_3D('',#{c},#1,#2);\n\
#{f}=CIRCLE('',#{b},{r});\n#{g}=CIRCLE('',#{e},{r});\n#{h}=CYLINDRICAL_SURFACE('',#{b},{r});\n",
            z0 = z.0,
            z1 = z.1,
            a = n,
            b = n + 1,
            c = n + 2,
            e = n + 3,
            f = n + 4,
            g = n + 5,
            h = n + 6,
        );
        // 縁の頂点と辺（全周なら始めと終わりが同じ頂点）
        let mut edges = Vec::new();
        for (k, (zz, circ)) in [(z.0, n + 4), (z.1, n + 5)].into_iter().enumerate() {
            let m = n + 10 + 10 * k as u32;
            *d += &format!(
                "#{q0}=CARTESIAN_POINT('',({},{},{zz}));\n#{v0}=VERTEX_POINT('',#{q0});\n\
#{q1}=CARTESIAN_POINT('',({},{},{zz}));\n#{v1}=VERTEX_POINT('',#{q1});\n\
#{ec}=EDGE_CURVE('',#{v0},#{ve},#{circ},.T.);\n",
                p0.0,
                p0.1,
                p1.0,
                p1.1,
                q0 = m,
                v0 = m + 1,
                q1 = m + 2,
                v1 = m + 3,
                ve = if span.is_some() { m + 3 } else { m + 1 },
                ec = m + 4,
            );
            edges.push(format!("#{}", m + 4));
        }
        *d += &format!(
            "#{i}=ADVANCED_FACE('',({}),#{h},.{o}.);\n",
            edges.join(","),
            i = n + 7,
            h = n + 6,
            o = if inward { "F" } else { "T" },
        );
        *id += 40;
        format!("#{}", n + 7)
    }

    fn solid(faces: &[(f64, f64, f64, f64, f64, bool)]) -> String {
        let mut d = String::from(
            "ISO-10303-21;\nDATA;\n#1=DIRECTION('',(0.,0.,1.));\n#2=DIRECTION('',(1.,0.,0.));\n",
        );
        let mut id = 100;
        let fs: Vec<String> = faces
            .iter()
            .map(|f| cyl_face(&mut d, &mut id, (f.0, f.1), (f.2, f.3), f.4, f.5))
            .collect();
        d += &format!(
            "#900=CLOSED_SHELL('',({}));\n#901=MANIFOLD_SOLID_BREP('part',#900);\nENDSEC;\n",
            fs.join(",")
        );
        d
    }

    /// Edmund の描き方: メスのねじは山の径（内径）の円柱（SM05 は 12.9794）。丸いケージ板のロッドの穴は外形の円の内側で、
    /// 間隔 26.87（ロッドの円の直径 38）。数の多い穴の組（絞りの羽根のピンなど）はロッドとしない
    #[test]
    fn edmund_threads_and_round_cage_plate() {
        // 10 mm の SM05 のチューブ: z 0〜3 はオスのねじ（外径 13.589）、z 3〜13 は外径 15.97、内側は z 0〜5 が内径 10、
        // z 6〜13 がメスのねじ
        let tube = solid(&[
            (0.0, 0.0, 0.0, 3.0, 6.7945, false),
            (0.0, 0.0, 3.0, 13.0, 7.985, false),
            (0.0, 0.0, 0.0, 5.0, 5.0, true),
            (0.0, 0.0, 5.0, 6.0, 6.8945, true),
            (0.0, 0.0, 6.0, 13.0, 6.4897, true),
        ]);
        let f = crate::step::read_step(&tube).unwrap().frame.unwrap();
        assert_eq!(f.thread, "SM05", "{f:?}");
        let ft = f.front_thread.clone().unwrap();
        assert_eq!(
            (ft.thread.as_str(), ft.male, ft.length),
            ("SM05", true, 3.0)
        );
        let bt = f.back_thread.clone().unwrap();
        assert_eq!(
            (bt.thread.as_str(), bt.male, bt.length),
            ("SM05", false, 7.0)
        );
        // C マウントのチューブ（Thorlabs の描き方）: 前はオス（外径 25.4）、後ろはメス（外径 25.4 の円柱と、奥の逃げの
        // 溝 26.416）
        let cml = solid(&[
            (0.0, 0.0, 0.0, 4.064, 12.7, false),
            (0.0, 0.0, 4.064, 14.064, 15.24, false),
            (0.0, 0.0, 0.0, 4.826, 11.557, true),
            (0.0, 0.0, 4.826, 5.588, 13.208, true),
            (0.0, 0.0, 5.588, 14.064, 12.7, true),
        ]);
        let f = crate::step::read_step(&cml).unwrap().frame.unwrap();
        let ft = f.front_thread.clone().unwrap();
        assert_eq!((ft.thread.as_str(), ft.male), ("C-Mount", true), "{f:?}");
        let bt = f.back_thread.clone().unwrap();
        assert_eq!(
            (bt.thread.as_str(), bt.male, bt.length),
            ("C-Mount", false, 8.476)
        );
        // 25.4 の素通しの穴は C マウントのメスとしない
        let ring = solid(&[
            (0.0, 0.0, 0.0, 10.0, 14.975, false),
            (0.0, 0.0, 0.0, 10.0, 12.7, true),
        ]);
        let f = crate::step::read_step(&ring).unwrap().frame.unwrap();
        assert_eq!((f.thread.as_str(), &f.front_thread), ("", &None), "{f:?}");
        // 丸いケージ板（外径 49.25、内径 25）とロッドの穴 4 つ
        let q = 13.435;
        let mut plate = vec![
            (0.0, 0.0, 0.0, 10.0, 24.625, false),
            (0.0, 0.0, 0.0, 10.0, 12.5, true),
        ];
        for (x, y) in [(q, q), (-q, q), (-q, -q), (q, -q)] {
            plate.push((x, y, 0.0, 10.0, 3.01, true));
        }
        let f = crate::step::read_step(&solid(&plate))
            .unwrap()
            .frame
            .unwrap();
        assert_eq!((f.outer_diameter, f.rod_spacing), (49.25, 26.87), "{f:?}");
        assert_eq!(f.outline, lensbench_project::MechOutline::Round);
        // 7 つのピンの穴（規格に合わない間隔）はロッドとしない
        let mut iris = vec![
            (0.0, 0.0, 0.0, 10.0, 15.75, false),
            (0.0, 0.0, 0.0, 10.0, 6.0, true),
        ];
        for k in 0..7 {
            let t = (k as f64) * std::f64::consts::TAU / 7.0;
            iris.push((17.05 * t.cos(), 17.05 * t.sin(), 0.0, 10.0, 1.5, true));
        }
        // 90° おきの組を作るため、ピンの 1 つの直交の向きにも置く
        let t = std::f64::consts::FRAC_PI_2;
        iris.push((17.05 * t.cos(), 17.05 * t.sin(), 0.0, 10.0, 1.5, true));
        let f = crate::step::read_step(&solid(&iris))
            .unwrap()
            .frame
            .unwrap();
        assert_eq!(f.rod_spacing, 0.0, "{f:?}");
    }

    /// 角の板（一辺 40、厚さ 10）: 円柱の面と、角の頂点を持つ面。`arcs` は (中心, 半径, 角度の範囲) の弧の溝
    /// 弧: (中心, 半径, 角度の範囲)
    type Arc = ((f64, f64), f64, (f64, f64));

    fn square_plate(holes: &[(f64, f64, f64)], arcs: &[Arc]) -> String {
        let mut d = String::from(
            "ISO-10303-21;\nDATA;\n#1=DIRECTION('',(0.,0.,1.));\n#2=DIRECTION('',(1.,0.,0.));\n\
#3=CARTESIAN_POINT('',(0.,0.,0.));\n#4=AXIS2_PLACEMENT_3D('',#3,#1,#2);\n",
        );
        let mut id = 100;
        let mut faces: Vec<String> = holes
            .iter()
            .map(|h| cyl_face(&mut d, &mut id, (h.0, h.1), (0.0, 10.0), h.2, true))
            .collect();
        for a in arcs {
            faces.push(arc_face(
                &mut d,
                &mut id,
                a.0,
                (0.0, 10.0),
                a.1,
                true,
                Some(a.2),
            ));
        }
        let mut verts = Vec::new();
        for (x, y) in [(20.0, 20.0), (-20.0, 20.0), (-20.0, -20.0), (20.0, -20.0)] {
            for z in [0.0, 10.0] {
                d += &format!(
                    "#{id}=CARTESIAN_POINT('',({x},{y},{z}));\n#{}=VERTEX_POINT('',#{id});\n",
                    id + 1
                );
                verts.push(format!("#{}", id + 1));
                id += 2;
            }
        }
        d += &format!(
            "#{id}=PLANE('',#4);\n#{}=ADVANCED_FACE('',({}),#{id},.T.);\n",
            id + 1,
            verts.join(",")
        );
        faces.push(format!("#{}", id + 1));
        d += &format!(
            "#9000=CLOSED_SHELL('',({}));\n#9001=MANIFOLD_SOLID_BREP('plate',#9000);\nENDSEC;\n",
            faces.join(",")
        );
        d
    }

    /// OptoSigma の描き方: ロッドの穴の周りの溝は、ロッドの穴を中心とする半径の大きい短い弧。光軸は半周以上の弧の
    /// 円のうち最大のもの（光軸の穴）に取る。穴の無い板はロッドの穴の正方形の中心を光軸にする。ロッドの間隔の規格に
    /// 合う組が 2 つなら大きい方（穴の並びの順によらない）
    #[test]
    fn optosigma_grooves_blank_plate_and_two_rod_patterns() {
        let rods = |r: f64| {
            [
                (15.0, 15.0, r),
                (-15.0, 15.0, r),
                (-15.0, -15.0, r),
                (15.0, -15.0, r),
            ]
        };
        // 光軸の穴（内径 25）・ロッドの穴（1 つだけ逃げの大きい 3.1）・ロッドの穴の周りの半径 45 の溝の弧
        let mut holes = vec![(0.0, 0.0, 12.5)];
        holes.extend(rods(3.0).into_iter().take(3));
        holes.push((15.0, -15.0, 3.1));
        let t = 200f64.to_radians();
        let plate = square_plate(&holes, &[((15.0, 15.0), 45.0, (t, t + 0.05))]);
        let f = crate::step::read_step(&plate).unwrap().frame.unwrap();
        assert_eq!(
            (f.outer_diameter, f.holds.map(|h| h.diameter), f.rod_spacing),
            (40.0, Some(25.0), 30.0),
            "{f:?}"
        );
        assert_eq!(f.outline, lensbench_project::MechOutline::Square);
        // 穴の無い板（中心に細い穴も無い）: ロッドの穴の正方形の中心が光軸で、一辺 40
        let blank = square_plate(&rods(3.0), &[]);
        let s = crate::step::read_step(&blank).unwrap();
        assert_eq!(s.diameter, json!(40.0), "{s:?}");
        // 中心に細い穴のある穴の無い板は、枠として測る
        let mut h = vec![(0.0, 0.0, 1.0)];
        h.extend(rods(3.0));
        let f = crate::step::read_step(&square_plate(&h, &[]))
            .unwrap()
            .frame
            .unwrap();
        assert_eq!((f.outer_diameter, f.rod_spacing), (40.0, 30.0), "{f:?}");
        // ロッドの間隔を変える板: 16 と 30 の組は 30
        let mut h = vec![(0.0, 0.0, 6.35)];
        h.extend(rods(3.0));
        h.extend([
            (8.0, 8.0, 2.0),
            (-8.0, 8.0, 2.0),
            (-8.0, -8.0, 2.0),
            (8.0, -8.0, 2.0),
        ]);
        for _ in 0..4 {
            let f = crate::step::read_step(&square_plate(&h, &[]))
                .unwrap()
                .frame
                .unwrap();
            assert_eq!(f.rod_spacing, 30.0, "{f:?}");
            h.rotate_left(1);
        }
    }

    /// メスのねじの長さは、同じ径の内壁が端から続くところまで（短い輪の内壁と本体の長い内壁が同じ径でも、面の並びに
    /// よらず本体の長さ）
    #[test]
    fn female_thread_reaches_the_longest_wall() {
        for order in [false, true] {
            let mut f = vec![
                (0.0, 0.0, 0.0, 26.0, 16.637, false),
                (0.0, 0.0, 17.3, 26.0, 12.0, true),
            ];
            let walls = [
                (0.0, 0.0, 0.0, 3.81, 13.1445, true),
                (0.0, 0.0, 0.0, 17.3, 13.1445, true),
            ];
            if order {
                f.extend(walls.iter().rev());
            } else {
                f.extend(walls);
            }
            let m = crate::step::read_step(&solid(&f)).unwrap().frame.unwrap();
            let t = m.front_thread.clone().unwrap();
            assert_eq!(
                (t.thread.as_str(), t.male, t.length),
                ("SM1", false, 17.3),
                "{order}: {m:?}"
            );
        }
    }

    /// 保持する素子の選び方（#182）: ねじ → 座（溝を除く） → 最も長い穴
    #[test]
    fn pick_hold_order() {
        let b = |z0: f64, z1: f64, diameter: f64| lensbench_project::BoreSpec { z0, z1, diameter };
        // 素子の入る短い SM1 のメスねじは、より長い別の穴より優先
        let sm1 = 1.035 * 25.4;
        let r = super::pick_hold(&[b(0.0, 3.0, sm1), b(3.0, 20.0, 30.0)]);
        assert_eq!((r.0.as_str(), r.1), ("SM1", 25.4));
        // 座: 長い入口の穴（20）より、肩（11）の手前の 12.7 の区間。間の 0.6 mm の溝（13.5）は除く
        let r = super::pick_hold(&[
            b(0.0, 8.0, 20.0),
            b(8.0, 9.5, 12.7),
            b(9.5, 10.1, 13.5),
            b(10.1, 10.6, 12.7),
            b(10.6, 12.0, 11.0),
        ]);
        assert_eq!((r.0.as_str(), r.1), ("", 12.7));
        // となりが細すぎる（0.75 倍未満）なら座ではなく、最も長い穴
        let r = super::pick_hold(&[b(0.0, 2.0, 20.0), b(2.0, 10.0, 8.0)]);
        assert_eq!((r.0.as_str(), r.1), ("", 8.0));
        // 素子を入れないねじ（C マウントのメス）の穴は、径はそのままでねじの名前を付ける
        let r = super::pick_hold(&[b(0.0, 9.525, 24.5364)]);
        assert_eq!((r.0.as_str(), r.1), ("C-Mount", 24.5364));
    }
}
