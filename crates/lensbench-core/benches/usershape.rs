//! 利用者定義の形（USERSHAPE）と内蔵の面の速さの比べ: `cargo bench -p lensbench-core --bench usershape`
//!
//! A0 の面 3 を、37 項の Zernike Standard Sag（内蔵）と、同じ ZernSag を包んだ利用者定義の形にして、2 万本を
//! 1 スレッドで追う。差は形の口の重さ（動的な呼び出し・裏返しの包み）。
#![allow(clippy::unwrap_used, clippy::panic)] // 試験・例・組み立ての補助は panic で知らせる

use std::path::PathBuf;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use lensbench_core::freeform::ZernSag;
use lensbench_core::plugin::Config;
use lensbench_core::surface::SurfType;
use lensbench_core::usershape::{ShapeInstance, ShapeModel, UserShape};
use lensbench_core::{FieldSel, Result, System};
use lensbench_formats::zmx::{self, LoadOptions};

const RAYS: usize = 20_000;

fn zern() -> ZernSag {
    ZernSag::new(
        false,
        6.0,
        (1..=37).map(|i| 1e-4 * (i as f64 * 0.7).sin()).collect(),
        true,
    )
}

struct Zern;
struct ZernInst(ZernSag);

impl ShapeModel for Zern {
    fn name(&self) -> &str {
        "bench_zernike"
    }
    fn prepare(&self, _config: &Config) -> Result<Arc<dyn ShapeInstance>> {
        Ok(Arc::new(ZernInst(zern())))
    }
}

impl ShapeInstance for ZernInst {
    fn sag(&self, x: f64, y: f64) -> (f64, f64, f64) {
        self.0.eval(x, y)
    }
}

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

fn a0(user: bool) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let s = zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap();
    let mut rx = s.into_prescription();
    let d = &mut rx.surfaces[3];
    if user {
        d.set_type(SurfType::UserShape);
        *d.user_shape_mut().unwrap() =
            Some(UserShape::new("bench_zernike", Config::default()).unwrap());
    } else {
        d.set_type(SurfType::ZernikeStd);
        *d.zern_mut().unwrap() = Some(Arc::new(zern()));
    }
    rx.build_raw().unwrap()
}

fn bench(c: &mut Criterion) {
    let mut e = lensbench_core::ext::Ext::default();
    e.add_shape_model(Arc::new(Zern));
    let (px, py) = pupil(RAYS);
    lensbench_core::ext::with(Arc::new(e), || {
        let mut g = c.benchmark_group("usershape");
        g.throughput(Throughput::Elements(RAYS as u64));
        for (name, user) in [("zernike_builtin", false), ("zernike_usershape", true)] {
            let s = a0(user);
            g.bench_function(name, |b| {
                lensbench_core::with_threads(Some(1), || {
                    b.iter(|| {
                        s.try_trace(FieldSel::Index(1), &px, &py, Some(s.primary), None, false)
                            .unwrap()
                    })
                })
            });
        }
        g.finish();
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = bench
}
criterion_main!(benches);
