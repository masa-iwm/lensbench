//! カタログの一覧で選んだ品の 1 行の説明。catalog.rs から分けた

use super::*;

/// 選んだ品の 1 行の説明（誤り・未完の品は 2 行目に出すので空）
pub(super) fn row_detail(r: &LensRow) -> String {
    if r.error.is_some() || r.is_incomplete() {
        return String::new();
    }
    if r.kind == RowKind::Splitter {
        return splitter_detail(r);
    }
    if r.kind == RowKind::Mech {
        // 簡易外形（丸は外径、角は一辺）と保持径
        let m = &r.raw["mech"];
        let od = fmt_num(m["outer_diameter"].as_f64().unwrap_or(0.0));
        let mut t = if m["outline"] == "square" {
            trf(
                "Mechanical part: length {} mm, square (side {} mm)",
                &[&fmt_num(r.thickness), &od],
            )
        } else {
            trf(
                "Mechanical part: length {} mm, round (outer diameter {} mm)",
                &[&fmt_num(r.thickness), &od],
            )
        };
        if let Some(h) = hold_diameter(r) {
            let max = hold_max(r);
            if max > 0.0 {
                t.push_str(&trf(
                    ", clamps optics of diameter {}–{} mm",
                    &[&fmt_num(h), &fmt_num(max)],
                ));
            } else {
                t.push_str(&trf(", holding diameter {} mm", &[&fmt_num(h)]));
            }
        }
        if let Some(rod) = m["rod_spacing"].as_f64().filter(|x| *x > 0.0) {
            t.push_str(&trf(", cage rods {} mm apart", &[&fmt_num(rod)]));
        }
        // 端のねじ（つなぐとき合うかを見る）
        for (key, front) in [("front_thread", true), ("back_thread", false)] {
            let e = &m[key];
            let Some(name) = e["thread"].as_str() else {
                continue;
            };
            let sex = if e["male"].as_bool().unwrap_or(false) {
                tr("male thread")
            } else {
                tr("female thread")
            };
            let len = fmt_num(e["length"].as_f64().unwrap_or(0.0));
            t.push_str(&if front {
                trf(", front end: {} {} ({} mm long)", &[&name, &sex, &len])
            } else {
                trf(", back end: {} {} ({} mm long)", &[&name, &sex, &len])
            });
            if let Some(rc) = e["recess"].as_f64().filter(|x| *x > 0.0) {
                t.push_str(&trf(", recessed {} mm", &[&fmt_num(rc)]));
            }
        }
        return t;
    }
    if let Some((radius, _)) = r.mirror {
        let mut t = if radius == 0.0 {
            trf("Flat mirror: dia. {} mm", &[&fmt_num(r.diameter)])
        } else {
            trf(
                "Mirror: R = {} mm, f = {} mm, diameter {} mm",
                &[&fmt_num(radius), &fmt_num(r.efl), &fmt_num(r.diameter)],
            )
        };
        if !r.coat.is_empty() {
            t.push_str("  ");
            t.push_str(&trf("coating {}", &[&r.coat]));
        }
        return t;
    }
    trf(
        "f = {} mm, diameter {} mm, thickness {} mm, {} element(s), glass {}",
        &[
            &fmt_num(r.efl),
            &fmt_num(r.diameter),
            &fmt_num(r.thickness),
            &r.pieces,
            &r.glasses.join(" / "),
        ],
    )
}

/// BS の詳細の行
fn splitter_detail(r: &LensRow) -> String {
    let sp = r.splitter.clone().unwrap_or_default();
    let form = sp["form"].as_str().unwrap_or("cube");
    let size = if form == "cube" {
        trf("cube, side {} mm", &[&fmt_num(r.diameter)])
    } else if form == "plate" {
        trf(
            "plate, thickness {} mm, dia. {} mm",
            &[&fmt_num(r.thickness), &fmt_num(r.diameter)],
        )
    } else {
        tr("ideal surface").to_string()
    };
    let split = if !r.coat.is_empty() {
        trf("coating {}", &[&r.coat])
    } else if sp["pbs"].is_object() {
        "PBS".to_string()
    } else {
        trf("T {}", &[&fmt_num(sp["ratio"].as_f64().unwrap_or(0.5))])
    };
    let mut v = vec![size, split];
    if !r.glasses.is_empty() {
        v.push(trf("glass {}", &[&r.glasses.join(" / ")]));
    }
    if let Some(ar) = sp["ar"].as_str().filter(|a| !a.is_empty()) {
        v.push(trf("AR {}", &[&ar]));
    }
    format!("BS: {}", v.join(tr(", ")))
}
