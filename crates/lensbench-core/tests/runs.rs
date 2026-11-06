//! 一括実行: 組の作り方（抽選・相関・格子）、並列・逐次書き出し・再開・進み具合。

use lensbench_core::param::{Kind, Param};
use lensbench_core::runs::{self, Dist, Grid, Plan, RunOpts, Sampler};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
        let c = runs::norm_cdf(x);
        assert!(
            (c - p).abs() <= 1e-15 * p.max(1e-300) + 1e-16,
            "Φ({x}) = {c} ≠ {p}"
        );
        let back = runs::norm_ppf(p);
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
    let cancel = AtomicBool::new(false);
    let k = AtomicUsize::new(0);
    let cb = |pr: runs::Progress| {
        assert!(pr.done <= pr.total);
        if k.fetch_add(1, Ordering::Relaxed) + 1 >= 7 {
            cancel.store(true, Ordering::Relaxed);
        }
    };
    let opts = RunOpts {
        threads: Some(1),
        out: Some(&f),
        header: "{\"what\":\"test\"}".into(),
        resume: true,
        progress: Some(&cb),
        cancel: Some(&cancel),
    };
    let r = runs::run(30, 5, &opts, task).unwrap();
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
    let r = runs::run(30, 5, &opts, |i, s| {
        seen.lock().unwrap().push(i);
        task(i, s)
    })
    .unwrap();
    assert_eq!((r.skipped, r.ran), (7, 23));
    assert!(seen.lock().unwrap().iter().all(|&i| i >= 7));
    let (h, rows) = runs::read_results(&f).unwrap();
    assert_eq!(h, "{\"what\":\"test\"}");
    assert_eq!(rows.len(), 30);
    for (i, s, res) in &rows {
        assert_eq!(*s, runs::sub_seed(5, *i));
        assert_eq!(*res, format!("{{\"sq\":{},\"s\":{}}}", i * i, s % 1000));
    }
    // 全部終わっていれば何もしない
    let r = runs::run(30, 5, &opts, task).unwrap();
    assert_eq!((r.skipped, r.ran), (30, 0));
    // 頭が違えば再開しない
    let o2 = RunOpts {
        header: "{\"what\":\"other\"}".into(),
        ..opts
    };
    assert!(runs::run(30, 5, &o2, task).is_err());
    // 頭に改行は書けない
    let o3 = RunOpts {
        header: "{\"a\":\n1}".into(),
        ..opts
    };
    assert!(runs::run(1, 5, &o3, task).is_err());
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
    let e = runs::run(100, 0, &opts, |i, _| {
        if i == 3 {
            Err(lensbench_core::Error::Invalid("だめ".into()))
        } else {
            Ok("1".into())
        }
    });
    assert!(e.unwrap_err().to_string().contains("組 3"));
    let (_, rows) = runs::read_results(&f).unwrap();
    assert!(rows.len() < 100);
    std::fs::remove_file(&f).unwrap();
}
