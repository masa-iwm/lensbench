//! 休んでいた GPU の 1 回目の Huygens PSF が遅い理由の切り分け: `cargo run --release --features gpu --example huygens_warmup -- [系]`
//! （tests/refs の名前、既定 A0）。瞳 256・像 256、視野 1、主波長、`device` は gpu。
//!
//! 1. 休む長さ（0〜5 s）を変えて 1 回ずつ: 休みが長いほど遅ければ GPU のクロックの立ち上がり
//! 2. 5 s 休んでから小さな計算（瞳 16・像 2）を投げ、すぐに本番: 予熱で消えるか
//! 3. 休まずに格子の大きさを入れ替えて続ける: 新しい大きさのバッファの初回の確保が効くか
//!
//! `LENSBENCH_GPU_TRACE` を付けると各回の段（バッファの準備・投入ごと・読み戻し）の時間も標準エラーに出る。
#![allow(clippy::unwrap_used, clippy::panic)] // 試験・例・組み立ての補助は panic で知らせる
use lensbench_core::Waves;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use lensbench_analysis::huygens::{gpu_name, huygens_psf, HuygensDevice, HuygensOpts};
use lensbench_core::FieldSel;
use lensbench_formats::zmx::{self, LoadOptions};

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    // exe の横に data/ があればそこから（別の PC で測る zip）、無ければ作業ツリー
    let root = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("data")))
        .filter(|d| d.join("tests/refs").is_dir())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let name = std::env::args().nth(1).unwrap_or_else(|| "A0".into());
    let t = Instant::now();
    let g = gpu_name();
    println!(
        "GPU: {} (found in {:.3} s)",
        g.unwrap_or_else(|| "none (or a software adapter only)".into()),
        t.elapsed().as_secs_f64()
    );
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    let s = zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap();
    let run = |pupil: usize, image: usize, device: HuygensDevice| -> f64 {
        let t0 = Instant::now();
        let o = HuygensOpts {
            pupil: Some(pupil),
            image,
            pixel: None,
            exact: false,
            device,
        };
        let p = huygens_psf(&s, FieldSel::Index(0), Waves::One(s.primary), o).unwrap();
        if device == HuygensDevice::Gpu && p.device.as_deref().is_none_or(|d| !d.starts_with("gpu"))
        {
            println!("  (computed on the CPU: {:?})", p.warnings);
        }
        t0.elapsed().as_secs_f64()
    };
    let gpu = HuygensDevice::Gpu;
    // 準備（光線追跡・光源）の目安と、装置・パイプラインを作る分
    let t_first = run(16, 2, gpu);
    let prep = (0..3)
        .map(|_| run(256, 2, HuygensDevice::Cpu))
        .fold(f64::INFINITY, f64::min);
    println!(
        "first tiny GPU call {t_first:.3} s; CPU preparation of 256/256 (image grid 2) {prep:.3} s"
    );
    let cpu = run(256, 256, HuygensDevice::Cpu);
    println!("CPU default 256/256 {cpu:.3} s");

    println!("== 1. idle time before one GPU 256/256 run (each after a warm run)");
    for idle in [0.0, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0] {
        run(256, 256, gpu);
        std::thread::sleep(Duration::from_secs_f64(idle));
        let t = run(256, 256, gpu);
        println!("idle {idle:>5.2} s: {t:.3} s");
    }

    println!("== 2. 5 s idle, then a tiny warm-up call, then 256/256");
    for k in 0..3 {
        run(256, 256, gpu);
        std::thread::sleep(Duration::from_secs(5));
        let tw = run(16, 2, gpu);
        let t = run(256, 256, gpu);
        println!("try {k}: warm-up {tw:.3} s, then 256/256 {t:.3} s");
    }
    println!("== 2b. 5 s idle, then a medium warm-up call (64/64), then 256/256");
    for k in 0..3 {
        run(256, 256, gpu);
        std::thread::sleep(Duration::from_secs(5));
        let tw = run(64, 64, gpu);
        let t = run(256, 256, gpu);
        println!("try {k}: warm-up {tw:.3} s, then 256/256 {t:.3} s");
    }

    println!("== 3. no idle, alternating grid sizes");
    for (p, i) in [
        (256, 256),
        (128, 128),
        (256, 256),
        (192, 256),
        (256, 256),
        (256, 192),
        (256, 256),
    ] {
        let t = run(p, i, gpu);
        println!("{p}/{i}: {t:.3} s");
    }
}
