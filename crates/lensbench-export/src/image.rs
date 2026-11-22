//! 格子の画像（16 bit TIFF・32 bit float TIFF・16 bit PNG）。外部の画像ライブラリなしで、
//! TIFF は非圧縮・ストリップ 1 本・リトルエンディアン、PNG は 16 bit グレー（像の模擬は 16 bit の RGB も、読むのは png の crate）。
//! 画像の行 0 = 上 = y の最大（格子は行 0 = y の最小で渡す）。説明（JSON）は TIFF の ImageDescription にも入れる。

use std::path::Path;

/// 画像の形式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Tiff16,
    TiffFloat,
    Png16,
}

impl ImageFormat {
    /// 拡張子から（tif・tiff は 16 bit、float なら 32 bit float）
    pub fn from_path(p: &Path, float: bool) -> Option<ImageFormat> {
        let e = p.extension()?.to_str()?.to_ascii_lowercase();
        match e.as_str() {
            "tif" | "tiff" => Some(if float {
                ImageFormat::TiffFloat
            } else {
                ImageFormat::Tiff16
            }),
            "png" => Some(ImageFormat::Png16),
            _ => None,
        }
    }
}

/// 格子 `grid`（nx × ny、行 0 = y の最小）を画像の行の並び（行 0 = 上）にして 16 bit に割り付ける。
/// `scale`: Some((lo, hi)) なら lo → 0、hi → 65535 に線形（NaN は 0）、None なら値をそのまま丸める（生画像）
pub fn to_u16(grid: &[f64], nx: usize, ny: usize, scale: Option<(f64, f64)>) -> Vec<u16> {
    let mut u = vec![0u16; nx * ny];
    for r in 0..ny {
        for i in 0..nx {
            let v = grid[(ny - 1 - r) * nx + i];
            let w = if v.is_nan() {
                0.0
            } else {
                match scale {
                    Some((lo, hi)) => (v - lo) / (hi - lo) * 65535.0,
                    None => v,
                }
            };
            u[r * nx + i] = w.round_ties_even().clamp(0.0, 65535.0) as u16;
        }
    }
    u
}

/// 値の範囲（NaN を除く）。幅がなければ (0, 1)
pub fn data_range(grid: &[f64]) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for &v in grid {
        if !v.is_nan() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if hi > lo {
        (lo, hi)
    } else {
        (0.0, 1.0)
    }
}

/// TIFF（16 bit の値 `u`、または 32 bit float の値 `f`。行 0 = 上）を書く。`description` は ASCII にして入れる
pub fn write_tiff(
    path: &Path,
    nx: usize,
    ny: usize,
    u: Option<&[u16]>,
    f: Option<&[f32]>,
    description: &str,
    software: &str,
) -> std::io::Result<()> {
    let float = f.is_some();
    let (bps, fmt) = if float { (32u32, 3u32) } else { (16, 1) };
    let mut desc: Vec<u8> = description
        .chars()
        .flat_map(|c| {
            if (c as u32) < 128 {
                vec![c as u8]
            } else {
                // BMP の外はサロゲートの対で（JSON のまま読める）
                let mut b = [0u16; 2];
                c.encode_utf16(&mut b)
                    .iter()
                    .flat_map(|x| format!("\\u{x:04x}").into_bytes())
                    .collect()
            }
        })
        .collect();
    desc.push(0);
    let mut soft = software.as_bytes().to_vec();
    soft.push(0);
    let data_bytes = (nx * ny) as u32 * bps / 8;
    let off_desc = 8u32;
    let off_soft = off_desc + desc.len() as u32;
    let mut off_data = off_soft + soft.len() as u32;
    off_data += off_data & 1;
    let mut off_ifd = off_data + data_bytes;
    off_ifd += off_ifd & 1;
    let tags: [(u16, u16, u32, u32); 13] = [
        (256, 4, 1, nx as u32),
        (257, 4, 1, ny as u32),
        (258, 3, 1, bps),
        (259, 3, 1, 1),
        (262, 3, 1, 1),
        (270, 2, desc.len() as u32, off_desc),
        (273, 4, 1, off_data),
        (277, 3, 1, 1),
        (278, 4, 1, ny as u32),
        (279, 4, 1, data_bytes),
        (284, 3, 1, 1),
        (305, 2, soft.len() as u32, off_soft),
        (339, 3, 1, fmt),
    ];
    let mut b: Vec<u8> = Vec::with_capacity(off_ifd as usize + 200);
    b.extend_from_slice(b"II");
    b.extend_from_slice(&42u16.to_le_bytes());
    b.extend_from_slice(&off_ifd.to_le_bytes());
    b.extend_from_slice(&desc);
    b.extend_from_slice(&soft);
    b.resize(off_data as usize, 0);
    if let Some(f) = f {
        for v in f {
            b.extend_from_slice(&v.to_le_bytes());
        }
    } else if let Some(u) = u {
        for v in u {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    b.resize(off_ifd as usize, 0);
    b.extend_from_slice(&(tags.len() as u16).to_le_bytes());
    for (tag, typ, count, value) in tags {
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&typ.to_le_bytes());
        b.extend_from_slice(&count.to_le_bytes());
        if typ == 3 && count == 1 {
            b.extend_from_slice(&(value as u16).to_le_bytes());
            b.extend_from_slice(&0u16.to_le_bytes());
        } else {
            b.extend_from_slice(&value.to_le_bytes());
        }
    }
    b.extend_from_slice(&0u32.to_le_bytes());
    lensbench_core::vfs::write(path, b)
}

/// TIFF（[`write_tiff`] で書いたもの）を読み戻す: 値（行 0 = 上）と nx・ny
pub fn read_tiff(path: &Path) -> std::io::Result<(Vec<f64>, usize, usize)> {
    let b = lensbench_core::vfs::read(path)?;
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "TIFF");
    let u16at = |o: usize| {
        b.get(o..o + 2)
            .map(|s| u16::from_le_bytes([s[0], s[1]]))
            .ok_or_else(bad)
    };
    let u32at = |o: usize| {
        b.get(o..o + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
            .ok_or_else(bad)
    };
    let ifd = u32at(4)? as usize;
    let n = u16at(ifd)? as usize;
    let (mut nx, mut ny, mut off, mut bps, mut fmt) = (0usize, 0usize, 0usize, 16u32, 1u32);
    for k in 0..n {
        let e = ifd + 2 + 12 * k;
        let (tag, typ) = (u16at(e)?, u16at(e + 2)?);
        let val = if typ == 3 {
            u16at(e + 8)? as u32
        } else {
            u32at(e + 8)?
        };
        match tag {
            256 => nx = val as usize,
            257 => ny = val as usize,
            258 => bps = val,
            273 => off = val as usize,
            339 => fmt = val,
            _ => {}
        }
    }
    let mut v = vec![0.0; nx * ny];
    for (i, x) in v.iter_mut().enumerate() {
        *x = if bps == 32 && fmt == 3 {
            f32::from_bits(u32at(off + 4 * i)?) as f64
        } else {
            u16at(off + 2 * i)? as f64
        };
    }
    Ok((v, nx, ny))
}

/// 16 bit グレーの PNG（行 0 = 上）
pub fn write_png16(path: &Path, nx: usize, ny: usize, u: &[u16]) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut e = png::Encoder::new(&mut buf, nx as u32, ny as u32);
    e.set_color(png::ColorType::Grayscale);
    e.set_depth(png::BitDepth::Sixteen);
    let mut w = e.write_header().map_err(std::io::Error::other)?;
    let mut data = Vec::with_capacity(nx * ny * 2);
    for v in u {
        data.extend_from_slice(&v.to_be_bytes());
    }
    w.write_image_data(&data).map_err(std::io::Error::other)?;
    w.finish().map_err(std::io::Error::other)?;
    lensbench_core::vfs::write(path, buf)
}

/// PNG を読む（グレー・RGB、8・16 bit、パレット・α 付きも。α は捨てる）: (幅, 高さ, チャンネル（1 か 3、行 0 = 上、0〜1）)
pub fn read_png(path: &Path) -> std::io::Result<(usize, usize, Vec<Vec<f64>>)> {
    let b = lensbench_core::vfs::read(path)?;
    let mut d = png::Decoder::new(std::io::Cursor::new(b));
    d.set_transformations(png::Transformations::EXPAND);
    let mut r = d.read_info().map_err(std::io::Error::other)?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).map_err(std::io::Error::other)?;
    let (w, h) = (info.width as usize, info.height as usize);
    let (bytes, max) = match info.bit_depth {
        png::BitDepth::Sixteen => (2, 65535.0),
        png::BitDepth::Eight => (1, 255.0),
        d => return Err(std::io::Error::other(format!("PNG bit depth {d:?}"))),
    };
    let (nc, color) = match info.color_type {
        png::ColorType::Grayscale => (1, 1),
        png::ColorType::GrayscaleAlpha => (2, 1),
        png::ColorType::Rgb => (3, 3),
        png::ColorType::Rgba => (4, 3),
        c => return Err(std::io::Error::other(format!("PNG color type {c:?}"))),
    };
    let at = |k: usize| -> f64 {
        let o = k * bytes;
        let v = if bytes == 2 {
            u16::from_be_bytes([buf[o], buf[o + 1]]) as f64
        } else {
            buf[o] as f64
        };
        v / max
    };
    let line = info.line_size / bytes;
    let ch = (0..color)
        .map(|c| {
            (0..w * h)
                .map(|k| at((k / w) * line + (k % w) * nc + c))
                .collect()
        })
        .collect();
    Ok((w, h, ch))
}

/// 16 bit の PNG（チャンネル 1 ならグレー、3 なら RGB。値は 0〜1 を 0〜65535 に、外は切る。行 0 = 上）
pub fn write_png16_channels(
    path: &Path,
    w: usize,
    h: usize,
    ch: &[Vec<f64>],
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut e = png::Encoder::new(&mut buf, w as u32, h as u32);
    e.set_color(if ch.len() == 3 {
        png::ColorType::Rgb
    } else {
        png::ColorType::Grayscale
    });
    e.set_depth(png::BitDepth::Sixteen);
    let mut wr = e.write_header().map_err(std::io::Error::other)?;
    let mut data = Vec::with_capacity(w * h * 2 * ch.len());
    for k in 0..w * h {
        for c in ch {
            let v = c[k];
            let u = if v.is_finite() {
                (v.clamp(0.0, 1.0) * 65535.0).round() as u16
            } else {
                0
            };
            data.extend_from_slice(&u.to_be_bytes());
        }
    }
    wr.write_image_data(&data).map_err(std::io::Error::other)?;
    wr.finish().map_err(std::io::Error::other)?;
    lensbench_core::vfs::write(path, buf)
}

/// JSON の説明を横に書く（拡張子を .json に）
pub fn write_sidecar(path: &Path, json: &str) -> std::io::Result<()> {
    lensbench_core::vfs::write(path.with_extension("json"), json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_channels_read_back() {
        let dir = std::env::temp_dir().join(format!("lb_png_{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        let f = dir.join("c.png");
        let ch = vec![
            vec![0.0, 0.5, 1.0, 2.0],
            vec![1.0, 0.0, 0.25, 0.0],
            vec![0.0; 4],
        ];
        write_png16_channels(&f, 2, 2, &ch).unwrap();
        let (w, h, back) = read_png(&f).unwrap();
        assert_eq!((w, h, back.len()), (2, 2, 3));
        assert!((back[0][1] - 0.5).abs() < 1e-4 && back[0][3] == 1.0 && back[1][2] > 0.24);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tiff_reads_back() {
        let dir = std::env::temp_dir().join(format!("lb_img_{}", std::process::id()));
        lensbench_core::vfs::create_dir_all(&dir).unwrap();
        let grid = vec![0.0, 1.0, 2.0, f64::NAN, 4.0, 5.0];
        let u = to_u16(&grid, 3, 2, Some((0.0, 5.0)));
        // 行 0 = 上 = 格子の最後の行
        assert_eq!(u, vec![0, 52428, 65535, 0, 13107, 26214]);
        let p = dir.join("a.tif");
        write_tiff(&p, 3, 2, Some(&u), None, "{\"a\": \"µ\"}", "lensbench").unwrap();
        let (v, nx, ny) = read_tiff(&p).unwrap();
        assert_eq!((nx, ny), (3, 2));
        assert_eq!(v, u.iter().map(|&x| x as f64).collect::<Vec<_>>());
        let f: Vec<f32> = vec![1.5, f32::NAN, -2.0, 3.0];
        let p2 = dir.join("b.tif");
        write_tiff(&p2, 2, 2, None, Some(&f), "{}", "lensbench").unwrap();
        let (v2, _, _) = read_tiff(&p2).unwrap();
        assert_eq!(v2[0], 1.5);
        assert!(v2[1].is_nan());
        write_png16(&dir.join("c.png"), 3, 2, &u).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
