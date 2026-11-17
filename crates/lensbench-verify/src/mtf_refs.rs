//! OpticStudio 26.1 の MTF 系の基準（tests/refs/<系>/mtf.json・mtf_weights.json・mtf_family.json・psf_os.npz）との照合。
//!
//! [`crate::zemax_refs`] と同じく、このクレートのテスト（`tests/mtf_refs.rs`）と、別の基準・ガラスで同じ照合を
//! かけたい側の両方から呼ぶ（照合の中身を 1 か所に置く）。各関数は合わなかった項目を返す（空なら合格）。
//! `only` に系の名前（カンマ区切り）を渡すとそれだけ。
//!
//! # FFT MTF（mtf.json）
//! 回折 MTF を OpticStudio 26.1 の FFT MTF（mtf.json、MTFT・MTFS 演算子の多色の値）と照合する。
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
//!
//! # スルーフォーカス MTF・MTF vs 視野・RMS・幾何 MTF・FFT PSF（mtf_family.json）
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
#![allow(clippy::unwrap_used, clippy::panic, clippy::neg_cmp_op_on_partial_ord)] // 試験から呼ぶ照合の中身。NaN も不合格にする

use lensbench_core::psf;
use lensbench_core::sweep::{self, FieldAxis, RmsOpts, RmsRef};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{analysis, Waves};
use ndarray::Array2;
use ndarray_npy::NpzReader;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn load(refs: &Path, glass: &[PathBuf], name: &str) -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: glass.to_vec(),
        ..Default::default()
    };
    zmx::load_with(&refs.join(name).join("system.zmx"), &opt).unwrap()
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap_or(f64::NAN))
        .collect()
}

/// 数の列（どれも数であること。mtf.json の形）
fn all_nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn wave(v: &Value) -> Option<usize> {
    v.as_u64().map(|w| w as usize - 1)
}

/// `refs` の下で `file` を持つ系の名前（名前の順、`only` で絞る）
pub fn systems_with(refs: &Path, file: &str, only: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(refs)
        .unwrap()
        .flatten()
        .filter(|e| e.path().join(file).exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    if let Some(only) = only {
        v.retain(|n| only.split(',').any(|o| o == n));
    }
    v
}

/// 照合する系（mtf_family.json があるもの）と基準
fn cases(refs: &Path, only: Option<&str>) -> Vec<(String, Value)> {
    systems_with(refs, "mtf_family.json", only)
        .into_iter()
        .map(|n| {
            let r = read(&refs.join(&n).join("mtf_family.json"));
            (n, r)
        })
        .collect()
}

/// MTF 系の許容差の倍率
fn mtf_scale(name: &str) -> f64 {
    if name == "D4" {
        1.5
    } else {
        1.0
    }
}

/// 基準 r（mtf.json の形）との差の最大
fn mtf_worst(s: &lensbench_core::system::System, r: &Value) -> f64 {
    let freqs = all_nums(&r["freqs"]);
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
        let m = analysis::mtf(
            s,
            FieldSel::Index(fi as usize - 1),
            Waves::All,
            None,
            &freqs,
        )
        .unwrap();
        for (got, want) in [(&m.tan, all_nums(&f["T"])), (&m.sag, all_nums(&f["S"]))] {
            for (g, w) in got.iter().zip(&want) {
                worst = worst.max((g - w).abs());
            }
        }
    }
    worst
}

/// 回折 MTF（FFT MTF、mtf.json）
pub fn fft_mtf(refs: &Path, glass: &[PathBuf], only: Option<&str>) -> Vec<String> {
    let mut bad = Vec::new();
    for name in systems_with(refs, "mtf.json", only) {
        let r = read(&refs.join(&name).join("mtf.json"));
        let tol = r["tolerance"].as_f64().unwrap();
        let w = mtf_worst(&load(refs, glass, &name), &r);
        eprintln!("{name}: max |MTF − OpticStudio| = {w:.2e}（許容 {tol}）");
        if !(w <= tol) {
            bad.push(format!("{name} {w:.2e} > {tol}"));
        }
    }
    tag("FFT MTF", bad)
}

/// 多色の波長の重み: A0 の重みを 1・2・0.5 に変えた系の OpticStudio の値（A0/mtf_weights.json、無ければ何もしない）。
/// 重みを 1 に戻すと合わなくなることも確かめる（重みが効いていることの確かめ）
pub fn mtf_weights(refs: &Path, glass: &[PathBuf]) -> Vec<String> {
    let p = refs.join("A0/mtf_weights.json");
    if !p.exists() {
        return Vec::new();
    }
    let r = read(&p);
    let mut s = load(refs, glass, "A0");
    s.edit_raw(|p| {
        p.weights = all_nums(&r["weights"]);
        Ok(())
    })
    .unwrap();
    let tol = r["tolerance"].as_f64().unwrap();
    let mut bad = Vec::new();
    let w = mtf_worst(&s, &r);
    if !(w <= tol) {
        bad.push(format!("A0 {w:.2e} > {tol}"));
    }
    s.edit_raw(|p| {
        p.weights = vec![1.0; p.weights.len()];
        Ok(())
    })
    .unwrap();
    let w1 = mtf_worst(&s, &r);
    if !(w1 > 3.0 * tol) {
        bad.push(format!("A0 重み 1 でも {w1:.2e}（重みが効いていない）"));
    }
    tag("MTF の波長の重み", bad)
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

/// 合わなかった項目に照合の名前を付ける
fn tag(what: &str, bad: Vec<String>) -> Vec<String> {
    bad.into_iter().map(|b| format!("{what}: {b}")).collect()
}

pub fn through_focus(refs: &Path, glass: &[PathBuf], only: Option<&str>) -> Vec<String> {
    let mut bad = Vec::new();
    for (name, r) in cases(refs, only) {
        let s = load(refs, glass, &name);
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
                Waves::all_or(wave(&it["wave"])),
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
    tag("スルーフォーカス MTF", bad)
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

pub fn mtf_vs_field(refs: &Path, glass: &[PathBuf], only: Option<&str>) -> Vec<String> {
    let mut bad = Vec::new();
    for (name, r) in cases(refs, only) {
        if name == "A6" || name == "A3" {
            continue;
        }
        let s = load(refs, glass, &name);
        let fm = &r["mtf_vs_field"];
        let nn = fm["nodes"].as_u64().unwrap() as usize;
        let nodes = sweep::steps01(nn);
        let tol = 5e-3 * mtf_scale(&name);
        let mut worst: f64 = 0.0;
        for it in fm["series"].as_array().unwrap() {
            let f = it["freq"].as_f64().unwrap();
            let m = sweep::field_mtf(
                &s,
                FieldAxis::Y,
                &nodes,
                Waves::all_or(wave(&it["wave"])),
                None,
                &[f],
            );
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
    tag("MTF vs 視野", bad)
}

pub fn rms_family(refs: &Path, glass: &[PathBuf], only: Option<&str>) -> Vec<String> {
    let mut bad = Vec::new();
    for (name, r) in cases(refs, only) {
        let s = load(refs, glass, &name);
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
                        sweep::rms_vs_field(&s, FieldAxis::Y, &h, Waves::all_or(cw), o)
                    }
                    "focus" => {
                        if data == "StrehlRatio" && w.is_none() {
                            continue;
                        }
                        sweep::rms_vs_focus(&s, FieldSel::Index(ci), &x, Waves::all_or(w), o)
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
    tag("RMS", bad)
}

pub fn geometric_mtf(refs: &Path, glass: &[PathBuf], only: Option<&str>) -> Vec<String> {
    let mut bad = Vec::new();
    for (name, r) in cases(refs, only) {
        if name == "D4" {
            continue;
        }
        let s = load(refs, glass, &name);
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
                Waves::all_or(wave(&it["wave"])),
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
    tag("幾何 MTF", bad)
}

pub fn fft_psf(refs: &Path, glass: &[PathBuf], only: Option<&str>) -> Vec<String> {
    let mut bad = Vec::new();
    for (name, r) in cases(refs, only) {
        let p = refs.join(&name).join("psf_os.npz");
        if !p.exists() {
            continue;
        }
        let s = load(refs, glass, &name);
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
                pixel: Some(it["delta_um"].as_f64().unwrap() * 1e-3),
                // OS の FFT PSF は主光線に垂直な面
                plane: psf::PsfPlane::ChiefPerp,
            };
            let g = psf::psf(
                &s,
                FieldSel::Index(field - 1),
                Waves::all_or(wave(&it["wave"])),
                o,
            )
            .unwrap();
            // OS の格子は x = (i − (N/2 − 1))·Δ、lensbench は (m − N/2)·Δ なので m = i + 1。
            // 全体の和で正規化してから OS の「和 / 最大」をかけて最大比にそろえる
            let sum: f64 = g.i.data().iter().sum();
            let scale = it["sum_over_max"].as_f64().unwrap() / sum;
            let (cy, cx) = want.dim();
            let i0 = image / 2 - 1 - cy / 2 + 1;
            let j0 = image / 2 - 1 - cx / 2 + 1;
            assert!(
                (g.i.x()[j0] * 1e3 - it["x0_um"].as_f64().unwrap()).abs() < 1e-6 * image as f64
            );
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
    tag("FFT PSF", bad)
}

#[cfg(test)]
mod tests {
    use super::natural_spline;

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
}
