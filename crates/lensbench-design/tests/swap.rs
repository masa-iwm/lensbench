//! カタログレンズの差し替え探索

use lensbench_core::catalog::{LensCatalog, ZmxFileSource};
use lensbench_core::optim::{self, DefaultMerit, DlsOpts};
use lensbench_core::param::{Kind, Param};
use lensbench_core::sweep::RmsRef;
use lensbench_design::swap::{self, Filter, PartRef, Slot, SwapSearch, SwapSpec};
use lensbench_project::{expand, expand_gaps, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn cat() -> LensCatalog {
    LensCatalog::load(&[root().join("examples/lenses/LB")], &[&ZmxFileSource]).unwrap()
}

fn project() -> (Project, PathBuf) {
    let path = root().join("examples/projects/P18_swap_two.json");
    let p = Project::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    (p, path.parent().unwrap().to_path_buf())
}

fn env(base: PathBuf) -> Env<'static> {
    Env {
        base,
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

fn spec(slots: Vec<Slot>) -> SwapSpec {
    SwapSpec::from_json(r#"{"slots": [], "vars": ["gap.t=0.5:20", "bfl.t=1:200"], "iters": 20}"#)
        .map(|mut s| {
            s.slots = slots;
            s
        })
        .unwrap()
}

fn slot(block: &str) -> Slot {
    Slot {
        block: block.into(),
        parts: Vec::new(),
        filter: Filter::default(),
        flip: false,
    }
}

#[test]
fn air_gaps_map_to_surface_thickness() {
    let (p, dir) = project();
    let c = cat();
    let look = |v: &str, q: &str| c.zmx(v, q);
    let e = Env {
        catalog: Some(&look),
        ..env(dir)
    };
    let (s, gaps) = expand_gaps(&p, &e).unwrap();
    let ids: Vec<&str> = gaps.iter().map(|g| g.0.as_str()).collect();
    assert_eq!(ids, ["gap", "bfl"]);
    // 面 1 は絞り、2〜3 が PCX、4〜6 が ACH（貼り合わせ）。gap は面 3 の後、bfl は最後の面の後
    assert_eq!(gaps[0].1, 3);
    assert_eq!(s.surfaces[3].t, 3.0);
    assert_eq!(gaps[1].1, s.image() - 1);
    assert_eq!(s.surfaces[s.image() - 1].t, 30.0);
    assert!(gaps.iter().all(|g| g.2 == 1.0));
    assert_eq!(
        format!("{:?}", expand(&p, &e).unwrap().surfaces),
        format!("{:?}", s.surfaces)
    );
}

#[test]
fn candidates_are_filtered_by_efl_and_diameter() {
    let c = cat();
    let g = vec![root().join("tests/glass")];
    let all = swap::candidates(&c, &Filter::default(), &g);
    assert_eq!(all.len(), 5);
    let pos = swap::candidates(
        &c,
        &Filter {
            efl: [Some(40.0), Some(120.0)],
            ..Default::default()
        },
        &g,
    );
    let names: Vec<&str> = pos.iter().map(|l| l.part.as_str()).collect();
    assert!(
        pos.iter().all(|l| l.efl >= 40.0 && l.efl <= 120.0),
        "{pos:?}"
    );
    assert!(names.contains(&"PCX-100") && names.contains(&"ACH-45") && !names.contains(&"BCC-32"));
    let d0 = pos[0].diameter;
    let narrow = swap::candidates(
        &c,
        &Filter {
            diameter: [Some(d0), Some(d0)],
            vendor: Some("lb".into()),
            ..Default::default()
        },
        &g,
    );
    assert!(narrow.iter().all(|l| l.diameter == d0) && !narrow.is_empty());
    let none = swap::candidates(
        &c,
        &Filter {
            vendor: Some("XX".into()),
            ..Default::default()
        },
        &g,
    );
    assert!(none.is_empty());
}

#[test]
fn candidate_summaries_are_shared_between_slots_and_independent_of_threads() {
    use lensbench_core::par;
    let (p, dir) = project();
    let c = cat();
    let g = vec![root().join("tests/glass")];
    let f = Filter {
        efl: [Some(40.0), None],
        ..Default::default()
    };
    let one = par::with_threads(Some(1), || swap::candidates(&c, &f, &g));
    let four = par::with_threads(Some(4), || swap::candidates(&c, &f, &g));
    assert_eq!(one, four);
    assert!(!one.is_empty());
    // 所ごとの候補は、所ごとに求めた候補と同じ（絞り込みの違う所・型番で選ぶ所が混ざっても）
    let mut a = slot("A");
    a.filter = f.clone();
    a.flip = true;
    let mut b = slot("B");
    b.filter.diameter = [Some(one[0].diameter), Some(one[0].diameter)];
    let fb = b.filter.clone();
    let mk = |t| {
        par::with_threads(Some(t), || {
            SwapSearch::new(&p, &env(dir.clone()), &c, spec(vec![a.clone(), b.clone()])).unwrap()
        })
    };
    let s1 = mk(1);
    let s4 = mk(4);
    assert_eq!(s1.picks, s4.picks);
    let la: Vec<_> = s1.picks[0]
        .iter()
        .step_by(2)
        .map(|k| k.lens.clone())
        .collect();
    assert_eq!(la, one);
    let lb: Vec<_> = s1.picks[1].iter().map(|k| k.lens.clone()).collect();
    assert_eq!(lb, swap::candidates(&c, &fb, &g));
}

#[test]
fn search_ranks_combinations_and_matches_a_direct_optimization() {
    let (p, dir) = project();
    let c = cat();
    let mut a = slot("A");
    a.filter.efl = [Some(40.0), None];
    a.flip = true;
    let mut b = slot("B");
    b.parts = vec![
        PartRef {
            vendor: "LB".into(),
            part: "ACH-45".into(),
        },
        PartRef {
            vendor: "LB".into(),
            part: "BCX-50".into(),
        },
    ];
    let s = SwapSearch::new(&p, &env(dir.clone()), &c, spec(vec![a, b])).unwrap();
    let na = s.picks[0].len();
    assert_eq!(na % 2, 0);
    assert_eq!(s.count(), na * 2);
    // 組の番号: 最後の所が速く回る
    assert_eq!(s.combo(0), [0, 0]);
    assert_eq!(s.combo(1), [0, 1]);
    assert_eq!(s.combo(2), [1, 0]);
    let r = vals(s.run(None));
    assert_eq!(r.len(), s.count());
    let m: Vec<f64> = r.iter().filter_map(|x| x["merit"].as_f64()).collect();
    assert!(m.len() >= s.count() / 2, "{r:?}");
    assert!(m.windows(2).all(|w| w[0] <= w[1]));
    // 最適化で評価関数は下がり、範囲を守る
    for x in &r {
        if let Some(mm) = x["merit"].as_f64() {
            assert!(mm <= x["merit0"].as_f64().unwrap());
            let v = &x["vars"];
            assert_eq!(v[0]["name"], "gap.t");
            let g = v[0]["x"].as_f64().unwrap();
            assert!((0.5..=20.0).contains(&g));
        }
    }
    // スレッド数によらず同じ
    let r1 = vals(s.run(Some(1)));
    assert_eq!(r, r1);
    // 先頭の組を、差し替えたプロジェクトを直接最適化したものと比べる
    let best = &r[0];
    let i = best["i"].as_u64().unwrap() as usize;
    let q = s.project(i).unwrap();
    let look = |v: &str, pt: &str| c.zmx(v, pt);
    let e = Env {
        catalog: Some(&look),
        ..env(dir)
    };
    let (sys, gaps) = expand_gaps(&q, &e).unwrap();
    assert_eq!(gaps[1].1, sys.image() - 1);
    let mut pr = optim::Problem::single(sys.clone());
    pr.vars
        .push(optim::Var::new(Param::new(gaps[0].1, Kind::Thick)).bounds(0.5, 20.0));
    pr.vars
        .push(optim::Var::new(Param::new(gaps[1].1, Kind::Thick)).bounds(1.0, 200.0));
    pr.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    // 差し替え探索の既定のヤコビアン（jac "forward"）と同じ
    pr.jac = optim::JacMode::Forward;
    let d = optim::dls(
        &pr,
        &DlsOpts {
            max_iter: 20,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(best["merit"].as_f64().unwrap(), d.merit);
    for (k, l) in best["lenses"].as_array().unwrap().iter().enumerate() {
        let pk = &s.picks[k][s.combo(i)[k]];
        assert_eq!(l["part"], pk.lens.part.as_str());
        assert_eq!(l["reversed"], pk.reversed);
    }
}

#[test]
fn prescreen_keeps_the_best_coarse_combinations_with_the_same_results() {
    let (p, dir) = project();
    let c = cat();
    let mut a = slot("A");
    a.filter.efl = [Some(40.0), None];
    a.flip = true;
    let mut b = slot("B");
    b.filter.efl = [Some(40.0), None];
    let full =
        SwapSearch::new(&p, &env(dir.clone()), &c, spec(vec![a.clone(), b.clone()])).unwrap();
    // ふるいなしなら頭に prescreen を書かない（前に書いた JSONL の続きから回せる）
    assert!(!full.header().contains("prescreen"));
    let n = full.count();
    assert!(n >= 8, "{n}");
    let keep = n / 2;
    let mut sp = spec(vec![a, b]);
    sp.prescreen = Some(lensbench_design::swap::Prescreen { rings: 2, keep });
    let s = SwapSearch::new(&p, &env(dir), &c, sp).unwrap();
    let idx = s.screened(None);
    assert!(idx.len() <= keep && !idx.is_empty());
    assert!(idx.windows(2).all(|w| w[0] < w[1]));
    // スレッド数によらない
    assert_eq!(idx, s.screened(Some(1)));
    // 残った組の結果は、ふるいなしの同じ組の結果と同じ
    let r = vals(s.run(None));
    assert_eq!(r.len(), idx.len());
    let rf = vals(full.run(None));
    for x in &r {
        let y = rf.iter().find(|y| y["i"] == x["i"]).unwrap();
        assert_eq!(x, y);
    }
    // ふるいなしの最良の組は残る
    assert_eq!(r[0], rf[0]);
}

#[test]
fn efl_target() {
    let (p, dir) = project();
    let c = cat();
    let mut a = slot("A");
    a.filter.efl = [Some(40.0), None];
    let mut sp = spec(vec![a]);
    sp.vars = vec!["gap.t=0.5:40".into(), "bfl.t=1:200".into()];
    sp.efl = Some(40.0);
    sp.iters = 40;
    let s = SwapSearch::new(&p, &env(dir), &c, sp).unwrap();
    let r = vals(s.run(None));
    let best = &r[0];
    let efl = best["efl"].as_f64().unwrap();
    assert!((efl - 40.0).abs() < 0.05, "{best}");
    // 像だけの評価関数も出す（目標がなければ merit と同じ）
    assert!(best["image_merit"].as_f64().unwrap() > 0.0);
    let s0 = SwapSearch::new(
        &p,
        &env(root().join("examples/projects")),
        &c,
        spec(vec![slot("A")]),
    )
    .unwrap();
    let x = s0.eval(0).to_value();
    assert_eq!(x["image_merit"], x["merit"]);
}

#[test]
fn run_to_file_and_resume() {
    let (p, dir) = project();
    let c = cat();
    let mut a = slot("A");
    a.filter.efl = [Some(40.0), None];
    let s = SwapSearch::new(&p, &env(dir.clone()), &c, spec(vec![a])).unwrap();
    let mem = vals(s.run(None));
    let tmp = std::env::temp_dir().join(format!("lb-swap-run-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let f = tmp.join("r.jsonl");
    let _ = std::fs::remove_file(&f);
    let r = s.run_to(Some(&f), true, None, None).map(vals).unwrap();
    assert_eq!(r, mem);
    // 最後の行を途中で切り、1 行消して再開すると、残りだけ回して同じ結果
    let txt = std::fs::read_to_string(&f).unwrap();
    let lines: Vec<&str> = txt.lines().collect();
    assert_eq!(lines.len(), 1 + s.count());
    let keep = lines[..lines.len() - 1].join("\n") + "\n" + &lines[lines.len() - 1][..10];
    std::fs::write(&f, keep).unwrap();
    let ran = std::sync::atomic::AtomicUsize::new(0);
    let cb = |pr: lensbench_core::runs::Progress| {
        ran.store(pr.ran, std::sync::atomic::Ordering::Relaxed);
    };
    let r2 = s
        .run_to(Some(&f), true, Some(1), Some(&cb))
        .map(vals)
        .unwrap();
    assert_eq!(r2, mem);
    assert_eq!(ran.load(std::sync::atomic::Ordering::Relaxed), 1);
    // 設定が違えば頭が違い、再開できない（resume false なら書き直す）
    let mut b = slot("A");
    b.filter.efl = [Some(40.0), None];
    let mut sp = spec(vec![b]);
    sp.iters = 5;
    let s3 = SwapSearch::new(&p, &env(dir), &c, sp).unwrap();
    assert!(s3.run_to(Some(&f), true, None, None).is_err());
    let r3 = s3.run_to(Some(&f), false, None, None).map(vals).unwrap();
    assert_eq!(r3.len(), s3.count());
}

#[test]
fn errors() {
    let (p, dir) = project();
    let c = cat();
    let e = env(dir);
    // 無いブロック・玉でないブロック・候補なし・同じ所 2 回・多すぎる
    assert!(SwapSearch::new(&p, &e, &c, spec(vec![slot("X")])).is_err());
    assert!(SwapSearch::new(&p, &e, &c, spec(vec![slot("gap")])).is_err());
    let mut a = slot("A");
    a.filter.efl = [Some(1e4), None];
    assert!(SwapSearch::new(&p, &e, &c, spec(vec![a])).is_err());
    assert!(SwapSearch::new(&p, &e, &c, spec(vec![slot("A"), slot("A")])).is_err());
    let mut sp = spec(vec![slot("A"), slot("B")]);
    sp.max_count = 10;
    assert!(SwapSearch::new(&p, &e, &c, sp).is_err());
    assert!(SwapSpec::from_json(r#"{"slots": [], "unknown": 1}"#).is_err());
    // 変数の誤りは組の結果の error に
    let mut sp = spec(vec![slot("A")]);
    sp.vars = vec!["nothere.t".into()];
    let s = SwapSearch::new(&p, &e, &c, sp).unwrap();
    let r = s.eval(0).to_value();
    assert!(r["error"].as_str().is_some(), "{r}");
}

/// 評価関数に足す項（片側の条件 min・max）と出発点
#[test]
fn terms_with_bounds_and_starts() {
    let (p, dir) = project();
    let c = cat();
    let mut a = slot("A");
    a.filter.efl = [Some(40.0), None];
    let base = {
        let mut sp = spec(vec![a.clone()]);
        sp.vars = vec!["gap.t=0.5:40".into(), "bfl.t=1:200".into()];
        sp.iters = 40;
        sp
    };
    let run = |sp: SwapSpec, i: usize| {
        let s = SwapSearch::new(&p, &env(dir.clone()), &c, sp).unwrap();
        s.eval(i).to_value()
    };
    let free = run(base.clone(), 0);
    let efl0 = free["efl"].as_f64().unwrap();
    // 満たしている片側の条件は効かない（同じ解）
    let mut sp = base.clone();
    sp.terms = vec![
        serde_json::json!({"type": "paraxial", "q": "efl", "max": efl0 + 50.0, "weight": 1e4}),
    ];
    let r = run(sp, 0);
    assert!(
        (r["efl"].as_f64().unwrap() - efl0).abs() < 1e-6,
        "{r} {free}"
    );
    assert!(
        (r["image_merit"].as_f64().unwrap() - free["image_merit"].as_f64().unwrap()).abs() < 1e-9
    );
    // 破っている条件は境界まで押し戻す（目標の等号と違い、内側へは引かない）
    let mut sp = base.clone();
    sp.terms =
        vec![serde_json::json!({"type": "paraxial", "q": "efl", "min": efl0 + 3.0, "weight": 1e6})];
    let r = run(sp, 0);
    let e = r["efl"].as_f64().unwrap();
    assert!(e > efl0 + 3.0 - 1e-3 && e < efl0 + 3.0 + 0.05, "{r}");
    // 出発点: すべてから最適化して最も小さい解。結果に番号
    let mut sp = base.clone();
    sp.starts = vec![
        [("gap.t".to_string(), 0.5)].into_iter().collect(),
        [("gap.t".to_string(), 30.0)].into_iter().collect(),
    ];
    let r = run(sp.clone(), 0);
    let m = r["merit"].as_f64().unwrap();
    assert!(r["start"].as_u64().unwrap() < 2, "{r}");
    for st in &sp.starts {
        let mut one = sp.clone();
        one.starts = vec![st.clone()];
        let ri = run(one, 0);
        assert!(m <= ri["merit"].as_f64().unwrap() + 1e-12, "{r} {ri}");
    }
    // vars にない変数の出発点はエラー
    let mut bad = base.clone();
    bad.starts = vec![[("nope.t".to_string(), 1.0)].into_iter().collect()];
    assert!(run(bad, 0)["error"].as_str().unwrap().contains("nope.t"));
    // 目標のない項に min は書けない
    let mut bad = base;
    bad.terms = vec![serde_json::json!({"type": "spot", "min": 0.1})];
    assert!(
        SwapSearch::new(&p, &env(dir.clone()), &c, bad.clone()).is_err()
            || run(bad, 0)["error"].is_string()
    );
}

/// 状態をまたいだ探索: 状態ごとの変数と、状態ごとの結果
#[test]
fn states_with_per_state_vars() {
    let path = root().join("examples/projects/P17_zoom_three_groups.json");
    let p = Project::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let dir = path.parent().unwrap().to_path_buf();
    let c = cat();
    let mut g1 = slot("G1");
    g1.filter.efl = [Some(30.0), Some(200.0)];
    let mut sp = spec(vec![g1]);
    sp.vars = vec![];
    sp.states = vec!["wide".into(), "tele".into()];
    sp.state_vars = vec!["G2.shift=-10:30".into(), "G3.shift=-10:30".into()];
    sp.iters = 20;
    let s = SwapSearch::new(&p, &env(dir.clone()), &c, sp.clone()).unwrap();
    assert!(s.count() > 0);
    let r = s.eval(0).to_value();
    assert!(r.get("error").is_none(), "{r}");
    let vars = r["vars"].as_array().unwrap();
    assert_eq!(vars.len(), 4, "{r}");
    assert_eq!(vars[0]["state"], "wide");
    assert_eq!(vars[2]["state"], "tele");
    let st = r["states"].as_array().unwrap();
    assert_eq!(st.len(), 2);
    // 全体の像の評価関数は状態ごとの値から（項の重みが同じなら二乗平均）
    let m: Vec<f64> = st
        .iter()
        .map(|x| x["image_merit"].as_f64().unwrap())
        .collect();
    let all = r["image_merit"].as_f64().unwrap();
    assert!(
        (all - ((m[0] * m[0] + m[1] * m[1]) / 2.0).sqrt()).abs() < 1e-9 * all.max(1.0),
        "{r}"
    );
    // state と states は同時に使えない、state_vars だけも不可
    let mut bad = sp.clone();
    bad.state = Some("wide".into());
    assert!(SwapSearch::new(&p, &env(dir.clone()), &c, bad).is_err());
    let mut bad = sp;
    bad.states.clear();
    assert!(SwapSearch::new(&p, &env(dir), &c, bad).is_err());
}

/// 有効径の条件: 差し替えた玉の面で瞳の縁の光線が半径の内側
#[test]
fn clear_aperture_terms_keep_rays_inside() {
    let (p, dir) = project();
    let c = cat();
    let mut sp = spec(vec![slot("A")]);
    sp.clear_aperture = true;
    let s = SwapSearch::new(&p, &env(dir), &c, sp).unwrap();
    for i in 0..s.count() {
        let r = s.eval(i).to_value();
        if r.get("error").is_some() {
            continue;
        }
        let (sys, pr) = s.problem(i).unwrap();
        // 有効径の項が足されている（視野 3 × 面 × 4 本）
        assert!(pr.terms.len() > 3, "{}", pr.terms.len());
        let u = sys.units.iter().find(|u| u.name == "A").unwrap();
        let x: Vec<f64> = r["vars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["x"].as_f64().unwrap())
            .collect();
        let fin = pr.systems(&x).unwrap().remove(0);
        for j in u.spans[0][0]..=u.spans[0][1] {
            let semi = fin.surfaces[j].semi;
            for f in 0..fin.fields.len() {
                let t = fin
                    .try_trace(
                        lensbench_core::FieldSel::Index(f),
                        &[0.0, 0.0],
                        &[1.0, -1.0],
                        None,
                        Some(j),
                        false,
                    )
                    .unwrap();
                for ray in &t.rays {
                    if ray.err == 0 {
                        assert!(
                            ray.xyz[1].abs() <= semi * (1.0 + 1e-3),
                            "面 {j} 視野 {f}: {} > {semi}",
                            ray.xyz[1]
                        );
                    }
                }
            }
        }
    }
}

/// 有効径の条件は、半直径を固定したカタログの玉（浮動開口でける）でも評価できる（けられた光線も面までの位置は出る）
#[test]
fn clear_aperture_with_fixed_semi_diameters() {
    let tmp = std::env::temp_dir().join(format!("lensbench-swap-fixed-{}", std::process::id()));
    let dir = tmp.join("LB");
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(root().join("examples/lenses/LB"))
        .unwrap()
        .flatten()
    {
        let txt = lensbench_core::glass::read_text(&e.path()).unwrap();
        // DIAM 値 0 … → DIAM 値 1 …（固定）
        let fixed: Vec<String> = txt
            .lines()
            .map(|l| {
                let t: Vec<&str> = l.split_whitespace().collect();
                if t.first() == Some(&"DIAM")
                    && t.get(1).is_some_and(|v| *v != "0")
                    && t.get(2) == Some(&"0")
                {
                    format!("  DIAM {} 1 0 0 1 \"\"", t[1])
                } else {
                    l.to_string()
                }
            })
            .collect();
        std::fs::write(dir.join(e.file_name()), fixed.join("\r\n")).unwrap();
    }
    let c = LensCatalog::load(std::slice::from_ref(&dir), &[&ZmxFileSource]).unwrap();
    let (p, pdir) = project();
    let mut sp = spec(vec![slot("A")]);
    sp.clear_aperture = true;
    let s = SwapSearch::new(&p, &env(pdir), &c, sp).unwrap();
    let mut ok = 0;
    for i in 0..s.count() {
        let r = s.eval(i).to_value();
        if r.get("error").is_none() {
            ok += 1;
            let (sys, pr) = s.problem(i).unwrap();
            let x: Vec<f64> = r["vars"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["x"].as_f64().unwrap())
                .collect();
            let fin = pr.systems(&x).unwrap().remove(0);
            let u = sys.units.iter().find(|u| u.name == "A").unwrap();
            for j in u.spans[0][0]..=u.spans[0][1] {
                // 半直径を固定した面は浮動開口
                assert!(matches!(
                    fin.surfaces[j].ap,
                    lensbench_core::surface::Aperture::Floating { .. }
                ));
                let semi = fin.surfaces[j].semi;
                for f in 0..fin.fields.len() {
                    let t = fin
                        .try_trace(
                            lensbench_core::FieldSel::Index(f),
                            &[0.0, 0.0],
                            &[1.0, -1.0],
                            None,
                            Some(j),
                            false,
                        )
                        .unwrap();
                    for ray in &t.rays {
                        if ray.err == 0 {
                            assert!(
                                ray.xyz[1].abs() <= semi * (1.0 + 1e-3),
                                "面 {j}: {} > {semi}",
                                ray.xyz[1]
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(ok > 0);
    std::fs::remove_dir_all(&tmp).ok();
}

/// 結果を JSON の値に（試験は鍵で見る）
fn vals(v: Vec<lensbench_design::swap::SwapResult>) -> Vec<serde_json::Value> {
    v.iter().map(|x| x.to_value()).collect()
}
