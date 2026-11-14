//! MTF の FFT（[`MtfMethod::Fft`]）と補間（既定）の比べ: OpticStudio の値（tests/refs/<系>/mtf.json）との差、
//! 2 つの方法の差、時間。`cargo run --release --example mtf_fft -- [系 ...]`（既定は mtf.json のある全系）
use lensbench_core::Waves;
use std::path::PathBuf;
use std::time::Instant;

use lensbench_core::analysis::{self, MtfMethod};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::FieldSel;
use serde_json::Value;

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap_or(f64::NAN))
        .collect()
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        let mut v: Vec<String> = std::fs::read_dir(root.join("tests/refs"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| root.join(format!("tests/refs/{n}/mtf.json")).exists())
            .collect();
        v.sort();
        names = v;
    }
    let (mut ti, mut tf) = (0.0, 0.0);
    for name in &names {
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        };
        let s = zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap();
        // mtf.json がなければ全視野、遮断周波数の 0〜0.95 の 21 点で 2 つの方法だけを比べる
        let r: Value = match std::fs::read_to_string(
            root.join(format!("tests/refs/{name}/mtf.json")),
        ) {
            Ok(t) => serde_json::from_str(&t).unwrap(),
            Err(_) => {
                let m0 = analysis::mtf(&s, FieldSel::Index(0), Waves::One(s.primary), Some(4), &[])
                    .unwrap();
                let c = m0.cutoff.0.min(m0.cutoff.1);
                let freqs: Vec<f64> = (0..=20).map(|k| 0.95 * c * k as f64 / 20.0).collect();
                let nan = vec![f64::NAN; freqs.len()];
                let fields: Vec<Value> = (1..=s.fields.len())
                    .map(|i| serde_json::json!({"field": i, "T": nan, "S": nan}))
                    .collect();
                serde_json::json!({"freqs": freqs, "skip_fields": [], "tolerance": f64::NAN, "fields": fields})
            }
        };
        let freqs = nums(&r["freqs"]);
        let skip: Vec<u64> = r["skip_fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .collect();
        let tol = r["tolerance"].as_f64().unwrap_or(f64::NAN);
        let (mut ei, mut ef, mut d) = (0.0f64, 0.0f64, 0.0f64);
        let (mut a, mut b) = (0.0, 0.0);
        for f in r["fields"].as_array().unwrap() {
            let fi = f["field"].as_u64().unwrap();
            if skip.contains(&fi) {
                continue;
            }
            let fs = FieldSel::Index(fi as usize - 1);
            let t0 = Instant::now();
            let mi =
                analysis::mtf_with(&s, fs, Waves::All, None, &freqs, MtfMethod::Interp).unwrap();
            let t1 = Instant::now();
            let mf = analysis::mtf_with(&s, fs, Waves::All, None, &freqs, MtfMethod::Fft).unwrap();
            let t2 = Instant::now();
            a += (t1 - t0).as_secs_f64();
            b += (t2 - t1).as_secs_f64();
            for (gi, gf, w) in [
                (&mi.tan, &mf.tan, nums(&f["T"])),
                (&mi.sag, &mf.sag, nums(&f["S"])),
            ] {
                for k in 0..w.len() {
                    if w[k].is_finite() {
                        ei = ei.max((gi[k] - w[k]).abs());
                        ef = ef.max((gf[k] - w[k]).abs());
                    }
                    d = d.max((gi[k] - gf[k]).abs());
                }
            }
        }
        ti += a;
        tf += b;
        println!(
            "{name}: OS との差 補間 {ei:.2e} FFT {ef:.2e}（許容 {tol}）{}  補間−FFT {d:.2e}  時間 補間 {a:.3} s FFT {b:.3} s",
            if ef <= tol { "" } else { " ×" }
        );
    }
    println!("合計 補間 {ti:.2} s  FFT {tf:.2} s");
}
