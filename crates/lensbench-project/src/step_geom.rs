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

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(crate) type V3 = [f64; 3];

/// STEP の 1 つの値
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Arg {
    Ref(u64),
    Num(f64),
    List(Vec<Arg>),
    /// .T.・.F.
    Bool(bool),
    Other,
}

impl Arg {
    fn r(&self) -> Option<u64> {
        match self {
            Arg::Ref(r) => Some(*r),
            _ => None,
        }
    }
    fn num(&self) -> Option<f64> {
        match self {
            Arg::Num(x) => Some(*x),
            _ => None,
        }
    }
    fn list(&self) -> Option<&[Arg]> {
        match self {
            Arg::List(v) => Some(v),
            _ => None,
        }
    }
}

/// `(a,b,(c,d))` の中身を値の並びに（文字・.T./.F. 以外の列挙は Other）
pub(crate) fn parse_args(s: &[u8], i: &mut usize) -> Vec<Arg> {
    let mut out = Vec::new();
    // s[*i] は '('
    *i += 1;
    loop {
        while *i < s.len() && s[*i].is_ascii_whitespace() {
            *i += 1;
        }
        if *i >= s.len() {
            return out;
        }
        match s[*i] {
            b')' => {
                *i += 1;
                return out;
            }
            b',' => {
                *i += 1;
            }
            b'(' => out.push(Arg::List(parse_args(s, i))),
            b'\'' => {
                *i += 1;
                while *i < s.len() {
                    if s[*i] == b'\'' {
                        if *i + 1 < s.len() && s[*i + 1] == b'\'' {
                            *i += 2;
                            continue;
                        }
                        break;
                    }
                    *i += 1;
                }
                *i += 1;
                out.push(Arg::Other);
            }
            b'#' => {
                *i += 1;
                let st = *i;
                while *i < s.len() && s[*i].is_ascii_digit() {
                    *i += 1;
                }
                let n = std::str::from_utf8(&s[st..*i])
                    .ok()
                    .and_then(|x| x.parse().ok());
                out.push(n.map_or(Arg::Other, Arg::Ref));
            }
            c if c == b'-'
                || c == b'+'
                || c == b'.' && s.get(*i + 1).is_some_and(u8::is_ascii_digit)
                || c.is_ascii_digit() =>
            {
                let st = *i;
                *i += 1;
                while *i < s.len()
                    && (s[*i].is_ascii_digit() || matches!(s[*i], b'.' | b'e' | b'E' | b'-' | b'+'))
                {
                    *i += 1;
                }
                let x = std::str::from_utf8(&s[st..*i])
                    .ok()
                    .and_then(|x| x.parse::<f64>().ok());
                out.push(x.map_or(Arg::Other, Arg::Num));
            }
            _ => {
                // 列挙（.T.）・$・* ・型つきの値（名前(...)）
                let st = *i;
                while *i < s.len() && !matches!(s[*i], b',' | b')' | b'(') {
                    *i += 1;
                }
                let tok = std::str::from_utf8(&s[st..*i]).unwrap_or("").trim();
                if *i < s.len() && s[*i] == b'(' {
                    let _ = parse_args(s, i);
                }
                out.push(match tok {
                    ".T." => Arg::Bool(true),
                    ".F." => Arg::Bool(false),
                    _ => Arg::Other,
                });
            }
        }
    }
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
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

/// 読んだ STEP（実体の番号 → 部分の名前と値の並び。複合の実体は部分が 2 つ以上）
pub(crate) struct Step {
    ents: HashMap<u64, Vec<(String, Vec<Arg>)>>,
}

/// 名前（英数字と _）を読む
fn read_name(body: &[u8], j: &mut usize) -> String {
    let ns = *j;
    while *j < body.len() && (body[*j].is_ascii_alphanumeric() || body[*j] == b'_') {
        *j += 1;
    }
    String::from_utf8_lossy(&body[ns..*j]).into_owned()
}

fn skip_ws(body: &[u8], j: &mut usize) {
    while *j < body.len() && body[*j].is_ascii_whitespace() {
        *j += 1;
    }
}

impl Step {
    /// DATA 部を読む（大文字にした文字）
    pub(crate) fn parse(upper: &str) -> Result<Step, String> {
        let Some(data) = upper.find("DATA;") else {
            return Err("STEP の DATA 部が無い".into());
        };
        let body = &upper.as_bytes()[data + 5..];
        let mut ents: HashMap<u64, Vec<(String, Vec<Arg>)>> = HashMap::new();
        let mut i = 0;
        while i < body.len() {
            // #n = NAME(args); か #n = ( NAME(args) NAME(args) ... );
            if body[i] != b'#' {
                i += 1;
                continue;
            }
            let st = i + 1;
            let mut j = st;
            while j < body.len() && body[j].is_ascii_digit() {
                j += 1;
            }
            let Some(id) = std::str::from_utf8(&body[st..j])
                .ok()
                .and_then(|x| x.parse::<u64>().ok())
            else {
                i = j.max(i + 1);
                continue;
            };
            skip_ws(body, &mut j);
            if j >= body.len() || body[j] != b'=' {
                i = j;
                continue;
            }
            j += 1;
            skip_ws(body, &mut j);
            let mut parts = Vec::new();
            if j < body.len() && body[j] == b'(' {
                // 複合の実体
                j += 1;
                loop {
                    skip_ws(body, &mut j);
                    if j >= body.len() || body[j] == b')' || body[j] == b';' {
                        break;
                    }
                    let name = read_name(body, &mut j);
                    skip_ws(body, &mut j);
                    if j < body.len() && body[j] == b'(' {
                        let args = parse_args(body, &mut j);
                        if !name.is_empty() {
                            parts.push((name, args));
                        }
                    } else if name.is_empty() {
                        break;
                    }
                }
            } else {
                let name = read_name(body, &mut j);
                skip_ws(body, &mut j);
                if j < body.len() && body[j] == b'(' {
                    let args = parse_args(body, &mut j);
                    if !name.is_empty() {
                        parts.push((name, args));
                    }
                }
            }
            if !parts.is_empty() {
                ents.insert(id, parts);
            }
            // 次の ';' へ（文字の中の ';' は parse_args が飛ばしている）
            while j < body.len() && body[j] != b';' {
                j += 1;
            }
            i = j + 1;
        }
        Ok(Step { ents })
    }

    /// 実体 id の部分 ty の値
    fn part(&self, id: u64, ty: &str) -> Option<&Vec<Arg>> {
        self.ents
            .get(&id)?
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
            if let Some(ps) = self.ents.get(&i) {
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
const SOLIDS: [&str; 2] = ["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS"];

impl Step {
    /// 実体の集まりの円と頂点
    fn geo_of(&self, set: &HashSet<u64>) -> Geo {
        let mut g = Geo::default();
        for &j in set {
            if let Some(c) = self.circle(j) {
                g.circles.push(c);
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
                .ents
                .iter()
                .filter(|(_, ps)| ps.iter().any(|(n, _)| SOLIDS.contains(&n.as_str())))
                .map(|(i, _)| *i)
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
        for (id, ps) in &self.ents {
            for (n, a) in ps {
                if n.ends_with("REPRESENTATION") && !n.contains("RELATIONSHIP") {
                    if let Some(items) = a.get(1).and_then(Arg::list) {
                        reps.push(*id);
                        for it in items.iter().filter_map(Arg::r) {
                            if solids.contains(&it) {
                                rep_of_solid.insert(it, *id);
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
        for (id, ps) in &self.ents {
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
            rels.push((*id, r1, r2, tr));
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
        for ps in self.ents.values() {
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
        for ps in self.ents.values() {
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
    pub frame: Option<crate::MechSpec>,
    /// ガラスと枠が 1 つの立体（ガラスの径は見えている径）
    pub merged: bool,
    /// ガラスの無い機構部品（厚さ・径は枠の長さ・外径）
    pub mech: bool,
    /// 機構部品の付属の止め輪（枠に含めない）
    pub retaining_rings: Vec<RetainingRing>,
}

fn fmt(x: f64) -> String {
    let r = (x * 1e4).round() / 1e4;
    format!("{r}")
}

use crate::catalog_fill::RetainingRing;

/// ケージのロッドの間隔の規格 [mm]
const ROD_SPACINGS: [f64; 3] = [16.0, 30.0, 60.0];

/// 光学部品のねじ（名前、内側のねじの谷の径 [mm]、入る素子の径 [mm]）。STEP のねじは谷の径の円柱で描かれる
const THREADS: [(&str, f64, f64); 4] = [
    ("SM05", 0.535 * 25.4, 12.7),
    ("SM1", 1.035 * 25.4, 25.4),
    ("SM2", 2.035 * 25.4, 50.8),
    ("SM3", 3.035 * 25.4, 76.2),
];

/// 測る。長さは `k` 倍し `round` で丸める（mm にする）
pub(crate) fn measure(upper: &str, k: f64, round: fn(f64) -> f64) -> Result<Measure, String> {
    let st = Step::parse(upper)?;
    let sc = |x: f64| round(x * k);
    let inst = st.instances();
    // 立体ごとの形（共通の座標）
    let mut cache: HashMap<u64, Geo> = HashMap::new();
    let geos: Vec<Geo> = inst
        .iter()
        .map(|(s, m)| cache.entry(*s).or_insert_with(|| st.solid_geo(*s)).moved(m))
        .collect();
    // 全体の円と頂点（立体が無ければ全部の実体から）
    let all: Geo = if geos.is_empty() {
        let ids: HashSet<u64> = st.ents.keys().copied().collect();
        st.geo_of(&ids)
    } else {
        let mut g = Geo::default();
        for x in &geos {
            g.circles.extend(x.circles.iter().copied());
            g.verts.extend(x.verts.iter().copied());
        }
        g
    };
    let biggest = all
        .circles
        .iter()
        .copied()
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let Some((o, axis, rmax)) = biggest else {
        return boxed(&st, &all, sc);
    };
    // 軸の向きは STEP の座標の向きにそろえる（絶対値の最大の成分を正に）。光の進む向きは STEP から分からない
    let axis = {
        let m = (0..3)
            .max_by(|&a, &b| {
                axis[a]
                    .abs()
                    .partial_cmp(&axis[b].abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(2);
        if axis[m] < 0.0 {
            scale(axis, -1.0)
        } else {
            axis
        }
    };
    let z_of = |p: V3| dot(sub(p, o), axis);
    let rho = |p: V3| {
        let d = sub(p, o);
        let z = dot(d, axis);
        dot(d, d).sub_sq(z * z).sqrt()
    };
    let coax = |c: V3, a: V3| dot(a, axis).abs() > 0.999 && rho(c) <= 1e-4 * rmax + 1e-9;
    // 立体ごと: z の範囲、軸と同じ軸の円の最大の半径、穴の内壁の面
    struct Sol {
        z: [f64; 2],
        r: f64,
        ring: bool,
        walls: Vec<(f64, [f64; 2])>,
        /// 外向きの円柱の面（外径・外側のねじ）の半径と z の範囲
        outs: Vec<(f64, [f64; 2])>,
        caps: Vec<(f64, f64)>,
    }
    let sols: Vec<Sol> = geos
        .iter()
        .filter_map(|g| {
            let zs = g
                .verts
                .iter()
                .copied()
                .chain(g.circles.iter().map(|c| c.0))
                .map(z_of);
            let (lo, hi) = zs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| {
                (lo.min(z), hi.max(z))
            });
            if hi.is_nan() || lo.is_nan() || hi < lo {
                return None;
            }
            let r = g
                .circles
                .iter()
                .filter(|c| coax(c.0, c.1))
                .map(|c| c.2)
                .fold(0.0, f64::max);
            let cyl_z = |inward: bool| -> Vec<(f64, [f64; 2])> {
                g.cyls
                    .iter()
                    .filter(|f| f.inward == inward && coax(f.c, f.axis))
                    .filter_map(|f| {
                        let (a, b) = f
                            .pts
                            .iter()
                            .map(|p| z_of(*p))
                            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), z| {
                                (a.min(z), b.max(z))
                            });
                        (b > a).then_some((f.r, [a, b]))
                    })
                    .collect()
            };
            let outs = cyl_z(false);
            let walls: Vec<(f64, [f64; 2])> = g
                .cyls
                .iter()
                .filter(|f| f.inward && coax(f.c, f.axis))
                .filter_map(|f| {
                    let (a, b) = f
                        .pts
                        .iter()
                        .map(|p| z_of(*p))
                        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), z| {
                            (a.min(z), b.max(z))
                        });
                    (b > a).then_some((f.r, [a, b]))
                })
                .collect();
            // 光軸をふさぐ帽子（ガラスの面）の z と半径
            let caps: Vec<(f64, f64)> = g
                .caps
                .iter()
                .filter(|c| coax(c.0, c.1))
                .map(|c| (z_of(c.0), c.2))
                .collect();
            Some(Sol {
                z: [lo, hi],
                r,
                ring: !walls.is_empty(),
                walls,
                outs,
                caps,
            })
        })
        .collect();
    // ガラスと枠が 1 つの立体（穴の内壁と、光軸をふさぐ面の両方を持つ）: 光軸をふさぐ面の間をガラスとみなす。
    // ガラスの径は見えている径（帽子の縁）で、枠に隠れた部分は分からない
    let mut merged = 0;
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
            (hi - lo > 1e-6 * rmax && r > 0.0).then(|| {
                merged += 1;
                Sol {
                    z: [lo, hi],
                    r,
                    ring: false,
                    walls: Vec::new(),
                    outs: Vec::new(),
                    caps: Vec::new(),
                }
            })
        })
        .collect();
    let mut sols = sols;
    sols.extend(virt);
    let rings: Vec<&Sol> = sols.iter().filter(|s| s.ring).collect();
    let glass: Vec<&Sol> = sols.iter().filter(|s| !s.ring && s.r > 0.0).collect();
    let span = |v: &[&Sol]| {
        v.iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), s| {
                (lo.min(s.z[0]), hi.max(s.z[1]))
            })
    };
    let eps = 1e-6 * rmax;
    // z での内壁の最小の半径（内壁が無ければ無限大）
    let wall_min = |walls: &[(f64, [f64; 2])], z: f64| {
        walls
            .iter()
            .filter(|w| w.1[0] - eps <= z && z <= w.1[1] + eps)
            .map(|w| w.0)
            .fold(f64::INFINITY, f64::min)
    };
    // 穴: 内壁の面の端で z を区切り、区間ごとに区間をまたぐ内壁の最小の半径。z は all0 から、長さ length（mm）
    let bores_of = |walls: &[(f64, [f64; 2])], all0: f64, length: f64| {
        let mut zs: Vec<f64> = walls.iter().flat_map(|w| [w.1[0], w.1[1]]).collect();
        zs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        zs.dedup_by(|a, b| (*a - *b).abs() <= eps);
        let mut bores: Vec<crate::BoreSpec> = Vec::new();
        for w in zs.windows(2) {
            if w[1] - w[0] <= eps {
                continue;
            }
            let r = wall_min(walls, 0.5 * (w[0] + w[1]));
            if !r.is_finite() {
                continue;
            }
            let (z0, z1, d) = (sc(w[0] - all0), sc(w[1] - all0), sc(2.0 * r));
            match bores.last_mut() {
                Some(b) if b.diameter == d && (b.z1 - z0).abs() < 1e-9 => b.z1 = z1,
                _ => bores.push(crate::BoreSpec {
                    z0,
                    z1,
                    diameter: d,
                }),
            }
        }
        // 内壁の円柱の無い区間（面取りの円錐など）は、となりの穴の太い方でうめる（穴の無い区間は詰まった
        // 円柱として描かれ、確かめられるので）
        let mut filled: Vec<crate::BoreSpec> = Vec::new();
        let mut z = 0.0;
        for (k, b) in bores.iter().enumerate() {
            if b.z0 > z + 1e-9 {
                let prev = filled.last().map_or(0.0, |x: &crate::BoreSpec| x.diameter);
                filled.push(crate::BoreSpec {
                    z0: z,
                    z1: b.z0,
                    diameter: prev.max(b.diameter),
                });
            }
            filled.push(*b);
            z = b.z1;
            if k + 1 == bores.len() && z < length - 1e-9 {
                filled.push(crate::BoreSpec {
                    z0: z,
                    z1: length,
                    diameter: b.diameter,
                });
            }
        }
        let mut bores: Vec<crate::BoreSpec> = Vec::new();
        for b in filled {
            match bores.last_mut() {
                Some(x) if x.diameter == b.diameter && (x.z1 - b.z0).abs() < 1e-9 => x.z1 = b.z1,
                _ => bores.push(b),
            }
        }
        bores
    };
    if !rings.is_empty() && !glass.is_empty() {
        let (g0, g1) = span(&glass);
        let gr = glass.iter().map(|s| s.r).fold(0.0, f64::max);
        let (all0, all1) = span(&sols.iter().collect::<Vec<_>>());
        let length = sc(all1 - all0);
        let walls: Vec<(f64, [f64; 2])> =
            rings.iter().flat_map(|s| s.walls.iter().copied()).collect();
        let bores = bores_of(&walls, all0, length);
        let wall_at = |z: f64| wall_min(&walls, z);
        let outer = rings.iter().map(|s| s.r).fold(0.0, f64::max);
        let hold_r = wall_at(0.5 * (g0 + g1));
        let hold_d = if hold_r.is_finite() {
            2.0 * hold_r
        } else {
            2.0 * gr
        };
        let clear = bores
            .iter()
            .map(|b| b.diameter)
            .fold(f64::INFINITY, f64::min);
        let frame = crate::MechSpec {
            bores,
            length,
            outer_diameter: sc(2.0 * outer),
            holds: Some(crate::HoldSpec {
                diameter: sc(hold_d),
                seat_z: sc(g0 - all0),
            }),
            ..Default::default()
        };
        return Ok(Measure {
            thickness: sc(g1 - g0),
            diameter: json!(sc(2.0 * gr)),
            note: format!(
                "立体 {} 個（ガラス {}・枠 {}）を組立の置き方どおりに置いて測った。ガラスは厚さ {} mm・径 {} mm、\
枠は長さ {} mm・外径 {} mm・最小の内径（クリア開口）{} mm、ガラスの前面は枠の端から {} mm",
                sols.len() - merged,
                glass.len(),
                rings.len(),
                fmt(sc(g1 - g0)),
                fmt(sc(2.0 * gr)),
                fmt(frame.length),
                fmt(frame.outer_diameter),
                fmt(clear),
                fmt(sc(g0 - all0)),
            ) + if merged > 0 {
                "。ガラスと枠が 1 つの立体なので、光軸をふさぐ面の間をガラスとし、ガラスの径は見えている径（枠に隠れた部分は分からない）"
            } else {
                ""
            },
            frame: Some(frame),
            merged: merged > 0,
            mech: false,
            retaining_rings: Vec::new(),
        });
    }
    // ガラスの無い機構部品（チューブ・ケージ板・マウント）: 全体を枠として測る
    if !rings.is_empty() && glass.is_empty() {
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
                    && x.r <= body_wall + 1e-3 * rmax
            })
        };
        let keep: Vec<&Sol> = sols.iter().filter(|x| !is_rr(x)).collect();
        let rrs: Vec<&Sol> = rings.iter().copied().filter(|x| is_rr(x)).collect();
        let (all0, all1) = span(&keep);
        let length = sc(all1 - all0);
        let walls: Vec<(f64, [f64; 2])> = keep
            .iter()
            .filter(|s| s.ring)
            .flat_map(|s| s.walls.iter().copied())
            .collect();
        let bores = bores_of(&walls, all0, length);
        let retaining: Vec<RetainingRing> = rrs
            .iter()
            .map(|x| RetainingRing {
                z0: sc(x.z[0] - all0),
                z1: sc(x.z[1] - all0),
                inner_diameter: sc(2.0 * x.walls.iter().map(|w| w.0).fold(f64::INFINITY, f64::min)),
                outer_diameter: sc(2.0 * x.r),
            })
            .collect();
        if !bores.is_empty() {
            // 光軸に直交する 2 方向
            let ex = {
                let t = if axis[0].abs() < 0.9 {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                let c = cross(axis, t);
                scale(c, 1.0 / dot(c, c).sqrt())
            };
            let ey = cross(axis, ex);
            let plane = |p: V3| {
                let d = sub(p, o);
                [dot(d, ex), dot(d, ey)]
            };
            // ケージのロッドの穴: 軸に平行で光軸に無い内壁の円柱のうち、同じ半径・同じ光軸からの距離に 4 つ以上
            let mut holes: Vec<(f64, [f64; 2])> = Vec::new();
            for g in &geos {
                for f in &g.cyls {
                    if f.inward && dot(f.axis, axis).abs() > 0.999 && !coax(f.c, f.axis) {
                        let q = plane(f.c);
                        if !holes.iter().any(|h| {
                            (h.0 - f.r).abs() <= 1e-4 * rmax
                                && (h.1[0] - q[0]).hypot(h.1[1] - q[1]) <= 1e-4 * rmax
                        }) {
                            holes.push((f.r, q));
                        }
                    }
                }
            }
            // 候補（同じ半径・同じ距離の穴が 4 つ以上、90° おき）のうち、ロッドの間隔の規格（16・30・60 mm）に合う
            // ものを先に（キューブなどの取付けのねじ穴を除く）
            let cands: Vec<f64> = holes
                .iter()
                .filter_map(|h| {
                    let rho = h.1[0].hypot(h.1[1]);
                    let same: Vec<&(f64, [f64; 2])> = holes
                        .iter()
                        .filter(|x| {
                            (x.0 - h.0).abs() <= 1e-4 * rmax
                                && (x.1[0].hypot(x.1[1]) - rho).abs() <= 1e-4 * rmax
                        })
                        .collect();
                    // 4 つの穴が光軸の周りに 90° おき（正方形の角）
                    let perp = same
                        .iter()
                        .find(|x| (x.1[0] * h.1[0] + x.1[1] * h.1[1]).abs() <= 1e-3 * rho * rho)?;
                    (same.len() >= 4 && rho > rmax && perp.0 > 0.0).then_some(rho)
                })
                .collect();
            let spacing_mm = |rho: f64| rho * std::f64::consts::SQRT_2 * k;
            let standard = |mm: f64| {
                ROD_SPACINGS
                    .iter()
                    .copied()
                    .find(|s| (s - mm).abs() <= 0.05)
            };
            let rods = cands
                .iter()
                .copied()
                .find(|r| standard(spacing_mm(*r)).is_some())
                .or_else(|| cands.first().copied());
            let verts: Vec<[f64; 2]> = geos
                .iter()
                .flat_map(|g| g.verts.iter().copied())
                .map(plane)
                .collect();
            // 外形: 光軸に直交する向きの幅の最小（ケージ板・キューブは一辺。取付けの出っ張りを除く）と、
            // 光軸と同じ軸の円の直径の大きい方（円柱は頂点が少ないので）
            let min_width = (0..180)
                .map(|i| {
                    let t = (i as f64).to_radians();
                    let (c, s) = (t.cos(), t.sin());
                    let (lo, hi) =
                        verts
                            .iter()
                            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| {
                                let x = q[0] * c + q[1] * s;
                                (lo.min(x), hi.max(x))
                            });
                    hi - lo
                })
                .fold(f64::INFINITY, f64::min);
            let circ = 2.0 * sols.iter().map(|s| s.r).fold(0.0, f64::max);
            let outer = if min_width.is_finite() {
                min_width.max(circ)
            } else {
                circ
            };
            // mm（規格の間隔に 0.05 mm 以内なら規格の値）
            let rod_spacing_mm = rods.map_or(0.0, |r| {
                let mm = spacing_mm(r);
                standard(mm).unwrap_or_else(|| round(mm))
            });
            // 保持する素子: 最も長い穴の径。ねじ（SM05・SM1・SM2・SM3）の内径なら、ねじの名前と入る素子の径
            // 止め輪を除いた本体の穴から
            let body_bores = body.map_or_else(Vec::new, |b| bores_of(&b.walls, all0, length));
            let mut runs: Vec<(f64, f64)> = Vec::new();
            for b in &body_bores {
                match runs.iter_mut().find(|r| (r.0 - b.diameter).abs() < 1e-9) {
                    Some(r) => r.1 += b.z1 - b.z0,
                    None => runs.push((b.diameter, b.z1 - b.z0)),
                }
            }
            let hold = runs
                .iter()
                .copied()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map_or(0.0, |r| r.0);
            let (thread, hold_d) = THREADS
                .iter()
                .find(|t| (t.1 - hold).abs() <= 0.05)
                .map_or((String::new(), hold), |t| (t.0.to_string(), t.2));
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
            let seat_mm = sc(ring_seat).max(shoulder);
            let seat = seat_mm;
            // 両端のねじ: 端に接する、ねじの谷の径の外向きの円柱（オス、外径より細い）か内壁（メス）
            let thread_of = |r: f64| THREADS.iter().find(|t| (t.1 - sc(2.0 * r)).abs() <= 0.05);
            let end_thread = |end: f64| -> Option<crate::ThreadEnd> {
                // 端から 1 mm 以内（面取りの分）
                let tol = 1.0 / k;
                let dist = |zz: [f64; 2]| (zz[0] - end).abs().min((zz[1] - end).abs());
                let far = |zz: [f64; 2]| (zz[0] - end).abs().max((zz[1] - end).abs());
                // オス: 外径より細い外向きの円柱が端から続く範囲（ねじと逃げ）がねじ込む長さ
                let narrow: Vec<(f64, [f64; 2])> = keep
                    .iter()
                    .flat_map(|x| x.outs.iter().copied())
                    .filter(|o| 2.0 * o.0 < outer - 1e-3 * rmax)
                    .collect();
                if let Some(t) = narrow
                    .iter()
                    .filter(|o| dist(o.1) <= tol)
                    .find_map(|o| thread_of(o.0))
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
                    return Some(crate::ThreadEnd {
                        thread: t.0.to_string(),
                        male: true,
                        length: sc(reach),
                    });
                }
                // メス: 端に接するねじの径の内壁
                keep.iter()
                    .flat_map(|x| x.walls.iter())
                    .filter(|w| dist(w.1) <= tol)
                    .find_map(|w| thread_of(w.0).map(|t| (t, far(w.1))))
                    .map(|(t, len)| crate::ThreadEnd {
                        thread: t.0.to_string(),
                        male: false,
                        length: sc(len),
                    })
            };
            let front_thread = end_thread(all0);
            let back_thread = end_thread(all1);
            let rings_n = keep.iter().filter(|x| x.ring).count();
            let frame = crate::MechSpec {
                bores,
                length,
                outer_diameter: sc(outer),
                thread: thread.clone(),
                rod_spacing: rod_spacing_mm,
                holds: Some(crate::HoldSpec {
                    diameter: hold_d,
                    seat_z: seat_mm,
                }),
                front_thread,
                back_thread,
            };
            let clear = frame
                .bores
                .iter()
                .map(|b| b.diameter)
                .fold(f64::INFINITY, f64::min);
            return Ok(Measure {
                thickness: frame.length,
                diameter: json!(frame.outer_diameter),
                note: format!(
                    "ガラスの無い機構部品として、立体 {} 個（穴のある立体 {rings_n}）を枠として測った。長さ {} mm・外径 {} mm{}・最小の内径（クリア開口）{} mm{}。座の位置は{}",
                    keep.len(),
                    fmt(frame.length),
                    fmt(frame.outer_diameter),
                    if rods.is_some() {
                        format!("（ケージ板の一辺）・ロッドの間隔 {} mm", fmt(frame.rod_spacing))
                    } else {
                        String::new()
                    },
                    fmt(clear),
                    if thread.is_empty() {
                        String::new()
                    } else {
                        format!("・内側のねじ {thread}（入る素子の径 {} mm）", fmt(frame.holds.map_or(0.0, |h| h.diameter)))
                    },
                    if seat <= 0.0 {
                        "枠の端（STEP から分からない）"
                    } else if seat == shoulder {
                        "保持する素子より細い穴の段"
                    } else {
                        "止め輪の奥の面"
                    }
                ) + &if retaining.is_empty() {
                    String::new()
                } else {
                    format!(
                        "。付属の止め輪 {} 個（内径 {} mm）は素子に合わせて動かすので枠に含めない",
                        retaining.len(),
                        retaining
                            .iter()
                            .map(|r| fmt(r.inner_diameter))
                            .collect::<Vec<_>>()
                            .join("・")
                    )
                },
                frame: Some(frame),
                merged: false,
                mech: true,
                retaining_rings: retaining,
            });
        }
    }
    // 枠の分からない品: 軸に平行な円の最大の直径と、軸の向きの広がり
    let par: Vec<&(V3, V3, f64)> = all
        .circles
        .iter()
        .filter(|c| dot(c.1, axis).abs() > 0.999)
        .collect();
    let r = par.iter().map(|c| c.2).fold(0.0, f64::max);
    let (lo, hi) = all
        .verts
        .iter()
        .copied()
        .chain(par.iter().map(|c| c.0))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            let z = z_of(p);
            (lo.min(z), hi.max(z))
        });
    let t = hi - lo;
    if t.is_nan() || t <= 0.0 {
        return Err("STEP の円の軸の向きに厚さが無い".into());
    }
    let solids = if geos.is_empty() {
        String::new()
    } else {
        format!("立体 {} 個、", geos.len())
    };
    Ok(Measure {
        thickness: sc(t),
        diameter: json!(sc(2.0 * r)),
        note: format!(
            "{solids}円 {} 個（軸に平行 {} 個）の最大の直径と、頂点 {} 個の軸の向きの広がり",
            all.circles.len(),
            par.len(),
            all.verts.len()
        ),
        frame: None,
        merged: false,
        mech: false,
        retaining_rings: Vec::new(),
    })
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
fn boxed(st: &Step, all: &Geo, sc: impl Fn(f64) -> f64) -> Result<Measure, String> {
    let pts: Vec<V3> = if all.verts.len() >= 2 {
        all.verts.clone()
    } else {
        st.ents
            .values()
            .filter_map(|ps| ps.iter().find(|(n, _)| n == "CARTESIAN_POINT"))
            .filter_map(|(_, a)| a.get(1).and_then(triple))
            .collect()
    };
    if pts.len() < 2 {
        return Err("STEP に点が無い".into());
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
        return Err("STEP の形に厚さが無い".into());
    }
    let d = if (ext[1] - ext[2]).abs() < 1e-9 {
        json!(sc(ext[2]))
    } else {
        json!([sc(ext[1]), sc(ext[2])])
    };
    Ok(Measure {
        thickness: sc(ext[0]),
        diameter: d,
        note: format!("円が無いので頂点 {} 個の箱（座標軸の向き）", pts.len()),
        frame: None,
        merged: false,
        mech: false,
        retaining_rings: Vec::new(),
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
            let s = crate::catalog_fill::read_step(&assembly_step(nauo)).unwrap();
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
        let s = crate::catalog_fill::read_step(d).unwrap();
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
        assert!(s.note.contains("1 つの立体"), "{}", s.note);
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
        let s = crate::catalog_fill::read_step(tube).unwrap();
        assert!(s.mech, "{s:?}");
        let f = s.frame.clone().expect("枠");
        assert_eq!((f.length, f.outer_diameter), (10.0, 30.48));
        let b: Vec<[f64; 3]> = f.bores.iter().map(|b| [b.z0, b.z1, b.diameter]).collect();
        assert_eq!(b, vec![[0.0, 2.0, 24.0], [2.0, 10.0, 26.289]]);
        assert_eq!(f.thread, "SM1");
        // 座は、保持する素子（25.4）より細い z 0〜2 の穴の段
        assert_eq!(f.holds.map(|h| (h.diameter, h.seat_z)), Some((25.4, 2.0)));
        assert!(s.note.contains("細い穴の段"), "{}", s.note);
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
        assert!(s.note.contains("止め輪 1 個"), "{}", s.note);
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
        let s = crate::catalog_fill::read_step(&d).unwrap();
        let f = s.frame.clone().expect("枠");
        assert!(s.mech);
        assert_eq!(
            (f.length, f.outer_diameter, f.rod_spacing),
            (8.0, 40.64, 30.0),
            "{s:?}"
        );
        assert_eq!(f.thread, "");
        assert_eq!(f.holds.unwrap().diameter, 22.9);
        assert!(s.note.contains("ケージ板"), "{}", s.note);
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
            println!("== {p}\n{:#?}", crate::catalog_fill::read_step(&t));
        }
    }
}
