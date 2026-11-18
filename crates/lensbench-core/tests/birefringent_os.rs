//! 複屈折材の板（OpticStudio の Birefringent In / Out、Mode 2 の波長板と Mode 0）を OpticStudio 26.1 と照合する。
//! 系（tests/birefringent）: 絞り → 空気 5 → 板 → 空気 20 → 像面、波長 0.55・0.6328・0.8 µm、視野 角度 (0,0)・(0,3)・(2,−2)°。
//! b1 は水晶 1 mm（多次、結晶軸 30°）、b2 は水晶 0.0175 mm（633 nm で λ/4）、b3 は水晶 0.0175 mm（30°）と MgF₂ 0.0141 mm
//! （120°）の貼り合わせ、b4_m0 は b1 の Mode 0（常光の成分だけ）、b2_coat は b2 の両面に理想膜 I.99999（LB379.DAT）、
//! b2_ax0・b2_ax60 は b2 の結晶軸を 0°・60° にしたもの。基準は Normalized Polarized の像面の電場（入力 x・y）
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::system::{FieldSel, PolIn, System};
use lensbench_core::zmx;
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

/// Mode 1・3（異常光の道）の板も読んで持ち（書き出しても残る）、追跡はどの入口でも常光の道で黙って追わない:
/// 誤りを返す入口（光線・偏光）は Mode を名指しで断り、誤りを返さない入口でも光線はその面で止まる
#[test]
fn extraordinary_modes_load_but_are_not_traced() {
    let b1 = zmx_write::write(&load("b1")).unwrap();
    assert_eq!(
        b1.matches("  PARM 1 2\r\n").count(),
        1,
        "b1 の入口の Mode 2 の行"
    );
    for mode in [1, 3] {
        let txt = b1.replacen("  PARM 1 2\r\n", &format!("  PARM 1 {mode}\r\n"), 1);
        let p = std::env::temp_dir().join(format!("lb_bire_m{mode}_{}.zmx", std::process::id()));
        std::fs::write(&p, &txt).unwrap();
        let opt = zmx::LoadOptions {
            catalog_dirs: vec![dir().join("../glass")],
            ..Default::default()
        };
        let s = zmx::load_with(&p, &opt).unwrap();
        std::fs::remove_file(&p).ok();
        let j = s
            .surfaces
            .iter()
            .position(|d| matches!(d.bire, Some(lensbench_core::surface::Bire::In { .. })))
            .unwrap();
        assert!(matches!(
            s.surfaces[j].bire,
            Some(lensbench_core::surface::Bire::In { mode: m, .. }) if m == mode
        ));
        // 書き出しても Mode は残る
        assert!(zmx_write::write(&s)
            .unwrap()
            .contains(&format!("  PARM 1 {mode}\r\n")));
        // 系の検査（プロジェクトの展開）は通る: ファイルは開ける
        s.validate().unwrap();
        let want = format!("Mode {mode}");
        let e = s.check_crystals().unwrap_err().to_string();
        assert!(e.contains(&want), "{e}");
        let e = s
            .try_trace(FieldSel::Index(0), &[0.0], &[0.0], None, None, false)
            .unwrap_err()
            .to_string();
        assert!(e.contains(&want), "{e}");
        let mut buf = lensbench_core::system::RayBuf::default();
        let e = s
            .try_trace_into(FieldSel::Index(0), &[0.0], &[0.0], None, None, &mut buf)
            .unwrap_err()
            .to_string();
        assert!(e.contains(&want), "{e}");
        let coat = Coatings::for_system(&s).unwrap();
        let pols = [[C64::new(1.0, 0.0), C64::new(0.0, 0.0)]];
        let e = s
            .try_trace_pol(
                FieldSel::Index(0),
                &[0.0],
                &[0.0],
                None,
                PolIn::new(&pols, &coat),
            )
            .unwrap_err()
            .to_string();
        assert!(e.contains(&want), "{e}");
        // 誤りを返さない入口（光線の開始点と方向を与える追跡）でも、光線は板の入口の面で止まる
        let p0 = [[0.0, 0.0, -1.0], [0.5, 0.0, -1.0]];
        let k0 = [[0.0, 0.0, 1.0], [0.0, 0.01, 1.0]];
        let l = lensbench_core::system::Launch::Rays { p: &p0, k: &k0 };
        s.trace_launch_into(&l, 0, None, &mut buf);
        for i in 0..2 {
            assert_eq!(
                buf.status[i].surf(),
                Some(j),
                "光線 {i}: {:?}",
                buf.status[i]
            );
        }
    }
}
