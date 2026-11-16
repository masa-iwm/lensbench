//! セルの右クリック・[...] の欄（ソルブ・ピックアップ・屈折率の追加設定・多重構成の行・組合せの上書きを外す）

use super::*;

/// ソルブの種類が曲率のものか
fn curv_kind(k: &str) -> bool {
    lensbench_core::solves::solve_kind_parse(k, 0.0, 0.0, Some(0)).is_ok_and(|x| x.is_curv())
}

/// セルの右クリックの欄の「上書きを外す」（組合せの系で、面の上書きのあるセル）
pub(super) fn override_item(ui: &mut Ui, c: &Ctx, j: usize, col: Col) -> Option<Act> {
    if !(c.overridden)(j, col) {
        return None;
    }
    let b = ui
        .button(tr("Remove override"))
        .on_hover_text(tr("Back to the value of the element"));
    ui.separator();
    if b.clicked() {
        ui.close_menu();
        return Some(Act::DropOverride { j, col });
    }
    None
}

/// セルの右クリック・[...] の欄: その値のソルブ・ピックアップ（OpticStudio のソルブの小欄のように）。硝材は
/// 屈折率の追加設定（dn・disp・複屈折）も。bire は面の複屈折の値（読んでいなければ None）、combo は組合せの系
#[allow(clippy::too_many_arguments)]
pub(super) fn cell_menu(
    ui: &mut Ui,
    m: &mut CellMenu,
    j: usize,
    col: Col,
    r: &Row,
    bire: Option<&Value>,
    combo: bool,
    cfg: (usize, &[String]),
) -> Vec<Act> {
    let mut ops: Vec<Value> = Vec::new();
    let mut acts = Vec::new();
    let weak = theme::weak_text(ui.visuals());
    if m.cell != Some((j, col)) {
        m.cell = Some((j, col));
        m.dn = crate::surf_edit::full(r.dn);
        m.disp = crate::surf_edit::full(r.disp);
    }
    let field = |ui: &mut Ui, label: &str, s: &mut String, hint: &str, w: f32| {
        ui.label(RichText::new(label).color(weak));
        ui.add(
            egui::TextEdit::singleline(s)
                .hint_text(hint)
                .desired_width(w),
        )
    };
    if let Some(w) = col.solve_what() {
        ui.label(RichText::new(tr("Solves")).strong());
        if let Some((_, t)) = r.solves.iter().find(|s| s.0 == w) {
            ui.horizontal(|ui| {
                ui.label(t);
                if ui.small_button(tr("Remove")).clicked() {
                    ops.push(json!({ "op": "remove_solve", "surface": j, "what": w }));
                }
            });
        }
        let kinds: Vec<&str> = lensbench_core::solves::SOLVE_KINDS
            .iter()
            .copied()
            .filter(|k| curv_kind(k) == (w == "c"))
            .collect();
        if !kinds.contains(&m.kind.as_str()) {
            m.kind = kinds.first().copied().unwrap_or_default().into();
        }
        ui.menu_button(trf("Kind: {}", &[&m.kind]), |ui| {
            for k in &kinds {
                if ui.selectable_label(m.kind == *k, *k).clicked() {
                    m.kind = k.to_string();
                }
            }
        });
        ui.horizontal(|ui| {
            field(ui, tr("Value"), &mut m.val, "0", 60.0);
            field(ui, "param", &mut m.param, "", 50.0);
            field(ui, tr("Other surface"), &mut m.other, "", 40.0);
        });
        if ui.button(tr("+ Make solve")).clicked() {
            let mut op = json!({ "op": "add_solve", "surface": j, "kind": m.kind });
            let f = |s: &str| s.trim().parse::<f64>().ok();
            if let Some(x) = f(&m.val) {
                op["value"] = json!(x);
            }
            if let Some(x) = f(&m.param) {
                op["param"] = json!(x);
            }
            if let Ok(o) = m.other.trim().parse::<usize>() {
                op["other"] = json!(o);
            }
            ops.push(op);
        }
        ui.separator();
    }
    let picked_mat = r.pickups.iter().any(|p| p.0 == "dn");
    if let Some(w) = col.pick_what() {
        let set = col.pick_set();
        ui.label(RichText::new(tr("Pickups")).strong());
        let now: Vec<&(String, String)> = r
            .pickups
            .iter()
            .filter(|p| p.0 == w || (col == Col::R && p.0 == "c"))
            .collect();
        for (_, t) in &now {
            ui.horizontal(|ui| {
                ui.label(t);
                if ui.small_button(tr("Remove")).clicked() {
                    // 硝材は dn・disp の組を外す
                    for p in r
                        .pickups
                        .iter()
                        .filter(|p| set.contains(&p.0) || (col == Col::R && p.0 == "c"))
                    {
                        ops.push(json!({ "op": "remove_pickup", "dst": j, "what": p.0 }));
                    }
                }
            });
        }
        // 硝材・膜は名前を写すだけ（倍率・オフセットは使わない）
        let named = matches!(col, Col::Material | Col::Coating);
        ui.horizontal(|ui| {
            field(ui, tr("Source surface"), &mut m.src, "", 40.0);
            if !named {
                field(ui, tr("Scale"), &mut m.scale, "1", 50.0);
                field(ui, tr("Offset"), &mut m.offset, "0", 50.0);
            }
        });
        if col == Col::Material {
            ui.label(
                RichText::new(tr("Follows the source surface's material (and dn, disp)"))
                    .small()
                    .color(weak),
            );
        }
        if ui.button(tr("+ Make pickup")).clicked() {
            match m.src.trim().parse::<usize>() {
                Ok(src) => {
                    let f = |x: &str, d: f64| x.trim().parse::<f64>().unwrap_or(d);
                    let (sc, off) = if named {
                        (1.0, 0.0)
                    } else {
                        (f(&m.scale, 1.0), f(&m.offset, 0.0))
                    };
                    for w in &set {
                        ops.push(json!({
                            "op": "add_pickup", "dst": j, "src": src, "what": w,
                            "scale": sc, "offset": off,
                        }));
                    }
                }
                Err(_) => acts.push(Act::Refused(
                    tr("A pickup needs the source surface number").into(),
                )),
            }
        }
        ui.separator();
    }
    // 硝材の追加設定: 屈折率のずれ・分散の変化・複屈折（鏡・座標ブレークには無い）
    if col == Col::Material && !r.is_cb() && !r.material.eq_ignore_ascii_case("MIRROR") {
        ui.label(RichText::new(tr("Index")).strong());
        let mut fields = Map::new();
        ui.add_enabled_ui(!picked_mat, |ui| {
            egui::Grid::new(("ld_index", j))
                .num_columns(2)
                .show(ui, |ui| {
                    for (key, name, cur, buf) in [
                        ("dn", tr("Index offset dn"), r.dn, &mut m.dn),
                        ("disp", tr("Dispersion change disp"), r.disp, &mut m.disp),
                    ] {
                        ui.label(RichText::new(name).color(weak));
                        let resp = ui.add(egui::TextEdit::singleline(buf).desired_width(110.0));
                        if resp.lost_focus() {
                            match buf.trim().parse::<f64>().ok().filter(|v| v.is_finite()) {
                                Some(v) if v != cur => {
                                    fields.insert(key.into(), json!(v));
                                }
                                Some(_) => {}
                                None => acts
                                    .push(Act::Refused(trf("{} is not a number", &[&buf.trim()]))),
                            }
                        }
                        ui.end_row();
                    }
                });
        });
        if picked_mat {
            ui.label(
                RichText::new(tr(
                    "Material is picked up (dn and disp come from the source too)",
                ))
                .small()
                .color(weak),
            );
        }
        if !fields.is_empty() {
            acts.push(Act::Fields { j, fields });
        }
        // 複屈折材の板の入口・出口（開いた系の標準面。組合せは要素の波長板で）
        if (!combo && r.typ == "STANDARD") || bire.is_some_and(|b| !b.is_null()) {
            ui.label(RichText::new(tr("Birefringence")).color(weak));
            match bire {
                Some(b) => {
                    if let Some(nb) = crate::surf_edit::bire_ui(ui, j, b) {
                        let mut data = Map::new();
                        data.insert("bire".into(), nb);
                        acts.push(Act::Data { j, data });
                    }
                }
                None => {
                    ui.weak(tr("Reading the surface…"));
                }
            }
        }
        ui.separator();
    }
    // 多重構成の行にする（開いた系）
    if let (false, Some(name)) = (combo, config_param(r, j, col)) {
        let have = cfg.1.iter().any(|p| p.eq_ignore_ascii_case(&name));
        let resp = ui.add_enabled(
            !have,
            egui::Button::new(if have {
                trf("Multi-config row {} exists", &[&name])
            } else {
                trf("Make a multi-config row ({})", &[&name])
            }),
        );
        if resp.clicked() {
            if cfg.0 < 2 {
                ops.push(json!({ "op": "set_config_count", "n": 2 }));
            }
            ops.push(json!({ "op": "add_config_row", "param": name }));
        }
        // 最適化の変数にする（硝材の dn は除く）
        if col != Col::Material
            && ui
                .button(trf("Make an optimization variable ({})", &[&name]))
                .on_hover_text(tr(
                    "Adds it to the Variables table of the Optimize panel and shows the panel",
                ))
                .clicked()
        {
            acts.push(Act::OptVar(name.clone()));
        }
        ui.separator();
    }
    if !ops.is_empty() {
        acts.push(Act::Ops(ops));
    }
    if !acts.is_empty() {
        ui.close_menu();
    }
    acts
}
