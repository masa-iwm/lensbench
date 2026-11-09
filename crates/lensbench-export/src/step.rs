//! STEP（ISO 10303-21、AP214）の書き出し: 玉と鏡は B-rep の立体、光線は折れ線、像面などは縁の円。
//!
//! - 球面は SPHERICAL_SURFACE、平面は PLANE、円錐係数・非球面係数のある面は、断面の曲線（3 次の B スプラインで
//!   補間）を軸のまわりに回した SURFACE_OF_REVOLUTION。側面は CYLINDRICAL_SURFACE。
//! - 立体の 2 面が同じ軸の上にないもの（玉の中で偏心・傾きのあるもの）は、側面を 2 つの縁の円を結ぶ線織面
//!   （有理 B スプライン）にする。
//! - トロイダル面（円柱面など）を含む立体は、面を極座標（周 × 半径）の網で補間した B スプライン曲面にし、
//!   縁は曲面の縁の等パラメータ線（B スプライン曲線）、輪と側面は縁の曲線を結ぶ線織面にする。
//! - 単位は mm、座標は大域座標。

use std::io::Write;

use lensbench_core::math::*;

use crate::scene::{Face, MechShape, Scene, Solid};

/// 書き出しの結果
#[derive(Debug, Clone, Default)]
pub struct StepReport {
    pub solids: usize,
    /// 機構部品の輪の立体
    pub mechs: usize,
    pub rays: usize,
}

#[derive(Debug, Clone)]
pub struct OptionsStep {
    /// 非球面の断面を補間する区間の数
    pub profile_segments: usize,
}

impl Default for OptionsStep {
    fn default() -> Self {
        OptionsStep {
            profile_segments: 64,
        }
    }
}

/// STEP の実数（小数点と大文字の E が要る）
fn real(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return "0.".into();
    }
    let s = format!("{:.15E}", v);
    let (m, e) = s.split_once('E').unwrap();
    let m = m.trim_end_matches('0');
    if e == "0" {
        m.to_string()
    } else {
        format!("{m}E{e}")
    }
}

fn text(s: &str) -> String {
    // ISO 10303-21 の文字列: ' は '' に、ASCII 以外は \X2\ 形式に
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\'' => o.push_str("''"),
            '\\' => o.push_str("\\\\"),
            c if c.is_ascii() && !c.is_ascii_control() => o.push(c),
            c => {
                let mut b = [0u16; 2];
                o.push_str("\\X2\\");
                for u in c.encode_utf16(&mut b) {
                    o.push_str(&format!("{:04X}", u));
                }
                o.push_str("\\X0\\");
            }
        }
    }
    o
}

struct Ents {
    lines: Vec<String>,
}

impl Ents {
    fn add(&mut self, s: String) -> usize {
        self.lines.push(s);
        self.lines.len()
    }
    fn point(&mut self, p: V3) -> usize {
        self.add(format!(
            "CARTESIAN_POINT('',({},{},{}))",
            real(p[0]),
            real(p[1]),
            real(p[2])
        ))
    }
    fn dir(&mut self, d: V3) -> usize {
        self.add(format!(
            "DIRECTION('',({},{},{}))",
            real(d[0]),
            real(d[1]),
            real(d[2])
        ))
    }
    fn axis2(&mut self, o: V3, z: V3, x: V3) -> usize {
        let p = self.point(o);
        let a = self.dir(z);
        let r = self.dir(x);
        self.add(format!("AXIS2_PLACEMENT_3D('',#{p},#{a},#{r})"))
    }
}

/// 3 次の B スプラインの基底関数 N_{i,3}(u)（Cox–de Boor）
fn basis(knots: &[f64], i: usize, p: usize, u: f64) -> f64 {
    if p == 0 {
        let last = knots[knots.len() - 1];
        if (knots[i] <= u && u < knots[i + 1]) || (u == last && knots[i] < u && u <= knots[i + 1]) {
            return 1.0;
        }
        return 0.0;
    }
    let mut v = 0.0;
    let d1 = knots[i + p] - knots[i];
    if d1 > 0.0 {
        v += (u - knots[i]) / d1 * basis(knots, i, p - 1, u);
    }
    let d2 = knots[i + p + 1] - knots[i + 1];
    if d2 > 0.0 {
        v += (knots[i + p + 1] - u) / d2 * basis(knots, i + 1, p - 1, u);
    }
    v
}

/// 点列を通る 3 次の B スプライン（等間隔のパラメータ、平均の節点）。制御点と節点を返す
pub fn interpolate(q: &[[f64; 2]]) -> (Vec<[f64; 2]>, Vec<f64>) {
    let v: Vec<Vec<f64>> = q.iter().map(|p| p.to_vec()).collect();
    let (c, k) = interpolate_n(&v);
    (c.iter().map(|p| [p[0], p[1]]).collect(), k)
}

/// 3 次元の点列の補間（[`interpolate`] と同じ節点）
fn interpolate3(q: &[V3]) -> (Vec<V3>, Vec<f64>) {
    let v: Vec<Vec<f64>> = q.iter().map(|p| p.to_vec()).collect();
    let (c, k) = interpolate_n(&v);
    (c.iter().map(|p| [p[0], p[1], p[2]]).collect(), k)
}

/// 任意の次元の点列の補間
fn interpolate_n(q: &[Vec<f64>]) -> (Vec<Vec<f64>>, Vec<f64>) {
    const P: usize = 3;
    let n = q.len() - 1;
    assert!(n >= P);
    let dim = q[0].len();
    let u: Vec<f64> = (0..=n).map(|k| k as f64 / n as f64).collect();
    let mut knots = vec![0.0; P + 1];
    for j in 1..=(n - P) {
        knots.push(u[j..j + P].iter().sum::<f64>() / P as f64);
    }
    knots.extend([1.0; P + 1]);
    // 連立方程式 N P = Q（密行列のガウス消去、n は数十）
    let m = n + 1;
    let mut a: Vec<Vec<f64>> = (0..m)
        .map(|k| (0..m).map(|i| basis(&knots, i, P, u[k])).collect())
        .collect();
    let mut b: Vec<Vec<f64>> = q.to_vec();
    for c in 0..m {
        let piv = (c..m)
            .max_by(|&x, &y| a[x][c].abs().partial_cmp(&a[y][c].abs()).unwrap())
            .unwrap();
        a.swap(c, piv);
        b.swap(c, piv);
        for r in c + 1..m {
            let f = a[r][c] / a[c][c];
            if f != 0.0 {
                for k in c..m {
                    a[r][k] -= f * a[c][k];
                }
                for d in 0..dim {
                    b[r][d] -= f * b[c][d];
                }
            }
        }
    }
    let mut x = vec![vec![0.0; dim]; m];
    for r in (0..m).rev() {
        let mut s = b[r].clone();
        for k in r + 1..m {
            for d in 0..dim {
                s[d] -= a[r][k] * x[k][d];
            }
        }
        x[r] = s.iter().map(|v| v / a[r][r]).collect();
    }
    // 端の制御点は端の点そのもの（隣の面と共有する縁の曲線が丸めでずれないよう）
    x[0] = q[0].clone();
    x[n] = q[n].clone();
    (x, knots)
}

/// B スプラインの点（テスト用）
pub fn bspline_eval(ctrl: &[[f64; 2]], knots: &[f64], u: f64) -> [f64; 2] {
    let mut s = [0.0; 2];
    for (i, c) in ctrl.iter().enumerate() {
        let b = basis(knots, i, 3, u);
        s[0] += b * c[0];
        s[1] += b * c[1];
    }
    s
}

/// 面の幾何（ADVANCED_FACE の面と、その自然な法線が頂点で外向きと同じ向きか）
fn surface(e: &mut Ents, f: &Face, outward: V3, o: &OptionsStep) -> (usize, bool) {
    let z = f.axis();
    let x = col(&f.frame.r, 0);
    let vtx = f.frame.o;
    if f.c == 0.0 && f.is_spherical() {
        let a = e.axis2(vtx, z, x);
        let s = e.add(format!("PLANE('',#{a})"));
        return (s, dot(z, outward) > 0.0);
    }
    if f.is_spherical() {
        let r = 1.0 / f.c;
        let a = e.axis2(add(vtx, scale(z, r)), z, x);
        let s = e.add(format!("SPHERICAL_SURFACE('',#{a},{})", real(r.abs())));
        // 球の法線は中心から外へ: 頂点では −sign(R)·z
        return (s, dot(scale(z, -r.signum()), outward) > 0.0);
    }
    // 回転面: 断面（局所 xz 面、r = 0〜semi）を補間した B スプライン
    let n = o.profile_segments.max(4);
    let q: Vec<[f64; 2]> = (0..=n)
        .map(|k| {
            let r = f.semi * k as f64 / n as f64;
            [r, f.sag(r)]
        })
        .collect();
    let (ctrl, knots) = interpolate(&q);
    // 節点を重複度でまとめる
    let mut kv: Vec<(f64, usize)> = Vec::new();
    for &k in &knots {
        match kv.last_mut() {
            Some((v, m)) if *v == k => *m += 1,
            _ => kv.push((k, 1)),
        }
    }
    let mults: Vec<String> = kv.iter().map(|(_, m)| m.to_string()).collect();
    let ks: Vec<String> = kv.iter().map(|(v, _)| real(*v)).collect();
    // 周（u、有理 2 次の円の 9 点）× 断面（v、3 次）の制御点
    let y = cross(z, x);
    let mut rows = Vec::new();
    let mut ws = Vec::new();
    for &(cx, cy, w) in &CIRCLE9 {
        let row: Vec<String> = ctrl
            .iter()
            .map(|c| {
                let p = add(
                    add(vtx, scale(z, c[1])),
                    add(scale(x, c[0] * cx), scale(y, c[0] * cy)),
                );
                format!("#{}", e.point(p))
            })
            .collect();
        rows.push(format!("({})", row.join(",")));
        ws.push(format!("({})", vec![real(w); ctrl.len()].join(",")));
    }
    let s = e.add(format!(
        "(BOUNDED_SURFACE()B_SPLINE_SURFACE(2,3,({}),.UNSPECIFIED.,.T.,.F.,.F.)\
B_SPLINE_SURFACE_WITH_KNOTS((3,2,2,2,3),({}),(0.,0.25,0.5,0.75,1.),({}),.UNSPECIFIED.)\
GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_SURFACE(({}))REPRESENTATION_ITEM('')SURFACE())",
        rows.join(","),
        mults.join(","),
        ks.join(","),
        ws.join(",")
    ));
    // u は軸のまわりに反時計回り、v は外へ: S_u × S_v は頂点の近くで −z
    (s, dot(scale(z, -1.0), outward) > 0.0)
}

/// 円を有理 2 次の B スプラインで表す 9 点（正方形の辺の中点と角、角の重みは 1/√2）
const CIRCLE9: [(f64, f64, f64); 9] = {
    const H: f64 = std::f64::consts::FRAC_1_SQRT_2;
    [
        (1.0, 0.0, 1.0),
        (1.0, 1.0, H),
        (0.0, 1.0, 1.0),
        (-1.0, 1.0, H),
        (-1.0, 0.0, 1.0),
        (-1.0, -1.0, H),
        (0.0, -1.0, 1.0),
        (1.0, -1.0, H),
        (1.0, 0.0, 1.0),
    ]
};

/// 軸 A の上の円（立体の座標系: 原点 o、軸 a、基準 x）
struct Circle {
    edge: usize,
}

fn circle(e: &mut Ents, o: V3, a: V3, x: V3, z: f64, r: f64) -> Circle {
    let c0 = add(o, scale(a, z));
    let pl = e.axis2(c0, a, x);
    let cu = e.add(format!("CIRCLE('',#{pl},{})", real(r)));
    let p = e.point(add(c0, scale(x, r)));
    let v = e.add(format!("VERTEX_POINT('',#{p})"));
    let edge = e.add(format!("EDGE_CURVE('',#{v},#{v},#{cu},.T.)"));
    Circle { edge }
}

/// 外向きの法線 n の面で、軸 a のまわりに反時計回りの円をたどる向き（外周なら T）
fn loop_sense(n_dot_a: f64, outer: bool) -> bool {
    (n_dot_a > 0.0) == outer
}

fn bound(e: &mut Ents, c: &Circle, sense: bool, outer: bool) -> usize {
    let oe = e.add(format!(
        "ORIENTED_EDGE('',*,*,#{},{})",
        c.edge,
        if sense { ".T." } else { ".F." }
    ));
    let l = e.add(format!("EDGE_LOOP('',(#{oe}))"));
    let kind = if outer {
        "FACE_OUTER_BOUND"
    } else {
        "FACE_BOUND"
    };
    e.add(format!("{kind}('',#{l},.T.)"))
}

fn face(e: &mut Ents, bounds: &[usize], surf: usize, same: bool) -> usize {
    let b: Vec<String> = bounds.iter().map(|x| format!("#{x}")).collect();
    e.add(format!(
        "ADVANCED_FACE('',({}),#{surf},{})",
        b.join(","),
        if same { ".T." } else { ".F." }
    ))
}

/// 円の座標系（中心の軸の原点・軸・基準の向き）
#[derive(Clone, Copy)]
struct Axes {
    o: V3,
    a: V3,
    x: V3,
}

/// 2 つの円（同じ向きにたどる）を結ぶ線織面（有理 B スプライン、u: 周 2 次、v: 前→後 1 次）
fn ruled(e: &mut Ents, c0: (Axes, f64, f64), c1: (Axes, f64, f64)) -> usize {
    let mut rows = Vec::new();
    let mut ws = Vec::new();
    for &(cx, cy, w) in &CIRCLE9 {
        let mut row = Vec::new();
        for &(ax, z, r) in [&c0, &c1] {
            let y = cross(ax.a, ax.x);
            let p = add(
                add(ax.o, scale(ax.a, z)),
                add(scale(ax.x, r * cx), scale(y, r * cy)),
            );
            row.push(format!("#{}", e.point(p)));
        }
        rows.push(format!("({})", row.join(",")));
        ws.push(format!("({},{})", real(w), real(w)));
    }
    e.add(format!(
        "(BOUNDED_SURFACE()B_SPLINE_SURFACE(2,1,({}),.UNSPECIFIED.,.T.,.F.,.F.)\
B_SPLINE_SURFACE_WITH_KNOTS((3,2,2,2,3),(2,2),(0.,0.25,0.5,0.75,1.),(0.,1.),.UNSPECIFIED.)\
GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_SURFACE(({}))REPRESENTATION_ITEM('')SURFACE())",
        rows.join(","),
        ws.join(",")
    ))
}

/// 凸多面体を MANIFOLD_SOLID_BREP にする（平面と直線の辺。辺は隣の面と共有する）
fn solid_poly(e: &mut Ents, name: &str, p: &crate::scene::Poly) -> usize {
    let verts: Vec<usize> = p
        .pts
        .iter()
        .map(|&q| {
            let c = e.point(q);
            e.add(format!("VERTEX_POINT('',#{c})"))
        })
        .collect();
    let mut edges: std::collections::HashMap<(usize, usize), usize> = Default::default();
    let mut faces = Vec::new();
    for (nv, f) in &p.faces {
        let mut oes = Vec::new();
        for k in 0..f.len() {
            let (a, b) = (f[k], f[(k + 1) % f.len()]);
            let key = (a.min(b), a.max(b));
            let ed = match edges.get(&key) {
                Some(&x) => x,
                None => {
                    let (i, j) = key;
                    let d = sub(p.pts[j], p.pts[i]);
                    let len = norm(d);
                    let c = e.point(p.pts[i]);
                    let dr = e.dir(normalize(d));
                    let v = e.add(format!("VECTOR('',#{dr},{})", real(len)));
                    let l = e.add(format!("LINE('',#{c},#{v})"));
                    let x = e.add(format!(
                        "EDGE_CURVE('',#{},#{},#{l},.T.)",
                        verts[i], verts[j]
                    ));
                    edges.insert(key, x);
                    x
                }
            };
            oes.push(e.add(format!(
                "ORIENTED_EDGE('',*,*,#{ed},{})",
                if a < b { ".T." } else { ".F." }
            )));
        }
        let ol: Vec<String> = oes.iter().map(|x| format!("#{x}")).collect();
        let lp = e.add(format!("EDGE_LOOP('',({}))", ol.join(",")));
        let bd = e.add(format!("FACE_OUTER_BOUND('',#{lp},.T.)"));
        let q0 = p.pts[f[0]];
        let x = normalize(sub(p.pts[f[1]], q0));
        let pa = e.axis2(q0, *nv, x);
        let pl = e.add(format!("PLANE('',#{pa})"));
        faces.push(e.add(format!("ADVANCED_FACE('',(#{bd}),#{pl},.T.)")));
    }
    let fl: Vec<String> = faces.iter().map(|x| format!("#{x}")).collect();
    let shell = e.add(format!("CLOSED_SHELL('',({}))", fl.join(",")));
    e.add(format!("MANIFOLD_SOLID_BREP('{}',#{shell})", text(name)))
}

/// 立体を MANIFOLD_SOLID_BREP にする
fn solid(e: &mut Ents, sd: &Solid, o: &OptionsStep) -> usize {
    if let Some(p) = &sd.poly {
        return solid_poly(e, &sd.name, p);
    }
    let dir = sd.direction();
    let fa = Axes {
        o: sd.front.frame.o,
        a: sd.front.axis(),
        x: col(&sd.front.frame.r, 0),
    };
    // 後ろの面の円は、軸を前の面の軸と同じ側へ向け、基準を前の面の x に近づける（線織面がねじれないよう）
    let mut ab = sd.back.axis();
    if dot(ab, fa.a) < 0.0 {
        ab = scale(ab, -1.0);
    }
    let ba = Axes {
        o: sd.back.frame.o,
        a: ab,
        x: normalize(sub(fa.x, scale(ab, dot(ab, fa.x)))),
    };
    let coax = sd.coaxial();
    let mut faces = Vec::new();
    // 前の面・後ろの面（外向きは立体の向きで −dir、+dir）
    let mut rims: Vec<(Circle, Axes, f64)> = Vec::new();
    for (f, ax, s) in [(&sd.front, fa, -dir), (&sd.back, ba, dir)] {
        let outward = scale(ax.a, s);
        let semi = if f.semi > 0.0 { f.semi } else { sd.rim };
        let flat = Face { semi, ..f.clone() };
        let zc = dot(sub(flat.point(semi, 0.0), ax.o), ax.a);
        let inner = circle(e, ax.o, ax.a, ax.x, zc, semi);
        let (srf, same) = if f.semi > 0.0 {
            surface(e, &flat, outward, o)
        } else {
            let pa = e.axis2(add(ax.o, scale(ax.a, zc)), ax.a, ax.x);
            (e.add(format!("PLANE('',#{pa})")), s > 0.0)
        };
        let b = bound(e, &inner, loop_sense(s, true), true);
        faces.push(face(e, &[b], srf, same));
        if sd.rim > semi * (1.0 + 1e-12) {
            // 平らな輪
            let outer = circle(e, ax.o, ax.a, ax.x, zc, sd.rim);
            let pa = e.axis2(add(ax.o, scale(ax.a, zc)), ax.a, ax.x);
            let pl = e.add(format!("PLANE('',#{pa})"));
            let bo = bound(e, &outer, loop_sense(s, true), true);
            let bi = bound(e, &inner, loop_sense(s, false), false);
            faces.push(face(e, &[bo, bi], pl, s > 0.0));
            rims.push((outer, ax, zc));
        } else {
            rims.push((inner, ax, zc));
        }
    }
    // 側面: 外向きの法線で、前の面の軸の向きに見て手前の円を反時計回り
    let zf = rims[0].2;
    let zb = dot(sub(add(ba.o, scale(ba.a, rims[1].2)), fa.o), fa.a);
    let lo_is_front = zf <= zb;
    let (cl, ch) = if lo_is_front {
        (&rims[0].0, &rims[1].0)
    } else {
        (&rims[1].0, &rims[0].0)
    };
    let (srf, same) = if coax {
        let pa = e.axis2(add(fa.o, scale(fa.a, zf.min(zb))), fa.a, fa.x);
        (
            e.add(format!("CYLINDRICAL_SURFACE('',#{pa},{})", real(sd.rim))),
            true,
        )
    } else {
        // v は前→後。自然な法線 S_u × S_v は、前が手前なら外向き
        (
            ruled(e, (fa, rims[0].2, sd.rim), (ba, rims[1].2, sd.rim)),
            lo_is_front,
        )
    };
    let b1 = bound(e, cl, true, true);
    let b2 = bound(e, ch, false, false);
    faces.push(face(e, &[b1, b2], srf, same));
    let fl: Vec<String> = faces.iter().map(|x| format!("#{x}")).collect();
    let shell = e.add(format!("CLOSED_SHELL('',({}))", fl.join(",")));
    e.add(format!(
        "MANIFOLD_SOLID_BREP('{}',#{shell})",
        text(&sd.name)
    ))
}

/// 機構部品の輪 [z0, z1, 内側の半径, 外側の半径] の立体（外側・内側の円柱の面と両端の平面の輪）
fn solid_ring(e: &mut Ents, m: &MechShape, k: usize, q: [f64; 4]) -> usize {
    let (o, a, x) = (
        m.frame.o,
        m.axis(),
        lensbench_core::math::col(&m.frame.r, 0),
    );
    let hollow = q[2] > 0.0;
    let ol = circle(e, o, a, x, q[0], q[3]);
    let oh = circle(e, o, a, x, q[1], q[3]);
    let mut faces = Vec::new();
    // 両端の平面（前は外向き −a、後ろは +a）
    for (z, s, c) in [(q[0], -1.0, &ol), (q[1], 1.0, &oh)] {
        let pa = e.axis2(add(o, scale(a, z)), a, x);
        let pl = e.add(format!("PLANE('',#{pa})"));
        let mut b = vec![bound(e, c, loop_sense(s, true), true)];
        if hollow {
            let ic = circle(e, o, a, x, z, q[2]);
            b.push(bound(e, &ic, loop_sense(s, false), false));
            faces.push((face(e, &b, pl, s > 0.0), Some(ic)));
        } else {
            faces.push((face(e, &b, pl, s > 0.0), None));
        }
    }
    let mut fl: Vec<usize> = faces.iter().map(|f| f.0).collect();
    // 外側の円柱（外向き）
    let pa = e.axis2(add(o, scale(a, q[0])), a, x);
    let cy = e.add(format!("CYLINDRICAL_SURFACE('',#{pa},{})", real(q[3])));
    let b1 = bound(e, &ol, true, true);
    let b2 = bound(e, &oh, false, false);
    fl.push(face(e, &[b1, b2], cy, true));
    // 内側の円柱（法線は軸向きなので、円柱の自然な向きと逆）
    if let (Some(il), Some(ih)) = (&faces[0].1, &faces[1].1) {
        let pa = e.axis2(add(o, scale(a, q[0])), a, x);
        let cy = e.add(format!("CYLINDRICAL_SURFACE('',#{pa},{})", real(q[2])));
        let b1 = bound(e, il, false, true);
        let b2 = bound(e, ih, true, false);
        fl.push(face(e, &[b1, b2], cy, false));
    }
    let fs: Vec<String> = fl.iter().map(|x| format!("#{x}")).collect();
    let shell = e.add(format!("CLOSED_SHELL('',({}))", fs.join(",")));
    e.add(format!(
        "MANIFOLD_SOLID_BREP('{}',#{shell})",
        text(&format!("{} {}", m.name, k + 1))
    ))
}

/// 節点を重複度でまとめた (重複度, 値) の STEP の書き方
fn knot_lists(knots: &[f64]) -> (String, String) {
    let mut kv: Vec<(f64, usize)> = Vec::new();
    for &k in knots {
        match kv.last_mut() {
            Some((v, m)) if *v == k => *m += 1,
            _ => kv.push((k, 1)),
        }
    }
    let m: Vec<String> = kv.iter().map(|(_, m)| m.to_string()).collect();
    let k: Vec<String> = kv.iter().map(|(v, _)| real(*v)).collect();
    (m.join(","), k.join(","))
}

/// 周の向き（u、θ = 0〜2π）に補間した閉じた曲線の制御点・節点
struct Ring {
    ctrl: Vec<V3>,
    knots: Vec<f64>,
}

/// 縁の曲線（3 次の B スプライン）の辺。頂点は θ = 0 の点
fn spline_edge(e: &mut Ents, r: &Ring) -> Circle {
    let pts: Vec<String> = r.ctrl.iter().map(|&p| format!("#{}", e.point(p))).collect();
    let (m, k) = knot_lists(&r.knots);
    let cu = e.add(format!(
        "B_SPLINE_CURVE_WITH_KNOTS('',3,({}),.UNSPECIFIED.,.T.,.F.,({m}),({k}),.UNSPECIFIED.)",
        pts.join(",")
    ));
    let p = e.point(r.ctrl[0]);
    let v = e.add(format!("VERTEX_POINT('',#{p})"));
    let edge = e.add(format!("EDGE_CURVE('',#{v},#{v},#{cu},.T.)"));
    Circle { edge }
}

/// 制御点の網（u: 周 3 次、v: 3 次か 1 次）の B スプライン曲面
fn spline_surface(e: &mut Ents, net: &[Vec<V3>], uk: &[f64], vk: &[f64], vdeg: usize) -> usize {
    let rows: Vec<String> = net
        .iter()
        .map(|row| {
            let r: Vec<String> = row.iter().map(|&p| format!("#{}", e.point(p))).collect();
            format!("({})", r.join(","))
        })
        .collect();
    let (um, uks) = knot_lists(uk);
    let (vm, vks) = knot_lists(vk);
    e.add(format!(
        "B_SPLINE_SURFACE_WITH_KNOTS('',3,{vdeg},({}),.UNSPECIFIED.,.T.,.F.,.F.,({um}),({vm}),({uks}),({vks}),.UNSPECIFIED.)",
        rows.join(",")
    ))
}

/// 2 つの周の曲線を結ぶ線織面（u: 周 3 次、v: 1 次）
fn spline_ruled(e: &mut Ents, a: &Ring, b: &Ring) -> usize {
    let net: Vec<Vec<V3>> = a
        .ctrl
        .iter()
        .zip(&b.ctrl)
        .map(|(&p, &q)| vec![p, q])
        .collect();
    spline_surface(e, &net, &a.knots, &[0.0, 0.0, 1.0, 1.0], 1)
}

/// 面 f の局所の点を、立体の円の座標系 ax の角度 θ・半径 ρ で
fn polar_point(f: &Face, ax: &Axes, th: f64, rho: f64) -> V3 {
    let y = cross(ax.a, ax.x);
    let d = add(scale(ax.x, th.cos()), scale(y, th.sin()));
    let (lx, ly) = (dot(d, col(&f.frame.r, 0)), dot(d, col(&f.frame.r, 1)));
    f.point(rho * lx, rho * ly)
}

/// 周の曲線（半径 rho の上の点を θ で補間）
fn polar_ring(f: &Face, ax: &Axes, rho: f64, m: usize) -> Ring {
    let q: Vec<V3> = (0..=m)
        .map(|i| {
            polar_point(
                f,
                ax,
                std::f64::consts::TAU * (i % m) as f64 / m as f64,
                rho,
            )
        })
        .collect();
    let (ctrl, knots) = interpolate3(&q);
    Ring { ctrl, knots }
}

/// 面を極座標の網（u: θ、v: ρ = 0〜semi）の B スプライン曲面にする。曲面と縁の曲線（v = 1 の等パラメータ線）を返す
fn polar_surface(
    e: &mut Ents,
    f: &Face,
    ax: &Axes,
    semi: f64,
    m: usize,
    n: usize,
) -> (usize, Ring) {
    // 先に ρ の向きに補間し、制御点の列を θ の向きに補間する（テンソル積）
    let mut vk = Vec::new();
    let cols: Vec<Vec<V3>> = (0..=m)
        .map(|i| {
            let th = std::f64::consts::TAU * (i % m) as f64 / m as f64;
            let q: Vec<V3> = (0..=n)
                .map(|j| polar_point(f, ax, th, semi * j as f64 / n as f64))
                .collect();
            let (c, k) = interpolate3(&q);
            vk = k;
            c
        })
        .collect();
    let mut uk = Vec::new();
    let mut net = vec![vec![[0.0; 3]; n + 1]; m + 1];
    for j in 0..=n {
        let q: Vec<V3> = cols.iter().map(|c| c[j]).collect();
        let (c, k) = interpolate3(&q);
        uk = k;
        for i in 0..=m {
            net[i][j] = c[i];
        }
    }
    // 頂点（ρ = 0）は 1 点に
    for row in net.iter_mut() {
        row[0] = cols[0][0];
    }
    let rim = Ring {
        ctrl: net.iter().map(|r| r[n]).collect(),
        knots: uk.clone(),
    };
    (spline_surface(e, &net, &uk, &vk, 3), rim)
}

/// トロイダル面を含む立体: 面・輪・側面をすべて B スプラインで書く（縁は B スプラインの曲線で共有する）。
/// 向きの決め方は [`solid`] と同じ（周の向き u は円の軸 a のまわりに反時計回り）
fn solid_spline(e: &mut Ents, sd: &Solid, o: &OptionsStep) -> usize {
    let m = o.profile_segments.max(8);
    let n = (o.profile_segments / 2).max(4);
    let dir = sd.direction();
    let fa = Axes {
        o: sd.front.frame.o,
        a: sd.front.axis(),
        x: col(&sd.front.frame.r, 0),
    };
    let mut ab = sd.back.axis();
    if dot(ab, fa.a) < 0.0 {
        ab = scale(ab, -1.0);
    }
    let ba = Axes {
        o: sd.back.frame.o,
        a: ab,
        x: normalize(sub(fa.x, scale(ab, dot(ab, fa.x)))),
    };
    let mut faces = Vec::new();
    let mut rims: Vec<(Circle, Ring, f64)> = Vec::new();
    for (f, ax, s) in [(&sd.front, fa, -dir), (&sd.back, ba, dir)] {
        let semi = if f.semi > 0.0 { f.semi } else { sd.rim };
        let flat = Face { semi, ..f.clone() };
        let (srf, inner_ring) = polar_surface(e, &flat, &ax, semi, m, n);
        let inner = spline_edge(e, &inner_ring);
        // 自然な法線 S_u × S_v は頂点の近くで −a
        let same = s < 0.0;
        let b = bound(e, &inner, loop_sense(s, true), true);
        faces.push(face(e, &[b], srf, same));
        let zc = dot(sub(flat.point(semi, 0.0), ax.o), ax.a);
        if sd.rim > semi * (1.0 + 1e-12) {
            // 輪: 縁の曲線と、同じ高さで外径まで広げた曲線を結ぶ線織面
            let outer_ring = polar_ring(&flat, &ax, sd.rim, m);
            let outer = spline_edge(e, &outer_ring);
            let pl = spline_ruled(e, &inner_ring, &outer_ring);
            let bo = bound(e, &outer, loop_sense(s, true), true);
            let bi = bound(e, &inner, loop_sense(s, false), false);
            faces.push(face(e, &[bo, bi], pl, same));
            rims.push((outer, outer_ring, zc));
        } else {
            rims.push((inner, inner_ring, zc));
        }
    }
    let zf = rims[0].2;
    let zb = dot(sub(add(ba.o, scale(ba.a, rims[1].2)), fa.o), fa.a);
    let lo_is_front = zf <= zb;
    let srf = spline_ruled(e, &rims[0].1, &rims[1].1);
    let (cl, ch) = if lo_is_front {
        (&rims[0].0, &rims[1].0)
    } else {
        (&rims[1].0, &rims[0].0)
    };
    let b1 = bound(e, cl, true, true);
    let b2 = bound(e, ch, false, false);
    faces.push(face(e, &[b1, b2], srf, lo_is_front));
    let fl: Vec<String> = faces.iter().map(|x| format!("#{x}")).collect();
    let shell = e.add(format!("CLOSED_SHELL('',({}))", fl.join(",")));
    e.add(format!(
        "MANIFOLD_SOLID_BREP('{}',#{shell})",
        text(&sd.name)
    ))
}

/// STEP を書き出す
pub fn write_step<W: Write>(w: &mut W, sc: &Scene, o: &OptionsStep) -> std::io::Result<StepReport> {
    let mut e = Ents { lines: Vec::new() };
    let mut rep = StepReport::default();
    // 製品の定義
    let app =
        e.add("APPLICATION_CONTEXT('core data for automotive mechanical design processes')".into());
    e.add(format!(
        "APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#{app})"
    ));
    let pc = e.add(format!("PRODUCT_CONTEXT('',#{app},'mechanical')"));
    let pdc = e.add(format!(
        "PRODUCT_DEFINITION_CONTEXT('part definition',#{app},'design')"
    ));
    let name = if sc.name.trim().is_empty() {
        "lensbench layout".to_string()
    } else {
        sc.name.trim().to_string()
    };
    let prod = e.add(format!("PRODUCT('{0}','{0}','',(#{pc}))", text(&name)));
    e.add(format!(
        "PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#{prod}))"
    ));
    let pdf = e.add(format!("PRODUCT_DEFINITION_FORMATION('','',#{prod})"));
    let pd = e.add(format!("PRODUCT_DEFINITION('design','',#{pdf},#{pdc})"));
    let pds = e.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{pd})"));
    let lu = e.add("(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.))".into());
    let au = e.add("(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.))".into());
    let su = e.add("(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT())".into());
    let un = e.add(format!(
        "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-07),#{lu},'distance_accuracy_value','confusion accuracy')"
    ));
    let ctx = e.add(format!(
        "(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{un}))GLOBAL_UNIT_ASSIGNED_CONTEXT((#{lu},#{au},#{su}))REPRESENTATION_CONTEXT('Context #1','3D Context with UNIT and UNCERTAINTY'))"
    ));
    // 立体
    let mut items = Vec::new();
    for sd in &sc.solids {
        let tor = sd.front.toroidal || sd.back.toroidal;
        items.push(if tor {
            solid_spline(&mut e, sd, o)
        } else {
            solid(&mut e, sd, o)
        });
        rep.solids += 1;
    }
    for m in &sc.mechs {
        for (k, q) in m.rings.iter().enumerate() {
            items.push(solid_ring(&mut e, m, k, *q));
            rep.mechs += 1;
        }
    }
    // 光線と面の縁
    let mut curves = Vec::new();
    for r in &sc.rays {
        if r.points.len() < 2 {
            continue;
        }
        let ps: Vec<String> = r
            .points
            .iter()
            .map(|&p| format!("#{}", e.point(p)))
            .collect();
        curves.push(e.add(format!(
            "POLYLINE('{} ({:.3},{:.3})',({}))",
            text(&crate::d2::ray_name(sc, r)),
            r.px,
            r.py,
            ps.join(",")
        )));
        rep.rays += 1;
    }
    for p in &sc.plates {
        let f = &p.face;
        if f.toroidal {
            // 縁は平面の円でないので、縁の点を補間した曲線
            let ax = Axes {
                o: f.frame.o,
                a: f.axis(),
                x: col(&f.frame.r, 0),
            };
            let r = polar_ring(f, &ax, f.semi, o.profile_segments.max(8));
            let pts: Vec<String> = r.ctrl.iter().map(|&q| format!("#{}", e.point(q))).collect();
            let (m, k) = knot_lists(&r.knots);
            curves.push(e.add(format!(
                "B_SPLINE_CURVE_WITH_KNOTS('{}',3,({}),.UNSPECIFIED.,.T.,.F.,({m}),({k}),.UNSPECIFIED.)",
                text(&p.name),
                pts.join(",")
            )));
            continue;
        }
        let c0 = f.to_global([0.0, 0.0, f.sag(f.semi)]);
        let pl = e.axis2(c0, f.axis(), col(&f.frame.r, 0));
        curves.push(e.add(format!(
            "CIRCLE('{}',#{pl},{})",
            text(&p.name),
            real(f.semi)
        )));
    }
    let origin = e.axis2([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
    let main_items: Vec<String> = std::iter::once(origin).map(|x| format!("#{x}")).collect();
    let main = e.add(format!(
        "SHAPE_REPRESENTATION('{}',({}),#{ctx})",
        text(&name),
        main_items.join(",")
    ));
    e.add(format!("SHAPE_DEFINITION_REPRESENTATION(#{pds},#{main})"));
    if !items.is_empty() {
        let mut it: Vec<String> = items.iter().map(|x| format!("#{x}")).collect();
        it.insert(0, format!("#{origin}"));
        let br = e.add(format!(
            "ADVANCED_BREP_SHAPE_REPRESENTATION('solids',({}),#{ctx})",
            it.join(",")
        ));
        e.add(format!(
            "SHAPE_REPRESENTATION_RELATIONSHIP('','',#{main},#{br})"
        ));
    }
    if !curves.is_empty() {
        let cl: Vec<String> = curves.iter().map(|x| format!("#{x}")).collect();
        let gs = e.add(format!("GEOMETRIC_CURVE_SET('rays',({}))", cl.join(",")));
        let wr = e.add(format!(
            "GEOMETRICALLY_BOUNDED_WIREFRAME_SHAPE_REPRESENTATION('rays',(#{origin},#{gs}),#{ctx})"
        ));
        e.add(format!(
            "SHAPE_REPRESENTATION_RELATIONSHIP('','',#{main},#{wr})"
        ));
    }
    writeln!(w, "ISO-10303-21;")?;
    writeln!(w, "HEADER;")?;
    writeln!(w, "FILE_DESCRIPTION(('lensbench layout'),'2;1');")?;
    writeln!(
        w,
        "FILE_NAME('{}','',(''),(''),'lensbench {}','lensbench','');",
        text(&name),
        lensbench_core::VERSION
    )?;
    writeln!(
        w,
        "FILE_SCHEMA(('AUTOMOTIVE_DESIGN {{ 1 0 10303 214 1 1 1 1 }}'));"
    )?;
    writeln!(w, "ENDSEC;")?;
    writeln!(w, "DATA;")?;
    for (i, l) in e.lines.iter().enumerate() {
        writeln!(w, "#{}={};", i + 1, l)?;
    }
    writeln!(w, "ENDSEC;")?;
    writeln!(w, "END-ISO-10303-21;")?;
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reals_have_a_decimal_point() {
        assert_eq!(real(0.0), "0.");
        assert_eq!(real(1.0), "1.");
        assert_eq!(real(-2.5), "-2.5");
        assert_eq!(real(1e-7), "1.E-7");
        assert_eq!(real(12345.678), "1.2345678E4");
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(text("a'b"), "a''b");
        assert_eq!(text("膜"), "\\X2\\819C\\X0\\");
    }

    #[test]
    fn spline_interpolates_and_reproduces_cubics() {
        let f = |r: f64| 0.3 * r * r - 0.01 * r * r * r;
        let q: Vec<[f64; 2]> = (0..=10).map(|k| [k as f64, f(k as f64)]).collect();
        let (c, k) = interpolate(&q);
        for (i, p) in q.iter().enumerate() {
            let v = bspline_eval(&c, &k, i as f64 / 10.0);
            assert!((v[0] - p[0]).abs() < 1e-12 && (v[1] - p[1]).abs() < 1e-12);
        }
        // 3 次以下の多項式はそのまま（間の点でも一致）
        for i in 0..100 {
            let u = i as f64 / 99.0;
            let v = bspline_eval(&c, &k, u);
            assert!((v[0] - 10.0 * u).abs() < 1e-12, "{u}");
            assert!((v[1] - f(10.0 * u)).abs() < 1e-11, "{u}");
        }
    }
}
