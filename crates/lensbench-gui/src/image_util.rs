//! 画面の写しの切り出しと PNG

/// 画面の写し（ColorImage）の矩形 r（画面の点）を切り出す
pub(crate) fn crop_image(
    img: &egui::ColorImage,
    r: egui::Rect,
    ppp: f32,
) -> Option<egui::ColorImage> {
    let [w, h] = img.size;
    let x0 = ((r.left() * ppp).round().max(0.0) as usize).min(w);
    let y0 = ((r.top() * ppp).round().max(0.0) as usize).min(h);
    let x1 = ((r.right() * ppp).round().max(0.0) as usize).min(w);
    let y1 = ((r.bottom() * ppp).round().max(0.0) as usize).min(h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut px = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0..y1 {
        px.extend_from_slice(&img.pixels[y * w + x0..y * w + x1]);
    }
    Some(egui::ColorImage {
        size: [x1 - x0, y1 - y0],
        pixels: px,
    })
}

/// ColorImage を PNG のバイト列に
pub(crate) fn png_bytes(img: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, img.size[0] as u32, img.size[1] as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
        w.write_image_data(&raw).map_err(|e| e.to_string())?;
    }
    Ok(buf)
}
