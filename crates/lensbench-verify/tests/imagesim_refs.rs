//! OpticStudio 2026 R1.01 の Image Simulation の出力（tests/refs/<系>/imagesim_os/、入力は tests/imagesim_src/）との照合。
//! OS から数値は取れず 8 bit の PNG だけなので、試験の図の点（元の 1 画素、64 画素ごと）ごとに、重心と明るさの和を比べる
//! （既定の画素・Oversampling なし・中心の視野 1 の設定だけ）。
//!
//! 許容差と理由:
//! - 重心 0.4 px: OS は偶数の画像の中心を画素 256 に置く（lensbench は 255.5 で上下左右に対称。README の
//!   「右の点が 1 px 内側」も同じ由来）ので、点ごとに 0.5 px 以内の食い違いが残る
//! - 明るさの和 −4〜+6 %: lensbench は元の画素の明るさに像の面積の比をかけ、一様な元の画像が相対照度そのものになる
//!   （相対照度は歪曲による面積の変化を含む）。OS は点の明るさに面積の比をかけない（A0 の角の点で 5 % ほど暗い）。
//!   回折の像は lensbench の PSF の裾が広く（OS の PSF は 32 × 32 の格子）、±12 px の窓の外に出る分だけ小さい（A0 の短い波長で −3 %）。
//!   D0（面積の変化が小さい）は 1.5 % 以内

use lensbench_analysis::imagesim::{image_sim, Aberrations, ImageSimOpts, SimImage};
use lensbench_export::image::read_png;
use lensbench_formats::zmx::{self, LoadOptions};
use lensbench_verify::zemax_refs::{glass_dirs, refs_dir};
use std::path::Path;

fn png(p: &Path) -> (usize, usize, Vec<Vec<f64>>) {
    read_png(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// 点 (64j, 64k)（j, k = 1〜7）のまわり ±12 px の重心 (行, 列) と和
fn dots(img: &[f64], w: usize) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::new();
    for j in 1..=7 {
        for k in 1..=7 {
            let (r, c) = (64 * j, 64 * k);
            let (mut t, mut sy, mut sx) = (0.0, 0.0, 0.0);
            for y in r - 12..=r + 12 {
                for x in c - 12..=c + 12 {
                    let v = img[y * w + x];
                    t += v;
                    sy += v * y as f64;
                    sx += v * x as f64;
                }
            }
            out.push((sy / t, sx / t, t));
        }
    }
    out
}

fn check(sys: &str, run: &str, fails: &mut Vec<String>) {
    let refs = refs_dir();
    let dir = refs.join(sys).join("imagesim_os");
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(format!("{run}.json"))).unwrap()).unwrap();
    let st = &meta["settings"];
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: glass_dirs(),
        ..Default::default()
    };
    let s = zmx::load_with(&refs.join(sys).join("system.zmx"), &opt).unwrap();
    // 入力の画像は系の基準のフォルダ（tests/refs の下は全部が系）の外に置く
    let (w, h, mut ch) = png(&refs
        .parent()
        .unwrap()
        .join("imagesim_src")
        .join(meta["input_image"].as_str().unwrap()));
    let rgb = meta["wavelength_mode"]
        .as_str()
        .unwrap()
        .eq_ignore_ascii_case("rgb");
    if !rgb {
        ch.truncate(1);
    }
    let o = ImageSimOpts {
        field_height: st["FieldHeight"].as_f64().unwrap(),
        psf_grid: (
            st["PSFXPoints"].as_u64().unwrap() as usize,
            st["PSFYPoints"].as_u64().unwrap() as usize,
        ),
        aberrations: if st["Aberrations"] == "None" {
            Aberrations::None
        } else {
            Aberrations::Diffraction
        },
        relative_illumination: st["UseRelativeIllumination"].as_bool().unwrap(),
        wave: (!rgb).then(|| meta["primary_wavelength"].as_u64().unwrap() as usize - 1),
        ..Default::default()
    };
    let r = image_sim(
        &s,
        &SimImage {
            width: w,
            height: h,
            channels: ch.clone(),
        },
        &o,
    )
    .unwrap();
    let px = meta["pixel_size_mm"].as_f64().unwrap();
    if (r.pixel / px - 1.0).abs() > 1e-3 {
        fails.push(format!("{sys} {run}: pixel {} (OS {px})", r.pixel));
    }
    let (ow, _, och) = png(&dir.join(meta["output_png"].as_str().unwrap()));
    // OS の単色の出力は 1 チャンネルだけ（波長 1・2・3 → B・G・R）
    let pairs: Vec<(usize, usize)> = if rgb {
        vec![(0, 0), (1, 1), (2, 2)]
    } else {
        let best = (0..3)
            .max_by(|&a, &b| {
                och[a]
                    .iter()
                    .sum::<f64>()
                    .total_cmp(&och[b].iter().sum::<f64>())
            })
            .unwrap();
        vec![(best, 0)]
    };
    let tol_sum = if sys == "D0" {
        (0.985, 1.015)
    } else {
        (0.96, 1.06)
    };
    for (oc, mc) in pairs {
        let (od, md) = (
            dots(&och[oc], ow),
            dots(&r.image.channels[mc], r.image.width),
        );
        for (i, (a, b)) in od.iter().zip(&md).enumerate() {
            if a.2 < 0.5 {
                continue; // RGB の図はチャンネルごとに一部の点だけ
            }
            let d = (b.0 - a.0).abs().max((b.1 - a.1).abs());
            let q = b.2 / a.2;
            if d > 0.4 || !(tol_sum.0..=tol_sum.1).contains(&q) {
                fails.push(format!(
                    "{sys} {run} channel {oc} dot {}: centroid {d:.3} px, sum ratio {q:.4}",
                    i + 1
                ));
            }
        }
    }
}

#[test]
fn image_simulation_matches_opticstudio() {
    let mut fails = Vec::new();
    for sys in ["A0", "D0"] {
        for run in ["gray_none", "gray_diff", "gray127_none", "rgb_diff"] {
            check(sys, run, &mut fails);
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
