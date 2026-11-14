//! GRIN の面（Gradium＝GRINSUR8、Gradient 9＝GRINSUR9）を OpticStudio と比べる。
//!
//! 基準は tests/grin（独自係数の合成データ LBTEST.GRD・SGRIN.DAT・GRADIENT_9.DAT と
//! 独自処方の系、OpticStudio 26.1 の OPTH・REAX… と OPDC、非公開側で測定）。
//! - G8A: Gradium（R 8、厚さ 3、Δz 1.5、LBTG1）+ R −20、視野 y 0・2°
//! - G8B: G8A に偏心 dx 0.1・dy −0.05、傾き tx 0.01・ty 0.02、視野 (0,0)・(0,2)・(2,0)°
//! - G8C: G8B の偏心・傾きを dx 0.2・dy 0.1・tx 0.05・ty 0.04 に（tx·dx + ty·dy ≠ 0 で dx・dy の符号が効く）
//! - G9A: Gradient 9（平面、厚さ 4、LBTSLW、tanα 0.05、tanβ −0.1）+ R −15
//!
//! OpticStudio は媒質内を固定刻み Δt で積分する。測定は Δt 0.001（収束した値）。
//! lensbench は刻みを自動で決めて収束値を出す（Δt は使わない）。
//! G9A の rays_dtnom.json・sysdata_dtnom.txt は Δt 0.5 のときの値で、収束値と 4e-6 ずれる（参考）。

use crate::common::root;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::zmx_write;
use lensbench_core::{analysis, FieldSel, System};
use serde_json::Value;

fn load(name: &str) -> (System, Value) {
    let dir = root().join("tests/grin").join(name);
    let s = zmx::load_with(
        &dir.join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/grin"), root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("rays.json")).unwrap()).unwrap();
    (s, v)
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

/// 光線ごと・面ごとの差の最大（位置 mm、方向、光路長 mm（面 1 からの差））
fn ray_diff(name: &str) -> (f64, f64, f64, usize) {
    let (s, v) = load(name);
    let (mut dp, mut dd, mut dl, mut n) = (0.0f64, 0.0f64, 0.0f64, 0);
    for r in v["rays"].as_array().unwrap() {
        let w = r["wave"].as_u64().unwrap() as usize;
        let (hx, hy) = (f(&r["Hx"]), f(&r["Hy"]));
        let t = s
            .try_trace(
                FieldSel::Norm(hx, hy),
                &[f(&r["Px"])],
                &[f(&r["Py"])],
                Some(w),
                None,
                true,
            )
            .unwrap();
        let surfs = r["surfaces"].as_array().unwrap();
        let ok = surfs.last().unwrap()["err"].as_f64().unwrap() == 0.0;
        assert_eq!(ok, t.rays[0].err == 0, "{name}: {r}");
        if !ok {
            continue;
        }
        let o1 = t.rec[0].opl[0];
        let z1 = f(&surfs[0]["opth"]);
        for (rs, z) in t.rec.iter().zip(surfs) {
            assert_eq!(rs.surf as u64, z["surf"].as_u64().unwrap());
            let e = rs.entry(0);
            for (a, key) in ["x", "y", "z"].iter().enumerate() {
                dp = dp.max((e.xyz[a] - f(&z[key])).abs());
            }
            for (a, key) in ["l", "m", "n"].iter().enumerate() {
                dd = dd.max((rs.parity * e.lmn[a] - f(&z[key])).abs());
            }
            dl = dl.max(((e.opl - o1) - (f(&z["opth"]) - z1)).abs());
        }
        n += 1;
    }
    println!("{name}: {n} 本、位置 {dp:.1e}、方向 {dd:.1e}、光路長 {dl:.1e}");
    (dp, dd, dl, n)
}

#[test]
fn gradium_rays() {
    for name in ["G8A", "G8B", "G8C"] {
        let (dp, dd, dl, n) = ray_diff(name);
        assert!(n >= 50);
        assert!(
            dp < 1e-7 && dd < 1e-7 && dl < 1e-6,
            "{name}: {dp:e} {dd:e} {dl:e}"
        );
    }
}

#[test]
fn grad9_rays() {
    let (dp, dd, dl, n) = ray_diff("G9A");
    assert!(n >= 80);
    assert!(dp < 1e-7 && dd < 1e-7 && dl < 1e-6, "{dp:e} {dd:e} {dl:e}");
}

/// 近軸の EFL。lensbench は小さい実光線の極限と一致する（刻みを詰めて確かめた）。OpticStudio の近軸は
/// GRIN の中を近似しており G8A で 7e-7、G9A で 2.3e-6 ずれる。G8B（分布の軸を傾けた系）は近軸の定義が
/// どちらも近似（lensbench は軸上の屈折率の変化だけ入れる）で 5e-5 ずれる。G8C（傾きを大きくした系）は 1.1e-3 ずれる
/// （小さい実光線の y の EFL 8.460561 に対し lensbench −2.8e-4、OS +7.8e-4。x の EFL は 8.460746 で、近軸の EFL は
/// 一つに決まらない）
#[test]
fn paraxial_efl() {
    for (name, efl, tol) in [
        ("G8A", 8.463892301271393, 1e-6),
        ("G8B", 8.46383632380902, 1e-4),
        ("G8C", 8.461342460927407, 1.5e-3),
        ("G9A", 1.9597927422854062, 3e-6),
    ] {
        let (s, _) = load(name);
        let e = s.paraxial().efl;
        println!("{name}: EFL {e} (OS {efl})");
        assert!((e - efl).abs() < tol, "{name}: {e}");
    }
}

/// パラメータ 3（Ref n）は頂点の基準波長の屈折率で、OpticStudio の値と一致する
#[test]
fn gradium_ref_n() {
    for (name, rn) in [
        ("G8A", 1.721734375),
        ("G8B", 1.721741039989491),
        ("G8C", 1.7220380626866847),
    ] {
        let (s, _) = load(name);
        let g = s.surfaces[1].grin.as_ref().unwrap();
        let r = g.ref_index().unwrap();
        println!("{name}: Ref n {r} (OS {rn})");
        assert!((r - rn).abs() < 1e-12, "{name}: {r}");
    }
}

/// OPDC（主光線基準、波長単位）。G9A は射出瞳（小さい視野の実主光線から求める）が OpticStudio と 7e-5 mm
/// ずれ（lensbench の値は刻みを詰めても変わらない。OS は視野の差 1e-6 rad 程度の光線の追跡の誤差を含むとみる）、
/// その分 4.5e-4 波ずれる
#[test]
fn opd_matches_opdc() {
    for (name, tol) in [("G8A", 1e-5), ("G8B", 1e-5), ("G8C", 1e-5), ("G9A", 1e-3)] {
        let (s, _) = load(name);
        let dir = root().join("tests/grin").join(name);
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("opd.json")).unwrap()).unwrap();
        let px: Vec<f64> = v["pupil_px"].as_array().unwrap().iter().map(f).collect();
        let py: Vec<f64> = v["pupil_py"].as_array().unwrap().iter().map(f).collect();
        let fi = v["field"].as_u64().unwrap() as usize;
        let o = analysis::opd(&s, FieldSel::Index(fi), &px, &py, None, true).unwrap();
        let mut worst = 0.0f64;
        for (k, r) in v["opd_waves_OPDC"].as_array().unwrap().iter().enumerate() {
            worst = worst.max((o.opd[k] - f(r)).abs());
        }
        println!("{name}: OPDC の差 {worst:.1e} 波");
        assert!(worst < tol, "{name}: {worst:e}");
    }
}

/// 書き出した .zmx を読み直すと同じ追跡になる（GRD ファイル名も引き継ぐ）
#[test]
fn write_roundtrip() {
    let tmp = std::env::temp_dir().join(format!("lb_grin_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    for name in ["G8A", "G8B", "G8C", "G9A"] {
        let (s, _) = load(name);
        let p = tmp.join(format!("{name}.zmx"));
        std::fs::write(&p, zmx_write::write(&s).unwrap()).unwrap();
        let s2 = zmx::load_with(
            &p,
            &LoadOptions {
                cfg: 1,
                catalog_dirs: vec![root().join("tests/grin"), root().join("tests/glass")],
                ..Default::default()
            },
        )
        .unwrap();
        let px = [0.0, 0.3, -0.7, 1.0];
        let py = [0.0, 0.9, 0.2, -0.4];
        for fi in 0..s.fields.len() {
            let a = s
                .try_trace(FieldSel::Index(fi), &px, &py, None, None, false)
                .unwrap();
            let b = s2
                .try_trace(FieldSel::Index(fi), &px, &py, None, None, false)
                .unwrap();
            for (x, y) in a.rays.iter().zip(&b.rays) {
                assert_eq!(x.err, y.err, "{name}");
                assert_eq!(x.xyz, y.xyz, "{name}");
                assert_eq!(x.lmn, y.lmn, "{name}");
                assert_eq!(x.opl.to_bits(), y.opl.to_bits(), "{name}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// (dx, dy, tx, ty, tz) の座標ブレーク（厚さ t）
fn cb(d: [f64; 5], order: u8, t: f64) -> lensbench_core::surface::Surface {
    lensbench_core::surface::Surface {
        typ: lensbench_core::surface::SurfType::CoordBrk,
        mat: "-".into(),
        cb: d,
        cb_order: order,
        t,
        n: vec![1.0; 24],
        ..Default::default()
    }
}

/// 像面での光線（位置・方向・光路長・止まり）
fn image_rays(s: &System) -> Vec<f64> {
    let px = [0.0, 0.3, -0.7, 1.0, 0.0, -0.5];
    let py = [0.0, 0.9, 0.2, -0.4, -1.0, 0.5];
    let mut out = Vec::new();
    for fi in 0..s.fields.len() {
        for w in 0..s.wavelengths.len() {
            let t = s
                .try_trace(FieldSel::Index(fi), &px, &py, Some(w), None, false)
                .unwrap();
            for r in &t.rays {
                assert_eq!(r.err, 0);
                out.extend(r.xyz.iter().chain(&r.lmn));
                out.push(r.opl);
            }
        }
    }
    out
}

fn max_diff(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f64::max)
}

/// GRIN の媒質の中の座標ブレーク: 媒質は GRIN の面の座標のまま続く。厚さを座標ブレークで分けても、
/// 傾けて戻す座標ブレークの組を挟んでも、元の系と同じ追跡・近軸量になる
#[test]
fn coordinate_breaks_inside_the_medium() {
    for name in ["G8A", "G8B", "G9A"] {
        let (s, _) = load(name);
        let t = s.surfaces[1].t;
        let base = image_rays(&s);
        let efl = s.paraxial().efl;
        // 厚さを分ける
        let mut a = s.clone();
        a.surfaces[1].t = 0.4 * t;
        a.surfaces.insert(2, cb([0.0; 5], 0, 0.6 * t));
        a.changed().unwrap();
        let d = max_diff(&base, &image_rays(&a));
        println!(
            "{name}: 厚さを分ける {d:.1e}、EFL {}",
            a.paraxial().efl - efl
        );
        assert!(d < 1e-9, "{name}: {d:e}");
        assert!((a.paraxial().efl - efl).abs() < 1e-9);
        // 傾けて戻す
        let g = [0.05, -0.1, 3.0, -2.0, 1.0];
        let mut b = s.clone();
        b.surfaces[1].t = 0.4 * t;
        b.surfaces.insert(2, cb(g, 0, 0.0));
        b.surfaces
            .insert(3, cb([-g[0], -g[1], -g[2], -g[3], -g[4]], 1, 0.6 * t));
        b.changed().unwrap();
        let d = max_diff(&base, &image_rays(&b));
        println!("{name}: 傾けて戻す {d:.1e}");
        assert!(d < 1e-9, "{name}: {d:e}");
        // 偏光の追跡も同じ電場（最後の面の局所座標）
        let pol = |x: &System| {
            let c = lensbench_core::coating::Coatings::default();
            let one = lensbench_core::coating::C64::new(1.0, 0.0);
            let zero = lensbench_core::coating::C64::new(0.0, 0.0);
            let p = x
                .try_trace_pol(
                    FieldSel::Index(x.fields.len() - 1),
                    &[0.3],
                    &[0.2],
                    None,
                    &[[one, zero], [zero, one]],
                    &c,
                )
                .unwrap();
            p.e[0].clone()
        };
        let (e0, e1) = (pol(&s), pol(&b));
        let mut de = 0.0f64;
        for (u, v) in e0.iter().zip(&e1) {
            for k in 0..3 {
                de = de.max((u[k] - v[k]).norm());
            }
        }
        println!("{name}: 偏光の電場 {de:.1e}");
        assert!(de < 1e-6, "{name}: {de:e}");
    }
}

/// GRIN の面だけを動かす玉・群（範囲の後ろの座標ブレークが媒質の中に入る）も、座標ブレークにした系と一致する
#[test]
fn unit_motion_inside_the_medium() {
    for name in ["G8A", "G9A"] {
        let (mut s, _) = load(name);
        s.define_unit("F", &[[1, 1]], [0.0; 3]).unwrap();
        let ch = [
            (s.param("F.tx").unwrap(), 0.5),
            (s.param("F.dy").unwrap(), 0.02),
            (s.param("lens1.ty").unwrap(), -0.3),
        ];
        let a = s.with_params(&ch).unwrap();
        let f = a.flatten_units().unwrap();
        assert!(f.surfaces.len() > a.surfaces.len());
        let d = max_diff(&image_rays(&a), &image_rays(&f));
        println!("{name}: 玉・群の動きと座標ブレーク {d:.1e}");
        assert!(d < 1e-9, "{name}: {d:e}");
    }
}

/// GRIN の媒質の次の面にできない面タイプは読込で断る
#[test]
fn unsupported_surface_after_the_medium_is_rejected() {
    let (s, _) = load("G8A");
    let mut a = s.clone();
    a.surfaces.insert(2, cb([0.0, 0.0, 1.0, 0.0, 0.0], 0, 0.0));
    a.surfaces[3].typ = lensbench_core::surface::SurfType::Paraxial;
    a.surfaces[3].focal = 50.0;
    let e = a.changed().unwrap_err().to_string();
    assert!(e.contains("GRIN"), "{e}");
}
