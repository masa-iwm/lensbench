//! 結果をクリップボードに写す・ファイルに保存するメニュー（解析のグラフとレイアウト図）

use crate::i18n::tr;
use egui::Ui;
use serde_json::Value;

/// 解析の結果のメニューから頼むこと
pub(super) enum ResOut {
    /// 図を画像にする（true は写す、false は PNG で保存）
    Shot(bool),
    /// 保存先を選んで書く（書く中身はメニューを押したときの結果）
    Save(crate::files::FileDialog, String),
}

/// 結果をクリップボードに写すメニュー（図の画像・JSON・テキスト（タブ区切り）・CSV）。解析のグラフとレイアウト図で
/// 同じ並び。図の画像を頼まれたら true（画面の写しを待って切り出すのは呼ぶ側）
pub(super) fn copy_menu(ui: &mut Ui, v: &Value, fig: bool) -> bool {
    use crate::export_table::{delimited, tables};
    let mut shot = false;
    ui.menu_button(tr("Copy ▼"), |ui| {
        if fig && ui.button(tr("Figure as image")).clicked() {
            shot = true;
            ui.close();
        }
        if ui.button(tr("JSON")).clicked() {
            ui.ctx()
                .copy_text(serde_json::to_string_pretty(v).unwrap_or_default());
            ui.close();
        }
        if ui
            .button(tr("Text (tab-separated)"))
            .on_hover_text(tr(
                "Pastes into a spreadsheet. Multiple tables are separated by \"# name\" lines",
            ))
            .clicked()
        {
            ui.ctx().copy_text(delimited(&tables(v), '\t'));
            ui.close();
        }
        if ui.button(tr("CSV")).clicked() {
            ui.ctx().copy_text(delimited(&tables(v), ','));
            ui.close();
        }
    });
    shot
}

/// 解析の結果を写す・保存するメニュー（JSON・テキスト（タブ区切り）・CSV。図の画像）
pub(super) fn result_export_menu(ui: &mut Ui, cmd: &str, v: &Value, fig: bool) -> Option<ResOut> {
    use crate::export_table::{delimited, tables};
    let mut out = copy_menu(ui, v, fig).then_some(ResOut::Shot(true));
    ui.menu_button(tr("Save ▼"), |ui| {
        for (label, ext) in [(tr("JSON…"), "json"), (tr("CSV…"), "csv")] {
            if ui.button(label).clicked() {
                ui.close();
                let text = if ext == "json" {
                    serde_json::to_string_pretty(v).unwrap_or_default()
                } else {
                    delimited(&tables(v), ',')
                };
                let d = crate::files::FileDialog::new()
                    .add_filter(ext.to_uppercase(), &[ext])
                    .set_file_name(format!("{cmd}.{ext}"));
                out = Some(ResOut::Save(d, text));
            }
        }
        if fig && ui.button(tr("Figure as image (PNG)…")).clicked() {
            ui.close();
            out = Some(ResOut::Shot(false));
        }
    });
    out
}

/// Web 版のレイアウトの書き出しの形式（メニューの名前・拡張子）。2D は断面、3D は立体
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) const LAYOUT_EXPORT_FORMATS: [(&str, &str); 8] = [
    ("SVG (2D)", "svg"),
    ("PNG (2D)", "png"),
    ("DXF (2D)", "dxf"),
    ("STEP (3D)", "step"),
    ("GLB (3D)", "glb"),
    ("glTF (3D)", "gltf"),
    ("OBJ (3D)", "obj"),
    ("STL (3D)", "stl"),
];

/// レイアウトの「エクスポート…」。押したら Some(拡張子)（None は保存の窓で選ぶ）。
/// アプリ版はボタン（形式は保存の窓のフィルタ・拡張子で選ぶ）。Web 版は保存の窓のフィルタが効かない
/// （そのままダウンロードになる）ので、形式を選ぶメニューにする
pub(super) fn layout_export_menu(ui: &mut Ui) -> Option<Option<&'static str>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.button(tr("Export…"))
            .on_hover_text(tr(
                "Export with the current view and overlaid configurations. The format follows the extension: svg, png, dxf for 2D; step, glb, gltf, obj, stl for 3D",
            ))
            .clicked()
            .then_some(None)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let mut pick = None;
        ui.menu_button(tr("Export…"), |ui| {
            for (name, ext) in LAYOUT_EXPORT_FORMATS {
                if ui.button(name).clicked() {
                    pick = Some(Some(ext));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(tr(
            "Export with the current view and overlaid configurations. 2D formats draw the section, 3D formats the solids",
        ));
        pick
    }
}

#[cfg(test)]
mod tests {
    use super::LAYOUT_EXPORT_FORMATS;
    use lensbench_export::LayoutFormat;
    use std::path::Path;

    /// Web 版のメニューの形式は全部書き出せて、名前の 2D・3D が書き出しの側と合う
    #[test]
    fn layout_export_formats_are_writable() {
        for (name, ext) in LAYOUT_EXPORT_FORMATS {
            let f = LayoutFormat::from_path(Path::new(&format!("a.{ext}")))
                .unwrap_or_else(|| panic!("{ext} を書き出せない"));
            assert_eq!(f.is_2d(), name.ends_with("(2D)"), "{name}");
        }
    }
}
