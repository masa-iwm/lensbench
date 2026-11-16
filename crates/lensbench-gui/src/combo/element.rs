//! 要素の設定欄（図の下の欄とレンズデータの下の欄の 1 つの作り、GPN-17）。図の下は置き方を偏心 y・傾き x
//! だけにした [`Detail::Compact`]、レンズデータの下は置き方を全部と群の欄を出す [`Detail::Full`]

use egui::{RichText, Ui};

use super::*;

/// 欄の詳しさ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// 図の下: 置き方は偏心 y・傾き x
    Compact,
    /// レンズデータの下: 置き方は偏心 x・y、傾き x・y・z と群の役割
    Full,
}

impl Pick {
    /// 反転できる（玉。ユーザーカタログの項目は群として入るので反転は無い）
    pub fn can_reverse(&self) -> bool {
        matches!(
            self.kind,
            ElemKind::Catalog | ElemKind::Lens | ElemKind::Zmx
        ) && !(self.kind == ElemKind::Catalog && !self.path.is_empty())
    }
}

/// 経路の k 番目の要素の設定欄。head は見出しの行に足すもの（図の下のボタンなど）
pub(super) fn element_settings(
    ui: &mut Ui,
    combo: &mut Combo,
    k: usize,
    detail: Detail,
    head: impl FnOnce(&mut Ui, &mut Pick),
) {
    let chain = combo.chain();
    let last = k + 1 == chain.len();
    let Some(step) = chain.get(k).cloned() else {
        return;
    };
    let air_gap = matches!(step.after, Some(Gap::Air(_)));
    let st = combo.active_state().map(str::to_string);
    let st = st.as_deref();
    // 膜の欄で選べる、系の設定で定義した膜の名前
    let coats: Vec<String> = combo.coatings.keys().cloned().collect();
    let Some(p) = combo.get_mut(&step.loc) else {
        return;
    };
    let weak = theme::weak_text(ui.visuals());
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{}. {}", k + 1, p.label())).strong());
        ui.label(RichText::new(p.kind.label()).color(weak));
        head(ui, p);
        ui.checkbox(&mut p.locked, tr("Lock")).on_hover_text(tr(
            "A locked element keeps its contents (surfaces, glass, flip); position and the air after can still change (toggle with Ctrl+L)",
        ));
        if p.locked {
            ui.checkbox(&mut p.lock_place, tr("Position too")).on_hover_text(tr(
                "Also lock the position (drag, keys, decenter, tilt) and the air after",
            ));
        }
        // 上書きの中身はレンズデータの表のセルの印で見る（#453）
        if !p.edits.is_empty() || !p.ops.is_empty() {
            ui.label(
                RichText::new(trf(
                    "{} values changed in Lens Data",
                    &[&(p.edits.len() + p.ops.len())],
                ))
                .color(ui.visuals().warn_fg_color),
            );
            if ui
                .add_enabled(!p.locked, egui::Button::new(tr("Revert")))
                .on_hover_text(tr(
                    "Drop the values changed in the surface table (including stop, solves and pickups)",
                ))
                .clicked()
            {
                p.edits.clear();
                p.ops.clear();
            }
        }
    });
    // ロックは中身（種類ごとの欄・面・反転）だけを止める。位置・後の空気は「位置も」で止める。ON/OFF はいつも変えられる
    let locked = p.locked;
    ui.add_enabled_ui(!locked, |ui| {
        ui.horizontal_wrapped(|ui| {
            kind_fields(ui, p, st, k, weak, &coats);
            if p.can_reverse() {
                ui.checkbox(&mut p.reversed, tr("Reverse"));
            }
        });
        if p.kind == ElemKind::Surfaces {
            surf_table(ui, p, k, weak);
        }
    });
    ui.add_enabled_ui(!p.fixed(), |ui| {
        ui.horizontal_wrapped(|ui| {
            if place_fields(p.kind) {
                match detail {
                    Detail::Full => place_ui(ui, p, st, &[0, 1], &[0, 1, 2]),
                    Detail::Compact => place_ui(ui, p, st, &[1], &[0]),
                }
            }
            if air_gap {
                let mut air = p.air_in(st);
                let label = if last {
                    tr("To image")
                } else {
                    tr("Air after")
                };
                num(ui, label, &mut air, " mm");
                if air != p.air_in(st) {
                    *p.air_mut(st) = air;
                }
            }
        });
    });
    // 構成への入れる・外すは「位置も」のロックでも変えられる（動かすのではないので）
    ui.horizontal_wrapped(|ui| onoff_ui(ui, p, st, k, weak));
    if p.kind == ElemKind::Splitter {
        ui.add_enabled_ui(!locked, |ui| bs_fields(ui, combo, &step.loc, weak));
    }
    // 要素の群の役割・移動（レイアウトのパネルの「群」と同じ、#136）
    let g = combo
        .get(&step.loc)
        .map(|p| p.group.trim().to_string())
        .unwrap_or_default();
    if detail == Detail::Full && !g.is_empty() {
        ui.horizontal_wrapped(|ui| groups_ui(ui, combo, weak, Some(&g)));
    }
}
