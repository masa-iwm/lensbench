//! 光線追跡の速さ: `cargo run --release --example bench [zmx] [--glass-dir D] [--threads N]`
//! 既定は tests/refs の A0・C0（トリプレット、傾き偏心）を 20 万本ずつ。

use std::path::PathBuf;
use std::time::Instant;

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

fn bench(label: &str, s: &System, f: FieldSel, n: usize) {
    let (px, py) = pupil(n);
    s.trace(f, &px[..100], &py[..100], Some(s.primary), None, false);
    let t0 = Instant::now();
    let mut k = 0;
    while t0.elapsed().as_secs_f64() < 1.0 {
        std::hint::black_box(s.trace(f, &px, &py, Some(s.primary), None, false));
        k += 1;
    }
    let dt = t0.elapsed().as_secs_f64() / k as f64;
    println!(
        "{label:<10} {:>6.2} Mray/s  ({} 面)",
        n as f64 / dt / 1e6,
        s.surfaces.len()
    );
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dirs = vec![root.join("tests/glass")];
    if let Some(i) = args.iter().position(|a| a == "--glass-dir") {
        dirs.push(PathBuf::from(args.remove(i + 1)));
        args.remove(i);
    }
    if let Some(i) = args.iter().position(|a| a == "--threads") {
        let n: usize = args.remove(i + 1).parse().unwrap();
        args.remove(i);
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .unwrap();
    }
    let files: Vec<PathBuf> = if args.is_empty() {
        ["A0", "C0"]
            .iter()
            .map(|n| root.join(format!("tests/refs/{n}/system.zmx")))
            .collect()
    } else {
        args.iter().map(PathBuf::from).collect()
    };
    println!("threads {}", rayon::current_num_threads());
    for f in &files {
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: dirs.clone(),
            ..Default::default()
        };
        let s = zmx::load_with(f, &opt).unwrap();
        let name = f
            .parent()
            .and_then(|p| p.file_name())
            .unwrap()
            .to_string_lossy();
        bench(
            &name,
            &s,
            FieldSel::Index(1.min(s.fields.len() - 1)),
            200_000,
        );
    }
}
