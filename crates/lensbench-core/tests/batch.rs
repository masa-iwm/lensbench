//! まとめ追跡（SIMD、`batch`）が 1 本ずつの追跡とビット単位で一致する。
//!
//! 同じ系・光線を、まとめ追跡を切って追った結果と比べる（交点・方向・光路長・止まった面・けられた面・
//! 面ごとの記録、実光線エイミングを含む）。光線は瞳の外（けられ・追跡の失敗）まで含み、本数は L の倍数にしない。

use std::path::PathBuf;
use std::sync::Mutex;

use lensbench_core::batch;
use lensbench_core::freeform::{GridSag, ZernSag};
use lensbench_core::surface::{Aperture, SurfType};
use lensbench_core::system::Trace;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{FieldSel, System};
use std::sync::Arc;

// まとめ追跡の切り替えはプロセス全体なので、テストを 1 つずつ動かす
static LOCK: Mutex<()> = Mutex::new(());

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(name: &str) -> System {
    // wasm（WASI）では系・ガラスの読み込みが vfs の置き場を見るので、ディスクから写す（公開 #99）
    #[cfg(target_arch = "wasm32")]
    for p in [
        root().join(format!("tests/refs/{name}/system.zmx")),
        root().join("tests/glass"),
    ] {
        mirror(&p);
    }
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root().join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

#[cfg(target_arch = "wasm32")]
fn mirror(p: &std::path::Path) {
    if p.is_dir() {
        for e in std::fs::read_dir(p).unwrap().flatten() {
            mirror(&e.path());
        }
    } else {
        lensbench_core::vfs::insert(p, std::fs::read(p).unwrap()).unwrap();
    }
}

fn grid(n: usize, r: f64) -> (Vec<f64>, Vec<f64>) {
    let mut px = Vec::new();
    let mut py = Vec::new();
    for i in 0..n {
        for j in 0..n {
            px.push(-r + 2.0 * r * i as f64 / (n - 1) as f64);
            py.push(-r + 2.0 * r * j as f64 / (n - 1) as f64);
        }
    }
    (px, py)
}

fn bits3(v: &[f64; 3]) -> [u64; 3] {
    v.map(f64::to_bits)
}

fn same(a: &Trace, b: &Trace, what: &str) {
    assert_eq!(a.rays.len(), b.rays.len());
    for (i, (x, y)) in a.rays.iter().zip(&b.rays).enumerate() {
        let w = format!("{what} 光線 {i}");
        assert_eq!((x.err, x.vig), (y.err, y.vig), "{w}");
        assert_eq!(bits3(&x.xyz), bits3(&y.xyz), "{w}");
        assert_eq!(bits3(&x.lmn), bits3(&y.lmn), "{w}");
        assert_eq!(bits3(&x.p), bits3(&y.p), "{w}");
        assert_eq!(bits3(&x.k), bits3(&y.k), "{w}");
        assert_eq!(x.opl.to_bits(), y.opl.to_bits(), "{w}");
        assert_eq!(x.lens_opd.to_bits(), y.lens_opd.to_bits(), "{w}");
        assert_eq!(bits3(&a.p0[i]), bits3(&b.p0[i]), "{w}");
        assert_eq!(bits3(&a.k0[i]), bits3(&b.k0[i]), "{w}");
        assert_eq!(a.opl_os[i].to_bits(), b.opl_os[i].to_bits(), "{w}");
    }
    assert_eq!(a.rec.len(), b.rec.len());
    for (ra, rb) in a.rec.iter().zip(&b.rec) {
        for i in 0..a.rays.len() {
            let (x, y) = (ra.entry(i), rb.entry(i));
            let w = format!("{what} 面 {} 光線 {i}", ra.surf);
            assert_eq!(bits3(&x.xyz), bits3(&y.xyz), "{w}");
            assert_eq!(bits3(&x.lmn), bits3(&y.lmn), "{w}");
            assert_eq!(bits3(&x.p), bits3(&y.p), "{w}");
            assert_eq!(x.opl.to_bits(), y.opl.to_bits(), "{w}");
        }
    }
}

/// 全視野・全波長、record あり・なしで、まとめ追跡あり・なしが一致する
fn check(name: &str, s: &System, px: &[f64], py: &[f64]) -> usize {
    let mut stopped = 0;
    let mut compared = 0;
    for fi in 0..s.fields.len() {
        for w in 0..s.wavelengths.len() {
            for record in [false, true] {
                let f = FieldSel::Index(fi);
                batch::set_enabled(false);
                let a = s.try_trace(f, px, py, Some(w), None, record);
                batch::set_enabled(true);
                let b = s.try_trace(f, px, py, Some(w), None, record);
                let (Ok(a), Ok(b)) = (a, b) else {
                    continue;
                };
                same(
                    &a,
                    &b,
                    &format!("{name} 視野 {fi} 波長 {w} record {record}"),
                );
                compared += 1;
                stopped += a.rays.iter().filter(|r| r.err != 0 || r.vig != 0).count();
            }
        }
    }
    assert!(compared > 0, "{name}: 比べた追跡がない");
    stopped
}

#[test]
fn refs_match_scalar() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 37² = 1369 本（L の倍数でない）、瞳の 3 倍まで（面を外れる・全反射する光線を含む）。
    // wasm（CI の Node の WASI、1 スレッド）は 13² = 169 本に減らす
    let (px, py) = grid(if cfg!(target_arch = "wasm32") { 13 } else { 37 }, 3.0);
    let mut names: Vec<String> = std::fs::read_dir(root().join("tests/refs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut stopped = 0;
    for n in &names {
        stopped += check(n, &load(n), &px, &py);
    }
    assert!(stopped > 0, "けられ・失敗の光線を含まない");
}

/// 開口の形（円環・遮蔽・矩形・楕円）とトロイダル面。開口の境界の近くを細かく通す
#[test]
fn apertures_and_toroidal_match_scalar() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (px, py) = grid(73, 1.2);
    let mut s = load("A0");
    let semi = s.surfaces[2].semi.max(1.0);
    s.surfaces[2].ap = Aperture::Circular {
        min: 0.2 * semi,
        max: 0.9 * semi,
        dx: 0.05 * semi,
        dy: -0.03 * semi,
    };
    s.surfaces[3].ap = Aperture::Obsc {
        min: 0.0,
        max: 0.15 * semi,
        dx: 0.0,
        dy: 0.02 * semi,
    };
    s.surfaces[4].ap = Aperture::Rect {
        hx: 0.8 * semi,
        hy: 0.7 * semi,
        dx: 0.01,
        dy: 0.0,
    };
    s.surfaces[5].ap = Aperture::Ellipse {
        hx: 0.85 * semi,
        hy: 0.75 * semi,
        dx: 0.0,
        dy: 0.02,
    };
    let v = check("A0+開口", &s, &px, &py);
    assert!(v > 100, "けられた光線が少ない（{v}）");

    // スパイダー（腕 3 本、偏心）
    let mut s = load("A0");
    let semi = s.surfaces[3].semi;
    s.surfaces[3].ap = Aperture::Spider {
        width: 0.2 * semi,
        arms: 3,
        dx: 0.05 * semi,
        dy: -0.02 * semi,
    };
    let v = check("A0+スパイダー", &s, &px, &py);
    assert!(v > 50, "けられた光線が少ない（{v}）");

    let mut s = load("A0");
    let d = &mut s.surfaces[1];
    d.typ = SurfType::Toroidal;
    d.cx = d.c * 0.7;
    d.asph = vec![1e-5, -2e-7];
    check("A0+トロイダル", &s, &px, &py);

    // Zernike・Grid Sag
    let mut s = load("A0");
    let d = &mut s.surfaces[6];
    d.typ = SurfType::ZernikeStd;
    d.zern = Some(Arc::new(ZernSag::new(
        false,
        5.0,
        (1..=37).map(|i| 1e-3 * (i as f64).sin()).collect(),
        true,
    )));
    check("A0+Zernike", &s, &px, &py);
    let mut s = load("A0");
    let d = &mut s.surfaces[3];
    d.typ = SurfType::GridSag;
    let (nx, ny) = (13, 11);
    d.grid = Some(Arc::new(GridSag {
        nx,
        ny,
        dx: 0.9,
        dy: 1.0,
        xdec: 0.1,
        ydec: -0.2,
        z: (0..nx * ny)
            .map(|k| {
                let a = k as f64 * 0.37;
                [1e-3 * a.sin(), 1e-4 * a.cos(), -2e-4 * a.sin(), 1e-5]
            })
            .collect(),
        // 値のない点を混ぜ、そこに掛かる升目で止まる光線を作る（格子の端で止まる数本だけだと、視野の最後の桁でも
        // 数が変わる）
        nodata: (0..nx * ny).map(|k| k % 17 == 5).collect(),
        linear: false,
    }));
    let v = check("A0+Grid Sag", &s, &px, &py);
    assert!(v > 100, "値のない升目で止まる光線が少ない（{v}）");
}

/// 少ない本数（1 本ずつの方へ回る）と、まとめ追跡の端の組
#[test]
fn small_counts_match_scalar() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = load("C1");
    for n in [1, 3, 4, 5, 15, 16, 17, 33] {
        let px: Vec<f64> = (0..n).map(|i| -0.9 + 1.8 * i as f64 / n as f64).collect();
        let py: Vec<f64> = (0..n).map(|i| 0.5 - 0.04 * i as f64).collect();
        check(&format!("C1 {n} 本"), &s, &px, &py);
    }
}

/// 絞り面の実座標（Launch::Stop）、開始位置と方向（Launch::Rays）、符号つきの瞳座標の追跡
#[test]
fn launches_match_scalar() {
    use lensbench_core::system::{Launch, ObjSpec};
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (px, py) = grid(23, 1.3);
    let mut names: Vec<String> = std::fs::read_dir(root().join("tests/refs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let both = |s: &System, l: &Launch, record: bool, what: &str| {
        batch::set_enabled(false);
        let a = s.trace_launch(l, s.primary, None, record);
        batch::set_enabled(true);
        let b = s.trace_launch(l, s.primary, None, record);
        same(&a, &b, what);
    };
    for n in &names {
        let s = load(n);
        let spec: ObjSpec = {
            let (hx, hy) = s.hxhy(s.fields.len() - 1);
            s.object_spec(hx, hy).unwrap()
        };
        let semi = s.surfaces[s.stop()].semi.max(1.0);
        let x: Vec<f64> = px.iter().map(|v| v * semi).collect();
        let y: Vec<f64> = py.iter().map(|v| v * semi).collect();
        for record in [false, true] {
            both(
                &s,
                &Launch::Stop { spec, x: &x, y: &y },
                record,
                &format!("{n} Stop record {record}"),
            );
            let l = Launch::Pupil {
                spec,
                signed: true,
                px: &px,
                py: &py,
                vig: None,
                hxy: None,
            };
            both(&s, &l, record, &format!("{n} Pupil signed record {record}"));
            // 瞳座標の光線の開始点・方向を少しずらして直接渡す（長さ 0 の方向も含む）
            let t = s.trace_launch(&l, s.primary, None, false);
            let p: Vec<_> = t.p0.iter().map(|p| [p[0] + 0.01, p[1], p[2]]).collect();
            let mut k: Vec<_> =
                t.k0.iter()
                    .map(|k| [k[0] * 2.0, k[1] * 2.0 + 0.01, k[2] * 2.0])
                    .collect();
            k[3] = [0.0; 3];
            both(
                &s,
                &Launch::Rays { p: &p, k: &k },
                record,
                &format!("{n} Rays record {record}"),
            );
        }
    }
}
