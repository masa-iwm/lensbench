//! ガラスの置き換え: モデルガラスで最適化した色消しのトリプレットが実在の硝材の組（凸に低分散・凹に高分散）になること、
//! dn・disp を変数にした硝材から寄せる経路、実在の硝材を入れ替えて比べる経路、設定の誤り。

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
use lensbench_analysis::sweep::RmsRef;
use lensbench_core::glass::load_catalogs;
use lensbench_core::material::Material;
use lensbench_core::System;
use lensbench_design::glass_sub::{effective_nd_vd, substitute, GlassSubOpts};
use lensbench_formats::zmx::{self, LoadOptions};
use lensbench_optim::{dls, DefaultMerit, DlsOpts, Problem};
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

/// トリプレット（A0）の曲率 6 つを変数、既定の RMS スポットを評価関数にした問題
fn problem(s: System) -> Problem {
    let mut p = Problem::single(s);
    p.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    for n in ["1.c", "2.c", "3.c", "4.c", "6.c", "7.c"] {
        p.add_var(n).unwrap();
    }
    p
}

fn opts(surfaces: Vec<usize>, catalogs: &[&str]) -> GlassSubOpts {
    let names: Vec<String> = catalogs.iter().map(|c| c.to_string()).collect();
    // 同梱の全カタログ（試験の tests/glass は数種だけ）
    let (g, missing) = load_catalogs(&names, &[]);
    assert!(missing.is_empty());
    GlassSubOpts {
        surfaces,
        glasses: Arc::new(g),
        candidates: 8,
        passes: 2,
        local: DlsOpts::default(),
    }
}

fn glass_name(s: &System, j: usize) -> String {
    match &s.surfaces[j].mat {
        Material::Glass(m) => m.clone(),
        m => panic!("surface {j}: {m:?} is not a catalog glass"),
    }
}

#[test]
fn model_glass_triplet_becomes_real_glasses() {
    // 硝材をモデルガラスに替える（凸 2 枚は低分散、凹 1 枚は高分散。どれも実在の硝材からずらした値）
    let mut s = load("A0");
    s.set_material(1, "1.64,58").unwrap();
    s.set_material(3, "1.60,38").unwrap();
    s.set_material(6, "1.64,58").unwrap();
    let p = problem(s);
    let m = dls(&p, &DlsOpts::default()).unwrap();
    let pm = p.with_configs(m.systems.clone());
    let x = pm.x0().unwrap();
    let r = substitute(&pm, &x, &opts(vec![1, 3, 6], &["SCHOTT"])).unwrap();
    let out = &r.problem.configs[0];
    let schott = opts(vec![], &["SCHOTT"]).glasses;
    for j in [1, 3, 6] {
        let g = glass_name(out, j);
        let base = lensbench_core::glass::base_name(&g);
        assert!(schott.contains_key(base), "{g}");
        assert_eq!(out.glasses[&g], schott[base]);
        assert_eq!(out.surfaces[j].dn, 0.0);
    }
    // 凸は低分散（クラウン）、凹は高分散（フリント）
    let (_, vd1) = effective_nd_vd(out, 1).unwrap();
    let (_, vd3) = effective_nd_vd(out, 3).unwrap();
    let (_, vd6) = effective_nd_vd(out, 6).unwrap();
    assert!(vd1 > 50.0 && vd6 > 50.0 && vd3 < 45.0, "{vd1} {vd3} {vd6}");
    // 置き換えの記録: 最初の周で 3 面ともモデルガラスから
    let first: Vec<usize> = r.changes.iter().take(3).map(|c| c.surface).collect();
    assert_eq!(first, vec![1, 3, 6]);
    assert!(r.changes[..3].iter().all(|c| c.from.contains(',')));
    for c in &r.changes {
        assert!(
            (c.nd - c.target_nd).abs() < 0.1 && (c.vd - c.target_vd).abs() < 15.0,
            "{c:?}"
        );
    }
    // 実在の硝材でも、モデルガラスの性能に近い（色消しが保たれる）
    assert!(
        r.result.merit < 1.5 * m.merit,
        "{} vs model {}",
        r.result.merit,
        m.merit
    );
    assert!(r.result.merit.is_finite());
    assert!(r.tried >= 3 * 8);
    // 結果の系は最適化した値が入っている（問題の今の値 = 結果の x）
    let xr = r.problem.x0().unwrap();
    for (a, b) in xr.iter().zip(&r.result.x) {
        assert!((a - b).abs() < 1e-12);
    }
}

#[test]
fn index_offset_variables_snap_to_a_real_glass() {
    // 凹の硝材 F2 に dn・disp を変数にして最適化してから、近い実在の硝材へ寄せる
    let s = load("A0");
    let mut p = problem(s);
    p.add_var("3.dn").unwrap().lo = -0.05;
    p.vars.last_mut().unwrap().hi = 0.05;
    p.add_var("3.disp").unwrap().lo = -0.2;
    p.vars.last_mut().unwrap().hi = 0.2;
    let m = dls(&p, &DlsOpts::default()).unwrap();
    let pm = p.with_configs(m.systems.clone());
    let x = pm.x0().unwrap();
    let r = substitute(&pm, &x, &opts(vec![3], &["SCHOTT"])).unwrap();
    let out = &r.problem.configs[0];
    glass_name(out, 3);
    assert_eq!(out.surfaces[3].dn, 0.0);
    assert_eq!(out.surfaces[3].disp, 0.0);
    // 外した dn・disp の変数は残らない
    assert_eq!(r.problem.vars.len(), 6);
    assert_eq!(r.changes[0].surface, 3);
    assert!(r.changes[0].from == "F2");
}

#[test]
fn real_glasses_are_swapped_only_when_better() {
    let s = load("A0");
    let p = problem(s);
    let m = dls(&p, &DlsOpts::default()).unwrap();
    let pm = p.with_configs(m.systems.clone());
    let x = pm.x0().unwrap();
    let mut o = opts(vec![1, 3, 6], &["SCHOTT", "OHARA"]);
    o.candidates = 4;
    let r = substitute(&pm, &x, &o).unwrap();
    // 入れ替えたなら評価関数は下がり、入れ替えなければ元のまま
    assert!(
        r.result.merit <= m.merit * (1.0 + 1e-9),
        "{} {}",
        r.result.merit,
        m.merit
    );
    for w in r.changes.windows(2) {
        assert!(w[1].merit < w[0].merit);
    }
    if let Some(c) = r.changes.first() {
        assert!(c.merit < m.merit);
    }
    assert!(r.passes <= 2);
}

#[test]
fn glass_sub_checks() {
    let s = load("A0");
    let p = problem(s);
    let x = p.x0().unwrap();
    // 空気・像面・面の重複・候補の数 0
    for (surfaces, cands) in [
        (vec![2], 4),
        (vec![9], 4),
        (vec![1, 1], 4),
        (vec![], 4),
        (vec![1], 0),
    ] {
        let mut o = opts(surfaces, &["SCHOTT"]);
        o.candidates = cands;
        assert!(substitute(&p, &x, &o).is_err());
    }
    // 候補の表: カタログが見つからない
    assert!(lensbench_design::glass_sub::candidate_pool(
        &p.configs[0],
        &["NO_SUCH_CAT".into()],
        &[]
    )
    .is_err());
    // カタログを省くと系の読み込んだガラス
    let g = lensbench_design::glass_sub::candidate_pool(&p.configs[0], &[], &[]).unwrap();
    assert!(g.contains_key("F2"));
}
