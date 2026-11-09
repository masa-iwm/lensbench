//! 2D の書き出し（SVG・PNG・DXF）: 場面を大域の yz 面（または xz 面）へ投影した断面図。
//!
//! 横軸は大域 z（光軸）、縦軸は大域 y（`Plane::XZ` なら x）。単位は mm。

use std::io::Write;

use lensbench_core::math::V3;

use crate::scene::{
    face_section, face_section_xz, solid_section, solid_section_xz, PlateKind, Scene, SolidKind,
};
use crate::{palette, Rgb};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    YZ,
    XZ,
}

impl std::str::FromStr for Plane {
    type Err = String;
    fn from_str(s: &str) -> Result<Plane, String> {
        match s.to_ascii_lowercase().as_str() {
            "yz" => Ok(Plane::YZ),
            "xz" => Ok(Plane::XZ),
            _ => Err(format!("投影面 {s} は yz か xz")),
        }
    }
}

/// 光線の色の付け方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBy {
    Field,
    Wave,
    /// 重ね描きの構成（[`crate::scene::overlay`]）
    Config,
}

impl std::str::FromStr for ColorBy {
    type Err = String;
    fn from_str(s: &str) -> Result<ColorBy, String> {
        match s.to_ascii_lowercase().as_str() {
            "field" => Ok(ColorBy::Field),
            "wave" => Ok(ColorBy::Wave),
            "cfg" | "config" => Ok(ColorBy::Config),
            _ => Err(format!("色分け {s} は field・wave・cfg のどれか")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options2D {
    pub plane: Plane,
    pub color: ColorBy,
    /// PNG の横幅 [px]（SVG の width 属性にも使う）
    pub width: u32,
    /// 面の線の分割数
    pub segments: usize,
}

impl Default for Options2D {
    fn default() -> Self {
        Options2D {
            plane: Plane::YZ,
            color: ColorBy::Field,
            width: 1600,
            segments: 64,
        }
    }
}

/// 図の要素の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Lens,
    Mirror,
    Surface,
    /// 機構部品
    Mech,
    Rays,
}

impl Layer {
    pub fn name(self) -> &'static str {
        match self {
            Layer::Lens => "LENS",
            Layer::Mirror => "MIRROR",
            Layer::Surface => "SURFACE",
            Layer::Mech => "MECH",
            Layer::Rays => "RAYS",
        }
    }
}

/// 2D の図形（座標は [横, 縦] mm）
#[derive(Debug, Clone)]
pub struct Shape {
    pub layer: Layer,
    pub pts: Vec<[f64; 2]>,
    pub closed: bool,
    pub fill: Option<Rgb>,
    pub stroke: Rgb,
    /// 線の太さ [px]
    pub width: f64,
    pub name: String,
}

/// 場面を投影した図形の並び（塗りつぶしを先、光線を後）
pub fn shapes(sc: &Scene, o: &Options2D) -> Vec<Shape> {
    let pr = |p: V3| -> [f64; 2] {
        match o.plane {
            Plane::YZ => [p[2], p[1]],
            Plane::XZ => [p[2], p[0]],
        }
    };
    let n = o.segments.max(4);
    let black = Rgb(0, 0, 0);
    let mut v = Vec::new();
    for sd in &sc.solids {
        let pts = if o.plane == Plane::YZ {
            solid_section(sd, n)
        } else {
            solid_section_xz(sd, n)
        };
        let (layer, fill) = match sd.kind {
            SolidKind::Glass => (Layer::Lens, Rgb(205, 228, 248)),
            SolidKind::Mirror => (Layer::Mirror, Rgb(190, 190, 196)),
        };
        v.push(Shape {
            layer,
            pts: pts.into_iter().map(pr).collect(),
            closed: true,
            fill: Some(fill),
            stroke: black,
            width: 1.2,
            name: sd.name.clone(),
        });
    }
    for m in &sc.mechs {
        for q in m.section(o.plane == Plane::XZ) {
            v.push(Shape {
                layer: Layer::Mech,
                pts: q.into_iter().map(pr).collect(),
                closed: true,
                fill: Some(Rgb(214, 204, 188)),
                stroke: Rgb(110, 96, 80),
                width: 1.0,
                name: m.name.clone(),
            });
        }
    }
    for p in &sc.plates {
        let pts = if o.plane == Plane::YZ {
            face_section(&p.face, n)
        } else {
            face_section_xz(&p.face, n)
        };
        v.push(Shape {
            layer: Layer::Surface,
            pts: pts.into_iter().map(pr).collect(),
            closed: false,
            fill: None,
            stroke: if p.kind == PlateKind::Paraxial {
                Rgb(90, 90, 90)
            } else {
                black
            },
            width: 1.2,
            name: p.name.clone(),
        });
    }
    for r in &sc.rays {
        if r.points.len() < 2 {
            continue;
        }
        let c = match o.color {
            ColorBy::Field => palette(r.field),
            ColorBy::Wave => palette(r.wave),
            ColorBy::Config => palette(r.cfg),
        };
        v.push(Shape {
            layer: Layer::Rays,
            pts: r.points.iter().map(|&p| pr(p)).collect(),
            closed: false,
            fill: None,
            stroke: c,
            width: 1.0,
            name: ray_name(sc, r),
        });
    }
    v
}

/// 光線の名前（重ね描きなら頭に構成の名前）
pub(crate) fn ray_name(sc: &Scene, r: &crate::scene::RayPath) -> String {
    let n = format!("F{} W{}", r.field + 1, r.wave + 1);
    match sc.configs.get(r.cfg).filter(|_| sc.configs.len() > 1) {
        Some(c) => format!("{c} {n}"),
        None => n,
    }
}

/// 図形の箱（[横min, 縦min], [横max, 縦max]）。空なら単位の箱
pub fn bbox(v: &[Shape]) -> ([f64; 2], [f64; 2]) {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for s in v {
        for p in &s.pts {
            if p[0].is_finite() && p[1].is_finite() {
                for i in 0..2 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
            }
        }
    }
    if !lo[0].is_finite() {
        return ([0.0, 0.0], [1.0, 1.0]);
    }
    for i in 0..2 {
        if hi[i] - lo[i] < 1e-9 {
            lo[i] -= 0.5;
            hi[i] += 0.5;
        }
    }
    (lo, hi)
}

/// 余白（大きい方の辺の 4%）を付けた箱
fn framed(v: &[Shape]) -> ([f64; 2], [f64; 2]) {
    let (lo, hi) = bbox(v);
    let m = 0.04 * (hi[0] - lo[0]).max(hi[1] - lo[1]);
    ([lo[0] - m, lo[1] - m], [hi[0] + m, hi[1] + m])
}

fn num(v: f64) -> String {
    // 0.1 µm より細かい桁は書かない
    let s = format!("{:.5}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

/// SVG（viewBox は mm、縦は上向きが正になるよう反転して書く）
pub fn write_svg<W: Write>(w: &mut W, sc: &Scene, o: &Options2D) -> std::io::Result<()> {
    let v = shapes(sc, o);
    let (lo, hi) = framed(&v);
    let (dw, dh) = (hi[0] - lo[0], hi[1] - lo[1]);
    let px_w = o.width.max(16) as f64;
    let px_h = (px_w * dh / dw).round().max(1.0);
    writeln!(
        w,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{} {} {} {}">"#,
        px_w,
        px_h,
        num(lo[0]),
        num(-hi[1]),
        num(dw),
        num(dh)
    )?;
    writeln!(w, "<title>{}</title>", xml_escape(&sc.name))?;
    writeln!(
        w,
        r#"<rect x="{}" y="{}" width="{}" height="{}" fill="white"/>"#,
        num(lo[0]),
        num(-hi[1]),
        num(dw),
        num(dh)
    )?;
    let mut cur: Option<Layer> = None;
    for s in &v {
        if cur != Some(s.layer) {
            if cur.is_some() {
                writeln!(w, "</g>")?;
            }
            writeln!(w, r#"<g id="{}">"#, s.layer.name().to_ascii_lowercase())?;
            cur = Some(s.layer);
        }
        let pts: Vec<String> = s
            .pts
            .iter()
            .map(|p| format!("{},{}", num(p[0]), num(-p[1])))
            .collect();
        let tag = if s.closed { "polygon" } else { "polyline" };
        let fill = match s.fill {
            Some(c) => c.hex(),
            None => "none".into(),
        };
        writeln!(
            w,
            r#"<{tag} points="{}" fill="{fill}" stroke="{}" stroke-width="{}" vector-effect="non-scaling-stroke" stroke-linejoin="round"><title>{}</title></{tag}>"#,
            pts.join(" "),
            s.stroke.hex(),
            s.width,
            xml_escape(&s.name)
        )?;
    }
    if cur.is_some() {
        writeln!(w, "</g>")?;
    }
    writeln!(w, "</svg>")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// AutoCAD の色番号（ACI）に近いもの
fn aci(c: Rgb, layer: Layer) -> i32 {
    if layer != Layer::Rays {
        return 7;
    }
    // 光線の色: 赤 1、黄 2、緑 3、シアン 4、青 5、マゼンタ 6 のうち近いもの
    let t = [
        (1, Rgb(255, 0, 0)),
        (2, Rgb(255, 255, 0)),
        (3, Rgb(0, 255, 0)),
        (4, Rgb(0, 255, 255)),
        (5, Rgb(0, 0, 255)),
        (6, Rgb(255, 0, 255)),
        (30, Rgb(255, 127, 0)),
    ];
    let d = |a: Rgb, b: Rgb| {
        let f = |x: u8, y: u8| (x as i32 - y as i32).pow(2);
        f(a.0, b.0) + f(a.1, b.1) + f(a.2, b.2)
    };
    t.iter().min_by_key(|(_, r)| d(c, *r)).unwrap().0
}

/// DXF（R12 の ASCII、単位 mm。レイヤー LENS・MIRROR・SURFACE・MECH・RAYS に分ける）
pub fn write_dxf<W: Write>(w: &mut W, sc: &Scene, o: &Options2D) -> std::io::Result<()> {
    let v = shapes(sc, o);
    let (lo, hi) = bbox(&v);
    let g = |w: &mut W, code: i32, val: &str| writeln!(w, "{code:>3}\n{val}");
    g(w, 0, "SECTION")?;
    g(w, 2, "HEADER")?;
    g(w, 9, "$ACADVER")?;
    g(w, 1, "AC1009")?;
    g(w, 9, "$EXTMIN")?;
    g(w, 10, &num(lo[0]))?;
    g(w, 20, &num(lo[1]))?;
    g(w, 9, "$EXTMAX")?;
    g(w, 10, &num(hi[0]))?;
    g(w, 20, &num(hi[1]))?;
    g(w, 0, "ENDSEC")?;
    g(w, 0, "SECTION")?;
    g(w, 2, "TABLES")?;
    g(w, 0, "TABLE")?;
    g(w, 2, "LAYER")?;
    g(w, 70, "5")?;
    for (l, c) in [
        (Layer::Lens, 5),
        (Layer::Mirror, 8),
        (Layer::Surface, 7),
        (Layer::Mech, 9),
        (Layer::Rays, 1),
    ] {
        g(w, 0, "LAYER")?;
        g(w, 2, l.name())?;
        g(w, 70, "0")?;
        g(w, 62, &c.to_string())?;
        g(w, 6, "CONTINUOUS")?;
    }
    g(w, 0, "ENDTAB")?;
    g(w, 0, "ENDSEC")?;
    g(w, 0, "SECTION")?;
    g(w, 2, "ENTITIES")?;
    for s in &v {
        g(w, 0, "POLYLINE")?;
        g(w, 8, s.layer.name())?;
        g(w, 62, &aci(s.stroke, s.layer).to_string())?;
        g(w, 66, "1")?;
        g(w, 10, "0")?;
        g(w, 20, "0")?;
        g(w, 30, "0")?;
        g(w, 70, if s.closed { "1" } else { "0" })?;
        for p in &s.pts {
            g(w, 0, "VERTEX")?;
            g(w, 8, s.layer.name())?;
            g(w, 10, &num(p[0]))?;
            g(w, 20, &num(p[1]))?;
            g(w, 30, "0")?;
        }
        g(w, 0, "SEQEND")?;
        g(w, 8, s.layer.name())?;
    }
    g(w, 0, "ENDSEC")?;
    g(w, 0, "EOF")
}

/// 画素の画像（RGB、白地）
pub struct Raster {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 3]>,
}

impl Raster {
    fn blend(&mut self, x: i64, y: i64, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let p = &mut self.px[y as usize * self.w + x as usize];
        let cc = [c.0, c.1, c.2].map(|v| v as f32 / 255.0);
        for i in 0..3 {
            p[i] = p[i] * (1.0 - a) + cc[i] * a;
        }
    }

    /// 多角形の塗りつぶし（偶奇規則、縦 4 段の副走査線で縁をなめらかに）
    fn fill(&mut self, pts: &[[f64; 2]], c: Rgb) {
        const SUB: usize = 4;
        let n = pts.len();
        if n < 3 {
            return;
        }
        let ymin = pts
            .iter()
            .map(|p| p[1])
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as usize;
        let ymax = (pts
            .iter()
            .map(|p| p[1])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil() as i64)
            .min(self.h as i64 - 1);
        if ymax < 0 {
            return;
        }
        let mut cov = vec![0.0f32; self.w + 2];
        for y in ymin..=ymax as usize {
            cov.iter_mut().for_each(|v| *v = 0.0);
            let mut touched = false;
            for s in 0..SUB {
                let yy = y as f64 + (s as f64 + 0.5) / SUB as f64;
                let mut xs: Vec<f64> = Vec::new();
                for i in 0..n {
                    let (a, b) = (pts[i], pts[(i + 1) % n]);
                    if (a[1] <= yy) != (b[1] <= yy) {
                        xs.push(a[0] + (yy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                    }
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                for p in xs.chunks(2) {
                    if p.len() < 2 {
                        break;
                    }
                    let (x0, x1) = (p[0].max(0.0), p[1].min(self.w as f64));
                    if x1 <= x0 {
                        continue;
                    }
                    touched = true;
                    let (i0, i1) = (x0.floor() as usize, (x1.ceil() as usize).min(self.w));
                    for i in i0..i1 {
                        let l = (i as f64).max(x0);
                        let r = ((i + 1) as f64).min(x1);
                        if r > l {
                            cov[i] += ((r - l) / SUB as f64) as f32;
                        }
                    }
                }
            }
            if touched {
                for x in 0..self.w {
                    if cov[x] > 0.0 {
                        self.blend(x as i64, y as i64, c, cov[x]);
                    }
                }
            }
        }
    }

    /// 太さ w [px] の線分（端は丸めない、縁は距離でなめらかに）
    fn line(&mut self, a: [f64; 2], b: [f64; 2], c: Rgb, w: f64) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        let hw = w / 2.0;
        let ext = hw + 1.0;
        let xmin = (a[0].min(b[0]) - ext).floor().max(0.0) as i64;
        let xmax = ((a[0].max(b[0]) + ext).ceil() as i64).min(self.w as i64 - 1);
        let ymin = (a[1].min(b[1]) - ext).floor().max(0.0) as i64;
        let ymax = ((a[1].max(b[1]) + ext).ceil() as i64).min(self.h as i64 - 1);
        if xmax < xmin || ymax < ymin {
            return;
        }
        // 主な向きに沿って 1 画素ずつ進み、横切る向きの数画素だけを見る
        let steep = dy.abs() > dx.abs();
        let (u0, u1) = if steep { (ymin, ymax) } else { (xmin, xmax) };
        for u in u0..=u1 {
            let uc = u as f64 + 0.5;
            // 中心線の横切る座標
            let vc = if len < 1e-12 {
                if steep {
                    a[0]
                } else {
                    a[1]
                }
            } else if steep {
                if dy.abs() < 1e-12 {
                    a[0]
                } else {
                    a[0] + (uc - a[1]) / dy * dx
                }
            } else if dx.abs() < 1e-12 {
                a[1]
            } else {
                a[1] + (uc - a[0]) / dx * dy
            };
            let span = ext
                * (1.0
                    + if steep {
                        (dx / dy).abs()
                    } else {
                        (dy / dx).abs()
                    });
            let v0 = (vc - span).floor() as i64;
            let v1 = (vc + span).ceil() as i64;
            for v in v0..=v1 {
                let (x, y) = if steep { (v, u) } else { (u, v) };
                let p = [x as f64 + 0.5, y as f64 + 0.5];
                let d = seg_dist(p, a, b);
                let cov = (hw + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(x, y, c, cov as f32);
                }
            }
        }
    }

    fn rgb8(&self) -> Vec<u8> {
        self.px
            .iter()
            .flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect()
    }
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

/// 場面を画素に描く
pub fn rasterize(sc: &Scene, o: &Options2D) -> Raster {
    let v = shapes(sc, o);
    let (lo, hi) = framed(&v);
    let w = o.width.max(16) as usize;
    let sc_px = w as f64 / (hi[0] - lo[0]);
    let h = ((hi[1] - lo[1]) * sc_px).round().max(1.0) as usize;
    let mut r = Raster {
        w,
        h,
        px: vec![[1.0; 3]; w * h],
    };
    let tr = |p: [f64; 2]| [(p[0] - lo[0]) * sc_px, (hi[1] - p[1]) * sc_px];
    // 線の太さは横幅 1600 px で 1 倍
    let k = (w as f64 / 1600.0).max(0.5);
    for s in &v {
        let pts: Vec<[f64; 2]> = s.pts.iter().map(|&p| tr(p)).collect();
        if let Some(f) = s.fill {
            r.fill(&pts, f);
        }
        let n = pts.len();
        let m = if s.closed { n } else { n.saturating_sub(1) };
        for i in 0..m {
            r.line(pts[i], pts[(i + 1) % n], s.stroke, s.width * k);
        }
    }
    r
}

/// PNG（8 bit RGB）
pub fn write_png<W: Write>(w: W, sc: &Scene, o: &Options2D) -> std::io::Result<()> {
    let r = rasterize(sc, o);
    let mut e = png::Encoder::new(w, r.w as u32, r.h as u32);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    let mut wr = e.write_header().map_err(std::io::Error::other)?;
    wr.write_image_data(&r.rgb8())
        .map_err(std::io::Error::other)?;
    wr.finish().map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_fill_and_line() {
        let mut r = Raster {
            w: 20,
            h: 20,
            px: vec![[1.0; 3]; 400],
        };
        r.fill(
            &[[2.0, 2.0], [10.0, 2.0], [10.0, 10.0], [2.0, 10.0]],
            Rgb(0, 0, 0),
        );
        // 内側は黒、外側は白、縁の画素はちょうど覆われる
        assert_eq!(r.px[5 * 20 + 5], [0.0; 3]);
        assert_eq!(r.px[15 * 20 + 15], [1.0; 3]);
        assert_eq!(r.px[2 * 20 + 2], [0.0; 3]);
        assert_eq!(r.px[20 + 1], [1.0; 3]);
        r.line([12.0, 15.5], [19.0, 15.5], Rgb(0, 0, 0), 1.0);
        assert_eq!(r.px[15 * 20 + 15], [0.0; 3]);
        assert_eq!(r.px[14 * 20 + 15], [1.0; 3]);
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(-0.0000001), "0");
        assert_eq!(num(2.5), "2.5");
    }
}
