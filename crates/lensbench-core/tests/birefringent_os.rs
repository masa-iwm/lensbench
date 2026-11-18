//! 複屈折材の板（OpticStudio の Birefringent In / Out、Mode 2 の波長板と Mode 0）を OpticStudio 26.1 と照合する。
//! 系（tests/birefringent）: 絞り → 空気 5 → 板 → 空気 20 → 像面、波長 0.55・0.6328・0.8 µm、視野 角度 (0,0)・(0,3)・(2,−2)°。
//! b1 は水晶 1 mm（多次、結晶軸 30°）、b2 は水晶 0.0175 mm（633 nm で λ/4）、b3 は水晶 0.0175 mm（30°）と MgF₂ 0.0141 mm
//! （120°）の貼り合わせ、b4_m0 は b1 の Mode 0（常光の成分だけ）、b2_coat は b2 の両面に理想膜 I.99999（LB379.DAT）、
//! b2_ax0・b2_ax60 は b2 の結晶軸を 0°・60° にしたもの。基準は Normalized Polarized の像面の電場（入力 x・y）
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::system::{FieldSel, PolIn, System};
use lensbench_formats::zmx;
use lensbench_formats::zmx_write;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/birefringent")
}

fn load(name: &str) -> System {
    let opt = zmx::LoadOptions {
        catalog_dirs: vec![dir().join("../glass")],
        ..Default::default()
    };
    zmx::load_with(&dir().join(format!("{name}.zmx")), &opt).unwrap()
}

/// 波長ごとに (光線ごとの複素の比 s を除いた残りの最大, |s| − 1 の最大)。比を除くのは、OS の光路の位相の数え方
/// （軸外の視野）の違いのため（tests/projects/A3 の照合と同じ）
fn pol_diff(s: &System, name: &str) -> Vec<(f64, f64)> {
    let txt = std::fs::read_to_string(dir().join(format!("os_pol_{name}.json"))).unwrap();
    let os: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let coat = Coatings::for_system(s).unwrap();
    let pols = [
        [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
        [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
    ];
    let nw = os["wavelengths_um"].as_array().unwrap().len();
    let mut worst = vec![(0.0f64, 0.0f64); nw];
    for fd in os["fields"].as_array().unwrap() {
        let v = |k: &str| -> Vec<f64> {
            fd[k]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        };
        let (hx, hy) = (fd["hx"].as_f64().unwrap(), fd["hy"].as_f64().unwrap());
        let (px, py) = (v("px"), v("py"));
        for (w, wv) in worst.iter_mut().enumerate() {
            let t = s
                .try_trace_pol(
                    FieldSel::Norm(hx, hy),
                    &px,
                    &py,
                    Some(w),
                    PolIn::new(&pols, &coat),
                )
                .unwrap();
            for (i, e) in t.e.iter().enumerate() {
                let o = |m: usize, c: usize| {
                    C64::new(
                        fd["re"][w][m][i][c].as_f64().unwrap(),
                        fd["im"][w][m][i][c].as_f64().unwrap(),
                    )
                };
                let mc = || (0..2).flat_map(|m| (0..3).map(move |c| (m, c)));
                let (mut num, mut den) = (C64::new(0.0, 0.0), 0.0);
                for (m, c) in mc() {
                    num += o(m, c).conj() * e[m][c];
                    den += o(m, c).norm_sqr();
                }
                let sc = num / den;
                for (m, c) in mc() {
                    let d = (e[m][c] - sc * o(m, c)).norm();
                    assert!(d.is_finite(), "{name} 波長 {w} 光線 {i}");
                    wv.0 = wv.0.max(d);
                }
                wv.1 = wv.1.max((sc.norm() - 1.0).abs());
            }
        }
    }
    worst
}

#[test]
fn birefringent_plates_match_opticstudio() {
    for name in ["b1", "b2", "b3", "b4_m0", "b2_coat", "b2_ax0", "b2_ax60"] {
        let s = load(name);
        let d = pol_diff(&s, name);
        println!("{name}（波長ごとの 比を除いた残り, |s| − 1） {d:?}");
        // Mode 0（常光の成分だけ）と、板の両面に理想膜（I.99999）を付けた b2_coat は、斜めの視野も含め 1e-12 で合う。
        // 無膜の Mode 2 の残り（最大 4.8e-4、既知の差）は境界の Fresnel の扱いの違い: OS は面に垂直な光線で s・p を
        // 実験室の x・y に取り、結晶軸に近い方に ne、もう一方に no の透過を入口の面だけで使う（結晶軸の向きと無関係。
        // OpticStudio との第 2 段の照合で 1.9e-9 で確認）。lensbench は入口・出口とも常光・異常光の成分に分けて no・n(θ) の
        // Fresnel（物理的に正しい式を採る）。結晶軸が x の b2_ax0 は OS の取り方と向きが合い 2.5e-6
        // 貼り合わせの b3 は、境界 1 回の係数（水晶の o → MgF₂ の e は t(no₁→ne₂)）にしたため OS と
        // 振幅で 1.2e-4 違う（OS は出口・入口の 2 段 t(no₁→no₂)·t(no₂→ne₂) と同じ値、前は 3e-6 で一致していた）
        let tol = match name {
            "b4_m0" | "b2_coat" => (1e-10, 1e-10),
            "b2_ax0" => (3e-6, 1e-8),
            "b3" => (2e-4, 1.5e-4),
            _ => (5.5e-4, 5e-6),
        };
        assert!(
            d.iter().all(|&(r, a)| r < tol.0 && a < tol.1),
            "{name} {d:?}"
        );
    }
}

/// 書き出して読み直しても同じ板（種類・Mode・結晶軸・材料・厚さ）
#[test]
fn birefringent_zmx_roundtrip() {
    for name in ["b1", "b3", "b4_m0"] {
        let s = load(name);
        let txt = zmx_write::write(&s).unwrap();
        assert!(txt.contains("TYPE BIRE__IN \"\"") && txt.contains("TYPE BIRE_OUT"));
        let p = std::env::temp_dir().join(format!("lb_bire_{name}_{}.zmx", std::process::id()));
        std::fs::write(&p, &txt).unwrap();
        let opt = zmx::LoadOptions {
            catalog_dirs: vec![dir().join("../glass")],
            ..Default::default()
        };
        let r = zmx::load_with(&p, &opt).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(r.surfaces.len(), s.surfaces.len());
        for (a, b) in s.surfaces.iter().zip(&r.surfaces) {
            assert_eq!(a.bire, b.bire);
            assert_eq!(a.mat, b.mat);
            assert_eq!(a.t, b.t);
        }
    }
}

/// b1 の板を Mode mode・結晶軸 axis（入口の局所座標）にした系
fn b1_with(mode: u8, axis: [f64; 3]) -> System {
    let txt = zmx_write::write(&load("b1")).unwrap();
    let i = txt.find("TYPE BIRE__IN").unwrap();
    let j = i + txt[i..].find("SURF ").unwrap();
    let sec: String = txt[i..j]
        .split_inclusive("\r\n")
        .map(
            |l| match l.split_whitespace().take(2).collect::<Vec<_>>()[..] {
                ["PARM", "1"] => format!("  PARM 1 {mode}\r\n"),
                ["PARM", k @ ("2" | "3" | "4")] => {
                    let c = k.parse::<usize>().unwrap() - 2;
                    format!("  PARM {k} {:.17}\r\n", axis[c])
                }
                _ => l.to_string(),
            },
        )
        .collect();
    let txt = format!("{}{sec}{}", &txt[..i], &txt[j..]);
    // 並行の試験が同じ Mode・軸の系を作っても、ファイルが重ならないように
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "lb_bire_m{mode}_{}_{}_{seq}.zmx",
        axis.map(|x| (x * 1e6) as i64)
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join("_"),
        std::process::id()
    ));
    std::fs::write(&p, &txt).unwrap();
    let opt = zmx::LoadOptions {
        catalog_dirs: vec![dir().join("../glass")],
        ..Default::default()
    };
    let s = zmx::load_with(&p, &opt).unwrap();
    std::fs::remove_file(&p).ok();
    s
}

/// 入口の面の番号と、常光・異常光の屈折率（波長 w）
fn plate_index(s: &System, w: usize) -> (usize, f64, f64) {
    let j = s
        .surfaces
        .iter()
        .position(|d| matches!(d.bire, Some(lensbench_core::surface::Bire::In { .. })))
        .unwrap();
    let Some(lensbench_core::surface::Bire::In { ne, .. }) = &s.surfaces[j].bire else {
        unreachable!()
    };
    (j, s.n(j)[w], ne[w])
}

/// Mode 1・3（異常光の道）: 板（水晶 1 mm、結晶軸を面の法線から 30° 傾けた）に垂直に入る光線は、波面法線が
/// 法線のまま屈折率 n(θ) で進み、光線は walk-off の角 ρ だけ傾いて進む。出ると向きは元に戻り、像面では
/// 常光の道（Mode 0）より L tan ρ だけ結晶軸の側へずれ、光路長は (n(θ) − no) L だけ長い
#[test]
fn extraordinary_path_walks_off() {
    let th = 30f64.to_radians();
    let axis = [0.0, th.sin(), th.cos()];
    let o = b1_with(0, axis);
    for mode in [1, 3] {
        let s = b1_with(mode, axis);
        s.check_crystals().unwrap();
        // 書き出しても Mode は残る
        assert!(zmx_write::write(&s)
            .unwrap()
            .contains(&format!("  PARM 1 {mode}\r\n")));
        for w in 0..s.wavelengths.len() {
            let (j, no, ne) = plate_index(&s, w);
            let len = s.surfaces[j].t;
            let (sn, cs) = th.sin_cos();
            let nth = 1.0 / (cs * cs / (no * no) + sn * sn / (ne * ne)).sqrt();
            let tan = (ne * ne - no * no) * sn * cs / (no * no * sn * sn + ne * ne * cs * cs);
            let a = s
                .try_trace(
                    FieldSel::Norm(0.0, 0.0),
                    &[0.0],
                    &[0.0],
                    Some(w),
                    None,
                    false,
                )
                .unwrap()
                .rays[0];
            let b = o
                .try_trace(
                    FieldSel::Norm(0.0, 0.0),
                    &[0.0],
                    &[0.0],
                    Some(w),
                    None,
                    false,
                )
                .unwrap()
                .rays[0];
            assert!(a.status.is_ok(), "{:?}", a.status);
            assert!((a.xyz[0] - b.xyz[0]).abs() < 1e-12);
            assert!(
                (a.xyz[1] - b.xyz[1] - len * tan).abs() < 1e-12,
                "Mode {mode} w {w}: {} {}",
                a.xyz[1] - b.xyz[1],
                len * tan
            );
            for c in 0..3 {
                assert!((a.lmn[c] - b.lmn[c]).abs() < 1e-14);
            }
            assert!(
                (a.opl - b.opl - (nth - no) * len).abs() < 1e-12,
                "Mode {mode} w {w}: {} {}",
                a.opl - b.opl,
                (nth - no) * len
            );
        }
    }
}

/// Mode 1・3 の偏光: 垂直に入る光線では、Mode 3 の常光・異常光の位相の差は Mode 2（常光の道の波長板）と同じ
/// （全体の位相だけ違う）。Mode 1 は異常光の成分（結晶軸を波面法線に垂直な面へ写した向き）だけを通す
#[test]
fn extraordinary_path_polarization() {
    let th = 30f64.to_radians();
    let axis = [0.0, th.sin(), th.cos()];
    let pol = |s: &System, w: usize| {
        let coat = Coatings::for_system(s).unwrap();
        let pols = [
            [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
            [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
        ];
        s.try_trace_pol(
            FieldSel::Norm(0.0, 0.0),
            &[0.0],
            &[0.0],
            Some(w),
            PolIn::new(&pols, &coat),
        )
        .unwrap()
        .e[0]
            .to_vec()
    };
    let (m1, m2, m3) = (b1_with(1, axis), b1_with(2, axis), b1_with(3, axis));
    for w in 0..m2.wavelengths.len() {
        let (e1, e2, e3) = (pol(&m1, w), pol(&m2, w), pol(&m3, w));
        // Mode 3 と Mode 2 は全体の位相 g だけ違う
        let g = e3[0][0] / e2[0][0];
        assert!((g.norm() - 1.0).abs() < 1e-9, "{g}");
        for q in 0..2 {
            for c in 0..3 {
                let d = e3[q][c] - g * e2[q][c];
                assert!(
                    d.norm() < 1e-9,
                    "w {w} 入力 {q} 成分 {c}: {} {}",
                    e3[q][c],
                    e2[q][c]
                );
            }
        }
        // Mode 1: x の入力（常光の向き）は通らず、y の入力（異常光の向き）は通る
        let n = |e: &[C64]| e.iter().map(|x| x.norm_sqr()).sum::<f64>().sqrt();
        assert!(n(&e1[0]) < 1e-9, "{:?}", e1[0]);
        assert!(
            (n(&e1[1]) - n(&e2[1])).abs() < 1e-5,
            "{:?} {:?}",
            e1[1],
            e2[1]
        );
    }
}

/// 板の中に屈折する面がある Mode 1・3 の系は、追跡の入口で名指しで断る
#[test]
fn extraordinary_path_rejects_refracting_surface_inside() {
    let s = b1_with(1, [0.0, 0.5, 0.866]);
    s.check_crystals().unwrap();
    let txt = zmx_write::write(&s).unwrap();
    // 出口の面を標準面（屈折する面）にすると、板は出口なしで次の面の媒質へ屈折する
    let bad = txt.replacen("TYPE BIRE_OUT", "TYPE STANDARD", 1);
    let p = std::env::temp_dir().join(format!("lb_bire_inside_{}.zmx", std::process::id()));
    std::fs::write(&p, &bad).unwrap();
    let opt = zmx::LoadOptions {
        catalog_dirs: vec![dir().join("../glass")],
        ..Default::default()
    };
    let r = zmx::load_with(&p, &opt);
    std::fs::remove_file(&p).ok();
    // 出口の無い板を読込が断るか、追跡の入口が断る
    if let Ok(r) = r {
        let e = r.check_crystals().unwrap_err().to_string();
        assert!(e.contains("Mode 1"), "{e}");
    }
}

/// 結晶軸を面の法線から 30° 傾けた板（b1 の軸を (0, 0.5, 0.866) に）の Mode 0〜3 を OpticStudio 26.1 と照合する
/// （tests/birefringent/os_b5.json）。光線の位置・向き（異常光の walk-off を含む）と、Mode 1 の偏光（異常光の成分だけ）は
/// OS と合う。OS は異常光の道の光路長を no × 光線の長さで数え（物理では波数ベクトル m·ŝ × 長さ、垂直入射で n(θ)·厚さ）、
/// Mode 2・3 の常光・異常光の位相の差にも no·(光線の長さ − 厚さ) のずれが入る。ここではその差が式どおりかを確かめる
#[test]
fn extraordinary_path_matches_opticstudio() {
    let os: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("os_b5.json")).unwrap()).unwrap();
    let axis = [0.0, 0.5, 0.866_025_403_784_438_6];
    let f = |v: &serde_json::Value| v.as_f64().unwrap();
    let v3 = |v: &serde_json::Value| [f(&v[0]), f(&v[1]), f(&v[2])];
    let mut worst = (0.0f64, 0.0f64, 0.0f64);
    for mode in [0u8, 1] {
        let s = b1_with(mode, axis);
        let o = b1_with(0, axis);
        for r in os["rays"][mode.to_string()].as_array().unwrap() {
            let w = r["w"].as_u64().unwrap() as usize - 1;
            let fs = FieldSel::Norm(f(&r["hx"]), f(&r["hy"]));
            let (px, py) = ([f(&r["px"])], [f(&r["py"])]);
            let a = s
                .try_trace(fs, &px, &py, Some(w), None, false)
                .unwrap()
                .rays[0];
            let (x, l) = (v3(&r["xyz"]), v3(&r["lmn"]));
            for c in 0..3 {
                worst.0 = worst.0.max((a.xyz[c] - x[c]).abs());
                worst.1 = worst.1.max((a.lmn[c] - l[c]).abs());
            }
            // 光路長: Mode 0 は OS と同じ。Mode 1 は、こちらは異常光（m·ŝ × 長さ）、OS は no × 長さで数える。
            // 軸上の視野（板に垂直に入る）で、こちらの Mode 1 − Mode 0 は (n(θ) − no)·厚さ、OS のは no·厚さ·(1/cos ρ − 1)
            let want = if mode == 0 {
                f(&r["opl"])
            } else {
                let os0 = os["rays"]["0"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|q| ["hx", "hy", "w", "px", "py"].iter().all(|k| q[k] == r[k]))
                    .map(|q| f(&q["opl"]))
                    .unwrap();
                let b = o
                    .try_trace(fs, &px, &py, Some(w), None, false)
                    .unwrap()
                    .rays[0];
                if f(&r["hx"]) != 0.0 || f(&r["hy"]) != 0.0 {
                    continue;
                }
                let (j, no, ne) = plate_index(&s, w);
                let t = s.surfaces[j].t;
                let th = 30f64.to_radians();
                let (sn, cs) = th.sin_cos();
                let nth = 1.0 / (cs * cs / (no * no) + sn * sn / (ne * ne)).sqrt();
                let tan = (ne * ne - no * no) * sn * cs / (no * no * sn * sn + ne * ne * cs * cs);
                let os_extra = no * t * ((1.0 + tan * tan).sqrt() - 1.0);
                assert!(
                    ((f(&r["opl"]) - os0) - os_extra).abs() < 1e-9,
                    "OS の Mode 1 − Mode 0 {} 式 {os_extra}",
                    f(&r["opl"]) - os0
                );
                assert!(((a.opl - b.opl) - (nth - no) * t).abs() < 1e-12);
                // こちらの Mode 1 = OS の Mode 0 + (n(θ) − no)·厚さ
                os0 + (nth - no) * t
            };
            // 軸外の視野の光路長は、無限遠の物体の光路長の始まりの面の取り方が OS と違う（OS は瞳の点によらず同じ値）ので
            // 軸上の視野だけ比べる
            if f(&r["hx"]) == 0.0 && f(&r["hy"]) == 0.0 {
                worst.2 = worst.2.max((a.opl - want).abs());
            }
        }
    }
    eprintln!(
        "位置 {:.3e} mm、向き {:.3e}、光路長 {:.3e} mm",
        worst.0, worst.1, worst.2
    );
    assert!(
        worst.0 < 1e-9 && worst.1 < 1e-12 && worst.2 < 1e-9,
        "{worst:?}"
    );
}

/// os_b5.json の偏光（Mode k）とこちらの電場の、光線ごとの複素の比を除いた残りの最大と、|比| − 1 の最大
fn b5_pol_diff(os: &serde_json::Value, mode: u8, on_axis_only: bool) -> (f64, f64) {
    let s = b1_with(mode, [0.0, 0.5, 0.866_025_403_784_438_6]);
    let coat = Coatings::for_system(&s).unwrap();
    let pols = [
        [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
        [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
    ];
    let f = |v: &serde_json::Value| v.as_f64().unwrap();
    let (mut res, mut amp) = (0.0f64, 0.0f64);
    for r in os["pol"][mode.to_string()].as_array().unwrap() {
        if on_axis_only && (f(&r["hx"]) != 0.0 || f(&r["hy"]) != 0.0) {
            continue;
        }
        let w = r["w"].as_u64().unwrap() as usize - 1;
        let t = s
            .try_trace_pol(
                FieldSel::Norm(f(&r["hx"]), f(&r["hy"])),
                &[f(&r["px"])],
                &[f(&r["py"])],
                Some(w),
                PolIn::new(&pols, &coat),
            )
            .unwrap();
        let e = &t.e[0];
        let o = |q: usize, c: usize| C64::new(f(&r["e"][q][c][0]), f(&r["e"][q][c][1]));
        let (mut num, mut den) = (C64::new(0.0, 0.0), 0.0);
        for q in 0..2 {
            for c in 0..3 {
                num += e[q][c] * o(q, c).conj();
                den += o(q, c).norm_sqr();
            }
        }
        let k = num / den;
        amp = amp.max((k.norm() - 1.0).abs());
        for q in 0..2 {
            for c in 0..3 {
                res = res.max((e[q][c] - k * o(q, c)).norm());
            }
        }
    }
    (res, amp)
}

/// 偏光: Mode 1（異常光の成分だけ）は OS と合う。Mode 2・3 は、結晶軸が傾いて walk-off があると、OS の常光・異常光の
/// 位相の差に no·(光線の長さ − 厚さ) のずれが入るので合わない（OS の Mode 2 の光線は常光の道なのに、異常光の道の
/// 長さを使っているように見える）。ずれの大きさを確かめる
#[test]
fn extraordinary_path_polarization_vs_opticstudio() {
    let os: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("os_b5.json")).unwrap()).unwrap();
    for mode in [1u8, 2, 3] {
        let (res, amp) = b5_pol_diff(&os, mode, false);
        let (res0, amp0) = b5_pol_diff(&os, mode, true);
        eprintln!(
            "Mode {mode}: 残り {res:.3e}（軸上 {res0:.3e}）、|比| − 1 {amp:.3e}（軸上 {amp0:.3e}）"
        );
    }
    // Mode 1: 軸上は一致。軸外の差（1e-4 程度）は Mode 2 の板（b1〜b3）と同じ、OS の境界の係数の取り方の差
    let (res, amp) = b5_pol_diff(&os, 1, false);
    assert!(res < 5e-4 && amp < 5e-4, "{res} {amp}");
    let (res, amp) = b5_pol_diff(&os, 1, true);
    assert!(res < 1e-12 && amp < 1e-5, "{res} {amp}");
    // Mode 2・3 の軸上: 常光・異常光の位相の差（入力 y の Ey と入力 x の Ex の位相の差）は、こちらが
    // 2π(n(θ) − no)·厚さ/λ、OS はそれに 2π·no·厚さ·(1/cos ρ − 1)/λ を足した（符号は位相の約束による）もの
    let f = |v: &serde_json::Value| v.as_f64().unwrap();
    let th = 30f64.to_radians();
    for mode in [2u8, 3] {
        let s = b1_with(mode, [0.0, 0.5, 0.866_025_403_784_438_6]);
        let coat = Coatings::for_system(&s).unwrap();
        let pols = [
            [C64::new(1.0, 0.0), C64::new(0.0, 0.0)],
            [C64::new(0.0, 0.0), C64::new(1.0, 0.0)],
        ];
        for r in os["pol"][mode.to_string()].as_array().unwrap() {
            if f(&r["hx"]) != 0.0 || f(&r["hy"]) != 0.0 || f(&r["px"]) != 0.0 || f(&r["py"]) != 0.0
            {
                continue;
            }
            let w = r["w"].as_u64().unwrap() as usize - 1;
            let e = s
                .try_trace_pol(
                    FieldSel::Norm(0.0, 0.0),
                    &[0.0],
                    &[0.0],
                    Some(w),
                    PolIn::new(&pols, &coat),
                )
                .unwrap()
                .e[0]
                .to_vec();
            let ours = (e[1][1] / e[0][0]).arg();
            let o = |q: usize, c: usize| C64::new(f(&r["e"][q][c][0]), f(&r["e"][q][c][1]));
            let theirs = (o(1, 1) / o(0, 0)).arg();
            let (_, no, ne) = plate_index(&s, w);
            let t = 1.0;
            let (sn, cs) = th.sin_cos();
            let tan = (ne * ne - no * no) * sn * cs / (no * no * sn * sn + ne * ne * cs * cs);
            let lam = s.wavelengths[w] * 1e-3;
            let extra =
                2.0 * std::f64::consts::PI * no * t * ((1.0 + tan * tan).sqrt() - 1.0) / lam;
            let d = theirs - ours;
            // 2π の巻きを除いて、OS − こちら = extra
            let d = (d - extra + std::f64::consts::PI).rem_euclid(2.0 * std::f64::consts::PI)
                - std::f64::consts::PI;
            eprintln!("Mode {mode} w {w}: OS − こちら の位相の差の残り {d:.3e} rad（ずれ {extra:.4} rad）");
            assert!(d.abs() < 2e-3, "Mode {mode} w {w}: {d}");
        }
    }
}

/// 解析的な微分は Mode 1・3 の板を表の屈折率の等方な媒質として扱うので、その板を通る光線では断る
/// （最適化は差分に落ちる）。Mode 0 の板は今までどおり
#[test]
fn extraordinary_path_refuses_analytic_derivatives() {
    use lensbench_core::error::Error;
    use lensbench_core::param::{Kind, Param};
    let axis = [0.0, 0.5, 0.866_025_403_784_438_6];
    for mode in [0, 1, 3] {
        let s = b1_with(mode, axis);
        let (j, _, _) = plate_index(&s, 0);
        let ps = [Param::new(j, Kind::Thick)];
        let t = s
            .try_trace(
                FieldSel::Norm(0.0, 0.0),
                &[0.0, 0.5],
                &[0.3, 0.0],
                Some(0),
                None,
                true,
            )
            .unwrap();
        let mut g = vec![0.0; 2];
        let r = [
            s.opl_grad(&t, 0, &ps).map(|_| ()),
            s.land_grad(&t, 0, &ps, None).map(|_| ()),
            s.opl_grad_shift(&t, 0, &ps, 0.1, &mut g),
        ];
        for e in r {
            if mode == 0 {
                e.unwrap();
            } else {
                let e = e.unwrap_err();
                assert!(matches!(e, Error::NotImplemented(_)), "{e}");
                assert!(e.to_string().contains("extraordinary"), "{e}");
            }
        }
    }
}

/// .zmx を読むと、OpticStudio と値の数え方が違う板（Mode 1・3 の光路長、結晶軸を傾けた Mode 2・3 の位相差）を
/// 読込の注意で知らせる
#[test]
fn opticstudio_differences_are_noted_on_load() {
    let tilted = [0.0, 0.5, 0.866_025_403_784_438_6];
    let count = |s: &System, key: &str| s.import_notes.iter().filter(|n| n.contains(key)).count();
    for (mode, axis, opl, ret) in [
        (0, tilted, 0, 0),
        (1, tilted, 1, 0),
        (2, tilted, 0, 1),
        (3, tilted, 1, 1),
        (2, [0.0, 1.0, 0.0], 0, 0),
        (3, [0.0, 0.0, 1.0], 1, 0),
    ] {
        let s = b1_with(mode, axis);
        assert_eq!(
            count(&s, "extraordinary ray is counted"),
            opl,
            "Mode {mode} {:?}",
            s.import_notes
        );
        assert_eq!(
            count(&s, "retardance differs"),
            ret,
            "Mode {mode} {:?}",
            s.import_notes
        );
    }
}
