//! 硝材置換のテンプレート（プロジェクトの `glass_template`、.zmx の GSTD）の欄。最適化の硝材の置き換えで、候補の絞り込みを
//! 何も指定しないときに使う（lensbench-design の `glass_sub`）

use egui::{RichText, Ui};
use lensbench_core::system::GlassTemplate;
use lensbench_design::glass_sub::RESIST_NAMES;

use crate::i18n::tr;

/// 欄を描いて g を変える（既定に戻ったら None）。変えたら true
pub(crate) fn ui(ui: &mut Ui, g: &mut Option<GlassTemplate>, weak: egui::Color32) -> bool {
    let mut t = g.unwrap_or_default();
    ui.checkbox(&mut t.enabled, tr("Use the template"))
        .on_hover_text(tr(
            "Used by the glass substitution of the optimization when it sets no candidate filter of its own (GSTD of a .zmx)",
        ));
    ui.add_enabled_ui(t.enabled, |ui| {
        egui::Grid::new(ui.scope_id().with("glass_tpl_max"))
            .num_columns(2)
            .show(ui, |ui| {
                let names = std::iter::once(tr("Relative cost").to_string())
                    .chain(RESIST_NAMES.iter().map(|k| k.to_ascii_uppercase()));
                for ((name, on), max) in names.zip(t.check.iter_mut()).zip(t.max.iter_mut()) {
                    ui.checkbox(on, name);
                    let lbl = ui.label(tr("Max"));
                    ui.add_enabled(
                        *on,
                        crate::widgets::Num::new(max).range(0.0..=1e6).max_decimals(3),
                    )
                    .labelled_by(lbl.id);
                    ui.end_row();
                }
            });
        ui.checkbox(&mut t.exclude_incomplete, tr("Exclude glasses without the value"))
            .on_hover_text(tr(
                "Drop candidates that have no value for a property with a limit (a relative cost or class of 0 counts as no value, except FR)",
            ));
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Catalog status"));
            let labels = [tr("Standard"), tr("Preferred"), tr("Obsolete"), tr("Special")];
            for (on, l) in t.status.iter_mut().zip(labels) {
                ui.checkbox(on, l);
            }
        });
        ui.label(
            RichText::new(tr(
                "An upper limit of 0 or less is not checked. Melt glasses are always excluded",
            ))
            .small()
            .color(weak),
        );
    });
    let new = (!t.is_default()).then_some(t);
    let changed = new != *g;
    *g = new;
    changed
}
