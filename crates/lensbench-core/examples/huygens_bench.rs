//! Huygens PSF の時間: `cargo run --release --example huygens_bench -- [系 ...]`（tests/refs の名前、既定 A0 D0）。
//! 瞳/像の格子 64/128・128/128・256/256、視野 1、主波長
use std::path::PathBuf;
use std::time::Instant;

use lensbench_core::huygens::{huygens_psf, HuygensOpts};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::FieldSel;

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
            let t0 = Instant::now();
            let o = HuygensOpts {
                pupil: Some(pupil),
                image,
                delta: None,
            };
            let p = huygens_psf(&s, FieldSel::Index(0), Some(s.primary), o).unwrap();
            println!(
                "{name} {pupil}/{image}: {:.3} s  peak {:.12}",
                t0.elapsed().as_secs_f64(),
                p.peak
            );
        }
    }
}
