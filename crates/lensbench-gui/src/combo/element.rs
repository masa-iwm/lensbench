//! 要素の設定欄（図の下の欄とレンズデータの下の欄の 1 つの作り）。図の下は置き方を偏心 y・傾き x
//! だけにした [`Detail::Compact`]、レンズデータの下は置き方を全部と群の欄を出す [`Detail::Full`]

use egui::{RichText, Ui};

use super::*;

/// 欄の詳しさ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Detail {
    /// 図の下: 置き方は偏心 y・傾き x
    Compact,
    /// レンズデータの下: 置き方は偏心 x・y、傾き x・y・z と群の役割
    Full,
}

impl Pick {
    /// 種類を変えた要素。置き方・後の空気・ON/OFF・状態ごとの値・群・id・ロックは残し、種類ごとの欄は新しい種類の
    /// 既定にする（前の種類のファイル・面・上書きなどを持ち越さない）
    pub(crate) fn retyped(&self, kind: ElemKind) -> Pick {
        Pick {
            air: self.air,
            decenter: self.decenter,
            tilt: self.tilt,
            locked: self.locked,
            lock_place: self.lock_place,
            on: self.on,
            off_air: self.off_air,
            state_on: self.state_on.clone(),
            state_air: self.state_air.clone(),
            state_place: self.state_place.clone(),
            group: self.group.clone(),
            id: self.id.clone(),
            air_id: self.air_id.clone(),
            air_medium: self.air_medium.clone(),
            ..Pick::new(kind)
        }
    }

    /// 反転できる（玉。ユーザーカタログの項目は群として入るので反転は無い）。自作の玉は、係数の符号を変えるだけで
    /// 反転できる面だけのとき（Zernike・Grid Sag などの面があれば反転しない）。反転したものは戻せる
    pub(crate) fn can_reverse(&self) -> bool {
        let kind = matches!(
            self.kind,
            ElemKind::Catalog | ElemKind::Lens | ElemKind::Zmx
        ) && !(self.kind == ElemKind::Catalog && !self.path.is_empty());
        let faces = self.kind != ElemKind::Lens
            || self.surfs.iter().all(|r| {
                r.special().is_none()
                    || serde_json::from_value::<lensbench_project::SurfSpec>(r.json())
                        .is_ok_and(|q| lensbench_project::spec_reversible(&q))
            });
        kind && (faces || self.reversed)
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
        // 上書きの中身はレンズデータの表のセルの印で見る
        if !p.edits.is_empty() || !p.ops.is_empty() {
            ui.label(
                RichText::new(trf(
                    "Changed in Lens Data: {}",
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
    let mut remap = None;
    ui.add_enabled_ui(!locked, |ui| {
        ui.horizontal_wrapped(|ui| {
            kind_fields(ui, p, st, k, weak, &coats);
            if p.can_reverse() {
                ui.checkbox(&mut p.reversed, tr("Reverse"));
            }
        });
        if p.kind == ElemKind::Surfaces {
            remap = surf_table(ui, p, k, weak);
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
                // 間隔を満たす媒質（液浸）。鏡の後には書けないので、鏡では書いてあるときだけ出す（外せるように）
                if p.kind != ElemKind::Mirror || !p.air_medium.is_empty() {
                    ui.add(
                        egui::TextEdit::singleline(&mut p.air_medium)
                            .hint_text(tr("Air"))
                            .desired_width(60.0),
                    )
                    .on_hover_text(tr(
                        "Medium filling this gap (glass name or refractive index, e.g. for immersion); empty is air",
                    ));
                }
            }
        });
    });
    // 構成への入れる・外すは「位置も」のロックでも変えられる（動かすのではないので）
    ui.horizontal_wrapped(|ui| onoff_ui(ui, p, st, k, weak));
    // 面の並びの行を変えたら、ほかの要素の操作が指すこの要素の面・座標ブレークの番号も合わせる
    if let Some(m) = remap {
        let id = combo
            .ids_all()
            .into_iter()
            .find(|(l, _)| *l == step.loc)
            .map(|(_, id)| id);
        if let Some(id) = id {
            combo.shift_row_refs(&id, &m);
        }
    }
    let Some(p) = combo.get_mut(&step.loc) else {
        return;
    };
    if p.kind == ElemKind::Splitter {
        ui.add_enabled_ui(!locked, |ui| bs_fields(ui, combo, &step.loc, weak));
    }
    // 要素の群の役割・移動（レイアウトのパネルの「群」と同じ）
    let g = combo
        .get(&step.loc)
        .map(|p| p.group.trim().to_string())
        .unwrap_or_default();
    if detail == Detail::Full && !g.is_empty() {
        ui.horizontal_wrapped(|ui| groups_ui(ui, combo, weak, Some(&g)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自作の玉は、理想レンズの面など反転できない面があれば反転を出さない（反転したものは戻せる）。
    /// Zernike・Binary 2 は反転できる
    #[test]
    fn lens_with_special_faces_reverses_only_when_it_can() {
        let mut p = Pick::new(ElemKind::Lens);
        p.surfs = vec![
            SurfRow {
                radius: 50.0,
                t: 4.0,
                glass: "N-BK7".into(),
                ..Default::default()
            },
            SurfRow {
                radius: -50.0,
                ..Default::default()
            },
        ];
        assert!(p.can_reverse());
        let row = |shape: Value, action: Option<Value>| {
            let mut r = json!({ "shape": shape });
            if let Some(a) = action {
                r["action"] = a;
            }
            r
        };
        p.surfs[1].more.insert(
            "row".into(),
            row(json!({ "zernike_standard": { "c": -0.02 } }), None),
        );
        assert!(p.can_reverse());
        p.surfs[1].more.insert(
            "row".into(),
            row(
                json!({ "conic": { "c": -0.02 } }),
                Some(json!({ "paraxial": { "f": 50.0 } })),
            ),
        );
        // 読める面で、理想レンズの面だから反転しない
        assert!(serde_json::from_value::<lensbench_project::SurfSpec>(p.surfs[1].json()).is_ok());
        assert!(!p.can_reverse());
        p.reversed = true;
        assert!(p.can_reverse());
        p.reversed = false;
        p.surfs[1].more.insert(
            "row".into(),
            row(
                json!({ "even_asphere": { "c": -0.02 } }),
                Some(json!({ "binary2": { "phase": { "order": 1.0, "norm": 10.0, "coef": [-20.0] } } })),
            ),
        );
        assert!(p.can_reverse());
    }
}
