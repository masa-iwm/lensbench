//! Web 版: 別のタブ（窓）に出したパネルの画面を、元のタブから送る形（#137）。
//!
//! 元のタブはパネルを別の `egui::Context` で描いて三角形の網（[`egui::epaint::ClippedPrimitive`]）にし、
//! 変わったテクスチャと一緒にこの形のバイト列で送る。別のタブは受けた網をそのまま描き、入力（[`egui::Event`]）を
//! 元のタブに返す。系・計算・パネルの状態は元のタブだけが持つ。
//!
//! 形（数はすべて little endian）: `LBF1`、テクスチャの設定の数 u32 と各設定（id、位置の有無と位置、大きさ、
//! テクスチャの設定 4 バイト、RGBA）、消すテクスチャの数と id、網の数と各網（切り抜きの矩形、テクスチャ id、
//! 頂点（位置・uv・色）、添え字）

use egui::epaint::{
    textures::{TextureFilter, TextureOptions, TextureWrapMode, TexturesDelta},
    ClippedPrimitive, ImageData, ImageDelta, Mesh, Primitive, TextureId, Vertex,
};
use egui::{Color32, ColorImage, Pos2, Rect};

const MAGIC: &[u8; 4] = b"LBF1";

/// 受けた画面
#[derive(Debug, Default)]
pub struct Frame {
    /// テクスチャの設定（送った側の id、位置（部分の更新）、設定、画像）
    pub set: Vec<(TextureId, Option<[usize; 2]>, TextureOptions, ColorImage)>,
    pub free: Vec<TextureId>,
    /// 切り抜きの矩形と網（テクスチャ id は送った側のもの）
    pub meshes: Vec<(Rect, Mesh)>,
}

fn put_u32(b: &mut Vec<u8>, x: u32) {
    b.extend_from_slice(&x.to_le_bytes());
}
fn put_f32(b: &mut Vec<u8>, x: f32) {
    b.extend_from_slice(&x.to_le_bytes());
}
fn put_tex(b: &mut Vec<u8>, t: TextureId) {
    let (k, n) = match t {
        TextureId::Managed(n) => (0u8, n),
        TextureId::User(n) => (1u8, n),
    };
    b.push(k);
    b.extend_from_slice(&n.to_le_bytes());
}

fn filter_byte(f: TextureFilter) -> u8 {
    match f {
        TextureFilter::Nearest => 0,
        TextureFilter::Linear => 1,
    }
}
fn byte_filter(b: u8) -> TextureFilter {
    if b == 0 {
        TextureFilter::Nearest
    } else {
        TextureFilter::Linear
    }
}

/// 画面をバイト列に（テクスチャの変更と網）
pub fn encode(delta: &TexturesDelta, prims: &[ClippedPrimitive]) -> Vec<u8> {
    let mut b = Vec::with_capacity(64 * 1024);
    b.extend_from_slice(MAGIC);
    put_u32(&mut b, delta.set.len() as u32);
    for (id, d) in &delta.set {
        put_tex(&mut b, *id);
        match d.pos {
            Some([x, y]) => {
                b.push(1);
                put_u32(&mut b, x as u32);
                put_u32(&mut b, y as u32);
            }
            None => {
                b.push(0);
                put_u32(&mut b, 0);
                put_u32(&mut b, 0);
            }
        }
        let [w, h] = d.image.size();
        put_u32(&mut b, w as u32);
        put_u32(&mut b, h as u32);
        let o = d.options;
        b.push(filter_byte(o.magnification));
        b.push(filter_byte(o.minification));
        b.push(match o.wrap_mode {
            TextureWrapMode::ClampToEdge => 0,
            TextureWrapMode::Repeat => 1,
            TextureWrapMode::MirroredRepeat => 2,
        });
        b.push(o.mipmap_mode.map_or(0, |f| 1 + filter_byte(f)));
        match &d.image {
            ImageData::Color(c) => {
                for p in &c.pixels {
                    b.extend_from_slice(&p.to_array());
                }
            }
            ImageData::Font(f) => {
                for p in f.srgba_pixels(None) {
                    b.extend_from_slice(&p.to_array());
                }
            }
        }
    }
    put_u32(&mut b, delta.free.len() as u32);
    for id in &delta.free {
        put_tex(&mut b, *id);
    }
    let meshes: Vec<(&Rect, &Mesh)> = prims
        .iter()
        .filter_map(|p| match &p.primitive {
            Primitive::Mesh(m) if !m.indices.is_empty() => Some((&p.clip_rect, m)),
            _ => None,
        })
        .collect();
    put_u32(&mut b, meshes.len() as u32);
    for (r, m) in meshes {
        for x in [r.min.x, r.min.y, r.max.x, r.max.y] {
            put_f32(&mut b, x);
        }
        put_tex(&mut b, m.texture_id);
        put_u32(&mut b, m.vertices.len() as u32);
        for v in &m.vertices {
            put_f32(&mut b, v.pos.x);
            put_f32(&mut b, v.pos.y);
            put_f32(&mut b, v.uv.x);
            put_f32(&mut b, v.uv.y);
            b.extend_from_slice(&v.color.to_array());
        }
        put_u32(&mut b, m.indices.len() as u32);
        for i in &m.indices {
            put_u32(&mut b, *i);
        }
    }
    b
}

/// 網の部分だけの指紋（同じ画面を送り直さない）
pub fn mesh_hash(prims: &[ClippedPrimitive]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in prims {
        if let Primitive::Mesh(m) = &p.primitive {
            for x in [
                p.clip_rect.min.x,
                p.clip_rect.min.y,
                p.clip_rect.max.x,
                p.clip_rect.max.y,
            ] {
                x.to_bits().hash(&mut h);
            }
            m.texture_id.hash(&mut h);
            m.indices.hash(&mut h);
            for v in &m.vertices {
                (
                    v.pos.x.to_bits(),
                    v.pos.y.to_bits(),
                    v.uv.x.to_bits(),
                    v.uv.y.to_bits(),
                )
                    .hash(&mut h);
                v.color.to_array().hash(&mut h);
            }
        }
    }
    h.finish()
}

struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}

impl Rd<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self
            .b
            .get(self.i..self.i + n)
            .ok_or_else(|| "画面のデータが途中で切れている".to_string())?;
        self.i += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn tex(&mut self) -> Result<TextureId, String> {
        let k = self.u8()?;
        let n = u64::from_le_bytes(self.take(8)?.try_into().unwrap());
        Ok(if k == 0 {
            TextureId::Managed(n)
        } else {
            TextureId::User(n)
        })
    }
    fn rgba(&mut self) -> Result<Color32, String> {
        let s = self.take(4)?;
        Ok(Color32::from_rgba_premultiplied(s[0], s[1], s[2], s[3]))
    }
}

pub fn decode(b: &[u8]) -> Result<Frame, String> {
    if b.get(..4) != Some(MAGIC.as_slice()) {
        return Err("画面のデータではない".into());
    }
    let mut r = Rd { b, i: 4 };
    let mut f = Frame::default();
    for _ in 0..r.u32()? {
        let id = r.tex()?;
        let has = r.u8()? == 1;
        let (x, y) = (r.u32()? as usize, r.u32()? as usize);
        let (w, h) = (r.u32()? as usize, r.u32()? as usize);
        let mag = byte_filter(r.u8()?);
        let min = byte_filter(r.u8()?);
        let wrap = match r.u8()? {
            1 => TextureWrapMode::Repeat,
            2 => TextureWrapMode::MirroredRepeat,
            _ => TextureWrapMode::ClampToEdge,
        };
        let mip = match r.u8()? {
            0 => None,
            k => Some(byte_filter(k - 1)),
        };
        if w.saturating_mul(h) > 64 << 20 {
            return Err("テクスチャが大きすぎる".into());
        }
        let mut pixels = Vec::with_capacity(w * h);
        for _ in 0..w * h {
            pixels.push(r.rgba()?);
        }
        f.set.push((
            id,
            has.then_some([x, y]),
            TextureOptions {
                magnification: mag,
                minification: min,
                wrap_mode: wrap,
                mipmap_mode: mip,
            },
            ColorImage {
                size: [w, h],
                pixels,
            },
        ));
    }
    for _ in 0..r.u32()? {
        f.free.push(r.tex()?);
    }
    for _ in 0..r.u32()? {
        let rect = Rect::from_min_max(Pos2::new(r.f32()?, r.f32()?), Pos2::new(r.f32()?, r.f32()?));
        let mut m = Mesh::with_texture(r.tex()?);
        let nv = r.u32()? as usize;
        m.vertices.reserve(nv.min(1 << 22));
        for _ in 0..nv {
            let pos = Pos2::new(r.f32()?, r.f32()?);
            let uv = Pos2::new(r.f32()?, r.f32()?);
            let color = r.rgba()?;
            m.vertices.push(Vertex { pos, uv, color });
        }
        let ni = r.u32()? as usize;
        m.indices.reserve(ni.min(1 << 22));
        for _ in 0..ni {
            let i = r.u32()?;
            if i as usize >= nv {
                return Err("網の添え字が頂点の外".into());
            }
            m.indices.push(i);
        }
        f.meshes.push((rect, m));
    }
    Ok(f)
}

/// 受けたテクスチャの設定を ImageDelta に
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn delta_of(pos: Option<[usize; 2]>, options: TextureOptions, img: ColorImage) -> ImageDelta {
    let image = ImageData::Color(std::sync::Arc::new(img));
    match pos {
        Some(p) => ImageDelta::partial(p, image, options),
        None => ImageDelta::full(image, options),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実際のパネルを描いた網とテクスチャを送って受けると同じになる
    #[test]
    fn frame_round_trip() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0))),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("lensbench");
                let _ = ui.button("▶");
            });
        });
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        assert!(!out.textures_delta.set.is_empty());
        let b = encode(&out.textures_delta, &prims);
        let f = decode(&b).unwrap();
        assert_eq!(f.set.len(), out.textures_delta.set.len());
        assert_eq!(f.set[0].3.size, out.textures_delta.set[0].1.image.size());
        let meshes: Vec<&Mesh> = prims
            .iter()
            .filter_map(|p| match &p.primitive {
                Primitive::Mesh(m) if !m.indices.is_empty() => Some(m),
                _ => None,
            })
            .collect();
        assert_eq!(f.meshes.len(), meshes.len());
        for ((_, a), b) in f.meshes.iter().zip(meshes) {
            assert_eq!(a.indices, b.indices);
            assert_eq!(a.vertices.len(), b.vertices.len());
            assert_eq!(a.texture_id, b.texture_id);
        }
        assert_eq!(mesh_hash(&prims), mesh_hash(&prims));
        // 壊れたデータは断る
        assert!(decode(&b[..b.len() - 3]).is_err());
        assert!(decode(b"XXXX").is_err());
    }
}
