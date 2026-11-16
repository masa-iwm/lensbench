//! 一括実行: 組の作り方（抽選・相関・格子）、並列・逐次書き出し・再開・進み具合。

use lensbench_core::par;
use lensbench_core::param::{Kind, Param};
use lensbench_runs::{Dist, Grid, Plan, RunOpts, Sampler};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

#[test]
fn normal_cdf_and_ppf() {
    // 値は Python の math.erfc から
    let refs = [
        (0.0, 0.5),
        (1.0, 0.8413447460685429),
        (-1.0, 0.15865525393145707),
        (2.5, 0.9937903346742238),
        (-3.0, 0.0013498980316300957),
        (-6.0, 9.865876450377012e-10),
        (5.0, 0.9999997133484281),
    ];
    for (x, p) in refs {
        let c = lensbench_runs::norm_cdf(x);
        assert!(
            (c - p).abs() <= 1e-15 * p.max(1e-300) + 1e-16,
            "Φ({x}) = {c} ≠ {p}"
        );
        let back = lensbench_runs::norm_ppf(p);
        assert!(
            (back - x).abs() < 1e-9 * (1.0 + x.abs()),
            "Φ⁻¹({p}) = {back}"
        );
    }
}

fn p(j: usize) -> Param {
    Param::new(j, Kind::Thick)
}

#[test]
fn sampler_is_reproducible_and_correlated() {
    let vars = vec![
        (
            p(1),
            Dist::Normal {
                mean: 0.0,
                sigma: 2.0,
            },
        ),
        (
            p(2),
            Dist::Normal {
                mean: 1.0,
                sigma: 1.0,
            },
        ),
        (p(3), Dist::Uniform { lo: -1.0, hi: 3.0 }),
        (
            p(4),
            Dist::TruncNormal {
                mean: 0.0,
                sigma: 1.0,
                cut: 1.5,
            },
        ),
    ];
    let rho = 0.7;
    let corr = vec![
        vec![1.0, rho, 0.0, 0.0],
        vec![rho, 1.0, 0.0, 0.0],
        vec![0.0, 0.0, 1.0, 0.0],
        vec![0.0, 0.0, 0.0, 1.0],
    ];
    let s = Sampler::new(vars.clone(), Some(corr), 42).unwrap();
    // 同じ番号は同じ値（順序によらない）
    assert_eq!(s.draw(17), s.draw(17));
    assert_ne!(s.draw(17), s.draw(18));
    let other = Sampler::new(vars, None, 43).unwrap();
    assert_ne!(s.draw(0)[0], other.draw(0)[0]);
    let n = 40000;
    let d: Vec<Vec<f64>> = (0..n).map(|i| s.draw(i)).collect();
    let mean = |k: usize| d.iter().map(|x| x[k]).sum::<f64>() / n as f64;
    let var = |k: usize| {
        let m = mean(k);
        d.iter().map(|x| (x[k] - m).powi(2)).sum::<f64>() / n as f64
    };
    assert!(mean(0).abs() < 0.05 && (var(0).sqrt() - 2.0).abs() < 0.05);
    assert!((mean(1) - 1.0).abs() < 0.03);
    let (m0, m1) = (mean(0), mean(1));
    let cov = d.iter().map(|x| (x[0] - m0) * (x[1] - m1)).sum::<f64>() / n as f64;
    let r = cov / (var(0) * var(1)).sqrt();
    assert!((r - rho).abs() < 0.02, "相関 {r}");
    assert!(d.iter().all(|x| (-1.0..=3.0).contains(&x[2])));
    assert!((mean(2) - 1.0).abs() < 0.03);
    assert!(d.iter().all(|x| x[3].abs() <= 1.5));
    // 正定値でない相関行列は弾く
    let bad = vec![vec![1.0, 1.0], vec![1.0, 1.0]];
    assert!(Sampler::new(
        vec![(p(1), Dist::Fixed(0.0)), (p(2), Dist::Fixed(0.0))],
        Some(bad),
        1
    )
    .is_err());
}

#[test]
fn grid_order() {
    let g = Grid::new(vec![(p(1), vec![1.0, 2.0]), (p(2), vec![10.0, 20.0, 30.0])]).unwrap();
    assert_eq!(g.count(), Some(6));
    assert_eq!(g.values(0), vec![1.0, 10.0]);
    assert_eq!(g.values(1), vec![1.0, 20.0]);
    assert_eq!(g.values(5), vec![2.0, 30.0]);
}

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("lb_runs_{name}_{}.jsonl", std::process::id()))
}

#[test]
fn run_writes_resumes_and_matches_threads() {
    let f = tmp("resume");
    let _ = std::fs::remove_file(&f);
    let task = |i: usize, s: u64| -> lensbench_core::Result<String> {
        Ok(format!("{{\"sq\":{},\"s\":{}}}", i * i, s % 1000))
    };
    // 途中で止める（7 個終えたら cancel）
    let cancel = par::CancelToken::new();
    let k = std::sync::Arc::new(AtomicUsize::new(0));
    let (c2, k2) = (cancel.clone(), k.clone());
    let cb: par::ProgressFn = std::sync::Arc::new(move |e| {
        let par::Event::Count(pr) = e else {
            panic!("{e:?}")
        };
        assert!(pr.done <= pr.total);
        if k2.fetch_add(1, Ordering::Relaxed) + 1 >= 7 {
            c2.cancel();
        }
    });
    let opts = RunOpts {
        threads: Some(1),
        out: Some(&f),
        header: "{\"what\":\"test\"}".into(),
        resume: true,
        ..Default::default()
    };
    let r = par::with_cancel(Some(&cancel), || {
        par::with_progress(Some(cb), || lensbench_runs::run(30, 5, &opts, task))
    })
    .unwrap();
    assert!(r.cancelled && r.ran == 7, "{r:?}");
    // 最後の行を途中で切る（書いている途中で止まった）
    let mut txt = std::fs::read_to_string(&f).unwrap();
    // 閉じ括弧を欠いても `}` で終わる切れ方（{"result":{"a":1}} の外側の `}` が無い）
    txt.push_str("{\"i\":29,\"seed\":1,\"result\":{\"a\":1}");
    std::fs::write(&f, txt).unwrap();
    // 4 スレッドで続きから
    let seen = Mutex::new(Vec::new());
    let opts = RunOpts {
        threads: Some(4),
        out: Some(&f),
        header: "{\"what\":\"test\"}".into(),
        resume: true,
        ..Default::default()
    };
    let r = lensbench_runs::run(30, 5, &opts, |i, s| {
        seen.lock().unwrap().push(i);
        task(i, s)
    })
    .unwrap();
    assert_eq!((r.skipped, r.ran), (7, 23));
    assert!(seen.lock().unwrap().iter().all(|&i| i >= 7));
    let (h, rows) = lensbench_runs::read_results(&f).unwrap();
    assert_eq!(h, "{\"what\":\"test\"}");
    assert_eq!(rows.len(), 30);
    for (i, s, res) in &rows {
        assert_eq!(*s, lensbench_runs::sub_seed(5, *i));
        assert_eq!(*res, format!("{{\"sq\":{},\"s\":{}}}", i * i, s % 1000));
    }
    // 全部終わっていれば何もしない
    let r = lensbench_runs::run(30, 5, &opts, task).unwrap();
    assert_eq!((r.skipped, r.ran), (30, 0));
    // 頭が違えば再開しない
    let o2 = RunOpts {
        header: "{\"what\":\"other\"}".into(),
        ..opts
    };
    assert!(lensbench_runs::run(30, 5, &o2, task).is_err());
    // 頭に改行は書けない
    let o3 = RunOpts {
        header: "{\"a\":\n1}".into(),
        ..opts
    };
    assert!(lensbench_runs::run(1, 5, &o3, task).is_err());
    std::fs::remove_file(&f).unwrap();
}

#[test]
fn run_stops_on_error() {
    let f = tmp("err");
    let opts = RunOpts {
        threads: Some(2),
        out: Some(&f),
        header: "{}".into(),
        ..Default::default()
    };
    let e = lensbench_runs::run(100, 0, &opts, |i, _| {
        if i == 3 {
            Err(lensbench_core::Error::Invalid("だめ".into()))
        } else {
            Ok("1".into())
        }
    });
    assert!(e
        .unwrap_err()
        .message()
        .unwrap_or_default()
        .contains("set 3"));
    let (_, rows) = lensbench_runs::read_results(&f).unwrap();
    assert!(rows.len() < 100);
    std::fs::remove_file(&f).unwrap();
}

/// 抽選の End・円の中の組（IOO-14・30）: 値の範囲・割合・形、組の約束の誤り
#[test]
fn draws_end_and_disc() {
    use lensbench_runs::{Draws, Joint};
    let disc = |normal| Dist::Disc {
        center: 1.0,
        radius: 2.0,
        normal,
    };
    let d = Draws::new(
        vec![
            Dist::End { lo: -0.5, hi: 0.25 },
            disc(None),
            Dist::Disc {
                center: 0.0,
                radius: 0.5,
                normal: None,
            },
        ],
        None,
        vec![Joint::Disc { x: 1, y: 2 }],
        3,
    )
    .unwrap();
    let n = 20000;
    let v: Vec<Vec<f64>> = (0..n).map(|i| d.draw(i)).collect();
    assert!(v.iter().all(|x| x[0] == -0.5 || x[0] == 0.25));
    let lo = v.iter().filter(|x| x[0] == -0.5).count() as f64 / n as f64;
    assert!((lo - 0.5).abs() < 0.02, "{lo}");
    // 組は楕円（中心 (1, 0)、半径 (2, 0.5)）の中で一様: 正規化した |r|² の平均は 1/2、x・y は無相関
    let (mut r2, mut sxy) = (0.0, 0.0);
    for x in &v {
        let (a, b) = ((x[1] - 1.0) / 2.0, x[2] / 0.5);
        assert!(a * a + b * b <= 1.0 + 1e-12);
        r2 += a * a + b * b;
        sxy += a * b;
    }
    assert!((r2 / n as f64 - 0.5).abs() < 0.01, "{}", r2 / n as f64);
    assert!((sxy / n as f64).abs() < 0.01);
    // 同じ番号は同じ値、種が違えば違う
    assert_eq!(d.draw(5), d.draw(5));
    let d2 = Draws::new(d.dists().to_vec(), None, d.joints().to_vec(), 4).unwrap();
    assert_ne!(d.draw(5), d2.draw(5));
    // 組にしない円の中の分布・種類の違う組・同じ変数の重なり・範囲の外は誤り
    let bad = |dists: Vec<Dist>, joints: Vec<Joint>| Draws::new(dists, None, joints, 1).is_err();
    assert!(bad(vec![disc(None)], vec![]));
    assert!(bad(
        vec![disc(None), disc(Some(2.0))],
        vec![Joint::Disc { x: 0, y: 1 }]
    ));
    assert!(bad(
        vec![disc(None), disc(None)],
        vec![Joint::Disc { x: 0, y: 0 }]
    ));
    assert!(bad(
        vec![disc(None), disc(None), disc(None)],
        vec![Joint::Disc { x: 0, y: 1 }, Joint::Disc { x: 1, y: 2 }]
    ));
    assert!(bad(
        vec![disc(None), Dist::Fixed(0.0)],
        vec![Joint::Disc { x: 0, y: 1 }]
    ));
    assert!(bad(vec![Dist::End { lo: 1.0, hi: 0.0 }], vec![]));
    assert!(bad(
        vec![disc(Some(0.0)), disc(Some(0.0))],
        vec![Joint::Disc { x: 0, y: 1 }]
    ));
}
