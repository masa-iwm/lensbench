#![allow(clippy::neg_cmp_op_on_partial_ord)]
//! Huygens PSF・Huygens MTFを OpticStudio 26.1 と照合する（tests/huygens_refs）。
//!
//! 基準と許容差（理由）
//! - Huygens PSF: OS の値は瞳の標本で動き、瞳を増やすと lensbench に近づく（中心の格子の最大差 / OS のピークが
//!   D0 で瞳 64・128・256 = 0.023・0.006・0.003、A0 で 0.11・0.03・0.013）。lensbench は瞳 64→256 で 1e-4 しか
//!   動かない。そこで瞳 256 の OS を基準にし、許容差は D0 5e-3、A0 2.5e-2（OS の 128→256 の差より小さい）。
//!   像面を傾けた系（D0 25°・A0 10°）は瞳 64 の軸上だけ（軸外は傾きで大きくピントがずれ、OS が瞳 64 で収束しない）。
//!   瞳 64 の OS は傾けない系でも D0 0.023・A0 0.06 動くので、許容差は D0 1.5e-2（実際 1.1e-2）、A0 7.5e-2（実際 6.3e-2）。
//!   OS の格子は x = (i − N/2)Δ、強度は収差なしのピークを 1（Normalize なし、ピークがストレール比）で lensbench と同じ。
//! - Huygens MTF: OS の自動の像の間隔は結果に出ず、像の格子が狭いので低周波が高く出る（D0 で回折限界より 0.015 高い）。
//!   間隔を 0.5 µm に固めた D0（瞳 128・像 256）を基準に 5e-3（実際 3e-3）。A0 は OS の瞳 128 が PSF で 0.03 動く程度に
//!   未収束なので使わない。
//! - FFT PSF（主光線に垂直な面）より Huygens（像面の上）の方が OS の Huygens に近い（A6 の VANN の視野で 0.1 対 0.7）。

use crate::common::root;
use lensbench_core::huygens::{huygens_mtf, huygens_psf, HuygensOpts};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::Waves;
use ndarray::Array2;
use ndarray_npy::NpzReader;
use serde_json::Value;

fn load(rel: &str) -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root().join(rel), &opt).unwrap()
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn wave(v: &Value) -> Option<usize> {
    v.as_u64().map(|w| w as usize - 1)
}

fn cases() -> Vec<Value> {
    let p = root().join("tests/huygens_refs/cases.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    v["items"].as_array().unwrap().clone()
}

/// 許容差（OS の瞳の標本の収束から、モジュールの説明）
fn tol_psf(sys: &str, pupil: u64) -> f64 {
    match (sys.contains("D0"), pupil) {
        (true, 256) => 5e-3,
        (true, _) => 1.5e-2,
        (false, 256) => 2.5e-2,
        (false, _) => 7.5e-2,
    }
}

#[test]
fn huygens_psf_matches_opticstudio() {
    let mut npz = NpzReader::new(
        std::fs::File::open(root().join("tests/huygens_refs/huygens_os.npz")).unwrap(),
    )
    .unwrap();
    let mut bad = Vec::new();
    let mut n = 0;
    for it in cases().iter().filter(|c| c["kind"] == "psf") {
        let sys = it["system"].as_str().unwrap();
        let s = load(sys);
        let field = it["field"].as_u64().unwrap() as usize;
        let pupil = it["pupil"].as_u64().unwrap();
        let want: Array2<f32> = npz.by_name(it["key"].as_str().unwrap()).unwrap();
        // 基準は OS の格子の中心の切り出し（OS の (i − N/2)Δ の i = N/2 − C 〜 N/2 + C − 1）なので、
        // lensbench は一辺 2C の格子（(m − C)Δ）で同じ点だけ計算する
        let (ny, nx) = want.dim();
        assert_eq!(ny, nx);
        let c0 = it["crop_start"].as_u64().unwrap() as usize;
        assert_eq!(c0 + nx / 2, it["image"].as_u64().unwrap() as usize / 2);
        let o = HuygensOpts {
            pupil: Some(pupil as usize),
            image: nx,
            delta: Some(it["delta_um"].as_f64().unwrap() * 1e-3),
        };
        let p = huygens_psf(
            &s,
            FieldSel::Index(field - 1),
            Waves::all_or(wave(&it["wave"])),
            o,
        )
        .unwrap();
        let peak = it["os_peak"].as_f64().unwrap();
        let mut e: f64 = 0.0;
        for ((iy, ix), w) in want.indexed_iter() {
            e = e.max((p.i[iy][ix] - *w as f64).abs());
        }
        let e = e / peak;
        let tol = tol_psf(sys, pupil);
        eprintln!("{sys} 視野 {field} 波長 {} 瞳 {pupil}: {e:.2e}", it["wave"]);
        if !(e <= tol) {
            bad.push(format!(
                "{sys} 視野 {field} 波長 {}: {e:.2e} > {tol:.1e}",
                it["wave"]
            ));
        }
        n += 1;
    }
    assert!(n >= 20, "基準が少ない（{n}）");
    assert!(bad.is_empty(), "Huygens PSF:\n{}", bad.join("\n"));
}

#[test]
fn huygens_mtf_matches_opticstudio() {
    let mut bad = Vec::new();
    let mut n = 0;
    for it in cases().iter().filter(|c| c["kind"] == "mtf") {
        let sys = it["system"].as_str().unwrap();
        let s = load(sys);
        let field = it["field"].as_u64().unwrap() as usize;
        // D0 は視野によらずほぼ同じ値なので、時間を減らすため軸上と視野 3（10°）だけ
        if field != 1 && field != 3 {
            continue;
        }
        let o = HuygensOpts {
            pupil: Some(it["pupil"].as_u64().unwrap() as usize),
            image: it["image"].as_u64().unwrap() as usize,
            delta: Some(it["delta_um"].as_f64().unwrap() * 1e-3),
        };
        let fs = nums(&it["freqs"]);
        let m = huygens_mtf(
            &s,
            FieldSel::Index(field - 1),
            Waves::all_or(wave(&it["wave"])),
            o,
            &fs,
        )
        .unwrap();
        let d = |a: &[f64], b: &[f64]| {
            a.iter()
                .zip(b)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f64::max)
        };
        let e = d(&m.tan, &nums(&it["T"])).max(d(&m.sag, &nums(&it["S"])));
        eprintln!("{sys} 視野 {field} 波長 {}: {e:.2e}", it["wave"]);
        if !(e <= 5e-3) {
            bad.push(format!("{sys} 視野 {field} 波長 {}: {e:.2e}", it["wave"]));
        }
        n += 1;
    }
    assert!(n >= 4, "基準が少ない（{n}）");
    assert!(bad.is_empty(), "Huygens MTF:\n{}", bad.join("\n"));
}
