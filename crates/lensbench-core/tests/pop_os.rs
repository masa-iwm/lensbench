//! 簡易 POPを OpticStudio の POP（tests/pop_refs/<系>/pop.json、Top Hat のビームを開口で切ったもの）と照合する。
//!
//! 照度は「開始の面の開口の中の照度」で割った比で比べる（OpticStudio は開始の面でパワー 1、lensbench は
//! 正規化瞳の単位円の中でパワー 1）。断面（格子の中心の行・列）の差の最大を、OpticStudio の断面の最大で割る。
//! pop.json がない系は飛ばす。

use lensbench_core::pop::{self, OutGrid, PopMethod, RayBeamOpts};
use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use serde_json::Value;
use std::f64::consts::PI;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

/// 系・面ごとの許容差（断面の差の最大 / OpticStudio の断面の最大）。None は照合しない。
///
/// OpticStudio の POP は近軸（Fresnel 近似）で伝えるので、lensbench の Rayleigh-Sommerfeld（近似なし、
/// 解析解・数値積分と一致）と速い光束で離れる。差は OpticStudio 側の近似によるもの（の照合の記録）:
/// - aperture の end 4・5（100・500 mm、Fresnel 数 5・1）: OpticStudio は Fresnel の解析解（Hankel 変換の
///   数値積分）から最大の 1.5% 離れる点がある（500 mm、x = −0.129 で 2.159 対 2.099）。lensbench は解析解と合う
/// - aperture の end 2・3（開口から 5・20 mm、Fresnel 数 100・25）: 縁の標本化に敏感で、どちらも収束しない
///   （軸上の近似なしの値 0.577 に対し OpticStudio 0.990）。照合しない
/// - lens25 の end 2・4（f/25 の焦点の ∓1・+1.5 mm）: OpticStudio は軸上で近似なしの値より +3.2%・+0.7%
/// - lens5（f/5）: 焦点は OpticStudio が近軸の値（+0.85%）、ピントずれ（end 2・4）は 10〜40% 離れる。照合しない
/// - A0（f/5 の実レンズ、像面）: 断面で 5%
fn tolerance(system: &str, end: u64) -> Option<f64> {
    match (system, end) {
        ("aperture", 2 | 3) => None,
        ("aperture", _) => Some(2e-2),
        ("lens25", 3) => Some(3e-3),
        ("lens25", _) => Some(4e-2),
        ("lens5", 3) => Some(1.2e-2),
        ("lens5", _) => None,
        ("A0", _) => Some(6e-2),
        _ => Some(1e-2),
    }
}

#[test]
fn pop_matches_opticstudio() {
    let dir = root().join("tests/pop_refs");
    let cases: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let mut checked = 0;
    let mut bad = Vec::new();
    for c in cases["cases"].as_array().unwrap() {
        let name = c["system"].as_str().unwrap();
        let p = dir.join(name).join("pop.json");
        let Ok(txt) = std::fs::read_to_string(&p) else {
            eprintln!("{name}: pop.json がない（飛ばす）");
            continue;
        };
        let r: Value = serde_json::from_str(&txt).unwrap();
        let opt = LoadOptions {
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        };
        let s = zmx::load_with(&dir.join(name).join("system.zmx"), &opt).unwrap();
        let w = c["wave"].as_u64().unwrap() as usize - 1;
        let j = c["lb_surf"].as_u64().unwrap() as usize;
        let rb = pop::beam_from_rays(
            &s,
            FieldSel::Index(0),
            Some(w),
            j,
            &RayBeamOpts {
                n: 256,
                ..Default::default()
            },
        )
        .unwrap();
        let rep = s.paraxial().epd / 2.0;
        // lensbench の入射瞳の照度は 1/(π r²)
        let lb_in = 1.0 / (PI * rep * rep);
        let st = &r["start"];
        let snx = st["nx"].as_u64().unwrap() as usize;
        let os_in = nums(&st["x_line"])[snx / 2];
        let dists = nums(&c["lb_dist"]);
        for (k, e) in r["ends"].as_array().unwrap().iter().enumerate() {
            let Some(tol) = tolerance(name, e["end"].as_u64().unwrap()) else {
                eprintln!("{name} end {}: 照合しない（tolerance の説明）", e["end"]);
                continue;
            };
            let (nx, ny) = (
                e["nx"].as_u64().unwrap() as usize,
                e["ny"].as_u64().unwrap() as usize,
            );
            let (mx, my) = (e["min_x"].as_f64().unwrap(), e["min_y"].as_f64().unwrap());
            let (dx, dy) = (e["dx"].as_f64().unwrap(), e["dy"].as_f64().unwrap());
            let lines = [
                (
                    nums(&e["x_line"]),
                    OutGrid {
                        n: Some((nx, 1)),
                        half: Some((nx as f64 * dx / 2.0, 1e-9)),
                        center: Some((
                            mx + (nx as f64 - 1.0) / 2.0 * dx,
                            my + (ny / 2) as f64 * dy,
                        )),
                    },
                ),
                (
                    nums(&e["y_line"]),
                    OutGrid {
                        n: Some((1, ny)),
                        half: Some((1e-9, ny as f64 * dy / 2.0)),
                        center: Some((
                            mx + (nx / 2) as f64 * dx,
                            my + (ny as f64 - 1.0) / 2.0 * dy,
                        )),
                    },
                ),
            ];
            for (axis, (os, out)) in lines.iter().enumerate() {
                // OpticStudio の照度が最大の 1e-3 倍を超える範囲（の 1.2 倍）だけを比べる
                let os: Vec<f64> = os.iter().map(|v| v / os_in).collect();
                let m = os.iter().cloned().fold(0.0, f64::max);
                let lo = os.iter().position(|&v| v > 1e-3 * m).unwrap();
                let hi = os.iter().rposition(|&v| v > 1e-3 * m).unwrap();
                let pad = (hi - lo) / 10 + 2;
                let (lo, hi) = (lo.saturating_sub(pad), (hi + pad).min(os.len() - 1));
                let n = hi - lo + 1;
                let mut o = *out;
                let c = o.center.unwrap();
                let (mid, d) = if axis == 0 { (nx, dx) } else { (ny, dy) };
                let shift = ((lo + hi) as f64 / 2.0 - (mid as f64 - 1.0) / 2.0) * d;
                if axis == 0 {
                    o.n = Some((n, 1));
                    o.half = Some((n as f64 * d / 2.0, 1e-9));
                    o.center = Some((c.0 + shift, c.1));
                } else {
                    o.n = Some((1, n));
                    o.half = Some((1e-9, n as f64 * d / 2.0));
                    o.center = Some((c.0, c.1 + shift));
                }
                let pr = pop::propagate(&rb.beam, dists[k], PopMethod::Auto, o).unwrap();
                let lb: Vec<f64> = pr.beam.u.iter().map(|c| c.norm_sqr() / lb_in).collect();
                let os = &os[lo..=hi];
                let (mut worst, mut at) = (0.0f64, 0);
                for i in 0..n {
                    let d = (lb[i] - os[i]).abs();
                    if d > worst {
                        worst = d;
                        at = i + lo;
                    }
                }
                let rel = worst / m;
                let lbm = lb.iter().cloned().fold(0.0, f64::max);
                eprintln!(
                    "{name} end {} {}: 最大 OS {m:.5} lb {lbm:.5}、差の最大 {rel:.2e}（標本 {at}、{n} 点）、{:?} step {:.2} ferr {:.2e}",
                    e["end"],
                    ["x", "y"][axis],
                    pr.method,
                    pr.kernel_step,
                    pr.fresnel_error
                );
                if rel > tol {
                    bad.push(format!(
                        "{name} end {} {}: {rel:.2e}",
                        e["end"],
                        ["x", "y"][axis]
                    ));
                }
                checked += 1;
            }
        }
    }
    eprintln!("照合した断面 {checked}");
    assert!(bad.is_empty(), "{bad:?}");
}
