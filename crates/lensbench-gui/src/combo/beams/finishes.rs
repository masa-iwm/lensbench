//! 面の仕上げ（プロジェクトの `finishes`）の一覧と、足す・消す・変える欄（組合せの道具の列の「仕上げ」）。
//! 散乱と光を通さない面の反射率を、要素の面・玉の縁・機構部品に付ける。非順次の追跡と迷光の PST だけが使う。
//! 値は組合せの系の設定（[`SysExtra::finishes`](super::super::SysExtra)）にプロジェクトの形のまま持つ

use egui::{RichText, Ui};
use lensbench_core::scatter::{Finish, Scatter, ScatterModel, ScatterSide};
use lensbench_project::edit_ops::BlockFace;
use lensbench_project::finish::FinishSpec;
use serde_json::Value;

use super::super::Combo;
use crate::i18n::{tr, trf};
use crate::theme;

/// 組合せの仕上げ（無ければ空）。読めない値は誤り
pub(crate) fn finishes_of(c: &Combo) -> Result<Vec<FinishSpec>, String> {
    match &c.extra.finishes {
        None => Ok(Vec::new()),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
    }
}

/// 仕上げを組合せに書く（空なら欄ごと外す）
pub(crate) fn set_finishes(c: &mut Combo, v: &[FinishSpec]) {
    c.extra.finishes = (!v.is_empty()).then(|| serde_json::to_value(v).unwrap_or(Value::Null));
}

/// 仕上げの対象の種類
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Face,
    Edge,
    Mech,
}

impl Target {
    fn of(f: &FinishSpec) -> Target {
        if f.edge.is_some() {
            Target::Edge
        } else if f.mech.is_some() {
            Target::Mech
        } else {
            Target::Face
        }
    }

    fn label(self) -> &'static str {
        match self {
            Target::Face => tr("Surface"),
            Target::Edge => tr("Lens edge"),
            Target::Mech => tr("Mechanical part"),
        }
    }
}

fn model_label(m: Option<ScatterModel>) -> &'static str {
    match m {
        None => tr("(none)"),
        Some(ScatterModel::Lambert) => tr("Lambertian"),
        Some(ScatterModel::Gauss) => tr("Gaussian"),
        Some(ScatterModel::Abg) => "ABg",
    }
}

fn side_label(s: ScatterSide) -> &'static str {
    match s {
        ScatterSide::Both => tr("Reflect and transmit"),
        ScatterSide::Reflect => tr("Reflect only"),
        ScatterSide::Transmit => tr("Transmit only"),
    }
}

/// 仕上げの誤り（対象・値の範囲。ブロックの有無は展開で見る）
pub(crate) fn problems(v: &[FinishSpec]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, f) in v.iter().enumerate() {
        let k = i + 1;
        let empty = |b: &Option<BlockFace>| b.as_ref().is_some_and(|b| b.block.trim().is_empty());
        if empty(&f.face) || empty(&f.edge) || f.mech.as_ref().is_some_and(|m| m.trim().is_empty())
        {
            out.push(trf("Finish {}: choose the target", &[&k]));
            continue;
        }
        let g = Finish {
            surface: f.face.as_ref().map(|_| 0),
            edge: f.edge.as_ref().map(|_| 0),
            mech: f.mech.clone(),
            reflect: f.reflect,
            scatter: f.scatter.clone(),
        };
        if let Err(e) = g.check() {
            out.push(trf(
                "Finish {}: {}",
                &[&k, &lensbench_core::i18n::tr(&e.to_string())],
            ));
        }
    }
    out
}

/// 光学面の要素（id のある要素）と、縁のある要素（玉・自作の玉）の id、機構部品の名前
struct Choices {
    blocks: Vec<String>,
    lenses: Vec<String>,
    mechs: Vec<String>,
}

/// 道具の列の「仕上げ」のメニュー: 仕上げの一覧と、足す・消す・変える欄。変えたら true
pub(crate) fn menu(ui: &mut Ui, c: &mut Combo) -> bool {
    let mut v = match finishes_of(c) {
        Ok(v) => v,
        Err(e) => {
            ui.label(RichText::new(tr("Finishes")).color(theme::weak_text(ui.visuals())))
                .on_hover_text(trf(
                    "The finishes of this project can't be edited here ({})",
                    &[&e],
                ));
            return false;
        }
    };
    let title = if v.is_empty() {
        tr("Finishes").to_string()
    } else {
        format!("{} ({})", tr("Finishes"), v.len())
    };
    let before = v.clone();
    let ch = Choices {
        blocks: c
            .ids_all()
            .into_iter()
            .map(|x| x.1)
            .filter(|id| !id.is_empty())
            .collect(),
        lenses: super::lens_ids(c),
        mechs: c.mechanics.iter().map(|m| m.name.clone()).collect(),
    };
    ui.menu_button(title, |ui| {
        ui.set_min_width(640.0);
        ui.label(
            RichText::new(tr(
                "Scatter and reflectance of surfaces, lens edges and mechanical parts. Only the non-sequential trace and the stray light PST use them",
            ))
            .small()
            .color(theme::weak_text(ui.visuals())),
        );
        let mut drop = None;
        egui::Grid::new("combo_finishes")
            .striped(true)
            .num_columns(6)
            .show(ui, |ui| {
                for h in [tr("Applies to"), "", tr("Reflectance"), tr("Scatter"), "", ""] {
                    ui.label(RichText::new(h).small().strong());
                }
                ui.end_row();
                for (i, f) in v.iter_mut().enumerate() {
                    row(ui, i, f, &ch);
                    if ui
                        .small_button("✕")
                        .on_hover_text(tr("Remove the finish"))
                        .clicked()
                    {
                        drop = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = drop {
            v.remove(i);
        }
        if ui.button(trf("+ {}", &[&tr("Add a finish")])).clicked() {
            v.push(FinishSpec {
                mech: ch.mechs.first().cloned(),
                face: ch.mechs.is_empty().then(|| face(ch.blocks.first().cloned().unwrap_or_default())),
                scatter: Some(Scatter::lambert(0.01)),
                ..Default::default()
            });
        }
        for p in problems(&v) {
            ui.colored_label(theme::error(ui.visuals()), p);
        }
    });
    if v == before {
        return false;
    }
    set_finishes(c, &v);
    true
}

fn face(block: String) -> BlockFace {
    BlockFace {
        block,
        surface: 0,
        coordbrk: false,
        back: false,
        pass: 0,
    }
}

/// 1 行の欄: 対象の種類・対象・反射率・散乱のモデルと値（Grid の升ごとに描くので、id は行の番号 i つき）
fn row(ui: &mut Ui, i: usize, f: &mut FinishSpec, ch: &Choices) {
    // 対象の種類と対象
    let mut t = Target::of(f);
    let was = t;
    egui::ComboBox::from_id_salt(("finish-target", i))
        .selected_text(t.label())
        .show_ui(ui, |ui| {
            for x in [Target::Face, Target::Edge, Target::Mech] {
                ui.selectable_value(&mut t, x, x.label());
            }
        })
        .response
        .accessible_name(tr("Applies to"));
    if t != was {
        let block = f
            .face
            .take()
            .or(f.edge.take())
            .map(|b| b.block)
            .unwrap_or_else(|| ch.blocks.first().cloned().unwrap_or_default());
        f.mech = None;
        match t {
            Target::Face => f.face = Some(face(block)),
            Target::Edge => {
                let b = if ch.lenses.contains(&block) {
                    block
                } else {
                    ch.lenses.first().cloned().unwrap_or_default()
                };
                f.edge = Some(face(b));
            }
            Target::Mech => f.mech = Some(ch.mechs.first().cloned().unwrap_or_default()),
        }
        // 光学面に反射率は書けない
        if t == Target::Face {
            f.reflect = None;
        }
    }
    ui.horizontal(|ui| match t {
        Target::Face | Target::Edge => {
            let ids = if t == Target::Face { &ch.blocks } else { &ch.lenses };
            let b = if t == Target::Face { &mut f.face } else { &mut f.edge };
            let b = b.get_or_insert_with(|| face(String::new()));
            let shown = if b.block.is_empty() { tr("(none)").to_string() } else { b.block.clone() };
            egui::ComboBox::from_id_salt(("finish-block", i))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for id in ids {
                        ui.selectable_value(&mut b.block, id.clone(), id.as_str());
                    }
                })
                .response
                .accessible_name(tr("Element"));
            ui.add(
                egui::DragValue::new(&mut b.surface)
                    .range(0..=99)
                    .prefix(format!("{} ", tr("Surface"))),
            )
            .on_hover_text(if t == Target::Face {
                tr("Surface of the element from 0 (a stop or an aperture: the plate outside the opening)")
            } else {
                tr("A surface of the lens from 0 (the edge of that lens)")
            });
        }
        Target::Mech => {
            let m = f.mech.get_or_insert_with(String::new);
            let shown = if m.is_empty() { tr("(none)").to_string() } else { m.clone() };
            egui::ComboBox::from_id_salt(("finish-mech", i))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for n in &ch.mechs {
                        ui.selectable_value(m, n.clone(), n.as_str());
                    }
                })
                .response
                .accessible_name(tr("Mechanical part"));
        }
    });
    // 反射率（光を通さない面: 機構部品・絞りの板・塗った縁）
    ui.horizontal(|ui| {
        let mut on = f.reflect.is_some();
        let can = t != Target::Face;
        ui.add_enabled(can, egui::Checkbox::without_text(&mut on))
            .on_hover_text(if can {
                tr("Reflectance of the opaque surface (the rest is absorbed). On a lens edge it makes the edge opaque (a painted edge)")
            } else {
                tr("The reflection of an optical surface comes from the Fresnel equations with its coating")
            });
        if !can {
            on = false;
        }
        match (on, f.reflect) {
            (false, _) => f.reflect = None,
            (true, None) => f.reflect = Some(0.04),
            _ => {}
        }
        if let Some(r) = &mut f.reflect {
            ui.add(egui::DragValue::new(r).range(0.0..=1.0).speed(0.005).max_decimals(4));
        }
    });
    // 散乱のモデルと値
    let mut model = f.scatter.as_ref().map(|s| s.model);
    egui::ComboBox::from_id_salt(("finish-model", i))
        .selected_text(model_label(model))
        .show_ui(ui, |ui| {
            for m in [
                None,
                Some(ScatterModel::Lambert),
                Some(ScatterModel::Gauss),
                Some(ScatterModel::Abg),
            ] {
                ui.selectable_value(&mut model, m, model_label(m));
            }
        })
        .response
        .accessible_name(tr("Scatter"));
    match model {
        None => f.scatter = None,
        Some(m) => {
            let s = f.scatter.get_or_insert_with(|| Scatter::lambert(0.01));
            if s.model != m {
                *s = match m {
                    ScatterModel::Lambert => Scatter {
                        on: s.on,
                        ..Scatter::lambert(0.01)
                    },
                    ScatterModel::Gauss => Scatter {
                        model: m,
                        fraction: 0.01,
                        sigma: 0.05,
                        on: s.on,
                        ..Default::default()
                    },
                    ScatterModel::Abg => Scatter {
                        model: m,
                        a: 1e-3,
                        b: 1e-3,
                        g: 2.0,
                        on: s.on,
                        ..Default::default()
                    },
                };
            }
        }
    }
    ui.horizontal(|ui| {
        let Some(s) = &mut f.scatter else {
            return;
        };
        let num = |ui: &mut Ui, v: &mut f64, name: &str, hint: &str| {
            ui.add(egui::DragValue::new(v).speed(1e-3).max_decimals(6).prefix(format!("{name} ")))
                .on_hover_text(hint.to_string());
        };
        match s.model {
            ScatterModel::Lambert | ScatterModel::Gauss => {
                num(ui, &mut s.fraction, tr("Fraction"), tr("Fraction of the light that scatters (0 to 1)"));
                if s.model == ScatterModel::Gauss {
                    num(ui, &mut s.sigma, "σ", tr("Width of the Gaussian in direction cosine space"));
                }
            }
            ScatterModel::Abg => {
                let h = tr("BSDF = A / (B + |β − β0|^g) [1/sr]; the scattered fraction follows from the BSDF");
                num(ui, &mut s.a, "A", h);
                num(ui, &mut s.b, "B", h);
                num(ui, &mut s.g, "g", h);
            }
        }
        let mut depol = s.depolarizes();
        if ui
            .checkbox(&mut depol, tr("Depolarize"))
            .on_hover_text(tr(
                "The scattered rays lose the polarization (linear polarization in a random direction). Off keeps it, projected onto the plane across the scattered direction. Default: Lambertian loses it, Gaussian and ABg keep it",
            ))
            .changed()
        {
            // 既定と同じなら書かない
            let def = Scatter {
                depolarize: None,
                ..s.clone()
            }
            .depolarizes();
            s.depolarize = (depol != def).then_some(depol);
        }
        egui::ComboBox::from_id_salt(("finish-side", i))
            .selected_text(side_label(s.on))
            .show_ui(ui, |ui| {
                for x in [ScatterSide::Both, ScatterSide::Reflect, ScatterSide::Transmit] {
                    ui.selectable_value(&mut s.on, x, side_label(x));
                }
            })
            .response
            .accessible_name(tr("Scatter side"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仕上げはプロジェクトの形のまま持ち、組合せのプロジェクトに書いて読み戻せる。誤りは一覧に出る
    #[test]
    fn finishes_round_trip_through_the_project() {
        let mut c = Combo::default();
        assert!(finishes_of(&c).unwrap().is_empty());
        let v = vec![
            FinishSpec {
                face: Some(face("L1".into())),
                scatter: Some(Scatter::lambert(0.02)),
                ..Default::default()
            },
            FinishSpec {
                mech: Some("tube".into()),
                reflect: Some(0.05),
                ..Default::default()
            },
        ];
        set_finishes(&mut c, &v);
        let p = c.project();
        assert_eq!(
            p["finishes"][0]["face"],
            serde_json::json!({"block": "L1", "surface": 0})
        );
        assert_eq!(p["finishes"][1]["mech"], "tube");
        let e = crate::combo::SysExtra::read(&p).unwrap();
        assert_eq!(e.finishes, c.extra.finishes);
        assert!(problems(&v).is_empty(), "{:?}", problems(&v));
        let mut bad = v.clone();
        bad[0].scatter = Some(Scatter::lambert(2.0));
        bad[1].mech = Some(String::new());
        assert_eq!(problems(&bad).len(), 2, "{:?}", problems(&bad));
        // 光学面に反射率だけ・何も無い面は誤り
        let none = vec![FinishSpec {
            face: Some(face("L1".into())),
            ..Default::default()
        }];
        assert_eq!(problems(&none).len(), 1);
        // 偏光を失うかは既定と違うときだけ書く
        let mut d = v.clone();
        d[0].scatter.as_mut().unwrap().depolarize = Some(false);
        set_finishes(&mut c, &d);
        assert_eq!(c.project()["finishes"][0]["scatter"]["depolarize"], false);
        set_finishes(&mut c, &[]);
        assert!(c.extra.finishes.is_none() && c.project().get("finishes").is_none());
    }
}
