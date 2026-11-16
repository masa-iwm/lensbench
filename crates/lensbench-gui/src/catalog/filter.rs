//! カタログの一覧の絞り込み（品・ガラス）。catalog.rs から分けた（GPN-24）

use super::*;

/// 品の種類の選択肢（serve の `category`、表示名）
pub const CATEGORIES: &[(&str, &str)] = &[
    ("lens", n_("Lenses")),
    ("mirror", n_("Mirror")),
    ("splitter", "BS"),
    ("polarizer", n_("Polarizer")),
    ("waveplate", n_("Waveplate")),
    ("window", n_("Window")),
    ("filter", n_("Filter")),
    ("prism", n_("Prism")),
    ("tube", n_("Tube")),
    ("cage_plate", n_("Cage plate")),
    ("mount", n_("Mount")),
    ("rod", n_("Rod")),
    ("adapter", n_("Adapter")),
    ("other", n_("Other")),
];

/// 品の検索の対象の文字列（メーカー・品番・ガラス・説明・タグ・仕様を小文字で 1 本に）
fn lens_search_text(r: &LensRow) -> String {
    let mut h = format!(
        "{}\n{}\n{}\n{}",
        r.vendor,
        r.part,
        r.glasses.join(" "),
        r.description
    );
    if let Some(a) = r.raw["tags"].as_array() {
        for t in a.iter().filter_map(Value::as_str) {
            h.push('\n');
            h.push_str(t);
        }
    }
    for (_, v) in r.specs() {
        h.push('\n');
        h.push_str(&spec_text("", v));
    }
    h.to_lowercase()
}

/// 文字列の絞り込み: 空白で区切った語がすべて（大文字・小文字を区別せず）どこかに含まれる
pub(super) fn text_match(query: &str, fields: &[&str]) -> bool {
    let hay: Vec<String> = fields.iter().map(|x| x.to_lowercase()).collect();
    query.split_whitespace().all(|w| {
        let w = w.to_lowercase();
        hay.iter().any(|h| h.contains(&w))
    })
}

pub fn filter_lenses(rows: &[LensRow], q: &LensFilter, sort: LensSort, desc: bool) -> Vec<usize> {
    // 読めない仕様の条件は無視する（欄の横に誤りを出す）
    let specs = parse_spec_filters(&q.spec).unwrap_or_default();
    // 検索語は 1 度だけ小文字に。行の文字列は安い条件を通った行だけ、検索語があるときだけ作る（GPN-24）
    let words: Vec<String> = q.text.split_whitespace().map(str::to_lowercase).collect();
    let mut v: Vec<usize> = (0..rows.len())
        .filter(|&i| {
            let r = &rows[i];
            (q.vendor.is_empty() || r.vendor == q.vendor)
                && q.category.as_ref().is_none_or(|c| &r.category == c)
                && q.efl.contains(r.efl)
                && q.diameter.contains(r.diameter)
                && match q.sign {
                    Sign::Any => true,
                    Sign::Positive => r.efl > 0.0,
                    Sign::Negative => r.efl < 0.0,
                }
                && match q.pieces {
                    0 => true,
                    3 => r.pieces >= 3,
                    n => r.pieces == n,
                }
                && q.kind.is_none_or(|k| r.kind == k)
                && q.kind_group.is_none_or(|m| (r.kind == RowKind::Mech) == m)
                && q.fit.as_ref().is_none_or(|f| match f.mode {
                    FitMode::MechFor => {
                        r.kind == RowKind::Mech && lensbench_catalog::spec::fits_row(&r.raw, f.d)
                    }
                    FitMode::OpticsFor => {
                        r.kind != RowKind::Mech
                            && r.diameter.is_finite()
                            && fits_hold(f.d, r.diameter)
                    }
                })
                && specs.iter().all(|f| f.matches(&r.raw))
                && (words.is_empty() || {
                    let hay = lens_search_text(r);
                    words.iter().all(|w| hay.contains(w.as_str()))
                })
        })
        .collect();
    let key = |i: &usize| {
        let r = &rows[*i];
        match sort {
            LensSort::Vendor | LensSort::Part => 0.0,
            LensSort::Efl => r.efl,
            LensSort::Diameter => r.diameter,
            LensSort::Thickness => r.thickness,
        }
    };
    v.sort_by(|a, b| {
        let o = match sort {
            LensSort::Vendor => {
                (&rows[*a].vendor, &rows[*a].part).cmp(&(&rows[*b].vendor, &rows[*b].part))
            }
            LensSort::Part => rows[*a].part.cmp(&rows[*b].part),
            // NaN は最後
            _ => key(a).total_cmp(&key(b)),
        };
        if desc {
            o.reverse()
        } else {
            o
        }
    });
    v
}

pub fn filter_glasses(rows: &[GlassRow], q: &GlassFilter) -> Vec<usize> {
    (0..rows.len())
        .filter(|&i| {
            let r = &rows[i];
            (q.catalog.is_empty() || r.catalog == q.catalog)
                && text_match(&q.text, &[&r.name, &r.catalog])
                && q.nd.contains(r.nd)
                && q.vd.contains(r.vd)
        })
        .collect()
}

/// 範囲の入力（チェックで有効にする）
pub(super) fn range_ui(ui: &mut Ui, label: &str, r: &mut Range, default: (f64, f64), speed: f64) {
    ui.label(label);
    let mut on = r.min.is_some();
    if ui.checkbox(&mut on, tr("Min")).changed() {
        r.min = on.then_some(default.0);
    }
    if let Some(m) = r.min.as_mut() {
        ui.add(egui::DragValue::new(m).speed(speed));
    }
    let mut on = r.max.is_some();
    if ui.checkbox(&mut on, tr("Max")).changed() {
        r.max = on.then_some(default.1);
    }
    if let Some(m) = r.max.as_mut() {
        ui.add(egui::DragValue::new(m).speed(speed));
    }
}

/// 種類の選択（「機構部品（すべて）」と、一覧にある品の種類）
pub(super) fn kind_combo(ui: &mut Ui, q: &mut LensFilter, kinds: &KindChoices) {
    let name = |k: &str| {
        CATEGORIES
            .iter()
            .find(|(c, _)| *c == k)
            .map_or(k.to_string(), |(_, n)| tr(n).to_string())
    };
    let mech_all = tr("Mechanics (all)");
    let optics_all = tr("Optics (all)");
    let selected = match (&q.category, q.kind_group) {
        (Some(c), _) => name(c),
        (None, Some(true)) => mech_all.to_string(),
        (None, Some(false)) => optics_all.to_string(),
        (None, None) => tr("All").to_string(),
    };
    let has_mech = kinds.has_mech;
    egui::ComboBox::from_id_salt("category")
        .selected_text(selected)
        .height(480.0)
        .show_ui(ui, |ui| {
            let mut pick = |ui: &mut Ui, cat: Option<&str>, group: Option<bool>, label: String| {
                let sel = q.category.as_deref() == cat && (cat.is_some() || q.kind_group == group);
                if ui.selectable_label(sel, label).clicked() {
                    q.category = cat.map(str::to_string);
                    q.kind_group = if cat.is_some() { None } else { group };
                }
            };
            pick(ui, None, None, tr("All").to_string());
            if has_mech {
                pick(ui, None, Some(false), optics_all.to_string());
                pick(ui, None, Some(true), mech_all.to_string());
            }
            // 光学素子の種類、続けて機構部品の種類
            for mech in [false, true] {
                let ks = if mech { &kinds.mech } else { &kinds.optics };
                if ks.is_empty() {
                    continue;
                }
                ui.separator();
                if has_mech {
                    ui.weak(if mech {
                        tr("Mechanical parts")
                    } else {
                        tr("Optics")
                    });
                }
                for &k in ks {
                    pick(ui, Some(k), None, format!("  {}", name(k)));
                }
            }
        });
}
