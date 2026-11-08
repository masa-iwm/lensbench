//! ズーム軌跡: 状態ごとに群の移動を解き、焦点距離の目標と像のピントを満たすか

use lensbench_core::optim::{self, DefaultMerit, ParaxQ};
use lensbench_core::sweep::RmsRef;
use lensbench_core::zoom::{self, ZoomSpec};
use lensbench_core::System;
use lensbench_project::{expand_state, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn states() -> Vec<System> {
    let path = root().join("examples/projects/P17_zoom_three_groups.json");
    let p = Project::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let env = Env {
        base: path.parent().unwrap().to_path_buf(),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    };
    ["wide", "mid", "tele"]
        .iter()
        .map(|n| expand_state(&p, &env, Some(n)).unwrap())
        .collect()
}

fn spec(s: &System, targets: Vec<Option<f64>>) -> ZoomSpec {
    let moving = vec![s.param("G2.shift").unwrap(), s.param("G3.shift").unwrap()];
    ZoomSpec::new(moving, targets)
}

/// 像の項だけの評価関数（視野の重みで平均した RMS スポット）
fn image_merit(s: &System) -> f64 {
    let mut q = optim::Problem::single(s.clone());
    q.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    q.merit(&[]).unwrap()
}

fn track(s: &System) -> f64 {
    (1..s.image()).map(|j| s.surfaces[j].t).sum()
}

#[test]
fn group_shift_of_a_state_is_the_unit_value() {
    // 状態の群の移動は、展開した系の `G2.shift` の値になる（状態どうしで群の位置を比べられる）
    let cs = states();
    let g2 = cs[0].param("G2.shift").unwrap();
    let g3 = cs[0].param("G3.shift").unwrap();
    assert_eq!(cs[2].get_param(g2).unwrap(), 5.0);
    assert_eq!(cs[2].get_param(g3).unwrap(), 14.0);
    // 広角の系に望遠の値を入れると望遠の系と同じ
    let s = cs[0].with_params(&[(g2, 5.0), (g3, 14.0)]).unwrap();
    for j in 1..s.surfaces.len() {
        assert!(
            (s.surfaces[j].t - cs[2].surfaces[j].t).abs() < 1e-12,
            "面 {j}"
        );
    }
}

#[test]
fn states_reach_target_efl() {
    let cs = states();
    let targets = [95.0, 110.0, 130.0];
    let sp = spec(&cs[0], targets.iter().map(|&t| Some(t)).collect());
    let r = zoom::solve(&cs, &sp).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(r.states.len(), 3);
    assert_eq!(r.path.len(), 3);
    for (k, p) in r.states.iter().enumerate() {
        assert_eq!(p.state, Some(k));
        assert!(
            (p.value - targets[k]).abs() < sp.target_tol * targets[k],
            "状態 {k}: EFL {} 目標 {}",
            p.value,
            targets[k]
        );
        assert_eq!(r.systems[k].paraxial().efl, p.value);
        // 全長は変わらない（群の移動は前後の間隔の付け替え）
        assert!((track(&r.systems[k]) - track(&cs[k])).abs() < 1e-9);
        // 点の像の評価関数は解いた系のもの
        let m0 = image_merit(&r.systems[k]);
        assert!((m0 - p.merit).abs() <= 1e-12 * m0, "{m0} {}", p.merit);
        // 解: 少しずらした所から解き直しても同じ所に戻る
        let g = &sp.moving;
        let s2 = r.systems[k]
            .with_params(&[(g[0], p.x[0] + 0.3), (g[1], p.x[1] - 0.5)])
            .unwrap();
        let r2 = zoom::solve(
            std::slice::from_ref(&s2),
            &ZoomSpec {
                targets: vec![Some(targets[k])],
                ..sp.clone()
            },
        )
        .unwrap();
        for q in 0..2 {
            assert!(
                (r2.states[0].x[q] - p.x[q]).abs() < 1e-4,
                "状態 {k}: {:?} {:?}",
                r2.states[0].x,
                p.x
            );
        }
    }
    // 望遠ほど G2・G3 が後ろ（光の向き）
    for k in 0..2 {
        assert!(r.states[k + 1].x[0] > r.states[k].x[0]);
        assert!(r.states[k + 1].x[1] > r.states[k].x[1]);
    }
}

#[test]
fn path_between_states_is_continuous() {
    let cs = states();
    let mut sp = spec(&cs[0], vec![Some(95.0), Some(110.0), Some(130.0)]);
    sp.steps = 3;
    let r = zoom::solve(&cs, &sp).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(r.path.len(), 3 + 2 * 3);
    // 状態は path の 0・4・8 番目
    for (k, i) in [0, 4, 8].iter().enumerate() {
        assert_eq!(r.path[*i].state, Some(k));
        assert_eq!(r.path[*i].x, r.states[k].x);
    }
    for (i, p) in r.path.iter().enumerate() {
        assert!((p.pos - i as f64 * 0.25).abs() < 1e-12);
        let t = p.target.unwrap();
        assert!(
            (p.value - t).abs() < sp.target_tol * t,
            "{i}: {} {t}",
            p.value
        );
    }
    // 間の点の目標は状態の値の間に等比に並ぶ
    for (a, b) in [(0, 4), (4, 8)] {
        let ta = r.path[a].target.unwrap();
        let tb = r.path[b].target.unwrap();
        let ratio = (tb / ta).powf(0.25);
        for j in a + 1..=b {
            let t = (r.path[j].target.unwrap(), r.path[j - 1].target.unwrap());
            assert!((t.0 / t.1 - ratio).abs() < 1e-12, "{j}");
        }
    }
    // 群の位置は単調で滑らか（隣の差の比が 0.4〜2.5）
    for q in 0..2 {
        let d: Vec<f64> = r.path.windows(2).map(|w| w[1].x[q] - w[0].x[q]).collect();
        for e in d.windows(2) {
            assert!(
                e[0] > 0.0 && e[1] > 0.0 && (e[1] / e[0]) < 2.5 && (e[1] / e[0]) > 0.4,
                "群 {q}: {d:?}"
            );
        }
    }
}

#[test]
fn state_without_target_and_errors() {
    let cs = states();
    // 目標のない状態は像の項だけで解き、間の点の目標にはその値を使う
    let mut sp = spec(&cs[0], vec![Some(95.0), None, Some(130.0)]);
    sp.steps = 1;
    let r = zoom::solve(&cs, &sp).unwrap();
    assert!(r.states[1].target.is_none());
    let v1 = r.states[1].value;
    let t = r.path[1].target.unwrap();
    assert!((t - (95.0 * v1).sqrt()).abs() < 1e-9 * t, "{t} {v1}");
    // 目標の数・範囲の数・重み 0・動かすものなしは誤り
    let mut sp = spec(&cs[0], vec![Some(95.0)]);
    assert!(zoom::solve(&cs, &sp).is_err());
    sp.targets = vec![None; 3];
    sp.bounds = vec![(0.0, 1.0)];
    assert!(zoom::solve(&cs, &sp).is_err());
    sp.bounds.clear();
    sp.target_weight = 0.0;
    assert!(zoom::solve(&cs, &sp).is_err());
    sp.target_weight = 10.0;
    sp.moving.clear();
    assert!(zoom::solve(&cs, &sp).is_err());
    // 無限遠物体の倍率は 0 なので目標 0.1 には届かない → 警告
    let mut sp = spec(&cs[0], vec![Some(0.1), None, None]);
    sp.q = ParaxQ::Pmag;
    sp.opts.max_iter = 3;
    let r = zoom::solve(&cs, &sp).unwrap();
    assert!(
        r.warnings.iter().any(|w| w.contains("pmag")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn bounds_hold() {
    let cs = states();
    let mut sp = spec(&cs[0], vec![Some(95.0), Some(110.0), Some(130.0)]);
    sp.bounds = vec![(-10.0, 3.0), (-10.0, 30.0)];
    let r = zoom::solve(&cs, &sp).unwrap();
    // 望遠では G2 が上限に張り付き、残りの G3 で焦点距離を合わせる
    assert_eq!(r.states[2].x[0], 3.0);
    assert!((r.states[2].value - 130.0).abs() < sp.target_tol * 130.0);
    for p in &r.states {
        assert!(p.x[0] <= 3.0 && p.x[0] >= -10.0 && p.x[1] <= 30.0);
    }
}
