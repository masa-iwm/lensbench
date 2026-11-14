//! MTF の時間（OpticStudio の表示と同じ: 全視野、多色＋各波長、T/S、遮断周波数の 0〜0.95 を 21 点）:
//! `cargo run --release --example mtf_bench -- [系 ...]`（系は tests/refs の名前、既定 A0 A6 D4 F3 C0）。
//! 環境変数 MTF_N で求積の点数を固定、MTF_FREQS で周波数の数（既定 21）、MTF_METHOD=fft・direct で求め方、
//! MTF_SEPARATE=1 で多色と各波長を別々に呼ぶ（[`analysis::mtf_waves`] を使わない、補間のみ）
use lensbench_core::Waves;
use std::path::PathBuf;
use std::time::Instant;

// 計測は exe と同じアロケータで
#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use lensbench_core::analysis::{self, MtfMethod};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::FieldSel;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        names = ["A0", "A6", "D4", "F3", "C0"].map(String::from).to_vec();
    }
    let fixed_n = std::env::var("MTF_N")
        .ok()
        .map(|v| v.parse::<usize>().unwrap());
    let separate = std::env::var("MTF_SEPARATE").is_ok();
    let nf: usize = std::env::var("MTF_FREQS").map_or(21, |v| v.parse().unwrap());
    let method = match std::env::var("MTF_METHOD").as_deref() {
        Ok("fft") => MtfMethod::Fft,
        Ok("direct") => MtfMethod::Direct,
        _ => MtfMethod::Interp,
    };
    for name in &names {
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap();
        let t0 = Instant::now();
        let mut ns = Vec::new();
        for fi in 0..s.fields.len() {
            let f = FieldSel::Index(fi);
            let Ok(m0) = analysis::mtf(&s, f, Waves::One(s.primary), Some(4), &[]) else {
                continue;
            };
            let c = m0.cutoff.0.min(m0.cutoff.1);
            let freqs: Vec<f64> = (0..nf)
                .map(|k| 0.95 * c * k as f64 / (nf - 1) as f64)
                .collect();
            for w in 0..s.wavelengths.len() {
                ns.push(analysis::mtf_points(
                    analysis::opd_slope_max(&s, f, w).unwrap(),
                ));
            }
            if separate {
                let _ = analysis::mtf(&s, f, Waves::All, fixed_n, &freqs);
                for w in 0..s.wavelengths.len() {
                    let _ = analysis::mtf(&s, f, Waves::One(w), fixed_n, &freqs);
                }
            } else {
                let _ = analysis::mtf_waves(&s, f, fixed_n, &freqs, method);
            }
        }
        println!("{name}: {:.3} s  n {:?}", t0.elapsed().as_secs_f64(), ns);
    }
}
