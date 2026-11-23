//! 図の縮尺（スケールバー）。長さの軸の無い図（レイアウト図・スポット図・3D の正射影）に、きりのよい長さの棒と
//! その長さを出す。拡大縮小に合わせて長さを選び直す

use egui::{Align2, Color32, FontId, Painter, Pos2, Stroke};

/// 1・2・5×10^n mm のうち、max_mm 以下で最大の長さ。max_mm が正でなければ None
pub(crate) fn nice_length(max_mm: f64) -> Option<f64> {
    if !(max_mm.is_finite() && max_mm > 0.0) {
        return None;
    }
    let e = max_mm.log10().floor();
    let p = 10f64.powf(e);
    let m = max_mm / p;
    let k = if m >= 5.0 {
        5.0
    } else if m >= 2.0 {
        2.0
    } else {
        1.0
    };
    Some(k * p)
}

/// 1・2・5×10^n mm のうち、min_mm 以上で最小の長さ（スポット図の自動の枠の幅）。min_mm が正でなければ None
pub(crate) fn nice_up(min_mm: f64) -> Option<f64> {
    let d = nice_length(min_mm)?;
    if d >= min_mm * (1.0 - 1e-12) {
        return Some(d);
    }
    let e = d.log10().floor();
    let m = (d / 10f64.powf(e)).round();
    Some(
        if m < 2.0 {
            2.0
        } else if m < 5.0 {
            5.0
        } else {
            10.0
        } * 10f64.powf(e),
    )
}

/// 長さの文字（1 mm 以上は mm、1 µm 以上は µm、それより短ければ nm）
pub(crate) fn length_text(mm: f64) -> String {
    let (v, unit) = if mm >= 1.0 {
        (mm, "mm")
    } else if mm >= 1e-3 {
        (mm * 1e3, "µm")
    } else {
        (mm * 1e6, "nm")
    };
    // 1・2・5×10^n なので桁は丸めるだけでよい（浮動小数の誤差を消す）
    let v = (v * 1e6).round() / 1e6;
    format!("{} {unit}", crate::plots::fmt_sig(v, 3))
}

/// 縮尺の文字の字体
fn font() -> FontId {
    FontId::proportional(11.0)
}

/// 縮尺（文字と棒を合わせた箱）の高さ。文字を組んだ高さから求める（日本語の字体では µ などで行が高くなる）
pub(crate) fn height(ctx: &egui::Context) -> f32 {
    let th = ctx.fonts_mut(|f| {
        f.layout_no_wrap("200 µm nm".into(), font(), Color32::WHITE)
            .size()
            .y
    });
    th + 2.0 + 5.0
}

/// 縮尺を描く。px_per_mm は図の 1 mm の画面の長さ、max_px は棒の長さの上限、anchor は棒と文字を合わせた箱の
/// 角（align の向き。棒は寄せる側にそろえ、文字は棒の上の真ん中）。長さが決まらなければ描かない
pub(crate) fn draw(
    painter: &Painter,
    anchor: Pos2,
    align: Align2,
    px_per_mm: f64,
    max_px: f32,
    color: Color32,
) {
    let Some(mm) = nice_length(max_px as f64 / px_per_mm) else {
        return;
    };
    draw_len(painter, anchor, align, px_per_mm, mm, color);
}

/// 長さ mm を決めた縮尺を描く（[`draw`] と同じ置き方。利用者が長さを指定したとき）
pub(crate) fn draw_len(
    painter: &Painter,
    anchor: Pos2,
    align: Align2,
    px_per_mm: f64,
    mm: f64,
    color: Color32,
) {
    let w = (mm * px_per_mm) as f32;
    if w.is_nan() || w < 4.0 {
        return;
    }
    let galley = painter.layout_no_wrap(length_text(mm), font(), color);
    let th = galley.size().y + 2.0;
    // 棒と文字を合わせた箱を anchor に置く
    let size = egui::vec2(w.max(galley.size().x), th + 5.0);
    let r = align.anchor_size(anchor, size);
    // 棒は箱の寄せる側にそろえる（左下なら左端から）
    let x0 = match align.x() {
        egui::Align::Min => r.left(),
        egui::Align::Center => r.center().x - w / 2.0,
        egui::Align::Max => r.right() - w,
    };
    let x1 = x0 + w;
    let y = r.bottom() - 2.0;
    let st = Stroke::new(1.5_f32, color);
    painter.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], st);
    for x in [x0, x1] {
        painter.line_segment([Pos2::new(x, y - 4.0), Pos2::new(x, y + 1.0)], st);
    }
    let tx = ((x0 + x1) / 2.0 - galley.size().x / 2.0).max(r.left());
    painter.galley(Pos2::new(tx, r.top()), galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_lengths() {
        assert_eq!(nice_length(1.0), Some(1.0));
        assert_eq!(nice_length(1.9), Some(1.0));
        assert_eq!(nice_length(2.0), Some(2.0));
        assert_eq!(nice_length(4.99), Some(2.0));
        assert_eq!(nice_length(7.0), Some(5.0));
        assert_eq!(nice_length(73.0), Some(50.0));
        assert!((nice_length(0.03).unwrap() - 0.02).abs() < 1e-12);
        assert_eq!(nice_length(0.0), None);
        assert_eq!(nice_length(f64::NAN), None);
        assert_eq!(nice_up(1.0), Some(1.0));
        assert_eq!(nice_up(1.1), Some(2.0));
        assert_eq!(nice_up(2.5), Some(5.0));
        assert_eq!(nice_up(7.0), Some(10.0));
        assert!((nice_up(0.031).unwrap() - 0.05).abs() < 1e-12);
    }

    #[test]
    fn length_texts() {
        assert_eq!(length_text(20.0), "20 mm");
        assert_eq!(length_text(1.0), "1 mm");
        assert_eq!(length_text(0.5), "500 µm");
        assert_eq!(length_text(0.002), "2 µm");
        assert_eq!(length_text(0.0005), "500 nm");
        assert_eq!(length_text(nice_length(0.03).unwrap()), "20 µm");
    }
}
