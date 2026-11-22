//! 大域の探索（始点を変えて最適化を繰り返す）: 局所の最適化では抜けられない谷から、より良い解を見つけること、
//! 種で再現できること、範囲を守ること、中断で止まること。

use crate::common::root;
use lensbench_core::param::Param;
use lensbench_core::{Result, System};
use lensbench_formats::zmx::{self, LoadOptions};
use lensbench_optim::{
    dls, global_search, DlsOpts, GlobalOpts, Problem, Stop, Term, TermKind, UserTerm, UserTermRef,
};
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

/// 変数の値 x だけで決まる 2 つの谷: r = (x² − 1, 0.3 (x − 1))。x = 1 で 0、x ≈ −1 に局所の谷（評価関数 > 0）
struct DoubleWell {
    p: Param,
    /// 谷の位置をずらす（厚さの値のまま使うと負にできないので、x = 厚さ − off）
    off: f64,
}

impl UserTerm for DoubleWell {
    fn name(&self) -> &str {
        "double_well"
    }
    fn rows(&self) -> usize {
        2
    }
    fn eval(&self, systems: &[&System]) -> Result<Vec<Vec<f64>>> {
        systems
            .iter()
            .map(|s| {
                let x = s.get_param(self.p)? - self.off;
                Ok(vec![x * x - 1.0, 0.3 * (x - 1.0)])
            })
            .collect()
    }
}

/// A0 の像面の前の厚さを x + 10 とした二つの谷の問題（始点 x = −1.5、範囲 x ∈ [−3, 3]）
fn well_problem() -> Problem {
    let mut s = load("A0");
    let j = s.image() - 1;
    let p = s.param(&format!("{j}.t")).unwrap();
    s = s.with_params(&[(p, 10.0 - 1.5)]).unwrap();
    let mut q = Problem::single(s);
    q.terms.push(Term {
        side: Default::default(),
        config: 0,
        weight: 1.0,
        kind: TermKind::User(UserTermRef(Arc::new(DoubleWell { p, off: 10.0 }))),
    });
    q.add_var(&format!("{j}.t")).unwrap().lo = 10.0 - 3.0;
    q.vars[0].hi = 10.0 + 3.0;
    q
}

#[test]
fn global_escapes_local_valley() {
    let q = well_problem();
    // 局所の最適化だけでは x ≈ −1 の谷に止まる
    let loc = dls(&q, &DlsOpts::default()).unwrap();
    assert!(loc.x[0] - 10.0 < 0.0, "local {:?}", loc.x);
    assert!(loc.merit > 0.1, "local merit {}", loc.merit);
    let o = GlobalOpts {
        starts: 12,
        keep: 3,
        seed: 7,
        ..Default::default()
    };
    let g = global_search(&q, &o).unwrap();
    assert!((g.best.x[0] - 11.0).abs() < 1e-6, "global {:?}", g.best.x);
    assert!(g.best.merit < 1e-8, "global merit {}", g.best.merit);
    assert!(g.best.merit < loc.merit);
    // 今の値からの比べ
    assert!((g.best.merit0 - loc.merit0).abs() < 1e-12);
    assert_eq!(g.best.x0, loc.x0);
    // 残した解: 良い順、同じ解は 1 つ（二つの谷なので 2 つ）、先頭が最良
    assert_eq!(g.solutions.len(), 2, "{:?}", g.solutions);
    assert_eq!(g.solutions[0].start, g.best_start);
    assert!(g.solutions[0].merit <= g.solutions[1].merit);
    assert!(g.solutions[1].x[0] - 10.0 < 0.0);
    assert_eq!(g.ran, 12);
    assert_eq!(g.failed, 0);
    assert!(!g.cancelled);
    // 最良の系に値が入っている
    let j = q.configs[0].image() - 1;
    assert!((g.best.systems[0].surfaces[j].t - 11.0).abs() < 1e-6);
}

#[test]
fn global_is_reproducible_and_bounded() {
    let q = well_problem();
    let o = GlobalOpts {
        starts: 8,
        seed: 3,
        ..Default::default()
    };
    let a = global_search(&q, &o).unwrap();
    // 並列の数によらず同じ
    let b = lensbench_core::par::with_threads(Some(1), || global_search(&q, &o)).unwrap();
    assert_eq!(a.best.x, b.best.x);
    assert_eq!(a.best_start, b.best_start);
    let sa: Vec<_> = a.solutions.iter().map(|s| s.x_start.clone()).collect();
    let sb: Vec<_> = b.solutions.iter().map(|s| s.x_start.clone()).collect();
    assert_eq!(sa, sb);
    // 始点は範囲の中、始点 0 は今の値
    let x0 = q.x0().unwrap();
    for i in 0..50 {
        let x = lensbench_optim::global::start_point(&q, &x0, &o, i);
        assert!(x[0] >= q.vars[0].lo && x[0] <= q.vars[0].hi);
        if i == 0 {
            assert_eq!(x, x0);
        }
    }
    // 種が違えば始点が違う
    let o2 = GlobalOpts { seed: 4, ..o };
    assert_ne!(
        lensbench_optim::global::start_point(&q, &x0, &o, 1),
        lensbench_optim::global::start_point(&q, &x0, &o2, 1)
    );
    // 範囲の無い変数は今の値 ± spread × 目安の大きさの中
    let mut u = q.clone();
    u.vars[0].lo = f64::NEG_INFINITY;
    u.vars[0].hi = f64::INFINITY;
    for i in 1..50 {
        let x = lensbench_optim::global::start_point(&u, &x0, &o, i);
        assert!((x[0] - x0[0]).abs() <= o.spread * x0[0].abs() + 1e-12);
    }
}

#[test]
fn global_cancel_and_check() {
    let q = well_problem();
    let t = lensbench_core::par::CancelToken::new();
    t.cancel();
    // 始める前に止めると、どの始点も回らず中断の誤り
    let r =
        lensbench_core::par::with_cancel(Some(&t), || global_search(&q, &GlobalOpts::default()));
    assert!(r.is_err());
    // 設定の確かめ
    for o in [
        GlobalOpts {
            starts: 0,
            ..Default::default()
        },
        GlobalOpts {
            keep: 0,
            ..Default::default()
        },
        GlobalOpts {
            spread: 0.0,
            ..Default::default()
        },
    ] {
        assert!(global_search(&q, &o).is_err());
    }
    // 始点 1 つは局所の最適化と同じ
    let o = GlobalOpts {
        starts: 1,
        ..Default::default()
    };
    let g = global_search(&q, &o).unwrap();
    let l = dls(&q, &o.local).unwrap();
    assert_eq!(g.best.x, l.x);
    assert_ne!(g.best.stop, Stop::Cancelled);
}

/// トリプレット（A0）の曲率 6 つをずらした始点から: 局所の最適化は別の谷（RMS スポット 13.3 µm）に止まり、
/// 大域の探索はより良い谷（7.1 µm）を見つける（どちらも像面の手前の厚さは周辺光線の高さのソルブ）
#[test]
fn global_finds_better_triplet() {
    use lensbench_analysis::sweep::RmsRef;
    use lensbench_optim::DefaultMerit;
    let s = load("A0");
    let names = ["1.c", "2.c", "3.c", "4.c", "6.c", "7.c"];
    let start = [0.03, 0.0, -0.03, 0.03, 0.02, -0.03];
    let ch: Vec<_> = names
        .iter()
        .zip(start)
        .map(|(n, v)| (s.param(n).unwrap(), v))
        .collect();
    let mut q = Problem::single(s.with_params(&ch).unwrap());
    q.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    // 振った始点には光線が面に届かないものもある（けられたとみなす）
    q.stopped_as_vignetted = true;
    for n in names {
        let v = q.add_var(n).unwrap();
        v.lo = -0.08;
        v.hi = 0.08;
    }
    let l = dls(&q, &DlsOpts::default()).unwrap();
    let o = GlobalOpts {
        starts: 32,
        keep: 5,
        seed: 1,
        ..Default::default()
    };
    let g = global_search(&q, &o).unwrap();
    assert!(l.merit > 0.012, "local {}", l.merit);
    assert!(
        g.best.merit < 0.75 * l.merit,
        "global {} local {}",
        g.best.merit,
        l.merit
    );
    assert_ne!(g.best_start, 0);
    // 局所の最適化の谷も残した解に入っている
    assert!(
        g.solutions
            .iter()
            .any(|s| (s.merit - l.merit).abs() < 1e-6 * l.merit),
        "{:?}",
        g.solutions.iter().map(|s| s.merit).collect::<Vec<_>>()
    );
    assert!(g.solutions.len() <= 5);
    assert!(g.solutions.windows(2).all(|w| w[0].merit <= w[1].merit));
    for s in &g.solutions {
        assert!(s.x.iter().all(|&c| (-0.08..=0.08).contains(&c)));
    }
}
