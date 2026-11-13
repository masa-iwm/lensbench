#![allow(clippy::neg_cmp_op_on_partial_ord)]
//! スルーフォーカス MTF・MTF vs 視野・RMS の各種・FFT PSF・幾何 MTFを
//! OpticStudio 26.1 と照合する（tests/refs/<系>/mtf_family.json・psf_os.npz）。
//!
//! 許容差（理由）
//! - RMS: スポットは相対 1e-6、波面は相対 5e-5 + 2e-5 波（収差のない D 系で OS は 1e-5 波ほどの値を返す）、Strehl は 1e-4（Gauss 求積は OS と同じ点・重み）。
//!   A3（視野が経緯儀の角度）は視野の掃引の定義が違うので外す（MTF vs 視野も）。RMS vs Focus の多色 Strehl は OS が第 1 波長の
//!   値を返すので外す（lensbench は波長ごとの Strehl の重み付き平均）。OS の単色の視野・波長の掃引は実像高の
//!   視野を解析の波長の主光線で解く（A5。スポット・演算子・多色は主波長）ので、RmsOpts::field_wave で合わせる。
//! - スルーフォーカス MTF: 5e-3、最高周波数（遮断の 0.5）は 1.5e-2。OS 自身のスルーフォーカスと MTF vs 視野の
//!   値がピントの位置で最大 1.1e-2 違う（B4）ので、OS の sampling 256 の精度がこの程度。
//! - MTF vs 視野: OS は 0〜1 の等間隔 20 点の値を自然 3 次スプラインで補間して出すので、同じ 20 点の値を
//!   スプラインで補間して比べる。5e-3。
//! - 幾何 MTF: 7.5e-3（多くは 3e-3 以下）。
//! - FFT PSF: 瞳 128 の中心 32×32 を最大比で、max(0.03, OS の瞳 64 と 128 の差)。OS の PSF は瞳の標本化で
//!   動き（瞳 64→128 で 2〜8%）、lensbench との差はそれより小さい。
//! - D4（収差が大きく MTF の照合でも OS の sampling で値が動いた系）は MTF 系の許容差を 1.5 倍、幾何 MTF は外す
//!   （OS の多色の幾何 MTF は低い周波数で振れ、高い周波数（焦線）でも合わない）。D4 の FFT PSF は瞳 512・像 1024
//!   で取り直した基準（os_s64_vs_s128 の中身は瞳 256 と 512 の差）で照合し、視野 3 だけ外す（PSF が像の格子
//!   約 210 µm からはみ出すほど広く、和の正規化が合いきらない。差は 0.04〜0.08）。
//! - A6 の視野 4 は口径食の角度 VANN で瞳が回っている。OS の縮尺は cos(VAN) に近いが 15°〜30° でしか成り立たず、
//!   根拠がないので合わせない（で決定）。A6 の視野 4 は外す。
//! - A6 の MTF vs 視野は OS が VDY・VCY を視野の間で補間して使うため端（h > 0.75）で最大 0.05 違うので外し、
//!   口径食の係数をすべて 0 にした A6v で照合する（RMS vs 視野は A6 でも合う）。

use lensbench_core::psf;
use lensbench_core::sweep::{self, FieldAxis, RmsOpts, RmsRef};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx::{self, LoadOptions};
use ndarray::Array2;
use ndarray_npy::NpzReader;
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn refs_dir() -> PathBuf {
    std::env::var_os("LB_REFS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("tests/refs"))
}

fn load(name: &str) -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&refs_dir().join(name).join("system.zmx"), &opt).unwrap()
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap_or(f64::NAN))
        .collect()
}

fn wave(v: &Value) -> Option<usize> {
    v.as_u64().map(|w| w as usize - 1)
}

/// 照合する系（mtf_family.json があるもの）と基準
fn cases() -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = std::fs::read_dir(refs_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("mtf_family.json").exists())
        .map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let p = e.path().join("mtf_family.json");
            (
                n,
                serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
            )
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!v.is_empty());
    v
}

/// MTF 系の許容差の倍率
fn mtf_scale(name: &str) -> f64 {
    if name == "D4" {
        1.5
    } else {
        1.0
    }
}

/// 差の最大（両方有限の点だけ。lensbench だけ NaN なら無限大）
fn max_diff(got: &[f64], want: &[f64]) -> f64 {
    let mut m: f64 = 0.0;
    for (g, w) in got.iter().zip(want) {
        if !w.is_finite() {
            continue;
        }
        m = m.max(if g.is_finite() {
            (g - w).abs()
        } else {
            f64::INFINITY
        });
    }
    m
}

fn report(what: &str, bad: Vec<String>) {
    assert!(bad.is_empty(), "{what}: {bad:#?}");
}

#[test]
fn through_focus_mtf_matches_opticstudio() {
    let mut bad = Vec::new();
    for (name, r) in cases() {
        let s = load(&name);
        let fr = nums(&r["frequencies"]);
        let mut worst: f64 = 0.0;
        for it in r["through_focus"].as_array().unwrap() {
            let field = it["field"].as_u64().unwrap() as usize;
            if name == "A6" && field == 4 {
                continue;
            }
            let k = it["freq"].as_u64().unwrap() as usize;
            let tol = if k == 3 { 1.5e-2 } else { 5e-3 } * mtf_scale(&name);
            let dz = nums(&it["dz"]);
            let m = sweep::through_focus_mtf(
                &s,
                FieldSel::Index(field - 1),
                wave(&it["wave"]),
                None,
                &[fr[k - 1]],
                &dz,
            );
            let e = max_diff(&m.tan[0], &nums(&it["T"])).max(max_diff(&m.sag[0], &nums(&it["S"])));
            worst = worst.max(e);
            if !(e <= tol) {
                bad.push(format!(
                    "{name} 視野 {field} 周波数 {k} 波長 {}: {e:.2e} > {tol:.1e}",
                    it["wave"]
                ));
            }
        }
        eprintln!("{name}: スルーフォーカス MTF の差 最大 {worst:.2e}");
    }
    report("スルーフォーカス MTF", bad);
}

/// 自然 3 次スプライン（節 x の値 y）の点 t の値
fn natural_spline(x: &[f64], y: &[f64], t: f64) -> f64 {
    let n = x.len();
    // 2 次の係数 m（m0 = m_{n-1} = 0）を三重対角で解く
    let mut a = vec![0.0; n];
    let mut b = vec![1.0; n];
    let mut c = vec![0.0; n];
    let mut d = vec![0.0; n];
    for i in 1..n - 1 {
        let (h0, h1) = (x[i] - x[i - 1], x[i + 1] - x[i]);
        a[i] = h0;
        b[i] = 2.0 * (h0 + h1);
        c[i] = h1;
        d[i] = 6.0 * ((y[i + 1] - y[i]) / h1 - (y[i] - y[i - 1]) / h0);
    }
    for i in 1..n {
        let w = a[i] / b[i - 1];
        b[i] -= w * c[i - 1];
        d[i] -= w * d[i - 1];
    }
    let mut m = vec![0.0; n];
    for i in (0..n).rev() {
        let next = if i + 1 < n { c[i] * m[i + 1] } else { 0.0 };
        m[i] = (d[i] - next) / b[i];
    }
    let i = (0..n - 1).find(|&i| t <= x[i + 1]).unwrap_or(n - 2);
    let h = x[i + 1] - x[i];
    let (u, v) = ((x[i + 1] - t) / h, (t - x[i]) / h);
    u * y[i] + v * y[i + 1] + ((u * u * u - u) * m[i] + (v * v * v - v) * m[i + 1]) * h * h / 6.0
}

#[test]
fn natural_spline_reproduces_a_line_and_its_nodes() {
    let x = [0.0, 0.3, 0.5, 1.0];
    let y: Vec<f64> = x.iter().map(|v| 2.0 * v + 1.0).collect();
    assert!((natural_spline(&x, &y, 0.77) - 2.54).abs() < 1e-12);
    let y2 = [0.0, 1.0, -1.0, 0.5];
    for (xi, yi) in x.iter().zip(y2) {
        assert!((natural_spline(&x, &y2, *xi) - yi).abs() < 1e-12);
    }
}

#[test]
fn mtf_vs_field_matches_opticstudio() {
    let mut bad = Vec::new();
    for (name, r) in cases() {
        if name == "A6" || name == "A3" {
            continue;
        }
        let s = load(&name);
        let fm = &r["mtf_vs_field"];
        let nn = fm["nodes"].as_u64().unwrap() as usize;
        let nodes = sweep::steps01(nn);
        let tol = 5e-3 * mtf_scale(&name);
        let mut worst: f64 = 0.0;
        for it in fm["series"].as_array().unwrap() {
            let f = it["freq"].as_f64().unwrap();
            let m = sweep::field_mtf(&s, FieldAxis::Y, &nodes, wave(&it["wave"]), None, &[f]);
            let h = nums(&it["h"]);
            for (g, key) in [(&m.tan[0], "T"), (&m.sag[0], "S")] {
                let gi: Vec<f64> = h.iter().map(|&t| natural_spline(&nodes, g, t)).collect();
                let e = max_diff(&gi, &nums(&it[key]));
                worst = worst.max(e);
                if !(e <= tol) {
                    bad.push(format!(
                        "{name} {f:.1} {key} 波長 {}: {e:.2e} > {tol:.1e}",
                        it["wave"]
                    ));
                }
            }
        }
        eprintln!("{name}: MTF vs 視野の差 最大 {worst:.2e}");
    }
    report("MTF vs 視野", bad);
}

#[test]
fn rms_family_matches_opticstudio() {
    let mut bad = Vec::new();
    for (name, r) in cases() {
        let s = load(&name);
        let nw = s.wavelengths.len();
        let ny = FieldAxis::Y.value(&s, 1.0);
        let mut worst = [0.0f64; 3];
        for it in r["rms"].as_array().unwrap() {
            let vs = it["vs"].as_str().unwrap();
            let data = it["data"].as_str().unwrap();
            let o = RmsOpts {
                rings: 6,
                reference: if it["reference"] == "chiefray" {
                    RmsRef::Chief
                } else {
                    RmsRef::Centroid
                },
                // OS の単色の視野・波長の掃引は、実像高の視野を解析の波長で解く（A5）
                field_wave: true,
            };
            let x = nums(&it["x"]);
            let w = wave(&it["wave"]);
            let cols = it["columns"].as_object().unwrap();
            for (col, want) in cols {
                let want = nums(want);
                // 列の名前（JSON の対象は名前順になるので名前で）: 視野の掃引は Poly か波長 [µm]（単色）、
                // フォーカス・波長の掃引は「Field  k」
                let ci = col
                    .strip_prefix("Field")
                    .map(|k| k.trim().parse::<usize>().unwrap() - 1)
                    .unwrap_or(0);
                let sw = match vs {
                    "field" => {
                        if name == "A3" {
                            continue;
                        }
                        let cw = if col == "Poly" {
                            None
                        } else {
                            let l: f64 = col.parse().unwrap();
                            let c = (0..nw).find(|&i| (s.wavelengths[i] - l).abs() < 1e-4);
                            assert!(c.is_some() && (w.is_none() || w == c), "{name} {col}");
                            c
                        };
                        let h: Vec<f64> = x.iter().map(|v| v / ny).collect();
                        sweep::rms_vs_field(&s, FieldAxis::Y, &h, cw, o)
                    }
                    "focus" => {
                        if data == "StrehlRatio" && w.is_none() {
                            continue;
                        }
                        sweep::rms_vs_focus(&s, FieldSel::Index(ci), &x, w, o)
                    }
                    "wavelength" => sweep::rms_vs_wavelength(&s, FieldSel::Index(ci), &x, o),
                    _ => panic!("{vs}"),
                };
                let (got, rel, abs, k): (Vec<f64>, f64, f64, usize) = match data {
                    "SpotRadius" => (sw.rms.iter().map(|q| q.spot * 1e3).collect(), 1e-6, 1e-6, 0),
                    "SpotX" => (
                        sw.rms.iter().map(|q| q.spot_x * 1e3).collect(),
                        1e-6,
                        1e-6,
                        0,
                    ),
                    "SpotY" => (
                        sw.rms.iter().map(|q| q.spot_y * 1e3).collect(),
                        1e-6,
                        1e-6,
                        0,
                    ),
                    "Wavefront" => (sw.rms.iter().map(|q| q.wfe).collect(), 5e-5, 2e-5, 1),
                    "StrehlRatio" => (sw.rms.iter().map(|q| q.strehl).collect(), 0.0, 1e-4, 2),
                    _ => panic!("{data}"),
                };
                for (i, (g, v)) in got.iter().zip(&want).enumerate() {
                    if !v.is_finite() {
                        continue;
                    }
                    let e = (g - v).abs();
                    let t = rel * v.abs() + abs;
                    worst[k] = worst[k].max(e / t);
                    if !(e <= t) {
                        bad.push(format!(
                            "{name} {vs} {} {data} {col} x={}: {g} vs {v}",
                            it["reference"], x[i]
                        ));
                    }
                }
            }
        }
        eprintln!(
            "{name}: RMS の差 / 許容差 最大 スポット {:.2} 波面 {:.2} Strehl {:.2}",
            worst[0], worst[1], worst[2]
        );
    }
    report("RMS", bad);
}

#[test]
fn geometric_mtf_matches_opticstudio() {
    let mut bad = Vec::new();
    for (name, r) in cases() {
        if name == "D4" {
            continue;
        }
        let s = load(&name);
        let mut worst: f64 = 0.0;
        for it in r["geometric_mtf"].as_array().unwrap() {
            let field = it["field"].as_u64().unwrap() as usize;
            if name == "A6" && field == 4 {
                continue;
            }
            let freqs = nums(&it["freqs"]);
            let dl = it["diff_limit"].as_bool().unwrap();
            let m = psf::geometric_mtf(
                &s,
                FieldSel::Index(field - 1),
                wave(&it["wave"]),
                128,
                &freqs,
                dl,
            )
            .unwrap();
            let e = max_diff(&m.tan, &nums(&it["T"])).max(max_diff(&m.sag, &nums(&it["S"])));
            worst = worst.max(e);
            if !(e <= 7.5e-3) {
                bad.push(format!(
                    "{name} 視野 {field} 波長 {} 回折限界 {dl}: {e:.2e}",
                    it["wave"]
                ));
            }
        }
        eprintln!("{name}: 幾何 MTF の差 最大 {worst:.2e}");
    }
    report("幾何 MTF", bad);
}

#[test]
fn fft_psf_matches_opticstudio() {
    let mut bad = Vec::new();
    for (name, r) in cases() {
        let p = refs_dir().join(&name).join("psf_os.npz");
        if !p.exists() {
            continue;
        }
        let s = load(&name);
        let mut npz = NpzReader::new(std::fs::File::open(p).unwrap()).unwrap();
        let mut worst: f64 = 0.0;
        for it in r["psf"]["items"].as_array().unwrap() {
            let field = it["field"].as_u64().unwrap() as usize;
            if (name == "A6" && field == 4) || (name == "D4" && field == 3) {
                continue;
            }
            let key = it["key"].as_str().unwrap();
            let want: Array2<f32> = npz.by_name(key).unwrap();
            let image = it["image"].as_u64().unwrap() as usize;
            let o = psf::PsfOpts {
                pupil: Some(it["pupil"].as_u64().unwrap() as usize),
                image,
                delta: Some(it["delta_um"].as_f64().unwrap() * 1e-3),
            };
            let g = psf::psf(&s, FieldSel::Index(field - 1), wave(&it["wave"]), o).unwrap();
            // OS の格子は x = (i − (N/2 − 1))·Δ、lensbench は (m − N/2)·Δ なので m = i + 1。
            // 全体の和で正規化してから OS の「和 / 最大」をかけて最大比にそろえる
            let sum: f64 = g.i.iter().flatten().sum();
            let scale = it["sum_over_max"].as_f64().unwrap() / sum;
            let (cy, cx) = want.dim();
            let i0 = image / 2 - 1 - cy / 2 + 1;
            let j0 = image / 2 - 1 - cx / 2 + 1;
            assert!((g.x[j0] * 1e3 - it["x0_um"].as_f64().unwrap()).abs() < 1e-6 * image as f64);
            let mut e: f64 = 0.0;
            for i in 0..cy {
                for j in 0..cx {
                    e = e.max((g.i[i0 + i][j0 + j] * scale - want[[i, j]] as f64).abs());
                }
            }
            let tol = it["os_s64_vs_s128"].as_f64().unwrap().max(0.03);
            worst = worst.max(e / tol);
            if !(e <= tol) {
                bad.push(format!("{name} {key}: {e:.3} > {tol:.3}"));
            }
        }
        eprintln!("{name}: FFT PSF の差 / 許容差 最大 {worst:.2}");
    }
    report("FFT PSF", bad);
}
