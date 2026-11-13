#![allow(clippy::neg_cmp_op_on_partial_ord)]
//! 回折 MTF を OpticStudio 26.1 の FFT MTF（tests/refs/<系>/mtf.json、MTFT・MTFS 演算子の多色の値）と照合する。
//!
//! 基準は sampling 512（なければ 256）。OpticStudio 側も sampling を上げると値が動くので、許容差は系ごとに
//! 「2 × |最良の sampling − 1 つ前|」を 2.5e-3〜5e-3 に収めたもの（mtf.json の tolerance）。
//! skip_fields は照合しない視野（A6 の 4 番目: 口径食の角度 VANN で瞳が回っている視野。OpticStudio の
//! 縮尺は cos(VAN) に近いが根拠がないので合わせない）。A6v は A6 の口径食の係数をすべて 0 にした系。
//!
//! C0・C1・E0・E1（収差が大きい系）は視野ごとに方法を選んだ基準（fields[].method）: OPD の傾きが小さい視野
//! （E0・E1 の視野 1）だけ FFT MTF sampling 512、ほかは FFT が収束しない（傾き 70〜700 波）ので幾何 MTF ×
//! 回折限界 sampling 1024。幾何 MTF の最初の周波数は OS の多色の幾何 MTF が 0.01 ほど高く出る（sampling に
//! よらない、D4 の低い周波数と同じ挙動）ので FFT s512 の値に置き換えてある。許容差は 5e-3〜1e-2 に収めた値。
//! 多色の重みは A0 の重みを変えた系（mtf_weights.json）で別に確かめる。

use crate::common::{glass_dirs, refs_dir};
use lensbench_core::analysis;
use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use serde_json::Value;
use std::path::PathBuf;

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn load(name: &str) -> lensbench_core::system::System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: glass_dirs(),
        ..Default::default()
    };
    zmx::load_with(&refs_dir().join(name).join("system.zmx"), &opt).unwrap()
}

/// 基準 r（mtf.json の形）との差の最大
fn worst(s: &lensbench_core::system::System, r: &Value) -> f64 {
    let freqs = nums(&r["freqs"]);
    let skip: Vec<u64> = r["skip_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect();
    let mut worst: f64 = 0.0;
    for f in r["fields"].as_array().unwrap() {
        let fi = f["field"].as_u64().unwrap();
        if skip.contains(&fi) {
            continue;
        }
        let m = analysis::mtf(s, FieldSel::Index(fi as usize - 1), None, None, &freqs).unwrap();
        for (got, want) in [(&m.tan, nums(&f["T"])), (&m.sag, nums(&f["S"]))] {
            for (g, w) in got.iter().zip(&want) {
                worst = worst.max((g - w).abs());
            }
        }
    }
    worst
}

fn read(path: PathBuf) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn mtf_matches_opticstudio_fft_mtf() {
    let mut names: Vec<_> = std::fs::read_dir(refs_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("mtf.json").exists())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty());
    let mut bad = Vec::new();
    for name in &names {
        let r = read(refs_dir().join(name).join("mtf.json"));
        let tol = r["tolerance"].as_f64().unwrap();
        let w = worst(&load(name), &r);
        eprintln!("{name}: max |MTF − OpticStudio| = {w:.2e}（許容 {tol}）");
        if !(w <= tol) {
            bad.push(format!("{name} {w:.2e} > {tol}"));
        }
    }
    assert!(bad.is_empty(), "{bad:?}");
}

/// 多色の波長の重み: A0 の重みを 1・2・0.5 に変えた系の OpticStudio の値（tests/refs/A0/mtf_weights.json）
#[test]
fn polychromatic_mtf_uses_the_wavelength_weights() {
    let p = refs_dir().join("A0/mtf_weights.json");
    if !p.exists() {
        return;
    }
    let r = read(p);
    let mut s = load("A0");
    s.weights = nums(&r["weights"]);
    let tol = r["tolerance"].as_f64().unwrap();
    let w = worst(&s, &r);
    assert!(w <= tol, "{w:.2e} > {tol}");
    // 重みを 1 に戻すと合わなくなる（重みが効いていることの確かめ）
    s.weights = vec![1.0; s.weights.len()];
    assert!(worst(&s, &r) > 3.0 * tol);
}
