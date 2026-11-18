//! Huygens PSF の時間: `cargo run --release --example huygens_bench -- [系 ...]`（tests/refs の名前、既定 A0 D0）。
//! 瞳/像の格子 64/128・128/128・256/256、視野 1、主波長。厳密（exact）と既定（位相の展開）の時間と強度の差
#![allow(clippy::unwrap_used, clippy::panic)] // 試験・例・組み立ての補助は panic で知らせる
use lensbench_core::Waves;
use std::path::PathBuf;
use std::time::Instant;

use lensbench_analysis::huygens::{huygens_psf, HuygensOpts};
use lensbench_core::FieldSel;
use lensbench_formats::zmx::{self, LoadOptions};

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        names = ["A0", "D0"].map(String::from).to_vec();
    }
    for name in &names {
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap();
        for (pupil, image) in [(64, 128), (128, 128), (256, 256)] {
            let run = |exact: bool| {
                let t0 = Instant::now();
                let o = HuygensOpts {
                    pupil: Some(pupil),
                    image,
                    pixel: None,
                    exact,
                };
                let p = huygens_psf(&s, FieldSel::Index(0), Waves::One(s.primary), o).unwrap();
                (t0.elapsed().as_secs_f64(), p)
            };
            let ((t, p), (te, pe)) = (run(true), run(false));
            let diff =
                p.i.rows()
                    .flatten()
                    .zip(pe.i.rows().flatten())
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0, f64::max);
            println!(
                "{name} {pupil}/{image}: {t:.3} s  peak {:.12}  default {te:.3} s  max diff {diff:.2e}",
                p.peak
            );
        }
    }
}
