//! 一覧を入れ替えても、選んでいた品・ガラスを鍵で探し直す

use super::*;

fn lens(v: &str, p: &str) -> LensRow {
    LensRow {
        vendor: v.into(),
        part: p.into(),
        efl: 50.0,
        diameter: 25.4,
        thickness: 3.0,
        pieces: 1,
        ..Default::default()
    }
}

fn glass(c: &str, n: &str) -> GlassRow {
    GlassRow {
        name: n.into(),
        catalog: c.into(),
        nd: 1.5,
        vd: 60.0,
        range: None,
    }
}

#[test]
fn reload_keeps_selected_lens_by_vendor_and_part() {
    let mut b = Browser::default();
    b.set_lenses(vec![lens("A", "1"), lens("B", "2"), lens("C", "3")]);
    b.lens_sel = Some(1);
    // 並びが変わっても同じ品を指す
    b.set_lenses(vec![
        lens("C", "3"),
        lens("X", "9"),
        lens("A", "1"),
        lens("B", "2"),
    ]);
    assert_eq!(b.lens_sel, Some(3));
    // 無くなったら外す
    b.set_lenses(vec![lens("A", "1")]);
    assert_eq!(b.lens_sel, None);
}

#[test]
fn reload_keeps_selected_glass_by_catalog_and_name() {
    let mut b = Browser::default();
    b.set_glasses(vec![glass("S", "N-BK7"), glass("O", "S-BSL7")]);
    b.glass_sel = Some(1);
    b.set_glasses(vec![glass("O", "S-BSL7"), glass("S", "N-BK7")]);
    assert_eq!(b.glass_sel, Some(0));
    // 同じ名前でもカタログが違えば別のガラス
    b.set_glasses(vec![glass("S", "S-BSL7")]);
    assert_eq!(b.glass_sel, None);
}
