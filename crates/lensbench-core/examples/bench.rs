//! 光線追跡の速さ: `cargo run --release --example bench -- [zmx ...] [オプション]`
//! 既定は tests/refs の A0・C0（トリプレット、傾き偏心）を 20 万本ずつ。
//!
//! オプション:
//! - `--glass-dir D`  ガラスカタログの場所を足す（複数可。tests/glass より先に探す）
//! - `--threads N`    スレッド数（既定は全コア）
//! - `--rays N`       1 回の追跡の光線数（既定 200000）
//! - `--field I`      視野の番号（0 始まり、既定 1。視野が 1 つなら 0）
//! - `--secs S`       1 系あたりの計測時間 [s]（既定 1）
//! - `--record`       面ごとの交点も記録する追跡（record）を測る
//! - `--json`         1 系 1 行の JSON で出す（ほかのツールと比べるスクリプト向け）
//!
//! 瞳の点は xorshift64（種 12345）で単位円内に一様。同じ点列を別の実装でも作れるように、
//! 乱数は `pupil` の手順どおり（1 点につき r²・角の順に 2 回引く）。

use std::path::PathBuf;
use std::time::Instant;

// 計測は exe と同じアロケータで
#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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

struct Opts {
    rays: usize,
    field: Option<usize>,
    secs: f64,
    json: bool,
    record: bool,
}

fn bench(label: &str, s: &System, o: &Opts) {
    let (px, py) = pupil(o.rays);
    let fi = o.field.unwrap_or(1).min(s.fields.len() - 1);
    let f = FieldSel::Index(fi);
    // 初回の準備（エイミングの表など）を計測から外す
    let m = 100.min(o.rays);
    s.trace(f, &px[..m], &py[..m], Some(s.primary), None, o.record);
    let r = s.trace(f, &px, &py, Some(s.primary), None, o.record);
    let ok = (0..r.rays.len()).filter(|&i| r.ok(i)).count();
    let t0 = Instant::now();
    let mut k = 0;
    while k == 0 || t0.elapsed().as_secs_f64() < o.secs {
        std::hint::black_box(s.trace(f, &px, &py, Some(s.primary), None, o.record));
        k += 1;
    }
    let dt = t0.elapsed().as_secs_f64() / k as f64;
    let mray = o.rays as f64 / dt / 1e6;
    if o.json {
        println!(
            "{{\"name\":\"{}\",\"surfaces\":{},\"rays\":{},\"ok\":{},\"field\":{},\"threads\":{},\"reps\":{},\"sec_per_trace\":{:.6e},\"mray_s\":{:.4}}}",
            label.replace('\\', "\\\\").replace('"', "\\\""),
            s.surfaces.len(),
            o.rays,
            ok,
            fi,
            rayon::current_num_threads(),
            k,
            dt,
            mray
        );
    } else {
        println!("{label:<10} {mray:>6.2} Mray/s  ({} 面)", s.surfaces.len());
    }
}

fn take(args: &mut Vec<String>, key: &str) -> Option<String> {
    let i = args.iter().position(|a| a == key)?;
    assert!(i + 1 < args.len(), "{key} の値がない");
    let v = args.remove(i + 1);
    args.remove(i);
    Some(v)
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // --glass-dir を先に探す（tests/glass の同名の小さいカタログより優先）
    let mut dirs = Vec::new();
    while let Some(d) = take(&mut args, "--glass-dir") {
        dirs.push(PathBuf::from(d));
    }
    dirs.push(root.join("tests/glass"));
    if let Some(n) = take(&mut args, "--threads") {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n.parse().expect("--threads"))
            .build_global()
            .unwrap();
    }
    let o = Opts {
        rays: take(&mut args, "--rays").map_or(200_000, |v| v.parse().expect("--rays")),
        field: take(&mut args, "--field").map(|v| v.parse().expect("--field")),
        secs: take(&mut args, "--secs").map_or(1.0, |v| v.parse().expect("--secs")),
        json: args
            .iter()
            .position(|a| a == "--json")
            .map(|i| args.remove(i))
            .is_some(),
        record: args
            .iter()
            .position(|a| a == "--record")
            .map(|i| args.remove(i))
            .is_some(),
    };
    if let Some(a) = args.iter().find(|a| a.starts_with("--")) {
        panic!("知らないオプション {a}");
    }
    let files: Vec<PathBuf> = if args.is_empty() {
        ["A0", "C0"]
            .iter()
            .map(|n| root.join(format!("tests/refs/{n}/system.zmx")))
            .collect()
    } else {
        args.iter().map(PathBuf::from).collect()
    };
    if !o.json {
        println!("threads {}", rayon::current_num_threads());
    }
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
        bench(&name, &s, &o);
    }
}
