//! 非順次の追跡の面の散乱（段 3）: 散乱した光の検出器の照度が BSDF の解析解（Lambert は一様な放射輝度・cos 則）と
//! 3σ の範囲で合う、エネルギーの収支、散乱の割合 0 は散乱なしとビットまで同じ、スレッド数で同じ値、光を通さない面の
//! 反射率

use lensbench_core::coating::Coatings;
use lensbench_core::scatter::{Bsdf, Finish, Scatter};
use lensbench_core::System;
use lensbench_project::nsc::{system_source, Budget, DetectorSpec, NscOpts, Split, Tracer};
use lensbench_project::raypath::EventKind;
use lensbench_project::scene::{expansions, Scene};
use lensbench_project::{Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        step_files: None,
        base: root().join("tests/refs"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

/// プロジェクトの JSON の場面と系
fn project(txt: &str) -> (Scene, Vec<System>) {
    let p = Project::from_json(txt).unwrap();
    let e = env();
    let sc = Scene::from_project(&p, &e).unwrap();
    let xs = expansions(&p, &e).unwrap();
    (sc, xs.into_iter().map(|x| x.1.system).collect())
}

fn load(name: &str) -> System {
    let opt = lensbench_formats::zmx::LoadOptions {
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    lensbench_formats::zmx::load_with(&root().join(format!("tests/refs/{name}/system.zmx")), &opt)
        .unwrap()
}

fn with_finishes(s: &System, f: Vec<Finish>) -> System {
    let mut s = s.clone();
    s.edit_raw(|rx| {
        rx.finishes = f;
        Ok(())
    })
    .unwrap();
    s
}

fn check_budget(label: &str, b: &Budget, tol: f64) {
    let r = b.residual() / b.source;
    assert!(
        r.abs() < tol,
        "{label}: residual {r:e} of the source ({b:?})"
    );
}

/// 平らな鏡（絞り、反射率 1 の理想の膜）に垂直に細い光束を当て、散乱 scatter で戻る光を距離 D の像面（検出器）で受ける
const D: f64 = 50.0;
fn mirror_project(scatter: &str) -> String {
    format!(
        r#"{{"wavelengths": [{{"um": 0.55}}], "aperture": {{"type": "epd", "value": 0.02}},
        "fields": {{"type": "angle", "points": [[0, 0]]}},
        "blocks": [
          {{"type": "mirror", "id": "M", "semi": 5, "coat": "I.0", "surface_ops": [{{"op": "set_stop", "surface": 0}}]}},
          {{"type": "air", "t": {D}}}
        ],
        "finishes": [{{"face": {{"block": "M", "surface": 0}}, "scatter": {scatter}}}]}}"#
    )
}

/// 鏡で散乱した光の検出器の画素ごとのパワーを、BSDF の解析の値（照度 E = P·f(β)·cos²θ / R²、β = (x, y)/R、
/// cosθ = D/R）の画素の積分と比べる。全部の画素が 4σ 以内、3σ を超える画素は 1 % 以下、χ²/自由度 < 1.5
fn compare_with_bsdf(label: &str, scatter: &Scatter, split: Split) {
    let txt = mirror_project(&serde_json::to_string(scatter).unwrap());
    let (sc, systems) = project(&txt);
    let s = &systems[0];
    let coat = Coatings::for_system(s).unwrap();
    let half = 40.0;
    let n = 11;
    let opts = NscOpts {
        split,
        detector: DetectorSpec {
            half: Some([half, half]),
            pixels: [n, n],
        },
        seed: 3,
        ..Default::default()
    };
    let tr = Tracer::new(&sc, &[s], &coat, &opts).unwrap();
    let g: usize = std::env::var("NSC_GRID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(241);
    let src = system_source(s, &[0], g, s.primary, 1.0, None).unwrap();
    let r = tr.run(&src).unwrap();
    check_budget(
        label,
        &r.budget,
        if matches!(split, Split::Deterministic) {
            1e-12
        } else {
            1e-2
        },
    );
    if std::env::var("NSC_DEBUG").is_ok() {
        eprintln!(
            "{label}: {:?} total on det {}",
            r.budget,
            r.detectors[0].total()
        );
    }
    let det = &r.detectors[0];
    let bsdf: Bsdf = scatter.prepare().unwrap();
    // 鏡の反射率（膜なしの鏡は 1）と散乱の割合
    let tis = bsdf.tis(0.0);
    let (xs, ys) = det.centers();
    let d = 2.0 * half / n as f64;
    let m = 24;
    let (mut chi2, mut over3, mut dof) = (0.0, 0, 0);
    for (j, &yc) in ys.iter().enumerate() {
        for (i, &xc) in xs.iter().enumerate() {
            let mut want = 0.0;
            for a in 0..m {
                for b in 0..m {
                    let x = xc - 0.5 * d + (a as f64 + 0.5) * d / m as f64;
                    let y = yc - 0.5 * d + (b as f64 + 0.5) * d / m as f64;
                    let r2 = x * x + y * y + D * D;
                    let rr = r2.sqrt();
                    let beta = [x / rr, y / rr];
                    want += bsdf.value(beta, [0.0, 0.0]) * (D * D / r2) / r2;
                }
            }
            want *= (d / m as f64).powi(2);
            // 散乱しない分は鏡面の方向（真ん中の画素）へ戻る
            let center = i == n / 2 && j == n / 2;
            let spec = if center { 1.0 - tis } else { 0.0 };
            let got = det.power[j * n + i] - spec;
            // 決まった分岐: 散乱した光線 1 本のパワーは TIS/N で、画素の本数は二項分布 B(N, p)（p = want/TIS）。
            // モンテカルロ: 光線 1 本のパワーは 1/N で、本数は B(N, TIS·p)
            let nn = src.len() as f64;
            let q = match split {
                Split::Deterministic => want / tis,
                Split::MonteCarlo { .. } => want,
            };
            let w = match split {
                Split::Deterministic => tis / nn,
                Split::MonteCarlo { .. } => 1.0 / nn,
            };
            let sigma = w * (nn * q * (1.0 - q)).sqrt();
            let sigma = sigma.max(1e-15);
            let z = (got - want) / sigma;
            if std::env::var("NSC_DEBUG").is_ok() {
                eprintln!("{label} ({i}, {j}) got {got:e} want {want:e} z {z:.2}");
            }
            assert!(
                z.abs() < 4.0,
                "{label}: pixel ({i}, {j}) {got:e} vs {want:e} ({z:.2} σ)"
            );
            if z.abs() > 3.0 {
                over3 += 1;
            }
            chi2 += z * z;
            dof += 1;
        }
    }
    assert!(
        over3 * 100 <= dof,
        "{label}: {over3} of {dof} pixels over 3σ"
    );
    assert!(
        chi2 / (dof as f64) < 1.5,
        "{label}: χ²/dof {}",
        chi2 / dof as f64
    );
    eprintln!(
        "{label}: χ²/dof {:.3}, over 3σ {over3}/{dof}, detected {:.5}",
        chi2 / dof as f64,
        r.budget.detected
    );
}

#[test]
fn lambert_follows_the_cosine_law() {
    compare_with_bsdf("lambert", &Scatter::lambert(1.0), Split::Deterministic);
    compare_with_bsdf(
        "lambert mc",
        &Scatter::lambert(1.0),
        Split::MonteCarlo { seed: 4 },
    );
}

#[test]
fn gauss_and_abg_follow_their_bsdf() {
    compare_with_bsdf("gauss", &Scatter::gauss(0.3, 0.8), Split::Deterministic);
    compare_with_bsdf("abg", &Scatter::abg(0.02, 0.01, 2.0), Split::Deterministic);
    compare_with_bsdf(
        "abg g 1.5",
        &Scatter::abg(0.003, 0.002, 1.5),
        Split::MonteCarlo { seed: 9 },
    );
}

/// レンズの面の散乱: 収支が閉じる、散乱の割合 0 は散乱なしとビットまで同じ、スレッド数で同じ値、散乱の出来事が
/// 経路に出る
#[test]
fn scatter_on_lens_surfaces() {
    let s = load("A0");
    let coat = Coatings::for_system(&s).unwrap();
    let opts = NscOpts {
        detector: DetectorSpec {
            half: None,
            pixels: [16, 16],
        },
        ..Default::default()
    };
    let src = system_source(&s, &[0], 15, s.primary, 1.0, None).unwrap();
    let run = |s2: &System, o: &NscOpts| {
        let mut sc = Scene::new();
        sc.add_system("system", s2, None).unwrap();
        sc.finish().unwrap();
        let tr = Tracer::new(&sc, &[s2], &coat, o).unwrap();
        tr.run(&src).unwrap()
    };
    let plain = run(&s, &opts);
    let zero = with_finishes(
        &s,
        vec![Finish {
            surface: Some(2),
            scatter: Some(Scatter::lambert(0.0)),
            ..Default::default()
        }],
    );
    assert_eq!(run(&zero, &opts), plain);
    let rough = with_finishes(
        &s,
        vec![
            Finish {
                surface: Some(2),
                scatter: Some(Scatter::abg(1e-3, 1e-3, 2.0)),
                ..Default::default()
            },
            Finish {
                edge: Some(2),
                scatter: Some(Scatter::lambert(1.0)),
                ..Default::default()
            },
        ],
    );
    let o3 = NscOpts {
        scatter_rays: 3,
        ..opts.clone()
    };
    let a = lensbench_core::par::with_threads(Some(1), || run(&rough, &o3));
    let b = lensbench_core::par::with_threads(Some(4), || run(&rough, &o3));
    assert_eq!(a, b);
    check_budget("A0 abg", &a.budget, 1e-12);
    assert!(a
        .paths
        .iter()
        .any(|p| p.events.iter().any(|e| e.1 == EventKind::Scatter)));
    assert!(a
        .paths
        .iter()
        .any(|p| p.events.iter().any(|e| e.1 == EventKind::ScatterReflect)));
    // 散乱で鏡面の像から出ていくので、検出器の全パワーは少し減る（膜なしの面の TIS ≈ 2e-2）
    assert!(a.budget.detected < plain.budget.detected);
    let mc = NscOpts {
        split: Split::MonteCarlo { seed: 2 },
        ..o3.clone()
    };
    let m1 = lensbench_core::par::with_threads(Some(1), || run(&rough, &mc));
    let m4 = lensbench_core::par::with_threads(Some(3), || run(&rough, &mc));
    assert_eq!(m1, m4);
}

/// 光を通さない面: 機構部品・絞りの板の反射率で吸収が減り、収支は閉じる。光学面に反射率は書けない
#[test]
fn opaque_surfaces_reflect_their_reflectance() {
    let txt = std::fs::read_to_string(root().join("examples/projects/P19_tube_cage.json")).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let base = Project::from_json(&txt).unwrap();
    let e = Env::for_file(
        &root().join("examples/projects/P19_tube_cage.json"),
        &[root().join("tests/glass")],
        None,
    );
    let run = |p: &Project| {
        let sc = Scene::from_project(p, &e).unwrap();
        let xs = expansions(p, &e).unwrap();
        let systems: Vec<&System> = xs.iter().map(|x| &x.1.system).collect();
        let coat = Coatings::for_system(systems[0]).unwrap();
        let tr = Tracer::new(&sc, &systems, &coat, &NscOpts::default()).unwrap();
        let s = systems[0];
        let src = system_source(s, &[0, 1, 2], 21, s.primary, 1.0, None).unwrap();
        tr.run(&src).unwrap()
    };
    let black = run(&base);
    v["finishes"] = serde_json::json!([
        {"mech": "tube", "reflect": 0.3, "scatter": {"model": "lambert"}},
        {"face": {"block": "stop", "surface": 0}, "reflect": 0.5}
    ]);
    let shiny = Project::from_json(&v.to_string()).unwrap();
    let r = run(&shiny);
    check_budget("P19 shiny", &r.budget, 1e-12);
    check_budget("P19 black", &black.budget, 1e-12);
    assert!(r.budget.absorbed < black.budget.absorbed);
    assert!(r
        .paths
        .iter()
        .any(|p| p.events.iter().any(|e| e.1 == EventKind::ScatterReflect)));
    // 光学面に反射率は書けない
    v["finishes"] = serde_json::json!([{"face": {"block": "L1", "surface": 0}, "reflect": 0.5}]);
    let bad = Project::from_json(&v.to_string()).unwrap();
    let sc = Scene::from_project(&bad, &e).unwrap();
    let xs = expansions(&bad, &e).unwrap();
    let systems: Vec<&System> = xs.iter().map(|x| &x.1.system).collect();
    let coat = Coatings::for_system(systems[0]).unwrap();
    assert!(Tracer::new(&sc, &systems, &coat, &NscOpts::default()).is_err());
    // 無いブロック・対象が 2 つは読むときに誤り
    v["finishes"] = serde_json::json!([{"face": {"block": "nosuch", "surface": 0}, "scatter": {"model": "lambert"}}]);
    assert!(Project::from_json(&v.to_string()).is_err());
    v["finishes"] = serde_json::json!([{"face": {"block": "L1", "surface": 0}, "mech": "tube", "reflect": 0.1}]);
    assert!(Project::from_json(&v.to_string()).is_err());
}
