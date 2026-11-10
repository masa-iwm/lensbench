//! 解析ごとの時間（プロファイル用）: `cargo run --release --example profile -- [--threads N] [--glass-dir D] [系 ...]`
//! 系は tests/refs の名前か .zmx のパス（既定 A0 A2 C0 D4）。環境変数 SKIP_AUTO があれば多色・点数自動の MTF を飛ばす
use std::path::PathBuf;
use std::time::Instant;

// 計測は exe と同じアロケータで
#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use lensbench_core::analysis::{self, ZernKind};
use lensbench_core::coating::Coatings;
use lensbench_core::coating::C64;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{FieldSel, System};

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

fn time<R>(label: &str, f: impl Fn() -> R) {
    f();
    let t0 = Instant::now();
    let mut k = 0;
    while k == 0 || t0.elapsed().as_secs_f64() < 0.5 {
        std::hint::black_box(f());
        k += 1;
    }
    let dt = t0.elapsed().as_secs_f64() / k as f64;
    println!("  {label:<28} {:>10.3} ms", dt * 1e3);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut threads = None;
    let mut glass_dirs: Vec<PathBuf> = Vec::new();
    let mut names = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--glass-dir" {
            glass_dirs.push(PathBuf::from(&args[i + 1]));
            i += 2;
        } else if args[i] == "--threads" {
            threads = Some(args[i + 1].parse().unwrap());
            i += 2;
        } else {
            names.push(args[i].clone());
            i += 1;
        }
    }
    if names.is_empty() {
        names = ["A0", "A2", "C0", "D4"].map(String::from).to_vec();
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    lensbench_core::with_threads(threads, || {
        for name in &names {
            let p = if name.ends_with(".zmx") {
                PathBuf::from(name)
            } else {
                root.join(format!("tests/refs/{name}/system.zmx"))
            };
            let opt = LoadOptions {
                cfg: 1,
                catalog_dirs: glass_dirs
                    .iter()
                    .cloned()
                    .chain([root.join("tests/glass")])
                    .collect(),
                ..Default::default()
            };
            let s: System = zmx::load_with(&p, &opt).unwrap();
            println!("{name} ({} 面, aiming {:?})", s.surfaces.len(), s.aiming);
            let f = FieldSel::Index(1.min(s.fields.len() - 1));
            time("load", || zmx::load_with(&p, &opt).unwrap());
            let (px, py) = pupil(20000);
            time("trace 20k", || s.trace(f, &px, &py, None, None, false));
            time("trace 20k record", || {
                s.trace(f, &px, &py, None, None, true)
            });
            let img = s.image();
            time("trace 20k record (像面のみ)", || {
                s.try_trace_rec(
                    f,
                    &px,
                    &py,
                    None,
                    None,
                    lensbench_core::RecSel::Surfs(&[img]),
                )
                .unwrap()
            });
            time("trace 1 (chief)", || {
                s.trace(f, &[0.0], &[0.0], None, None, false)
            });
            time("spot (rings 6)", || analysis::spot(&s, f, None, 6).unwrap());
            time("wavefront 64", || {
                analysis::wavefront(&s, f, None, 64, false).unwrap()
            });
            time("wavefront 256", || {
                analysis::wavefront(&s, f, None, 256, false).unwrap()
            });
            time("zernike 37 / 64", || {
                analysis::zernike(&s, f, None, ZernKind::Standard, 37, 64, false).unwrap()
            });
            time("zernike 37 / 256", || {
                analysis::zernike(&s, f, None, ZernKind::Standard, 37, 256, false).unwrap()
            });
            let freqs: Vec<f64> = (0..=10).map(|k| k as f64 * 10.0).collect();
            time("opd_slope_max", || {
                analysis::opd_slope_max(&s, f, s.primary).unwrap()
            });
            if std::env::var("SKIP_AUTO").is_err() {
                time("mtf poly auto, 11 freq", || {
                    analysis::mtf(&s, f, None, None, &freqs)
                });
            }
            time("mtf mono n=64, 11 freq", || {
                analysis::mtf(&s, f, Some(s.primary), Some(64), &freqs)
            });
            let c = Coatings::for_system(&s).unwrap_or_default();
            let pols = [
                [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
                [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
            ];
            if s.try_trace_pol(f, &px[..1], &py[..1], None, &pols, &c)
                .is_ok()
            {
                time("trace_pol 20k x2", || {
                    s.try_trace_pol(f, &px, &py, None, &pols, &c).unwrap()
                });
            }
            let ps: Vec<_> = (1..s.image())
                .filter_map(|j| s.param(&format!("{j}.t")).ok())
                .collect();
            if s.surfaces
                .iter()
                .any(|x| x.typ == lensbench_core::surface::SurfType::Paraxial)
            {
                continue;
            }
            time(&format!("trace_grad 20k ({} p)", ps.len()), || {
                s.trace_grad(f, &px, &py, None, None, &ps).unwrap()
            });
        }
    });
}
