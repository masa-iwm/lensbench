//! 系にしまう瞳関数: 同じ系・同じ引数なら追い直さず、系を直すと捨てる。値はしまわずに追ったものと同じ

use crate::common::root;
use lensbench_core::pupil::{PupilFunction, PupilGrid};
use lensbench_core::system::FieldSel;
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};
use std::sync::Arc;

fn load(name: &str) -> System {
    zmx::load_with(
        &root().join("tests/refs").join(name).join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn pupil_function_is_kept_until_the_system_changes() {
    let mut s = load("A0");
    let f = FieldSel::Index(1);
    let g = PupilGrid::WavefrontMap(32);
    let a = PupilFunction::trace(&s, f, None, g, false).unwrap();
    // 主波長を番号で渡しても同じもの
    let b = PupilFunction::trace(&s, f, Some(s.primary), g, false).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    // 引数が違えば別
    let c = PupilFunction::trace(&s, f, None, g, true).unwrap();
    let d = PupilFunction::trace(&s, FieldSel::Index(0), None, g, false).unwrap();
    assert!(!Arc::ptr_eq(&a, &c) && !Arc::ptr_eq(&a, &d));
    // 複製はしまったものを持たず、追い直した値はビットまで同じ
    let e = PupilFunction::trace(&s.clone(), f, None, g, false).unwrap();
    assert!(!Arc::ptr_eq(&a, &e));
    assert_eq!(a.mask, e.mask);
    for q in 0..a.len() {
        if a.mask[q] {
            assert_eq!(a.opd[q].to_bits(), e.opd[q].to_bits());
        }
    }
    // 系を直すと捨てる
    s.edit(|p| {
        p.surfaces[2].t += 0.5;
        Ok(())
    })
    .unwrap();
    let h = PupilFunction::trace(&s, f, None, g, false).unwrap();
    assert!(!Arc::ptr_eq(&a, &h));
    assert!((0..a.len()).any(|q| a.mask[q] && h.mask[q] && a.opd[q] != h.opd[q]));
}
