//! ガラスの置き換え: モデルガラスで最適化した色消しのトリプレットが実在の硝材の組（凸に低分散・凹に高分散）になること、
//! dn・disp を変数にした硝材から寄せる経路、実在の硝材を入れ替えて比べる経路、設定の誤り。

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
use lensbench_analysis::sweep::RmsRef;
use lensbench_core::glass::load_catalogs;
use lensbench_core::material::Material;
use lensbench_core::System;
use lensbench_design::glass_sub::{effective_nd_vd, substitute, Closeness, GlassSubOpts};
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
        filter: Default::default(),
        closeness: Default::default(),
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

/// 候補の表の硝材 name に相対価格・耐性・状態を付ける
fn tag(o: &mut GlassSubOpts, name: &str, cost: f64, cr: f64, status: u8) {
    let mut g = (*o.glasses).clone();
    let x = g.get_mut(name).unwrap();
    let mut pr = x.props.unwrap_or_default();
    pr.cost = cost;
    pr.resist = Some([Some(cr), None, None, None, None]);
    x.props = Some(pr);
    x.status = Some(status);
    o.glasses = Arc::new(g);
}

/// モデルガラスで最適化したトリプレット（凹の面 3 だけをモデルガラスに）
fn model_problem() -> (Problem, Vec<f64>) {
    let mut s = load("A0");
    s.set_material(3, "1.60,38").unwrap();
    let p = problem(s);
    let m = dls(&p, &DlsOpts::default()).unwrap();
    let pm = p.with_configs(m.systems.clone());
    let x = pm.x0().unwrap();
    (pm, x)
}

#[test]
fn filter_excludes_names_status_cost_and_resistance() {
    let (pm, x) = model_problem();
    let mut o = opts(vec![3], &["SCHOTT"]);
    o.candidates = 1;
    let first = substitute(&pm, &x, &o).unwrap().changes[0].to.clone();
    // 名前で除くと別の硝材になる（大文字小文字は問わない）
    let mut f = o.clone();
    f.filter.exclude = vec![first.to_lowercase()];
    let second = substitute(&pm, &x, &f).unwrap().changes[0].to.clone();
    assert_ne!(first, second);
    // 名前@カタログ でも除ける。別のカタログ名なら除かない
    f.filter.exclude = vec![format!("{first}@schott")];
    assert_eq!(substitute(&pm, &x, &f).unwrap().changes[0].to, second);
    f.filter.exclude = vec![format!("{first}@OHARA")];
    assert_eq!(substitute(&pm, &x, &f).unwrap().changes[0].to, first);
    // 状態: 廃止にした硝材は「標準・推奨」だけなら選ばない。状態の無い硝材は除かない
    let mut g = o.clone();
    tag(&mut g, &first, 1.0, 1.0, 2);
    g.filter.status = Some(vec![0, 1]);
    assert_eq!(substitute(&pm, &x, &g).unwrap().changes[0].to, second);
    g.filter.status = Some(vec![2]);
    assert_eq!(substitute(&pm, &x, &g).unwrap().changes[0].to, first);
    // 相対価格・耐候性の上限
    let mut h = o.clone();
    tag(&mut h, &first, 5.0, 3.0, 0);
    h.filter.max_cost = Some(4.0);
    assert_eq!(substitute(&pm, &x, &h).unwrap().changes[0].to, second);
    h.filter.max_cost = Some(5.0);
    assert_eq!(substitute(&pm, &x, &h).unwrap().changes[0].to, first);
    h.filter.max_resist[0] = Some(2.0);
    assert_eq!(substitute(&pm, &x, &h).unwrap().changes[0].to, second);
    // カタログの置き換えから除く印
    let mut k = o.clone();
    let mut gl = (*k.glasses).clone();
    gl.get_mut(&first).unwrap().no_substitute = true;
    k.glasses = Arc::new(gl);
    assert_eq!(substitute(&pm, &x, &k).unwrap().changes[0].to, second);
    // 何も合わなければ誤り
    let mut e = o.clone();
    e.filter.max_cost = Some(0.5);
    e.glasses = Arc::new(
        e.glasses
            .iter()
            .map(|(n, g)| {
                let mut g = g.clone();
                g.props = Some(lensbench_core::glass::GlassProps {
                    cost: 1.0,
                    ..g.props.unwrap_or_default()
                });
                (n.clone(), g)
            })
            .collect(),
    );
    assert!(substitute(&pm, &x, &e).is_err());
}

#[test]
fn glass_failing_the_filter_is_always_replaced() {
    // 実在の硝材の面は良くなるときだけ入れ替えるが、絞り込みに合わない硝材（ここでは除く一覧の F2）は必ず置き換える
    let s = load("A0");
    let p = problem(s);
    let m = dls(&p, &DlsOpts::default()).unwrap();
    let pm = p.with_configs(m.systems.clone());
    let x = pm.x0().unwrap();
    let mut o = opts(vec![3], &["SCHOTT"]);
    o.candidates = 3;
    o.filter.exclude = vec!["F2".into()];
    let r = substitute(&pm, &x, &o).unwrap();
    assert_eq!(r.changes[0].from, "F2");
    assert_ne!(glass_name(&r.problem.configs[0], 3), "F2");
}

#[test]
fn catalog_no_substitute_mark_does_not_force_replacement() {
    // カタログの置き換えから除く印は「候補にしない」だけ。今の硝材 F2 に印があっても、利用者の絞り込みに合っていれば
    // 強制の置き換えにはならず、良くなるときだけ入れ替える
    let s = load("A0");
    let p = problem(s);
    let m = dls(&p, &DlsOpts::default()).unwrap();
    // 印は系の表（今の硝材を見る）と候補の表の両方の F2 に付ける
    let systems: Vec<_> = m
        .systems
        .iter()
        .map(|s| {
            let mut s = s.clone();
            let mut f2 = s.glasses["F2"].clone();
            f2.no_substitute = true;
            s.add_glasses([("F2".to_string(), f2)].into());
            s
        })
        .collect();
    let pm = p.with_configs(systems);
    let x = pm.x0().unwrap();
    let mut o = opts(vec![3], &["SCHOTT"]);
    o.candidates = 3;
    let mut gl = (*o.glasses).clone();
    gl.get_mut("F2").unwrap().no_substitute = true;
    o.glasses = Arc::new(gl);
    // 候補を F2 と（凹の玉には明らかに悪い）N-BK7 に絞る。F2 は絞り込みに合うので、印があっても強制の置き換えにならず残る
    let mut f = o.clone();
    f.filter.exclude = o
        .glasses
        .keys()
        .filter(|k| *k != "F2" && *k != "N-BK7")
        .cloned()
        .collect();
    let r = substitute(&pm, &x, &f).unwrap();
    assert_eq!(glass_name(&r.problem.configs[0], 3), "F2");
    assert!(
        r.changes.is_empty(),
        "{:?}",
        r.changes.iter().map(|c| &c.to).collect::<Vec<_>>()
    );
}

#[test]
fn closeness_steps_and_dpgf() {
    let (pm, x) = model_problem();
    let mut o = opts(vec![3], &["SCHOTT"]);
    o.candidates = 1;
    let a = substitute(&pm, &x, &o).unwrap().changes[0].clone();
    // vd をほとんど見ない刻みなら nd の最も近い硝材
    o.closeness = Closeness {
        nd_step: 0.01,
        vd_step: 1e6,
        dpgf_step: None,
    };
    let b = substitute(&pm, &x, &o).unwrap().changes[0].clone();
    assert!((b.nd - b.target_nd).abs() <= (a.nd - a.target_nd).abs());
    // 刻みの誤り
    o.closeness.nd_step = 0.0;
    assert!(substitute(&pm, &x, &o).is_err());
    o.closeness = Closeness {
        dpgf_step: Some(-1.0),
        ..Default::default()
    };
    assert!(substitute(&pm, &x, &o).is_err());
    // ΔPgF の刻みは候補の ΔPgF を見る: 最初に選んだ硝材だけ ΔPgF を大きくすると、細かい刻みでは選ばれない
    let mut gl = (*o.glasses).clone();
    let g = gl.get_mut(&a.to).unwrap();
    g.props = Some(lensbench_core::glass::GlassProps {
        dpgf: 0.05,
        ..g.props.unwrap_or_default()
    });
    o.glasses = Arc::new(gl);
    o.closeness = Closeness::default();
    assert_eq!(substitute(&pm, &x, &o).unwrap().changes[0].to, a.to);
    o.closeness.dpgf_step = Some(0.001);
    assert_ne!(substitute(&pm, &x, &o).unwrap().changes[0].to, a.to);
}

#[test]
fn configs_with_different_materials_are_substituted_separately() {
    // 構成 2 だけ面 3 をモデルガラス（多重構成の材料の行に当たる）。構成 1 の F2 は良くなるときだけ入れ替え、
    // 構成 2 のモデルガラスは必ず置き換える。それぞれの組の構成だけに入る
    let s1 = load("A0");
    let mut s2 = s1.clone();
    s2.set_material(3, "1.70,30").unwrap();
    let mut p = Problem::new(vec![s1, s2]);
    p.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    for n in ["1.c", "2.c", "6.c", "7.c"] {
        p.add_var(n).unwrap();
    }
    let x = p.x0().unwrap();
    let mut o = opts(vec![3], &["SCHOTT"]);
    o.candidates = 2;
    o.passes = 1;
    let r = substitute(&p, &x, &o).unwrap();
    let c2: Vec<_> = r
        .changes
        .iter()
        .filter(|c| c.configs == Some(vec![1]))
        .collect();
    assert_eq!(c2.len(), 1);
    assert_eq!(c2[0].from, "1.7,30");
    let out = &r.problem.configs;
    assert_eq!(glass_name(&out[1], 3), c2[0].to);
    // 構成 1 は F2 のままか、構成 1 だけの置き換えの硝材
    let m1 = glass_name(&out[0], 3);
    match r.changes.iter().find(|c| c.configs == Some(vec![0])) {
        Some(c) => assert_eq!(m1, c.to),
        None => assert_eq!(m1, "F2"),
    }
}
