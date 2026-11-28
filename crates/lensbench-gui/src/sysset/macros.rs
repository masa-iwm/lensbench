//! ZPL マクロ（プロジェクトの `macros`。.zmx を分けたときのソルブのマクロ）の文を編む欄。並びの番号と名前はソルブが
//! 指すので、ここでは文だけを変える（足す・消す・名前を変えるのは .zmx・プロジェクトの JSON で）

use egui::{RichText, Ui};
use serde_json::Value;

use crate::i18n::tr;

/// 欄を描く。文を変えたら true
pub(crate) fn ui(ui: &mut Ui, macros: &mut [Value], weak: egui::Color32) -> bool {
    let mut changed = false;
    ui.label(
        RichText::new(tr(
            "The macros called by the macro solves. Solves refer to them by position and name, so only the text is edited here",
        ))
        .small()
        .color(weak),
    );
    for (i, m) in macros.iter_mut().enumerate() {
        let name = m["name"].as_str().unwrap_or_default().to_string();
        let mut src = m["source"].as_str().unwrap_or_default().to_string();
        ui.add_space(4.0);
        let lbl = ui.label(RichText::new(format!("{}. {name}", i + 1)).strong());
        let r = ui
            .add(
                egui::TextEdit::multiline(&mut src)
                    .code_editor()
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(lbl.id);
        if let Some(e) = problem(&src) {
            ui.colored_label(
                crate::theme::error(ui.visuals()),
                lensbench_core::i18n::tr(&e),
            );
        }
        if r.changed() {
            m["source"] = Value::String(src);
            changed = true;
        }
    }
    changed
}

/// 文の誤り（解釈できない行の最初のもの。走らせるとそこで止まる）
fn problem(src: &str) -> Option<String> {
    match lensbench_core::zpl::Program::parse(src) {
        Ok(p) => p.first_problem(),
        Err(e) => Some(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shows_the_line_that_does_not_parse() {
        assert!(super::problem("x = 1\nSOLVERETURN (x +").is_some());
        assert_eq!(super::problem("x = 1\nSOLVERETURN x"), None);
    }
}
