//! モンテカルロ公差: 乱数・分布・統計、スレッド数によらない結果、補償、簡易設定。

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
use lensbench_core::optim::{DefaultMerit, Problem, Var};
use lensbench_core::param::{Kind, Param};
use lensbench_core::sweep::RmsRef;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use lensbench_design::montecarlo::{self as tolerance, Dist, LensTol, McSpec, Rng, Tol};

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

fn problem(s: &System) -> Problem {
    let mut p = Problem::single(s.clone());
    p.add_default(DefaultMerit::Wavefront, 3, RmsRef::Centroid);
    p
}

#[test]
fn distributions() {
    let n = 20000;
    let mut r = Rng::for_trial(7, 0);
    let u: Vec<f64> = (0..n).map(|_| r.draw(Dist::Uniform, -1.0, 3.0)).collect();
    let st = tolerance::stats(&u, &[50.0]);
    assert!(st.min >= -1.0 && st.max < 3.0);
    assert!((st.mean - 1.0).abs() < 0.03, "{}", st.mean);
    // 一様分布の標準偏差 = 幅 / √12
    assert!((st.std - 4.0 / 12f64.sqrt()).abs() < 0.02, "{}", st.std);
    let g: Vec<f64> = (0..n)
        .map(|_| r.draw(Dist::Normal { cut: 2.0 }, -2.0, 2.0))
        .collect();
    let st = tolerance::stats(&g, &[]);
    assert!(st.min >= -2.0 && st.max <= 2.0);
    assert!(st.mean.abs() < 0.03);
    // ±2σ で切った標準正規の標準偏差は 0.880
    assert!((st.std - 0.880).abs() < 0.02, "{}", st.std);
    let e: Vec<f64> = (0..n).map(|_| r.draw(Dist::End, -0.5, 0.25)).collect();
    assert!(e.iter().all(|&x| x == -0.5 || x == 0.25));
    let lo = e.iter().filter(|&&x| x == -0.5).count() as f64 / n as f64;
    assert!((lo - 0.5).abs() < 0.02);
    // 同じ種・番号は同じ列、番号が違えば違う列
    assert_eq!(
        Rng::for_trial(3, 5).next_u64(),
        Rng::for_trial(3, 5).next_u64()
    );
    assert_ne!(
        Rng::for_trial(3, 5).next_u64(),
        Rng::for_trial(3, 6).next_u64()
    );
}

/// 隣の試行の列が 1 つずらした同じ列にならない（試行 i の k 番目と試行 i+1 の k−1 番目の相関が小さい。以前の
/// 作り方では値がそのまま一致していた）
#[test]
fn trials_independent() {
    let (trials, k) = (2000u64, 8usize);
    let seq = |i: u64| {
        let mut r = Rng::for_trial(1, i);
        (0..k).map(|_| r.uniform()).collect::<Vec<f64>>()
    };
    let all: Vec<Vec<f64>> = (0..trials).map(seq).collect();
    for shift in 0..2usize {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for i in 0..trials as usize - 1 {
            for j in shift..k {
                a.push(all[i][j]);
                b.push(all[i + 1][j - shift]);
            }
        }
        assert!(a.iter().zip(&b).all(|(x, y)| x != y));
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
        let cov: f64 = a.iter().zip(&b).map(|(x, y)| (x - ma) * (y - mb)).sum();
        let va: f64 = a.iter().map(|x| (x - ma).powi(2)).sum();
        let vb: f64 = b.iter().map(|y| (y - mb).powi(2)).sum();
        let c = cov / (va * vb).sqrt();
        assert!(c.abs() < 0.05, "shift {shift}: {c}");
    }
}

#[test]
fn stats_percentiles() {
    let v: Vec<f64> = (0..=10).map(|i| i as f64).collect();
    let st = tolerance::stats(&v, &[0.0, 50.0, 90.0, 95.0, 100.0]);
    assert_eq!(st.n, 11);
    assert_eq!(st.mean, 5.0);
    let p: Vec<f64> = st.percentiles.iter().map(|x| x.1).collect();
    assert_eq!(p, vec![0.0, 5.0, 9.0, 9.5, 10.0]);
}

#[test]
fn zero_tolerance_is_nominal() {
    let s = load("A0");
    let tols = vec![Tol::pm("0", Param::new(2, Kind::Thick), 0.0)];
    let mut spec = McSpec::new(problem(&s), tols);
    spec.trials = 4;
    let r = tolerance::run(&spec).unwrap();
    assert_eq!(r.trials.len(), 4);
    for t in &r.trials {
        assert_eq!(t.eval.as_ref().unwrap().merit, r.nominal.merit);
    }
    assert_eq!(r.sensitivity[0].worst(r.nominal.merit), 0.0);
}

#[test]
fn deterministic_across_threads() {
    let s = load("A0");
    let lt = LensTol::preset("commercial").unwrap();
    let tols = tolerance::simple(&s, &|_| Some(lt), &|_| None, Dist::Normal { cut: 2.0 });
    assert!(!tols.is_empty());
    let mut p = problem(&s);
    p.vars
        .push(Var::new(Param::new(s.image() - 1, Kind::Thick)));
    let mut spec = McSpec::new(p, tols);
    spec.trials = 6;
    spec.seed = 42;
    let a = lensbench_core::par::with_threads(Some(1), || tolerance::run(&spec).unwrap());
    let b = lensbench_core::par::with_threads(Some(4), || tolerance::run(&spec).unwrap());
    assert_eq!(a.nominal, b.nominal);
    assert_eq!(a.trials, b.trials);
    assert_eq!(a.sensitivity, b.sensitivity);
    // 公差があるので公称より悪くなる試行がある
    assert!(a.merits().iter().any(|&m| m > a.nominal.merit));
    // 種を変えると変わる
    spec.seed = 43;
    let c = tolerance::run(&spec).unwrap();
    assert_ne!(a.trials[0].deltas, c.trials[0].deltas);
}

#[test]
fn compensator_recovers_focus() {
    // ピントの間隔そのものの公差は、ピントの補償子で元に戻る
    let s = load("A0");
    let f = Param::new(s.image() - 1, Kind::Thick);
    let tols = vec![Tol::pm("focus", f, 0.2)];
    let p0 = problem(&s);
    let mut spec = McSpec::new(p0.clone(), tols.clone());
    spec.trials = 5;
    spec.sensitivity = false;
    let free = tolerance::run(&spec).unwrap();
    let mut p = p0;
    p.vars.push(Var::new(f));
    let mut spec = McSpec::new(p, tols);
    spec.trials = 5;
    spec.sensitivity = false;
    let comp = tolerance::run(&spec).unwrap();
    for (a, b) in free.trials.iter().zip(&comp.trials) {
        assert_eq!(a.deltas, b.deltas);
        let (ma, mb) = (
            a.eval.as_ref().unwrap().merit,
            b.eval.as_ref().unwrap().merit,
        );
        assert!(mb <= ma + 1e-12);
        assert!(
            (mb - comp.nominal.merit).abs() < 1e-3 * comp.nominal.merit.max(1e-3),
            "{mb} {}",
            comp.nominal.merit
        );
    }
}

#[test]
fn simple_set_and_relative() {
    let s = load("C0");
    let lt = LensTol {
        focal_rel: 0.01,
        decenter: 0.05,
        space: 0.1,
        ..Default::default()
    };
    let tols = tolerance::simple(&s, &|_| Some(lt), &|_| None, Dist::Uniform);
    let labels: Vec<&str> = tols.iter().map(|t| t.label.as_str()).collect();
    assert!(labels.contains(&"lens1 焦点距離"), "{labels:?}");
    assert!(labels.contains(&"lens1 偏心x"));
    assert!(labels.contains(&"lens1 後の間隔"));
    // 焦点距離の公差は玉の曲率半径をすべて同じ比で変える
    let k = labels.iter().position(|&l| l == "lens1 焦点距離").unwrap();
    let spec = McSpec::new(problem(&s), tols.clone());
    let mut d = vec![0.0; tols.len()];
    d[k] = 0.01;
    let ps = tolerance::perturbed(&spec, &d).unwrap();
    for &p in &tols[k].params {
        let (r0, r1) = (s.get_param(p).unwrap(), ps[0].get_param(p).unwrap());
        assert!((r1 - 1.01 * r0).abs() < 1e-9 * r0.abs(), "{r0} {r1}");
    }
}

#[test]
fn disc_pairs() {
    // 隣り合う x・y の組を円の中で一様に: 大きさは公差以下、x・y の中心まわりの |r|² の平均は半径² / 2
    let s = load("C0");
    let lt = LensTol::preset("commercial").unwrap();
    let mut tols = tolerance::simple(&s, &|_| Some(lt), &|_| None, Dist::Uniform);
    let n = tolerance::pair_disc(&mut tols, None);
    // 玉ごとにくさび・偏心・傾きの 3 組
    let lenses = tols.iter().filter(|t| t.label.ends_with("偏心x")).count();
    assert!(lenses > 0);
    assert_eq!(
        n,
        3 * lenses,
        "{:?}",
        tols.iter().map(|t| (&t.label, t.dist)).collect::<Vec<_>>()
    );
    let k = tols
        .iter()
        .position(|t| t.label.ends_with("偏心x"))
        .unwrap();
    assert_eq!(
        (tols[k].dist, tols[k + 1].dist),
        (Dist::DiscX { normal: None }, Dist::DiscY { normal: None })
    );
    // 曲率・厚さなどは組にしない
    assert!(tols
        .iter()
        .filter(|t| t.label.contains("厚さ"))
        .all(|t| t.dist == Dist::Uniform));
    let mut spec = McSpec::new(problem(&s), tols.clone());
    spec.trials = 0;
    let (a, b) = (tols[k].hi, tols[k + 1].hi);
    let mut r2 = 0.0;
    let m = 4000;
    let mut outside_square = 0;
    for i in 0..m {
        let d = tolerance::draw_trial(&spec, i);
        let (x, y) = (d[k] / a, d[k + 1] / b);
        assert!(x * x + y * y <= 1.0 + 1e-12);
        if x.abs() > 0.7 && y.abs() > 0.7 {
            outside_square += 1;
        }
        r2 += x * x + y * y;
    }
    assert_eq!(outside_square, 0);
    assert!((r2 / m as f64 - 0.5).abs() < 0.02, "{}", r2 / m as f64);
    // 同じ種なら同じ
    assert_eq!(
        tolerance::draw_trial(&spec, 3),
        tolerance::draw_trial(&spec, 3)
    );
    // 1 つだけ引く Rng::draw は周辺分布（範囲の中、平均 0）
    let mut rng = Rng::for_trial(1, 0);
    let v: Vec<f64> = (0..20000)
        .map(|_| rng.draw(Dist::DiscX { normal: None }, -2.0, 2.0))
        .collect();
    let st = tolerance::stats(&v, &[]);
    assert!(st.min >= -2.0 && st.max <= 2.0 && st.mean.abs() < 0.03);
    // 円の中の x の標準偏差は 半径 / 2
    assert!((st.std - 1.0).abs() < 0.02, "{}", st.std);
    // 組になっていないと誤り
    let mut bad = tols.clone();
    bad.swap(k, k + 1);
    let mut spec = McSpec::new(problem(&s), bad);
    spec.trials = 1;
    assert!(tolerance::run(&spec).is_err());
    let mut lone = vec![tols[k].clone()];
    lone[0].dist = Dist::DiscY { normal: None };
    let spec = McSpec::new(problem(&s), lone);
    assert!(tolerance::run(&spec).is_err());
    // x と y で分布が違う組も誤り
    let mut mixed = tols.clone();
    mixed[k + 1].dist = Dist::DiscY { normal: Some(2.0) };
    let mut spec = McSpec::new(problem(&s), mixed);
    spec.trials = 1;
    assert!(tolerance::run(&spec).is_err());
}

#[test]
fn disc_normal_pairs() {
    // 円の中の 2 次元正規分布: 大きさは公差以下、|r|/半径 ≤ 0.5（1σ、cut 2）の割合は (1 − e^{-1/2}) / (1 − e^{-2})
    let s = load("C0");
    let lt = LensTol::preset("commercial").unwrap();
    let mut tols = tolerance::simple(&s, &|_| Some(lt), &|_| None, Dist::Uniform);
    let n = tolerance::pair_disc(&mut tols, Some(2.0));
    assert!(n > 0);
    let k = tols
        .iter()
        .position(|t| t.label.ends_with("偏心x"))
        .unwrap();
    assert_eq!(
        (tols[k].dist, tols[k + 1].dist),
        (
            Dist::DiscX { normal: Some(2.0) },
            Dist::DiscY { normal: Some(2.0) }
        )
    );
    assert_eq!(tols[k].dist.name(), "disc_x:normal");
    assert_eq!(
        Dist::parse("disc_y:normal:3").unwrap(),
        Dist::DiscY { normal: Some(3.0) }
    );
    assert_eq!(
        Dist::parse("disc_x:normal:3").unwrap().name(),
        "disc_x:normal:3"
    );
    assert_eq!(
        Dist::parse("disc_x:uniform").unwrap(),
        Dist::DiscX { normal: None }
    );
    assert!(Dist::parse("disc_x:end").is_err());
    let mut spec = McSpec::new(problem(&s), tols.clone());
    spec.trials = 0;
    let (a, b) = (tols[k].hi, tols[k + 1].hi);
    let m = 20000;
    let (mut inner, mut sx, mut sy, mut sxy) = (0, 0.0, 0.0, 0.0);
    for i in 0..m {
        let d = tolerance::draw_trial(&spec, i);
        let (x, y) = (d[k] / a, d[k + 1] / b);
        let r2 = x * x + y * y;
        assert!(r2 <= 1.0 + 1e-12);
        if r2 <= 0.25 {
            inner += 1;
        }
        sx += x * x;
        sy += y * y;
        sxy += x * y;
    }
    let want = (1.0 - (-0.5f64).exp()) / (1.0 - (-2.0f64).exp());
    let got = inner as f64 / m as f64;
    assert!((got - want).abs() < 0.01, "{got} {want}");
    // 等方（x・y の分散が同じで相関なし）
    let (vx, vy, c) = (sx / m as f64, sy / m as f64, sxy / m as f64);
    assert!((vx - vy).abs() < 0.01 && c.abs() < 0.005, "{vx} {vy} {c}");
    // 切った 2 次元正規の E[r²]: r²/σ² は自由度 2 の χ²（平均 2 の指数分布）を cut² で切ったもの。σ = 1/2
    let e = (-2.0f64).exp();
    let er2 = 0.25 * 2.0 * (1.0 - 2.0 * e / (1.0 - e));
    assert!((vx + vy - er2).abs() < 0.01, "{} {er2}", vx + vy);
    // cut が小さくても偏らない（cut → 0 で円の中の一様、E[r²] → 1/2）
    let mut rng = Rng::for_trial(2, 0);
    let m2: f64 = (0..20000)
        .map(|_| {
            let (x, y) = rng.disc_dist(Some(0.05));
            assert!(x * x + y * y <= 1.0);
            x * x + y * y
        })
        .sum::<f64>()
        / 20000.0;
    assert!((m2 - 0.5).abs() < 0.01, "{m2}");
    // 1 つだけ引くときも範囲の中
    let mut rng = Rng::for_trial(1, 0);
    for _ in 0..1000 {
        let v = rng.draw(Dist::DiscY { normal: Some(2.0) }, -2.0, 2.0);
        assert!(v.abs() <= 2.0);
    }
}

/// 光源の波長のずれ（#163）: 波長のパラメータで全媒質の屈折率が新しい波長の値になる（波長を振る解析と同じ値）
#[test]
fn wavelength_param_reindexes_media() {
    let s = load("A0");
    let nw = s.wavelengths.len();
    assert!(nw >= 3);
    let p = s.param("wave2").unwrap();
    assert_eq!(p.kind, Kind::Wave(1));
    assert_eq!(s.param_name(p), "wave2");
    assert!(s.param(&format!("wave{}", nw + 1)).is_err());
    let dl = 0.01;
    let ch: Vec<(Param, f64)> = (0..nw)
        .map(|w| (Param::new(0, Kind::Wave(w as u8)), s.wavelengths[w] + dl))
        .collect();
    let t = s.with_params(&ch).unwrap();
    for w in 0..nw {
        assert_eq!(
            t.get_param(Param::new(0, Kind::Wave(w as u8))).unwrap(),
            s.wavelengths[w] + dl
        );
        let (d, k) = lensbench_core::sweep::with_wavelength(&s, s.wavelengths[w] + dl).unwrap();
        for j in 0..s.surfaces.len() {
            let (a, b) = (t.surfaces[j].n[w], d.surfaces[j].n[k]);
            assert!((a - b).abs() < 1e-14, "面 {j} 波長 {w}: {a} {b}");
        }
    }
    // ガラスの屈折率は変わる（短い波長側へずらすと上がる）
    let j = (0..s.surfaces.len())
        .find(|&j| s.surfaces[j].n[1] > 1.2)
        .unwrap();
    assert!(t.surfaces[j].n[1] < s.surfaces[j].n[1]);
    // 戻すと元の屈折率
    let back: Vec<(Param, f64)> = (0..nw)
        .map(|w| (Param::new(0, Kind::Wave(w as u8)), s.wavelengths[w]))
        .collect();
    let u = t.with_params(&back).unwrap();
    for j in 0..s.surfaces.len() {
        for w in 0..nw {
            assert!((u.surfaces[j].n[w] - s.surfaces[j].n[w]).abs() < 1e-15);
        }
    }
    // 正でない波長・最適化の変数は不正
    assert!(s.with_params(&[(p, 0.0)]).is_err());
    let mut pr = problem(&s);
    pr.vars.push(Var::new(p));
    assert!(pr.check().is_err());
}

/// 波長の公差: ピントの補償子が波長のずれの軸上色収差を吸収し、± で補償子の向きが逆になる
#[test]
fn wavelength_tolerance_with_focus() {
    let s = load("A0");
    let nw = s.wavelengths.len();
    let tol = Tol {
        label: "wave".into(),
        params: (0..nw)
            .map(|w| Param::new(0, Kind::Wave(w as u8)))
            .collect(),
        lo: -0.02,
        hi: 0.02,
        relative: false,
        dist: Dist::Uniform,
        scale: Vec::new(),
    };
    let mut p = problem(&s);
    p.vars
        .push(Var::new(Param::new(s.image() - 1, Kind::Thick)));
    let mut spec = McSpec::new(p, vec![tol]);
    spec.trials = 8;
    let r = tolerance::run(&spec).unwrap();
    let sn = &r.sensitivity[0];
    let (lo, hi) = (sn.lo.as_ref().unwrap(), sn.hi.as_ref().unwrap());
    let c0 = r.nominal.comp[0];
    assert!(
        (lo.comp[0] - c0) * (hi.comp[0] - c0) < 0.0,
        "{} {} {}",
        lo.comp[0],
        c0,
        hi.comp[0]
    );
    // 補償なしのずれた系より、補償した評価関数は小さい
    let unc = {
        let sys = tolerance::perturbed(&spec, &[0.02]).unwrap();
        let mut q = problem(&sys[0]);
        q.vars.clear();
        q.merit(&[]).unwrap()
    };
    assert!(hi.merit <= unc + 1e-12, "{} {}", hi.merit, unc);
    assert!(r.trials.iter().all(|t| t.eval.is_some()));
    assert!(r.trials.iter().all(|t| t.deltas[0].abs() <= 0.02));
}
