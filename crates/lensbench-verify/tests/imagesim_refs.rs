//! OpticStudio 2026 R1.01 の Image Simulation の出力（tests/refs/<系>/imagesim_os/、入力は tests/imagesim_src/）との照合。
//! OS から数値は取れず 8 bit の PNG だけなので、試験の図の点（元の 1 画素、64 画素ごと）ごとに、重心と明るさの和を比べる
//! （既定の画素・Oversampling なしの設定。中心の視野が軸の外の D0 の f2・f4 も。細かい検出器の Oversampling X8 は
//! [`oversampled_point_matches_opticstudio`] で中央の点の形を比べる）。
//!
//! 許容差と理由:
//! - 重心 0.4 px: OS は偶数の画像の中心を画素 256 に置く（lensbench は 255.5 で上下左右に対称。README の
//!   「右の点が 1 px 内側」も同じ由来）ので、点ごとに 0.5 px 以内の食い違いが残る
//! - 明るさの和 −4〜+6 %: lensbench は元の画素の明るさに像の面積の比をかけ、一様な元の画像が相対照度そのものになる
//!   （相対照度は歪曲による面積の変化を含む）。OS は点の明るさに面積の比をかけない（A0 の角の点で 5 % ほど暗い）。
//!   回折の像は lensbench の PSF の裾が広く（OS の PSF は 32 × 32 の格子）、±12 px の窓の外に出る分だけ小さい（A0 の短い波長で −3 %）。
//!   D0（面積の変化が小さい）は 1.5 % 以内
//! - 中心の視野が軸の外（D0 の f2・f4）は重心 1.5 px・和 −4〜+6 %: OS は元の画素を検出器の画素の位置に丸めて置く（軸上でも
//!   点の重心がちょうど整数の画素に来る）。軸の外ではその丸めが中心に対して対称でなく、点ごとに最大 1.3 px ずれる
//!   （中心の点は 0.01 px 以内で合い、lensbench の点の位置は D0 の f·tanθ の写像そのもの）。和は視野 10° ほどの角で面積の比
//!   （sec³θ ≈ 1.05）の分だけ大きい

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
        aberrations: match st["Aberrations"].as_str().unwrap() {
            "None" => Aberrations::None,
            "Geometric" => Aberrations::Geometric,
            _ => Aberrations::Diffraction,
        },
        relative_illumination: st["UseRelativeIllumination"].as_bool().unwrap(),
        wave: (!rgb).then(|| meta["primary_wavelength"].as_u64().unwrap() as usize - 1),
        center_field: {
            let n = meta["center_field_number"].as_u64().unwrap_or(1) as usize;
            let f = &meta["fields"][n - 1];
            (f[0].as_f64().unwrap(), f[1].as_f64().unwrap())
        },
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
    let off_axis = meta["center_field_number"].as_u64().unwrap_or(1) != 1;
    let tol_sum = if sys == "D0" && !off_axis {
        (0.985, 1.015)
    } else {
        (0.96, 1.06)
    };
    let tol_pos = if off_axis { 1.5 } else { 0.4 };
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
            if d > tol_pos || !(tol_sum.0..=tol_sum.1).contains(&q) {
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
    // 中心の視野が軸の外（D0 の視野 2 = (0, 5°)・視野 4 = (3°, −4°)）
    for run in ["gray_none_f2", "gray_diff_f2", "gray_none_f4"] {
        check("D0", run, &mut fails);
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// 元の 1 画素の像より細かい検出器（D0、画素 2 µm、OS は Oversampling X8）: 中央の点（元の 1 画素、像は 17 µm 角）の
/// 重心と、行・列の半値幅を OS と比べる。lensbench は Oversampling なしで元の画素を細かく分けて置く。
///
/// 許容差: 重心 0.6 px・半値幅 1 px。OS は元の 1 画素の像を検出器の整数の画素に丸めて置く（収差なしの像が 9 × 9 の画素で、
/// 8.52 × 8.52 の正方形にならない）。回折の像の中央の明るさは、F/5 の Airy の PSF を 17.044 µm 角の正方形で積分した値
/// 0.9385（数値積分）に 2 % で合う（OS の PNG は最大で 255 に揃えるので、明るさは比べられない）。lensbench の回折の像は
/// 正方形の縁の外で裾が OS より広い（縁から 10 µm で 1〜2 %。円形の瞳の縁の像の裾はおよそ λF / (π²x) で、
/// 10 µm で約 3 %。OS の PSF は 32 × 32 の格子で裾が切れる）
#[test]
fn oversampled_point_matches_opticstudio() {
    let refs = refs_dir();
    let dir = refs.join("D0").join("imagesim_os");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: glass_dirs(),
        ..Default::default()
    };
    let s = zmx::load_with(&refs.join("D0").join("system.zmx"), &opt).unwrap();
    let mut fails = Vec::new();
    for (run, ab) in [
        ("gray_none_ov8", Aberrations::None),
        ("gray_diff_ov8", Aberrations::Diffraction),
        ("gray_geom_ov8", Aberrations::Geometric),
    ] {
        let meta: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(format!("{run}.json"))).unwrap())
                .unwrap();
        let st = &meta["settings"];
        let (w, h, mut ch) = png(&refs
            .parent()
            .unwrap()
            .join("imagesim_src")
            .join(meta["input_image"].as_str().unwrap()));
        ch.truncate(1);
        let n = st["XPixels"].as_u64().unwrap() as usize;
        let o = ImageSimOpts {
            field_height: st["FieldHeight"].as_f64().unwrap(),
            psf_grid: (
                st["PSFXPoints"].as_u64().unwrap() as usize,
                st["PSFYPoints"].as_u64().unwrap() as usize,
            ),
            aberrations: ab,
            pixel: Some(meta["pixel_size_mm"].as_f64().unwrap()),
            size: Some((n, n)),
            relative_illumination: st["UseRelativeIllumination"].as_bool().unwrap(),
            wave: Some(meta["primary_wavelength"].as_u64().unwrap() as usize - 1),
            ..Default::default()
        };
        let r = image_sim(
            &s,
            &SimImage {
                width: w,
                height: h,
                channels: ch,
            },
            &o,
        )
        .unwrap();
        let (ow, _, och) = png(&dir.join(meta["output_png"].as_str().unwrap()));
        assert_eq!(ow, n);
        let os = &och[1];
        let lb = &r.image.channels[0];
        // 重心（中央の ±12 px）と、重心の行・列の半値幅（自分の最大で割る）
        let shape = |im: &[f64]| {
            let (mut t, mut sy, mut sx) = (0.0, 0.0, 0.0);
            for y in n / 2 - 12..=n / 2 + 12 {
                for x in n / 2 - 12..=n / 2 + 12 {
                    let v = im[y * n + x];
                    t += v;
                    sy += v * y as f64;
                    sx += v * x as f64;
                }
            }
            let (cy, cx) = (sy / t, sx / t);
            let width = |line: &dyn Fn(usize) -> f64| {
                let m = (0..n).map(line).fold(0.0, f64::max);
                let above: Vec<usize> = (0..n).filter(|&k| line(k) >= m / 2.0).collect();
                let (a, b) = (above[0], *above.last().unwrap());
                // 半値を横切る所を線形に補間
                let cross = |i: usize, j: usize| {
                    let (vi, vj) = (line(i), line(j));
                    i as f64 + (m / 2.0 - vi) / (vj - vi) * (j as f64 - i as f64)
                };
                cross(b, b + 1) - cross(a, a - 1)
            };
            let (ry, rx) = (cy.round() as usize, cx.round() as usize);
            (
                cy,
                cx,
                width(&|k| im[ry * n + k]),
                width(&|k| im[k * n + rx]),
            )
        };
        let (a, b) = (shape(os), shape(lb));
        let dc = (a.0 - b.0).abs().max((a.1 - b.1).abs());
        let dw = (a.2 - b.2).abs().max((a.3 - b.3).abs());
        if dc > 0.6 || dw > 1.0 {
            fails.push(format!(
                "{run}: centroid {dc:.3} px, width OS ({:.2}, {:.2}) lensbench ({:.2}, {:.2})",
                a.2, a.3, b.2, b.3
            ));
        }
        if ab == Aberrations::Diffraction {
            let peak = lb.iter().copied().fold(0.0, f64::max);
            if (peak / 0.9385 - 1.0).abs() > 0.02 {
                fails.push(format!("{run}: peak {peak:.4} (Airy 0.9385)"));
            }
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
