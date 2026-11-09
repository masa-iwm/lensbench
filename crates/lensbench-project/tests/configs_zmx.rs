//! 経路・状態ごとの系を 1 つの多重構成の .zmx にまとめる: 読み戻した各構成が元の系と同じに追跡できる。

use lensbench_core::system::FieldSel;
use lensbench_core::{zmx, zmx_write, System};
use lensbench_project::{expand_path, paths, Env, Project};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn env() -> Env<'static> {
    Env {
        base: root().join("examples/projects"),
        glass_dirs: vec![root().join("tests/glass")],
        catalog: None,
    }
}

fn project(name: &str) -> Project {
    let t = std::fs::read_to_string(root().join("examples/projects").join(name)).unwrap();
    Project::from_json(&t).unwrap()
}

const PX: [f64; 5] = [0.0, 0.7, -0.5, 0.3, 0.0];
const PY: [f64; 5] = [0.0, 0.2, 0.6, -0.9, -1.0];

/// 像面の光線（像面の局所の位置・方向と光路長、けられ・エラーの有無）を比べる
fn same_trace(a: &System, b: &System, tol: f64, what: &str) {
    assert_eq!(a.fields.len(), b.fields.len());
    for f in 0..a.fields.len() {
        for w in 0..a.wavelengths.len() {
            let t0 = a
                .try_trace(FieldSel::Index(f), &PX, &PY, Some(w), None, false)
                .unwrap();
            let t1 = b
                .try_trace(FieldSel::Index(f), &PX, &PY, Some(w), None, false)
                .unwrap();
            for i in 0..PX.len() {
                let (r0, r1) = (&t0.rays[i], &t1.rays[i]);
                assert_eq!(
                    (r0.err != 0, r0.vig != 0),
                    (r1.err != 0, r1.vig != 0),
                    "{what} F{f} W{w} {i}"
                );
                if r0.err != 0 || r0.vig != 0 {
                    continue;
                }
                for k in 0..3 {
                    assert!(
                        (r0.p[k] - r1.p[k]).abs() < tol && (r0.k[k] - r1.k[k]).abs() < tol,
                        "{what} F{f} W{w} {i} {:?} {:?} / {:?} {:?}",
                        r0.p,
                        r1.p,
                        r0.k,
                        r1.k
                    );
                }
                assert!(
                    (r0.opl - r1.opl).abs() < tol,
                    "{what} {} {}",
                    r0.opl,
                    r1.opl
                );
            }
        }
    }
}

/// 半直径を固定した玉の面（前か後ろがガラス）で開口のない面を浮動開口にした系（.zmx の読込と同じ）
fn with_floating(s: &System) -> System {
    use lensbench_core::surface::Aperture;
    let glass = |m: &str| !m.is_empty() && m != "-" && !m.eq_ignore_ascii_case("AIR");
    let mut t = s.clone();
    for k in 1..t.surfaces.len().saturating_sub(1) {
        let lens = (glass(&s.surfaces[k].mat) && !s.surfaces[k].mirror)
            || (glass(&s.surfaces[k - 1].mat) && !s.surfaces[k - 1].mirror);
        let d = &mut t.surfaces[k];
        if !d.semi_auto && lens && d.ap == Aperture::None {
            d.ap = Aperture::Floating { semi: d.semi };
        }
    }
    t.changed().unwrap();
    t
}

fn round_trip(systems: &[System], names: &[String], tag: &str) {
    let dir = std::env::temp_dir().join(format!("lb_cfgzmx_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("multi.zmx");
    zmx_write::save_configs(&p, systems).unwrap();
    let opt = zmx::LoadOptions {
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    let back = zmx::load_configs(&p, &opt, &[]).unwrap();
    assert_eq!(back.len(), systems.len());
    for (k, (a, b)) in systems.iter().zip(&back).enumerate() {
        // 半直径を固定した玉の面で開口のない面は、読み直すと浮動開口でける（OpticStudio の読込と同じ）
        same_trace(&with_floating(a), b, 1e-9, &names[k]);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn splitter_paths_round_trip() {
    let p = project("P16_twyman_green.json");
    let ps = paths(&p, &env()).unwrap();
    assert_eq!(ps, ["TT", "TR", "RT", "RR"]);
    let ss: Vec<System> = ps
        .iter()
        .map(|q| expand_path(&p, &env(), Some(q)).unwrap().0)
        .collect();
    round_trip(&ss, &ps, "p16");
}

#[test]
fn states_round_trip() {
    // 状態で玉を抜き差しする（面の数が変わる）系
    let p = project("P6_states_focus_group.json");
    let names: Vec<String> = p.state_names().iter().map(|x| x.to_string()).collect();
    assert!(names.len() >= 2);
    let ss: Vec<System> = names
        .iter()
        .map(|n| lensbench_project::expand(&p.with_state(n).unwrap(), &env()).unwrap())
        .collect();
    round_trip(&ss, &names, "p6");
}

#[test]
fn different_system_values_are_refused() {
    let p = project("P16_twyman_green.json");
    let a = expand_path(&p, &env(), Some("TR")).unwrap().0;
    let mut b = expand_path(&p, &env(), Some("RT")).unwrap().0;
    b.wavelengths[0] = 0.55;
    let e = zmx_write::write_configs(&[a, b]).unwrap_err().to_string();
    assert!(e.contains("構成 2"), "{e}");
}

#[path = "../examples/bs_configs.rs"]
#[allow(dead_code)]
mod bs_configs;

/// OpticStudio との照合（tests/bs_configs/cfg_refs.json、make_cfg_refs.py で作成）: examples/bs_configs.rs で書いた
/// 多重構成の .zmx を OpticStudio で構成ごとに追跡した像面の局所の位置・方向・光路長が、lensbench の各経路・状態の系と同じ
#[test]
fn configs_match_opticstudio() {
    let f = root().join("tests/bs_configs/cfg_refs.json");
    let Ok(txt) = std::fs::read_to_string(&f) else {
        panic!("{} がない（make_cfg_refs.py で作る）", f.display());
    };
    let refs: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let g = |v: &serde_json::Value| v.as_f64().unwrap();
    let (mut wp, mut wd, mut wl) = (0.0f64, 0.0f64, 0.0f64);
    let mut n = 0;
    for (name, file, states) in bs_configs::CASES {
        let (names, ss) = bs_configs::systems(&root(), file, states);
        let case = &refs["cases"][name];
        let cfgs = case["configs"].as_array().unwrap();
        assert_eq!(cfgs.len(), ss.len(), "{name}");
        for (k, (s, c)) in ss.iter().zip(cfgs).enumerate() {
            for r in c["unpol"].as_array().unwrap() {
                let w = r["wave"].as_u64().unwrap() as usize - 1;
                let fi = r["field"].as_u64().unwrap() as usize - 1;
                let tr = s
                    .try_trace(
                        FieldSel::Index(fi),
                        &[g(&r["px"])],
                        &[g(&r["py"])],
                        Some(w),
                        None,
                        false,
                    )
                    .unwrap();
                let t = tr.rays[0];
                let (err, vig) = (r["err"].as_u64().unwrap(), r["vig"].as_u64().unwrap());
                // けられ・エラーの面の番号はまとめた系の番号なので、有無だけ比べる
                assert_eq!(
                    (t.err != 0, t.vig != 0),
                    (err != 0, vig != 0),
                    "{name} {} {r}",
                    names[k]
                );
                if err != 0 || vig != 0 {
                    continue;
                }
                for i in 0..3 {
                    wp = wp.max((t.xyz[i] - g(&r["xyz"][i])).abs());
                    wd = wd.max((tr.parity * t.lmn[i] - g(&r["lmn"][i])).abs());
                }
                // 無限遠の物体の軸外の視野は、光路長を数え始める面が違う（lensbench は入射の平面波に垂直な面、
                // OpticStudio は光軸に垂直な面）ので比べない（P7）
                let off_axis = g(&r["hx"]) != 0.0 || g(&r["hy"]) != 0.0;
                if !(s.surfaces[0].t.is_infinite() && off_axis) {
                    wl = wl.max((t.opl - g(&r["opl"])).abs());
                }
                n += 1;
            }
        }
    }
    println!("{n} 本: 位置 {wp:.2e} mm、方向 {wd:.2e}、光路長 {wl:.2e} mm");
    assert!(n > 0);
    assert!(wp < 1e-11 && wd < 1e-11 && wl < 1e-9, "{wp} {wd} {wl}");
}

/// 多重構成の行は OpticStudio が書く形（値の後に 0 0 0 1 1 倍率 オフセット 0 "" 0）。倍率を落とすと OpticStudio は
/// エラーを出さずに値を読み違える（IGNR が 0.01 に、THIC が 0 に）
#[test]
fn mce_rows_have_opticstudio_layout() {
    let p = project("P16_twyman_green.json");
    let ss: Vec<System> = ["TR", "RT"]
        .iter()
        .map(|q| expand_path(&p, &env(), Some(q)).unwrap().0)
        .collect();
    let txt = zmx_write::write_configs(&ss).unwrap();
    let rows: Vec<&str> = txt.lines().filter(|l| l.starts_with("IGNR ")).collect();
    assert!(!rows.is_empty());
    for r in rows {
        let t: Vec<&str> = r.split_whitespace().collect();
        assert_eq!(t.len(), 14, "{r}");
        assert_eq!(
            &t[4..],
            ["0", "0", "0", "1", "1", "1", "0", "0", "\"\"", "0"],
            "{r}"
        );
    }
}
