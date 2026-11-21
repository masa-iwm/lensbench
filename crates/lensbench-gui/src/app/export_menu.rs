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
            ui.close_menu();
        }
        if ui.button(tr("JSON")).clicked() {
            ui.ctx()
                .copy_text(serde_json::to_string_pretty(v).unwrap_or_default());
            ui.close_menu();
        }
        if ui
            .button(tr("Text (tab-separated)"))
            .on_hover_text(tr(
                "Pastes into a spreadsheet. Multiple tables are separated by \"# name\" lines",
            ))
            .clicked()
        {
            ui.ctx().copy_text(delimited(&tables(v), '\t'));
            ui.close_menu();
        }
        if ui.button(tr("CSV")).clicked() {
            ui.ctx().copy_text(delimited(&tables(v), ','));
            ui.close_menu();
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
                ui.close_menu();
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
            ui.close_menu();
            out = Some(ResOut::Shot(false));
        }
    });
    out
}
