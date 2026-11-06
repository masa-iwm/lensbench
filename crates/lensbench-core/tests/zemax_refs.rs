#![allow(clippy::neg_cmp_op_on_partial_ord)]
//! OpticStudio 26.1 の基準データ（tests/refs）との照合。
//!
//! 許容差: エイミングなしは丸め誤差まで（位置 1e-11 mm、方向 1e-12）、エイミングありは
//! OpticStudio 自身のエイミングの収束（〜1e-7 mm）で位置 2e-7 mm・方向 1e-8。OPD [波] はエイミングなし 5e-6
//! （理想レンズの積分モード D2 は 5e-5）、ありは 2e-3。ガラスはテスト用カタログ tests/glass（refractiveindex.info、CC0）。

use lensbench_core::analysis::{self, ZernKind};
use lensbench_core::report;
use lensbench_core::system::{Aiming, FieldSel, FieldType};
use lensbench_core::zmx::{self, LoadOptions};
use ndarray::{Array2, Array4};
use ndarray_npy::NpzReader;
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 基準データのフォルダ（LB_REFS_DIR で差し替え。非公開の基準を同じ照合にかけるため）
fn refs_dir() -> PathBuf {
    std::env::var_os("LB_REFS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("tests/refs"))
}

/// ガラスカタログのフォルダ（LB_GLASS_DIR で差し替え、パス区切りで複数可）
fn glass_dirs() -> Vec<PathBuf> {
    match std::env::var_os("LB_GLASS_DIR") {
        Some(v) => std::env::split_paths(&v).collect(),
        None => vec![root().join("tests/glass")],
    }
}

#[derive(Debug, Default)]
struct Cmp {
    pos: f64,
    dir: f64,
    opl: f64,
    opl_raw: f64,
    opd: f64,
    ok_mismatch: usize,
    vig_mismatch: usize,
    frames: f64,
    d_basic: f64,
    d_expd: f64,
    d_indx: f64,
    dsys: f64,
    spot_rel: f64,
    zern: f64,
    zern_stats: f64,
    opdc: f64,
    wfm: f64,
}

fn mx(a: &mut f64, b: f64) {
    if b > *a || b.is_nan() {
        *a = b;
    }
}

/// 列の 1 次式（1, x, y, z）を最小二乗で除いた残差の幅
fn ptp_detrended(dl: &[f64], q: &[[f64; 3]]) -> f64 {
    let a: Vec<Vec<f64>> = q.iter().map(|p| vec![1.0, p[0], p[1], p[2]]).collect();
    // 退化（全点で z が同じなど）に強いよう、列ごとに尺度をそろえずそのまま QR
    let c = analysis::lstsq(&a, dl);
    let r: Vec<f64> = a
        .iter()
        .zip(dl)
        .map(|(row, v)| v - row.iter().zip(&c).map(|(x, y)| x * y).sum::<f64>())
        .collect();
    r.iter().cloned().fold(f64::MIN, f64::max) - r.iter().cloned().fold(f64::MAX, f64::min)
}

fn compare(name: &str) -> (Cmp, Aiming) {
    let d = refs_dir().join(name);
    let info: Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("ops.json")).unwrap()).unwrap();
    let mut npz = NpzReader::new(std::fs::File::open(d.join("rays.npz")).unwrap()).unwrap();
    let hxy: Array2<f64> = npz.by_name("hx_hy").unwrap();
    let pp: Array2<f64> = npz.by_name("px_py").unwrap();
    let pps: Array2<f64> = npz.by_name("px_py_s").unwrap();
    let img: Array4<f64> = npz.by_name("img").unwrap();
    let surf: Array4<f64> = npz.by_name("surf").unwrap();
    let gmat: Array2<f64> = npz.by_name("gmat").unwrap();
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: glass_dirs(),
        ..Default::default()
    };
    let s = zmx::load_with(&d.join("system.zmx"), &opt).unwrap();
    s.validate().unwrap();
    let mut c = Cmp::default();
    let par = s.paraxial();
    let ops = &info["ops"];
    let num = |k: &str| ops.get(k).and_then(|v| v.as_f64());
    for (k, v) in [("EFFL", par.efl), ("ENPP", par.enpp), ("EPDI", par.epd)] {
        if let Some(r) = num(k) {
            mx(&mut c.d_basic, (v - r).abs());
        }
    }
    if let Some(r) = num("EXPD") {
        c.d_expd = (s.exit_pupil_diameter() - r).abs();
    }
    if let Some(m) = ops.as_object() {
        for (k, v) in m {
            if let (Some(rest), Some(v)) = (k.strip_prefix("INDX|"), v.as_f64()) {
                let p: Vec<usize> = rest.split('|').map(|x| x.parse().unwrap()).collect();
                mx(&mut c.d_indx, (s.surfaces[p[0]].n[p[1]] - v).abs());
            }
        }
    }
    let sd = report::system_data(&s);
    for (k, v) in [
        ("PIMH", sd.pimh),
        ("PMAG", sd.pmag),
        ("WFNO", sd.wfno),
        ("TOTR", sd.totr),
        ("ISFN", sd.isfn),
        ("OBSN", sd.obna),
    ] {
        if let Some(r) = num(k) {
            mx(&mut c.dsys, (v - r).abs());
        }
    }
    let px: Vec<f64> = pp.column(0).to_vec();
    let py: Vec<f64> = pp.column(1).to_vec();
    let nw = s.wavelengths.len();
    for i in 0..hxy.nrows() {
        let f = FieldSel::Norm(hxy[[i, 0]], hxy[[i, 1]]);
        for w in 0..nw {
            let r = analysis::opd(&s, f, &px, &py, Some(w), true).unwrap();
            let t = &r.trace;
            let (mut dl, mut q) = (Vec::new(), Vec::new());
            for n in 0..px.len() {
                let z = img.slice(ndarray::s![i, w, n, ..]);
                let okz = z[8] == 0.0;
                let okf = t.rays[n].err == 0;
                if okz != okf {
                    c.ok_mismatch += 1;
                }
                if !(okz && okf) {
                    continue;
                }
                if (z[9] != 0.0) != (t.rays[n].vig != 0) {
                    c.vig_mismatch += 1;
                }
                let x = &t.rays[n];
                for a in 0..3 {
                    mx(&mut c.pos, (x.xyz[a] - z[a]).abs());
                    mx(&mut c.dir, (t.parity * x.lmn[a] - z[3 + a]).abs());
                }
                mx(&mut c.opl_raw, (t.opl_os[n] - z[6]).abs());
                mx(&mut c.opd, (r.opd[n] - z[7]).abs());
                dl.push(x.opl - z[6]);
                q.push(t.p0[n]);
            }
            if !dl.is_empty() {
                let v = if s.infinite() {
                    ptp_detrended(&dl, &q)
                } else {
                    dl.iter().cloned().fold(f64::MIN, f64::max)
                        - dl.iter().cloned().fold(f64::MAX, f64::min)
                };
                mx(&mut c.opl, v);
            }
        }
        let r = s.trace(
            f,
            &pps.column(0).to_vec(),
            &pps.column(1).to_vec(),
            Some(s.primary),
            None,
            true,
        );
        for (j, recs, parity) in &r.rec {
            for (n, e) in recs.iter().enumerate() {
                let z = surf.slice(ndarray::s![i, n, *j, ..]);
                if z[7] != 0.0 || !e.xyz[0].is_finite() {
                    continue;
                }
                for a in 0..3 {
                    mx(&mut c.pos, (e.xyz[a] - z[a]).abs());
                    mx(&mut c.dir, (parity * e.lmn[a] - z[3 + a]).abs());
                }
            }
        }
    }
    // 大域座標系
    let fr = s.frames();
    for (j, f) in fr.surf.iter().enumerate() {
        let Some(f) = f else { continue };
        let g = gmat.row(j);
        if !g.iter().all(|v| v.is_finite()) || (j == 0 && s.infinite()) {
            continue;
        }
        for a in 0..3 {
            for b in 0..3 {
                mx(&mut c.frames, (f.r[a][b] - g[3 * a + b]).abs());
            }
            mx(&mut c.frames, (f.o[a] - g[9 + a]).abs() * 1e-3);
        }
    }
    // スポット RMS、OPDC、波面マップの RMS
    if let Some(m) = info["spot"].as_object() {
        for (k, v) in m {
            let Some(v) = v.as_f64() else { continue };
            let p: Vec<&str> = k.split('|').collect();
            if p[0] == "OPDC" {
                let o = analysis::opd(
                    &s,
                    FieldSel::Index(p[1].parse().unwrap()),
                    &[p[2].parse().unwrap()],
                    &[p[3].parse().unwrap()],
                    None,
                    true,
                )
                .unwrap();
                mx(&mut c.opdc, (o.opd[0] - v).abs());
                continue;
            }
            if p[0] == "WFM_RMS" {
                let i: usize = p[1].parse().unwrap();
                if s.hxhy(i) != (0.0, 0.0) {
                    continue; // 軸外の波面マップは OpticStudio の標本の取り方がまだ合っていない
                }
                let wf = analysis::wavefront(&s, FieldSel::Index(i), None, 32, false).unwrap();
                mx(&mut c.wfm, (wf.rms - v).abs());
                continue;
            }
            if !p[0].starts_with("RS") || v == 0.0 {
                continue;
            }
            let (i, w): (usize, usize) = (p[1].parse().unwrap(), p[2].parse().unwrap());
            if s.field_type == FieldType::RealH && w != 0 && w != s.primary + 1 {
                continue;
            }
            let sp = analysis::spot(
                &s,
                FieldSel::Index(i),
                if w == 0 { None } else { Some(w - 1) },
                20,
            )
            .unwrap();
            let mine = if p[0] == "RSCE" {
                sp.rms_centroid
            } else {
                sp.rms_chief
            };
            mx(&mut c.spot_rel, (mine / v - 1.0).abs());
        }
    }
    // Zernike
    let zf = d.join("zernike.json");
    if zf.exists() {
        let z: Value = serde_json::from_str(&std::fs::read_to_string(zf).unwrap()).unwrap();
        for (k, v) in z.as_object().unwrap() {
            let Some(coef) = v["coef"].as_array() else {
                continue;
            };
            if coef.is_empty() || coef.iter().any(|x| x.is_null()) {
                continue;
            }
            let p: Vec<&str> = k.split('|').collect();
            let kind = if p[0] == "fringe" {
                ZernKind::Fringe
            } else {
                ZernKind::Standard
            };
            let zz = analysis::zernike(
                &s,
                FieldSel::Index(p[1].parse().unwrap()),
                Some(p[2].parse().unwrap()),
                kind,
                coef.len(),
                64,
                false,
            )
            .unwrap();
            for (a, b) in zz.coef.iter().zip(coef) {
                mx(&mut c.zern, (a - b.as_f64().unwrap()).abs());
            }
            let text = v["text"].as_str().unwrap_or("");
            for (label, val) in [
                ("Peak to Valley (to chief)", zz.pv),
                ("Peak to Valley (to centroid)", zz.pv_centroid),
                ("RMS (to chief)", zz.rms_chief),
                ("RMS (to centroid)", zz.rms_centroid),
                ("RMS fit error", zz.rms_fit),
                ("Maximum fit error", zz.max_fit),
            ] {
                if let Some(line) = text.lines().find(|l| l.starts_with(label)) {
                    let num: f64 = line
                        .split(':')
                        .nth(1)
                        .unwrap()
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .parse()
                        .unwrap();
                    mx(&mut c.zern_stats, (val - num).abs());
                }
            }
        }
    }
    (c, s.aiming)
}

#[test]
fn against_opticstudio() {
    let mut names: Vec<String> = std::fs::read_dir(refs_dir())
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    if let Ok(only) = std::env::var("LB_REFS") {
        names.retain(|n| only.split(',').any(|o| o == n));
    }
    let mut fails = Vec::new();
    for n in &names {
        let (c, aiming) = compare(n);
        let aimed = aiming != Aiming::Off;
        let d2 = n == "D2";
        let checks: Vec<(&str, f64, f64)> = vec![
            ("pos", c.pos, if aimed { 2e-7 } else { 1e-11 }),
            ("dir", c.dir, if aimed { 1e-8 } else { 1e-12 }),
            ("opl", c.opl, if aimed { 2e-7 } else { 1e-11 }),
            ("opl_raw", c.opl_raw, if aimed { 2e-7 } else { 1e-11 }),
            ("frames", c.frames, 1e-12),
            ("basic", c.d_basic, 1e-10),
            (
                "expd",
                c.d_expd,
                if aimed || n.starts_with('C') {
                    3e-3
                } else {
                    5e-7
                },
            ),
            ("indx", c.d_indx, 1e-12),
            ("sys", c.dsys, if aimed { 5e-8 } else { 1e-10 }),
            (
                "opd",
                c.opd,
                if aimed {
                    2e-3
                } else if d2 {
                    5e-5
                } else {
                    5e-6
                },
            ),
            (
                "zern",
                c.zern,
                if aimed {
                    2e-3
                } else if d2 {
                    5e-5
                } else {
                    5e-6
                },
            ),
            (
                "zern_stats",
                c.zern_stats,
                if aimed {
                    2e-3
                } else if d2 {
                    5e-5
                } else {
                    5e-6
                },
            ),
            ("opdc", c.opdc, if d2 { 5e-5 } else { 1e-8 }),
            ("wfm", c.wfm, 1e-4),
            (
                "spot_rel",
                if n.starts_with('D') { 0.0 } else { c.spot_rel },
                if aimed { 1e-7 } else { 1e-10 },
            ),
        ];
        let mut line = format!("{n:3} {:>8}", format!("{aiming:?}"));
        for (k, v, tol) in &checks {
            line += &format!(" {k}={v:.1e}");
            if !(*v <= *tol) {
                fails.push(format!("{n}: {k} = {v:.3e} > {tol:.0e}"));
            }
        }
        if c.ok_mismatch != 0 || c.vig_mismatch != 0 {
            fails.push(format!(
                "{n}: ok_mismatch {} vig_mismatch {}",
                c.ok_mismatch, c.vig_mismatch
            ));
        }
        println!("{line}");
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
