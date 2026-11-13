//! 自動の半直径: OpticStudio が保存した .zmx の自動の値（DIAM 値 0）と、光線から求めた値が合う。
//! 基準は tests/refs の系（OpticStudio 2026 R1 が保存、Fast Semi-Diameters はオン）。PC の OpticStudio で
//! 開き直して自動の値が保存のままであることも確かめた（非公開側の data/validate/semi_auto）

use crate::common::root;
use lensbench_core::param::{Kind, Param};
use lensbench_core::surface::SurfType;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;

fn load(name: &str, fixed: bool) -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        fixed_semi: fixed,
        ..Default::default()
    };
    zmx::load_with(&root().join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

#[test]
fn matches_opticstudio_automatic() {
    // 実光線エイミングの系は絞り面で相対 1e-8（エイミングの収束の差）。合わないもの: A6（矩形の視野の正規化でケラレ係数・VAN のある視野、OpticStudio の光線の取り方が分からない）。
    // Grid Sag の系（G*）は OpticStudio の Grid Sag の追跡の差で相対 2e-6 まで
    let mut n = 0;
    for e in std::fs::read_dir(root().join("tests/refs"))
        .unwrap()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().to_string();
        if !e.path().join("system.zmx").exists() || name == "A6" {
            continue;
        }
        let tol = if name.starts_with('G') { 5e-6 } else { 1e-7 };
        let file = load(&name, true);
        let s = load(&name, false);
        for (j, (a, b)) in file.surfaces.iter().zip(&s.surfaces).enumerate() {
            if !b.semi_auto || b.ignore {
                continue;
            }
            assert!(
                (a.semi - b.semi).abs() <= tol * (1.0 + a.semi.abs()),
                "{name} 面 {j}: OpticStudio {} lensbench {}",
                a.semi,
                b.semi
            );
            n += 1;
        }
    }
    assert!(n > 250, "比べた面が少ない（{n}）");
}

#[test]
fn recomputed_on_change_and_fixed_stays() {
    let s = load("A0", false);
    assert!(s.surfaces[1..].iter().all(|d| d.semi_auto));
    // 入射瞳を大きくすると前の面の半径が大きくなる
    let s2 = s
        .with_params(&[(Param::new(1, Kind::Thick), s.surfaces[1].t + 1.0)])
        .unwrap();
    assert!((s2.surfaces[2].semi - s.surfaces[2].semi).abs() > 1e-3);
    // 固定の面は変えない
    let mut f = s.clone();
    f.surfaces[2].semi_auto = false;
    f.surfaces[2].semi = 20.0;
    let f2 = f
        .with_params(&[(Param::new(1, Kind::Thick), f.surfaces[1].t + 1.0)])
        .unwrap();
    assert_eq!(f2.surfaces[2].semi, 20.0);
    // カタログの玉の読み方（fixed_semi）はファイルの値で固定
    let c = load("A0", true);
    assert!(c.surfaces.iter().all(|d| !d.semi_auto));
}

#[test]
fn edit_semi_auto_and_zmx_roundtrip() {
    use lensbench_core::edit::SurfValue;
    let mut s = load("A0", false);
    let v = s.surfaces[3].semi;
    s.edit(|s| s.set_surface_fields(3, &[("semi".into(), SurfValue::Num(7.5))]))
        .unwrap();
    assert!(!s.surfaces[3].semi_auto);
    assert_eq!(s.surfaces[3].semi, 7.5);
    // 書き出すと固定は DIAM … 1、読み直しても固定
    let txt = lensbench_core::zmx_write::write(&s).unwrap();
    let r = zmx::build(
        &zmx::parse_text(&txt).unwrap(),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
        &lensbench_core::glass::catalog_dirs(&[root().join("tests/glass")]),
        None,
    )
    .unwrap();
    assert!(!r.surfaces[3].semi_auto && r.surfaces[3].semi == 7.5);
    assert!(r.surfaces[4].semi_auto);
    s.edit(|s| s.set_surface_fields(3, &[("semi".into(), SurfValue::Text("auto".into()))]))
        .unwrap();
    assert!(s.surfaces[3].semi_auto);
    assert!((s.surfaces[3].semi - v).abs() < 1e-12);
    assert_eq!(s.surfaces[3].typ, SurfType::Standard);
}

#[test]
fn legacy_lensbench_files_keep_fixed_semi() {
    // 自動の半直径を入れる前の lensbench が書いた .zmx（VERS 260327 42 …、MEMA の行なし）は、DIAM … 0 でも
    // ファイルの値で固定として読む（以前の書き出しは固定の半直径も 0 で書いていた）
    let p = root().join("tests/splitter/cube_T.zmx");
    let raw = zmx::parse_text(&lensbench_core::glass::read_text(&p).unwrap()).unwrap();
    assert!(raw.legacy_lensbench());
    let s = zmx::load_with(
        &p,
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(s.surfaces.iter().all(|d| !d.semi_auto));
    assert!(s.surfaces.iter().any(|d| d.semi == 8.0));
    // 浮動開口は足さない（以前の読み方と同じ）
    assert!(s
        .surfaces
        .iter()
        .all(|d| !matches!(d.ap, lensbench_core::surface::Aperture::Floating { .. })));
    // OpticStudio の保存（MEMA あり）と今の書き出しは当たらない
    let a0 = root().join("tests/refs/A0/system.zmx");
    let raw = zmx::parse_text(&lensbench_core::glass::read_text(&a0).unwrap()).unwrap();
    assert!(!raw.legacy_lensbench());
    let txt = lensbench_core::zmx_write::write(&load("A0", false)).unwrap();
    assert!(!zmx::parse_text(&txt).unwrap().legacy_lensbench());
}
