//! MTF の補間（既定）と直接追跡の差と時間: `cargo run --release --example mtf_interp -- [系 ...]`
//! 系は tests/refs の名前（既定は全系）。周波数は mtf.json の周波数（なければ遮断周波数の 0〜0.9 を 10 点）
use lensbench_core::Waves;
use std::path::PathBuf;
use std::time::Instant;

use lensbench_core::analysis::{self, MtfMethod};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::FieldSel;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    let fixed_n = std::env::var("MTF_N")
        .ok()
        .map(|v| v.parse::<usize>().unwrap());
    if names.is_empty() {
        let mut v: Vec<String> = std::fs::read_dir(root.join("tests/refs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        names = v;
    }
    let (mut td, mut ti, mut worst) = (0.0, 0.0, 0.0f64);
    for name in &names {
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap();
        for fi in 0..s.fields.len() {
            let f = FieldSel::Index(fi);
            let Ok(m0) = analysis::mtf(&s, f, Waves::One(s.primary), Some(4), &[]) else {
                println!("{name} 視野 {fi}: 遮断周波数が求まらない");
                continue;
            };
            let c = m0.cutoff.0.min(m0.cutoff.1);
            let freqs: Vec<f64> = (0..10).map(|k| c * 0.09 * k as f64).collect();
            let t0 = Instant::now();
            let d = analysis::mtf_with(&s, f, Waves::All, fixed_n, &freqs, MtfMethod::Direct);
            let t1 = Instant::now();
            let i = analysis::mtf_with(&s, f, Waves::All, fixed_n, &freqs, MtfMethod::Interp);
            let t2 = Instant::now();
            match (d, i) {
                (Ok(d), Ok(i)) => {
                    let e = d
                        .tan
                        .iter()
                        .zip(&i.tan)
                        .chain(d.sag.iter().zip(&i.sag))
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0, f64::max);
                    worst = worst.max(e);
                    let (a, b) = ((t1 - t0).as_secs_f64(), (t2 - t1).as_secs_f64());
                    td += a;
                    ti += b;
                    println!(
                        "{name} 視野 {fi}: 差 {e:.2e}  直接 {:.3} s  補間 {:.3} s ({:.1} 倍)",
                        a,
                        b,
                        a / b
                    );
                }
                (d, i) => println!("{name} 視野 {fi}: {:?} / {:?}", d.err(), i.err()),
            }
        }
    }
    println!(
        "最大の差 {worst:.2e}  合計 直接 {td:.2} s  補間 {ti:.2} s ({:.1} 倍)",
        td / ti
    );
}
