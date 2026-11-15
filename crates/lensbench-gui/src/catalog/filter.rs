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
                        r.kind == RowKind::Mech
                            && lensbench_project::catalog_spec::fits_row(&r.raw, f.d)
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
