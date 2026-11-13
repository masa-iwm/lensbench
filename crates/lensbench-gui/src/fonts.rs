//! 日本語のフォント。exe を小さく保つため同梱せず、OS のフォントを起動時に読む（無ければ欧文だけで動く）。
//! Web 版はページがフォントを読み込んで [`WEB_FONT`] に置く。

use std::sync::Arc;

use ab_glyph::Font;
use egui::{FontData, FontDefinitions, FontFamily};

/// Web 版: ページが読み込んだ日本語のフォントを置く場所（lensbench_core::vfs の置き場）
pub const WEB_FONT: &str = "/lensbench/fonts/jp.ttf";

/// 探すフォント（ファイル、.ttc の中の番号）。先に見つかったものを使う
const CANDIDATES: &[(&str, u32)] = &[
    // Web 版
    #[cfg(target_arch = "wasm32")]
    (WEB_FONT, 0),
    // Windows
    (r"C:\Windows\Fonts\YuGothM.ttc", 0),
    (r"C:\Windows\Fonts\meiryo.ttc", 0),
    (r"C:\Windows\Fonts\msgothic.ttc", 0),
    // Linux（CI・Linux で開くとき）
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
    (
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        0,
    ),
    ("/usr/share/fonts/truetype/fonts-japanese-gothic.ttf", 0),
    ("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", 0),
    // macOS
    ("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", 0),
];

/// 日本語のフォントを足す。読めたファイルを返す
pub fn install(ctx: &egui::Context) -> Option<String> {
    let windir = std::env::var("WINDIR").ok();
    let mut found = None;
    // LENSBENCH_GUI_FONT で好きなフォント（.ttf・.otf・.ttc の最初）を選べる
    if let Some(p) = std::env::var_os("LENSBENCH_GUI_FONT") {
        if let Ok(bytes) = lensbench_core::vfs::read(&p) {
            found = Some((p.to_string_lossy().into_owned(), bytes, 0));
        }
    }
    for (path, index) in CANDIDATES {
        // Windows のフォルダが C: でないときにも探す
        let p = match (&windir, path.strip_prefix(r"C:\Windows")) {
            (Some(w), Some(rest)) => format!("{w}{rest}"),
            _ => path.to_string(),
        };
        if found.is_some() {
            break;
        }
        if let Ok(bytes) = lensbench_core::vfs::read(&p) {
            found = Some((p, bytes, *index));
        }
    }
    let (path, bytes, index) = found?;
    let mut fonts = FontDefinitions::default();
    // LENSBENCH_GUI_FONT_Y で縦位置を手で決められる（字の大きさに対する割合、正で下へ）
    let tweak = std::env::var("LENSBENCH_GUI_FONT_Y")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .or_else(|| center_tweak(&bytes, index));
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    if let Some(y) = tweak {
        data.tweak.y_offset_factor = y;
    }
    fonts.font_data.insert("ja".into(), Arc::new(data));
    // 文章は日本語のフォントを先に（欧文・全角の括弧も同じフォントで並べ、高さを揃える）。
    // 記号（▶ ⟳ など）は欧文のフォントに残る。等幅は欧文の等幅を先に、足りない字だけ日本語
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "ja".into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("ja".into());
    ctx.set_fonts(fonts);
    Some(path)
}

/// 記号 `sym` がフォントに無ければ代わりの字 `alt`（豆腐 □ を出さない、GUX-24・GUX-29）
pub fn glyph_or<'a>(ui: &egui::Ui, sym: &'a str, alt: &'a str) -> &'a str {
    let id = egui::FontId::proportional(16.0);
    if ui.fonts(|f| f.has_glyphs(&id, sym)) {
        sym
    } else {
        alt
    }
}

/// 文字の縦位置の補正（字の大きさに対する割合）。
///
/// egui は行の高さをフォントの ascent・descent・line gap で決め、字を行の上から ascent の所に置く。游ゴシックなど
/// descent・line gap の大きいフォントでは字が行の上に寄って見えるので、全角の字面（ベースラインの下 0.12 em から
/// 上 0.88 em）が行の真ん中に来るようにずらす。egui の字の大きさ（PxScale）は ascent − descent の高さ
pub fn center_tweak(bytes: &[u8], index: u32) -> Option<f32> {
    let f = ab_glyph::FontRef::try_from_slice_and_index(bytes, index).ok()?;
    let (asc, desc, gap) = (
        f.ascent_unscaled(),
        f.descent_unscaled(),
        f.line_gap_unscaled(),
    );
    let h = asc - desc;
    let upm = f.units_per_em()?;
    if h <= 0.0 || upm <= 0.0 {
        return None;
    }
    let (a, g, e) = (asc / h, gap / h, upm / h);
    let y = (1.0 + g) / 2.0 - (a - 0.38 * e);
    // 壊れた寸法のフォントで大きくずらさない
    y.is_finite().then(|| y.clamp(-0.5, 0.5))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GUX-29: 無い記号は代わりの字に、ある記号はそのまま
    #[test]
    fn gux29_glyph_or_falls_back_for_missing_symbols() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert_eq!(glyph_or(ui, "A", "<"), "A");
                assert_eq!(glyph_or(ui, "\u{E000}", "<"), "<");
            });
        });
    }

    /// 手元にある日本語フォントで確かめる（無ければ何もしない）
    #[test]
    fn tweak_of_installed_fonts() {
        for (p, i) in CANDIDATES {
            let Ok(b) = lensbench_core::vfs::read(p) else {
                continue;
            };
            let y = center_tweak(&b, *i).unwrap();
            let f = ab_glyph::FontRef::try_from_slice_and_index(&b, *i).unwrap();
            println!(
                "{p}: {y:.3}（ascent {} descent {} line gap {} em {:?}）",
                f.ascent_unscaled(),
                f.descent_unscaled(),
                f.line_gap_unscaled(),
                f.units_per_em()
            );
            assert!(y.abs() < 0.5, "{p} {y}");
        }
    }
}
