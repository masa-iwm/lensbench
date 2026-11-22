//! Huygens PSF の時間: `cargo run --release --example huygens_bench -- [系 ...]`（tests/refs の名前、既定 A0 D0）。
//! 瞳/像の格子 64/128・128/128・256/256、視野 1、主波長。準備（像の格子 2 の時間）、厳密（exact）・既定（位相の展開）・
//! GPU（`--features gpu`、CPU で動くアダプタでも使う）の時間と、厳密との強度の差
#![allow(clippy::unwrap_used, clippy::panic)] // 試験・例・組み立ての補助は panic で知らせる
use lensbench_core::Waves;
use std::path::PathBuf;
use std::time::Instant;

use lensbench_analysis::huygens::{gpu_name, huygens_psf, HuygensDevice, HuygensOpts};
use lensbench_core::FieldSel;
use lensbench_formats::zmx::{self, LoadOptions};

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    // exe の横に data/（tests/refs・tests/glass の写し）があればそこから（別の PC で測る zip）、無ければ作業ツリー
    let root = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("data")))
        .filter(|d| d.join("tests/refs").is_dir())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        names = ["A0", "D0"].map(String::from).to_vec();
    }
    println!(
        "GPU: {}",
        gpu_name().unwrap_or_else(|| "none (or a software adapter only)".into())
    );
    for name in &names {
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap();
        // 準備（光線追跡・光源）の目安: 像の格子 2 の時間
        let run = |pupil: usize, image: usize, exact: bool, device: HuygensDevice| {
            let t0 = Instant::now();
            let o = HuygensOpts {
                pupil: Some(pupil),
                image,
                pixel: None,
                exact,
                device,
            };
            let p = huygens_psf(&s, FieldSel::Index(0), Waves::One(s.primary), o).unwrap();
            (t0.elapsed().as_secs_f64(), p)
        };
        // GPU の装置を先に作る（初回だけの時間を測る時間に入れない）
        let _ = run(16, 2, false, HuygensDevice::Gpu);
        for (pupil, image) in [(64, 128), (128, 128), (256, 256)] {
            let (tp, _) = run(pupil, 2, false, HuygensDevice::Cpu);
            let (te, pe) = run(pupil, image, true, HuygensDevice::Cpu);
            let diff = |p: &lensbench_analysis::psf::Psf| {
                p.i.rows()
                    .flatten()
                    .zip(pe.i.rows().flatten())
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0, f64::max)
            };
            let (td, pd) = run(pupil, image, false, HuygensDevice::Cpu);
            // GPU は 1 回目（休んでいた GPU の立ち上がりを含む）と、続けて 3 回の最短
            let (tg, pg) = run(pupil, image, false, HuygensDevice::Gpu);
            let tg3 = (0..3)
                .map(|_| run(pupil, image, false, HuygensDevice::Gpu).0)
                .fold(f64::INFINITY, f64::min);
            println!(
                "{name} {pupil}/{image}: prep {tp:.3} s  exact {te:.3} s (peak {:.12})  default {td:.3} s (diff {:.2e})  gpu {tg:.3} s, best of 3 {tg3:.3} s (diff {:.2e}{})",
                pe.peak,
                diff(&pd),
                diff(&pg),
                if pg.warnings.iter().any(|w| w.contains("CPU")) { ", fell back to the CPU" } else { "" }
            );
        }
    }
}
