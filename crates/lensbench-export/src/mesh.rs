//! 3D の書き出し（glTF・OBJ・STL）: 立体と面を三角形の網に、光線を線（または細い管）にする。
//!
//! 座標は大域座標（z が光軸、y が上）。OBJ・STL は mm、glTF は規格どおり m で書く。

use std::io::Write;

use lensbench_core::math::*;
use serde_json::{json, Value};

use crate::d2::ColorBy;
use crate::scene::{Face, MechShape, PlateKind, Scene, Solid, SolidKind};
use crate::{palette, Rgb};

#[derive(Debug, Clone)]
pub struct Options3D {
    /// 周方向の分割数
    pub segments: usize,
    /// 半径方向の分割数
    pub rings: usize,
    /// 光線を管にするときの半径 [mm]（None なら glTF・OBJ は線、STL は場面の大きさの 0.1%）
    pub tube: Option<f64>,
    pub color: ColorBy,
    /// 機構部品を、取った STEP の詳しい形で描く（[`crate::detail`]。無い部品は簡易外形）
    pub mech_detail: bool,
}

impl Default for Options3D {
    fn default() -> Self {
        Options3D {
            segments: 64,
            rings: 16,
            tube: None,
            color: ColorBy::Field,
            mech_detail: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshKind {
    Glass,
    Mirror,
    Surface,
    /// 機構部品
    Mech,
    Ray,
}

/// 三角形（と線）の網
#[derive(Debug, Clone)]
pub struct Mesh {
    pub name: String,
    pub kind: MeshKind,
    pub color: Rgb,
    pub pos: Vec<V3>,
    pub nrm: Vec<V3>,
    pub tris: Vec<[u32; 3]>,
    pub lines: Vec<[u32; 2]>,
}

impl Mesh {
    fn new(name: &str, kind: MeshKind, color: Rgb) -> Mesh {
        Mesh {
            name: name.into(),
            kind,
            color,
            pos: Vec::new(),
            nrm: Vec::new(),
            tris: Vec::new(),
            lines: Vec::new(),
        }
    }

    fn vert(&mut self, p: V3, n: V3) -> u32 {
        self.pos.push(p);
        self.nrm.push(n);
        (self.pos.len() - 1) as u32
    }

    /// 三角形を、頂点の法線の側が表になる向きで足す
    fn tri(&mut self, a: u32, b: u32, c: u32) {
        let (pa, pb, pc) = (
            self.pos[a as usize],
            self.pos[b as usize],
            self.pos[c as usize],
        );
        let g = cross(sub(pb, pa), sub(pc, pa));
        if norm(g) == 0.0 {
            return;
        }
        let n = add(
            add(self.nrm[a as usize], self.nrm[b as usize]),
            self.nrm[c as usize],
        );
        if dot(g, n) < 0.0 {
            self.tris.push([a, c, b]);
        } else {
            self.tris.push([a, b, c]);
        }
    }

    /// 面の円板（中心から rim まで）。sgn は外向きの法線が局所 +z なら +1。周の角度は局所の単位ベクトル
    /// (ux, uy) から測る（前後の面で周の点の組を合わせ、側面がねじれないように）
    fn disk(&mut self, f: &Face, rim: f64, sgn: f64, o: &Options3D, (ux, uy): (V3, V3)) {
        let na = o.segments.max(8);
        let nr = o.rings.max(1);
        let mut radii: Vec<f64> = (1..=nr).map(|i| f.semi * i as f64 / nr as f64).collect();
        if f.semi <= 0.0 {
            radii.clear();
        }
        if rim > f.semi * (1.0 + 1e-12) {
            if f.semi > 0.0 {
                // 段差の内側の縁（平らな輪の始まり）
                radii.push(rim);
            } else {
                radii = vec![rim];
            }
        }
        let c = self.vert(f.point(0.0, 0.0), scale(f.normal(0.0, 0.0), sgn));
        let mut prev: Option<u32> = None;
        for &r in &radii {
            let base = self.pos.len() as u32;
            for a in 0..na {
                let t = 2.0 * std::f64::consts::PI * a as f64 / na as f64;
                let (x, y) = (
                    r * (t.cos() * ux[0] + t.sin() * uy[0]),
                    r * (t.cos() * ux[1] + t.sin() * uy[1]),
                );
                // 平らな輪の法線は軸の向き
                let n = if r > f.semi * (1.0 + 1e-12) {
                    f.normal(f.semi * 2.0 + 1.0, 0.0)
                } else {
                    f.normal(x, y)
                };
                self.vert(f.point(x, y), scale(n, sgn));
            }
            for a in 0..na as u32 {
                let b = (a + 1) % na as u32;
                match prev {
                    None => self.tri(c, base + a, base + b),
                    Some(p) => {
                        self.tri(p + a, base + a, base + b);
                        self.tri(p + a, base + b, p + b);
                    }
                }
            }
            prev = Some(base);
        }
    }
}

fn solid_mesh(sd: &Solid, o: &Options3D) -> Mesh {
    let (kind, color) = match sd.kind {
        SolidKind::Glass => (MeshKind::Glass, Rgb(170, 205, 240)),
        SolidKind::Mirror => (MeshKind::Mirror, Rgb(190, 190, 196)),
    };
    let mut m = Mesh::new(&sd.name, kind, color);
    if let Some(p) = &sd.poly {
        // 凸多面体: 面ごとに平らな三角形の扇
        for (nv, f) in &p.faces {
            let v: Vec<u32> = f.iter().map(|&i| m.vert(p.pts[i], *nv)).collect();
            for k in 1..v.len().saturating_sub(1) {
                m.tri(v[0], v[k], v[k + 1]);
            }
        }
        return m;
    }
    let dir = sd.direction();
    // 周の角度の基準: 前の面は局所の x・y、後ろの面は Solid::back_basis（側面がねじれない組）
    let (bx, by) = sd.back_basis();
    let fb = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    let bb = (mtv(&sd.back.frame.r, bx), mtv(&sd.back.frame.r, by));
    m.disk(&sd.front, sd.rim, -dir, o, fb);
    // 後ろの面の外向きは、後ろの面の局所 z で前の面と反対の側
    let bd = if dot(sd.back.axis(), sd.front.axis()) * dir >= 0.0 {
        1.0
    } else {
        -1.0
    };
    m.disk(&sd.back, sd.rim, bd, o, bb);
    // 側面
    let na = o.segments.max(8);
    let base = m.pos.len() as u32;
    for a in 0..na {
        let t = 2.0 * std::f64::consts::PI * a as f64 / na as f64;
        let (c, s) = (t.cos(), t.sin());
        let n = mv(&sd.front.frame.r, [c, s, 0.0]);
        m.vert(sd.front.point(sd.rim * c, sd.rim * s), n);
        let lb = add(scale(bb.0, c), scale(bb.1, s));
        let nb = add(scale(bx, c), scale(by, s));
        m.vert(sd.back.point(sd.rim * lb[0], sd.rim * lb[1]), nb);
    }
    for a in 0..na as u32 {
        let b = (a + 1) % na as u32;
        let (f0, b0, f1, b1) = (
            base + 2 * a,
            base + 2 * a + 1,
            base + 2 * b,
            base + 2 * b + 1,
        );
        m.tri(f0, b0, b1);
        m.tri(f0, b1, f1);
    }
    m
}

/// 光線の折れ線を細い管（6 角柱）にする
fn tube(m: &mut Mesh, pts: &[V3], r: f64) {
    const N: usize = 6;
    for s in pts.windows(2) {
        let d = sub(s[1], s[0]);
        let l = norm(d);
        if l <= 0.0 {
            continue;
        }
        let d = scale(d, 1.0 / l);
        let t = if d[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let u = normalize(cross(d, t));
        let v = cross(d, u);
        let base = m.pos.len() as u32;
        for k in 0..N {
            let a = 2.0 * std::f64::consts::PI * k as f64 / N as f64;
            let n = add(scale(u, a.cos()), scale(v, a.sin()));
            m.vert(add(s[0], scale(n, r)), n);
            m.vert(add(s[1], scale(n, r)), n);
        }
        for k in 0..N as u32 {
            let j = (k + 1) % N as u32;
            let (a0, a1, b0, b1) = (
                base + 2 * k,
                base + 2 * k + 1,
                base + 2 * j,
                base + 2 * j + 1,
            );
            m.tri(a0, b0, b1);
            m.tri(a0, b1, a1);
        }
    }
}

/// 機構部品の網: 輪ごとに外側（円柱か、角なら正方形の柱）・内側の円柱の面と両端の輪の面
fn mech_mesh(m: &MechShape, o: &Options3D) -> Mesh {
    if o.mech_detail {
        if let Some(d) = m
            .step
            .as_ref()
            .and_then(|(f, flip)| Some((crate::detail::load(f)?, *flip)))
        {
            return detail_mesh(m, &d.0, d.1);
        }
    }
    let mut out = Mesh::new(&m.name, MeshKind::Mech, Rgb(150, 140, 128));
    // 角は正方形の角（45° + k·90°）を周の点に含める（4 の倍数、45° から）
    let (na, t0) = if m.square {
        (
            o.segments.max(8).div_ceil(4) * 4,
            std::f64::consts::FRAC_PI_4,
        )
    } else {
        (o.segments.max(8), 0.0)
    };
    let ang = |a: usize| t0 + 2.0 * std::f64::consts::PI * a as f64 / na as f64;
    // 角度 t の向きの外側の境の点（局所の xy）。角は正方形の辺の上
    let outer_at = |r: f64, t: f64| -> [f64; 2] {
        let (c, s) = (t.cos(), t.sin());
        let k = if m.square {
            r / c.abs().max(s.abs())
        } else {
            r
        };
        [k * c, k * s]
    };
    let ax = m.axis();
    for q in &m.rings {
        let gdir = |t: f64| mv(&m.frame.r, [t.cos(), t.sin(), 0.0]);
        // 外側の面
        if q[3] > 0.0 {
            if m.square {
                // 周の点（角を含む）を結ぶ平らな四角。法線は辺ごと（両端の輪と同じ点なので継ぎ目が合う）
                for a in 0..na {
                    let (p0, p1) = (outer_at(q[3], ang(a)), outer_at(q[3], ang((a + 1) % na)));
                    let mid = [0.5 * (p0[0] + p1[0]), 0.5 * (p0[1] + p1[1])];
                    let n = if mid[0].abs() >= mid[1].abs() {
                        [mid[0].signum(), 0.0, 0.0]
                    } else {
                        [0.0, mid[1].signum(), 0.0]
                    };
                    let n = mv(&m.frame.r, n);
                    let base = out.pos.len() as u32;
                    for p in [p0, p1] {
                        out.vert(m.to_global([p[0], p[1], q[0]]), n);
                        out.vert(m.to_global([p[0], p[1], q[1]]), n);
                    }
                    out.tri(base, base + 2, base + 3);
                    out.tri(base, base + 3, base + 1);
                }
            } else {
                let base = out.pos.len() as u32;
                for a in 0..na {
                    let t = ang(a);
                    let n = gdir(t);
                    let p = outer_at(q[3], t);
                    out.vert(m.to_global([p[0], p[1], q[0]]), n);
                    out.vert(m.to_global([p[0], p[1], q[1]]), n);
                }
                for a in 0..na as u32 {
                    let b = (a + 1) % na as u32;
                    out.tri(base + 2 * a, base + 2 * b, base + 2 * b + 1);
                    out.tri(base + 2 * a, base + 2 * b + 1, base + 2 * a + 1);
                }
            }
        }
        // 内側の円柱の面（法線は軸向き）
        if q[2] > 0.0 {
            let base = out.pos.len() as u32;
            for a in 0..na {
                let t = ang(a);
                let n = scale(gdir(t), -1.0);
                let r = q[2];
                out.vert(m.to_global([r * t.cos(), r * t.sin(), q[0]]), n);
                out.vert(m.to_global([r * t.cos(), r * t.sin(), q[1]]), n);
            }
            for a in 0..na as u32 {
                let b = (a + 1) % na as u32;
                out.tri(base + 2 * a, base + 2 * b, base + 2 * b + 1);
                out.tri(base + 2 * a, base + 2 * b + 1, base + 2 * a + 1);
            }
        }
        // 両端の輪（内側 0 なら円板・正方形の板）
        for (z, sgn) in [(q[0], -1.0), (q[1], 1.0)] {
            let n = scale(ax, sgn);
            let base = out.pos.len() as u32;
            for a in 0..na {
                let t = ang(a);
                let p = outer_at(q[3], t);
                out.vert(m.to_global([q[2] * t.cos(), q[2] * t.sin(), z]), n);
                out.vert(m.to_global([p[0], p[1], z]), n);
            }
            for a in 0..na as u32 {
                let b = (a + 1) % na as u32;
                out.tri(base + 2 * a, base + 2 * a + 1, base + 2 * b + 1);
                out.tri(base + 2 * a, base + 2 * b + 1, base + 2 * b);
            }
        }
    }
    out
}

/// 機構部品の詳しい形の網（三角形ごとに平らな法線）。裏返して置いた部品は z → 長さ − z、y → −y
/// （x 軸のまわりの 180° の回転）
fn detail_mesh(m: &MechShape, d: &crate::detail::Detail, flip: bool) -> Mesh {
    let mut out = Mesh::new(&m.name, MeshKind::Mech, Rgb(150, 140, 128));
    let len = m.rings.iter().map(|q| q[1]).fold(0.0, f64::max);
    let place = |p: V3| {
        let q = if flip { [p[0], -p[1], len - p[2]] } else { p };
        m.to_global(q)
    };
    for t in &d.tris {
        let [a, b, c] = t.map(place);
        let n = cross(sub(b, a), sub(c, a));
        let l = norm(n);
        if l.is_nan() || l <= 0.0 {
            continue;
        }
        let n = scale(n, 1.0 / l);
        let base = out.pos.len() as u32;
        for p in [a, b, c] {
            out.vert(p, n);
        }
        out.tri(base, base + 1, base + 2);
    }
    out
}

/// 場面を網の並びにする。tube が Some なら光線を管に、None なら線にする
pub fn meshes(sc: &Scene, o: &Options3D, tube_r: Option<f64>) -> Vec<Mesh> {
    let mut v: Vec<Mesh> = sc.solids.iter().map(|s| solid_mesh(s, o)).collect();
    for p in &sc.plates {
        let mut m = Mesh::new(&p.name, MeshKind::Surface, Rgb(120, 120, 120));
        if p.kind == PlateKind::Paraxial {
            m.color = Rgb(160, 160, 160);
        }
        m.disk(
            &p.face,
            p.face.semi,
            1.0,
            o,
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        );
        v.push(m);
    }
    v.extend(sc.mechs.iter().map(|m| mech_mesh(m, o)));
    // 光線は色ごとに 1 つの網
    let mut groups: Vec<(usize, Mesh)> = Vec::new();
    for r in &sc.rays {
        if r.points.len() < 2 {
            continue;
        }
        let key = match o.color {
            ColorBy::Field => r.field,
            ColorBy::Wave => r.wave,
            ColorBy::Config => r.cfg,
        };
        let gi = match groups.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let name = match o.color {
                    ColorBy::Field => format!("rays F{}", key + 1),
                    ColorBy::Wave => format!("rays W{}", key + 1),
                    ColorBy::Config => match sc.configs.get(key) {
                        Some(c) => format!("rays {c}"),
                        None => format!("rays C{}", key + 1),
                    },
                };
                groups.push((key, Mesh::new(&name, MeshKind::Ray, palette(key))));
                groups.len() - 1
            }
        };
        let m = &mut groups[gi].1;
        match tube_r {
            Some(t) => tube(m, &r.points, t),
            None => {
                let base = m.pos.len() as u32;
                for &p in &r.points {
                    m.vert(p, [0.0; 3]);
                }
                for i in 0..r.points.len() as u32 - 1 {
                    m.lines.push([base + i, base + i + 1]);
                }
            }
        }
    }
    v.extend(groups.into_iter().map(|(_, m)| m));
    v
}

fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect()
}

/// OBJ（mm）。光線は `l` の線（tube を指定すれば管）
pub fn write_obj<W: Write>(w: &mut W, sc: &Scene, o: &Options3D) -> std::io::Result<()> {
    writeln!(w, "# lensbench layout: {}", sc.name.replace('\n', " "))?;
    writeln!(w, "# units: mm, z = optical axis, y = up")?;
    let mut off = 1usize;
    for m in meshes(sc, o, o.tube) {
        writeln!(w, "o {}", safe_name(&m.name))?;
        for p in &m.pos {
            writeln!(w, "v {} {} {}", p[0], p[1], p[2])?;
        }
        let has_n = !m.tris.is_empty();
        if has_n {
            for n in &m.nrm {
                writeln!(w, "vn {} {} {}", n[0], n[1], n[2])?;
            }
        }
        for t in &m.tris {
            let [a, b, c] = t.map(|i| i as usize + off);
            writeln!(w, "f {a}//{a} {b}//{b} {c}//{c}")?;
        }
        for l in &m.lines {
            writeln!(w, "l {} {}", l[0] as usize + off, l[1] as usize + off)?;
        }
        off += m.pos.len();
    }
    Ok(())
}

/// STL（バイナリ、mm）。STL は三角形しか持てないので、光線は管にする
pub fn write_stl<W: Write>(w: &mut W, sc: &Scene, o: &Options3D) -> std::io::Result<()> {
    let r = o.tube.unwrap_or(sc.size() * 1e-3);
    let ms = meshes(sc, o, Some(r));
    let n: usize = ms.iter().map(|m| m.tris.len()).sum();
    let mut head = [b' '; 80];
    let h = b"lensbench layout (mm)";
    head[..h.len()].copy_from_slice(h);
    w.write_all(&head)?;
    w.write_all(&(n as u32).to_le_bytes())?;
    for m in &ms {
        for t in &m.tris {
            let [a, b, c] = t.map(|i| m.pos[i as usize]);
            let g = normalize(cross(sub(b, a), sub(c, a)));
            for v in [g, a, b, c] {
                for x in v {
                    w.write_all(&(x as f32).to_le_bytes())?;
                }
            }
            w.write_all(&0u16.to_le_bytes())?;
        }
    }
    Ok(())
}

/// glTF 2.0（m）。glb なら 1 つのバイナリ、そうでなければ JSON にバッファを base64 で埋め込む
pub fn write_gltf<W: Write>(
    w: &mut W,
    sc: &Scene,
    o: &Options3D,
    glb: bool,
) -> std::io::Result<()> {
    const MM: f64 = 1e-3;
    let ms = meshes(sc, o, o.tube);
    let mut buf: Vec<u8> = Vec::new();
    let mut views: Vec<Value> = Vec::new();
    let mut accs: Vec<Value> = Vec::new();
    let mut meshes_j: Vec<Value> = Vec::new();
    let mut nodes: Vec<Value> = Vec::new();
    let mut mats: Vec<Value> = Vec::new();
    let mut add_view = |buf: &mut Vec<u8>, bytes: &[u8], target: u32| -> usize {
        while !buf.len().is_multiple_of(4) {
            buf.push(0);
        }
        let off = buf.len();
        buf.extend_from_slice(bytes);
        views.push(
            json!({"buffer": 0, "byteOffset": off, "byteLength": bytes.len(), "target": target}),
        );
        views.len() - 1
    };
    for m in &ms {
        if m.pos.is_empty() || (m.tris.is_empty() && m.lines.is_empty()) {
            continue;
        }
        let mut pb = Vec::with_capacity(m.pos.len() * 12);
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in &m.pos {
            for i in 0..3 {
                let v = (p[i] * MM) as f32;
                lo[i] = lo[i].min(v);
                hi[i] = hi[i].max(v);
                pb.extend_from_slice(&v.to_le_bytes());
            }
        }
        let pv = add_view(&mut buf, &pb, 34962);
        accs.push(
            json!({"bufferView": pv, "componentType": 5126, "count": m.pos.len(),
            "type": "VEC3", "min": lo, "max": hi}),
        );
        let pa = accs.len() - 1;
        let mut attrs = json!({"POSITION": pa});
        if !m.tris.is_empty() {
            let mut nb = Vec::with_capacity(m.nrm.len() * 12);
            for n in &m.nrm {
                for x in n {
                    nb.extend_from_slice(&(*x as f32).to_le_bytes());
                }
            }
            let nv = add_view(&mut buf, &nb, 34962);
            accs.push(json!({"bufferView": nv, "componentType": 5126, "count": m.nrm.len(), "type": "VEC3"}));
            attrs["NORMAL"] = json!(accs.len() - 1);
        }
        let (idx, mode): (Vec<u32>, u32) = if !m.tris.is_empty() {
            (m.tris.iter().flatten().copied().collect(), 4)
        } else {
            (m.lines.iter().flatten().copied().collect(), 1)
        };
        let ib: Vec<u8> = idx.iter().flat_map(|i| i.to_le_bytes()).collect();
        let iv = add_view(&mut buf, &ib, 34963);
        accs.push(
            json!({"bufferView": iv, "componentType": 5125, "count": idx.len(), "type": "SCALAR"}),
        );
        let ia = accs.len() - 1;
        let c = m.color.linear();
        let mat = match m.kind {
            MeshKind::Glass => json!({"name": "glass",
                "pbrMetallicRoughness": {"baseColorFactor": [c[0], c[1], c[2], 0.45], "metallicFactor": 0.0, "roughnessFactor": 0.1},
                "alphaMode": "BLEND", "doubleSided": true}),
            MeshKind::Mirror => json!({"name": "mirror",
                "pbrMetallicRoughness": {"baseColorFactor": [c[0], c[1], c[2], 1.0], "metallicFactor": 1.0, "roughnessFactor": 0.2}}),
            MeshKind::Surface => json!({"name": "surface",
                "pbrMetallicRoughness": {"baseColorFactor": [c[0], c[1], c[2], 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.8},
                "doubleSided": true}),
            MeshKind::Mech => json!({"name": "mech",
                "pbrMetallicRoughness": {"baseColorFactor": [c[0], c[1], c[2], 1.0], "metallicFactor": 0.6, "roughnessFactor": 0.5}}),
            MeshKind::Ray => json!({"name": m.name,
                "pbrMetallicRoughness": {"baseColorFactor": [c[0], c[1], c[2], 1.0], "metallicFactor": 0.0, "roughnessFactor": 1.0},
                "emissiveFactor": [c[0], c[1], c[2]]}),
        };
        mats.push(mat);
        meshes_j.push(
            json!({"name": m.name, "primitives": [{"attributes": attrs, "indices": ia,
            "mode": mode, "material": mats.len() - 1}]}),
        );
        nodes.push(json!({"name": m.name, "mesh": meshes_j.len() - 1}));
    }
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
    let mut doc = json!({
        "asset": {"version": "2.0", "generator": format!("lensbench {}", lensbench_core::VERSION)},
        "scene": 0,
        "scenes": [{"name": sc.name, "nodes": (0..nodes.len()).collect::<Vec<_>>()}],
        "nodes": nodes, "meshes": meshes_j, "materials": mats,
        "accessors": accs, "bufferViews": views,
        "buffers": [{"byteLength": buf.len()}],
    });
    if glb {
        let mut js = serde_json::to_vec(&doc).unwrap();
        while !js.len().is_multiple_of(4) {
            js.push(b' ');
        }
        let total = 12 + 8 + js.len() + 8 + buf.len();
        w.write_all(b"glTF")?;
        w.write_all(&2u32.to_le_bytes())?;
        w.write_all(&(total as u32).to_le_bytes())?;
        w.write_all(&(js.len() as u32).to_le_bytes())?;
        w.write_all(b"JSON")?;
        w.write_all(&js)?;
        w.write_all(&(buf.len() as u32).to_le_bytes())?;
        w.write_all(b"BIN\0")?;
        w.write_all(&buf)?;
    } else {
        doc["buffers"][0]["uri"] = json!(format!(
            "data:application/octet-stream;base64,{}",
            base64(&buf)
        ));
        serde_json::to_writer_pretty(&mut *w, &doc).map_err(std::io::Error::other)?;
        writeln!(w)?;
    }
    Ok(())
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let v = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(v >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
