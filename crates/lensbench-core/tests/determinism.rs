//! 追跡・解析の結果がスレッド数（1・2・4・既定）によらず同じビットになることを確かめる。
//! 比べるのは f64 の Debug 表示（最短で元に戻る表記なので、ビットが違えば文字列も違う）。

use lensbench_core::analysis::{self, ZernKind};
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::system::{Aiming, FieldSel};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{with_threads, System};
use std::path::PathBuf;

fn load(name: &str) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

/// 並列になる本数（512 本以上）の瞳の格子
fn grid() -> (Vec<f64>, Vec<f64>) {
    let n = 48;
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for iy in 0..n {
        for ix in 0..n {
            px.push(2.0 * ix as f64 / (n - 1) as f64 - 1.0);
            py.push(2.0 * iy as f64 / (n - 1) as f64 - 1.0);
        }
    }
    (px, py)
}

/// 系 s の追跡と主な解析の結果をまとめた文字列（新しい系として読み直し、キャッシュも毎回作り直す）
fn fingerprint(name: &str) -> String {
    let s = load(name);
    let (px, py) = grid();
    let mut out = String::new();
    let nf = s.fields.len();
    for f in 0..nf {
        for w in 0..s.wavelengths.len() {
            let t = s
                .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, true)
                .unwrap();
            out += &format!("{:?}{:?}{:?}{:?}{:?}", t.rays, t.p0, t.k0, t.opl_os, t.rec);
        }
        let fs = FieldSel::Index(f);
        out += &format!("{:?}", analysis::spot(&s, fs, None, 6).unwrap());
        let wf = analysis::wavefront(&s, fs, None, 64, false).unwrap();
        out += &format!("{:?}{:?}{:?}", wf.w, wf.pv, wf.rms);
        out += &format!(
            "{:?}",
            analysis::zernike(&s, fs, None, ZernKind::Standard, 37, 64, false).unwrap()
        );
        out += &format!(
            "{:?}",
            analysis::mtf(&s, fs, None, Some(32), &[0.0, 10.0, 25.0, 50.0]).unwrap()
        );
    }
    let c = Coatings::for_system(&s).unwrap();
    let pols = [
        [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
        [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
    ];
    let p = s
        .try_trace_pol(FieldSel::Index(nf - 1), &px, &py, None, &pols, &c)
        .unwrap();
    out += &format!("{:?}", p.e);
    out
}

fn check(name: &str) -> Aiming {
    let base = with_threads(Some(1), || fingerprint(name));
    for th in [Some(2), Some(4), None] {
        let r = with_threads(th, || fingerprint(name));
        assert!(
            r == base,
            "{name}: threads {th:?} の結果が 1 スレッドと違う"
        );
    }
    load(name).aiming
}

#[test]
fn results_do_not_depend_on_the_thread_count() {
    // A1 は近軸、A7 は実光線のエイミング
    let aims: Vec<Aiming> = ["A0", "A1", "A7", "C0", "D0"].map(check).to_vec();
    assert!(aims.contains(&Aiming::Paraxial) && aims.contains(&Aiming::Real));
}

#[test]
fn single_thread_runs_on_the_calling_thread() {
    let id = std::thread::current().id();
    let r = with_threads(Some(1), || {
        assert!(lensbench_core::par::sequential());
        // 中から既定に戻せる
        with_threads(None, || assert!(!lensbench_core::par::sequential()));
        assert!(lensbench_core::par::sequential());
        std::thread::current().id()
    });
    assert_eq!(r, id);
    assert!(!lensbench_core::par::sequential());
    assert_eq!(
        with_threads(Some(3), lensbench_core::par::current_threads),
        3
    );
    assert_eq!(
        with_threads(Some(1), lensbench_core::par::current_threads),
        1
    );
}
