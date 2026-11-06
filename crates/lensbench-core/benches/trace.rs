//! 光線追跡の速さ（criterion）: `cargo bench -p lensbench-core --bench trace`
//!
//! tests/refs の系を 2 万本で、1 スレッドと全コアで測る。A0・C0 はエイミングなし（C0 は傾き偏心）、
//! A2・C1 は実光線エイミングあり。前回との差は criterion が target/criterion に残す。
//! 別の実装と同じ条件で比べるときは examples/bench.rs（`--json`）を使う。

use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{FieldSel, System};

const RAYS: usize = 20_000;
const SYSTEMS: [&str; 4] = ["A0", "C0", "A2", "C1"];

/// examples/bench.rs と同じ点列（xorshift64、種 12345、単位円内に一様）
fn pupil(n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut st = 12345u64;
    let mut rnd = || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        (st >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..n)
        .map(|_| {
            let (r, t) = (rnd().sqrt(), rnd() * std::f64::consts::TAU);
            (r * t.cos(), r * t.sin())
        })
        .unzip()
}

fn load(name: &str) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

fn trace(c: &mut Criterion) {
    let (px, py) = pupil(RAYS);
    let all = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut threads = vec![1];
    if all > 1 {
        threads.push(all);
    }
    let mut g = c.benchmark_group("trace");
    g.throughput(Throughput::Elements(RAYS as u64));
    for name in SYSTEMS {
        let s = load(name);
        let f = FieldSel::Index(1.min(s.fields.len() - 1));
        for &n in &threads {
            g.bench_with_input(BenchmarkId::new(name, format!("{n}T")), &s, |b, s| {
                lensbench_core::with_threads(Some(n), || {
                    b.iter(|| s.trace(f, &px, &py, Some(s.primary), None, false))
                })
            });
        }
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = trace
}
criterion_main!(benches);
