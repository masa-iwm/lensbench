//! OpticStudio 照合: ピックアップと厚さのソルブのある雛形（tests/projects/P0/template.zmx）の玉 1 を
//! 別の玉に置き換えて展開し、パラメータを変えた後の光線を、展開した系（expanded.zmx）を OpticStudio で開いて
//! 同じパラメータを変えた結果（os_rays.json）と比べる
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx_write;
use lensbench_project::{expand_report, Env, Project};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/projects/P0")
}

/// 展開した系（変える前）と、展開の注意
fn expanded() -> (System, Vec<String>) {
    let txt = std::fs::read_to_string(dir().join("project.json")).unwrap();
    let env = Env {
        base: dir(),
        glass_dirs: vec![dir().join("../../glass")],
        catalog: None,
    };
    expand_report(&Project::from_json(&txt).unwrap(), &env).unwrap()
}

/// 変えるパラメータ（展開した系の面番号）: 新しい玉の前の曲率（ソルブが動く）、座標ブレーク 6 の dx・ty
/// （面 9 がピックアップ）、玉 3 の前の円錐係数（面 8 がピックアップ）
const CHANGES: [(&str, f64); 4] = [("2.c", 0.04), ("6.dx", 0.25), ("6.ty", -0.8), ("7.k", -1.2)];

/// 追う光線: 視野の番号（0 から）と瞳の座標
fn pupil() -> (Vec<f64>, Vec<f64>) {
    (
        vec![0.0, 0.0, 0.0, 0.7, -0.5, 0.3, 0.0],
        vec![0.0, 1.0, -1.0, 0.0, 0.5, -0.6, 0.4],
    )
}

fn changed(s: &System) -> System {
    let ch: Vec<_> = CHANGES
        .iter()
        .map(|(n, v)| (s.param(n).unwrap(), *v))
        .collect();
    s.with_params(&ch).unwrap()
}

#[test]
fn expansion_keeps_the_pickups_and_the_solve_inside_the_range() {
    let (s, warn) = expanded();
    // 面 3 の曲率（元は面 2）と面 4 の厚さ（元は面 1）は範囲の外からなので値で固定
    assert_eq!(
        warn.iter().filter(|w| w.contains("範囲の外")).count(),
        2,
        "{warn:?}"
    );
    // 残るのは、座標ブレーク 9 ← 6（dx・ty）と円錐係数 8 ← 7
    assert_eq!(s.pickups.len(), 3, "{:?}", s.pickups);
    assert_eq!(s.solves.len(), 1);
    assert_eq!(s.solves[0].surf, 10);
}

/// expanded.zmx を書き直す（照合データを取り直すとき）: cargo test -p lensbench-project --test os_p0 -- --ignored
#[test]
#[ignore]
fn write_expanded_zmx() {
    let (s, _) = expanded();
    zmx_write::save(&dir().join("expanded.zmx"), &s).unwrap();
}

#[test]
fn expanded_zmx_is_up_to_date() {
    let (s, _) = expanded();
    let want = zmx_write::write(&s).unwrap();
    let have = lensbench_core::glass::read_text(&dir().join("expanded.zmx")).unwrap();
    // 数は相対 1e-12 で比べる（自動の半径は光線から求めるので OS・libm で最後の桁が違う）
    let (h, w): (Vec<&str>, Vec<&str>) = (have.lines().collect(), want.lines().collect());
    assert_eq!(h.len(), w.len());
    for (a, b) in h.iter().zip(&w) {
        let (ta, tb): (Vec<&str>, Vec<&str>) = (
            a.split_whitespace().collect(),
            b.split_whitespace().collect(),
        );
        assert_eq!(ta.len(), tb.len(), "{a} | {b}");
        for (x, y) in ta.iter().zip(&tb) {
            match (x.parse::<f64>(), y.parse::<f64>()) {
                (Ok(u), Ok(v)) => assert!(
                    u == v || (u - v).abs() <= 1e-12 * u.abs().max(v.abs()),
                    "{a} | {b}"
                ),
                _ => assert_eq!(x, y, "{a} | {b}"),
            }
        }
    }
}

#[test]
fn matches_opticstudio_after_the_changes() {
    let path = dir().join("os_rays.json");
    let Ok(txt) = std::fs::read_to_string(&path) else {
        panic!("{} がない（OpticStudio で取る）", path.display());
    };
    let os: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let s = changed(&expanded().0);
    // 厚さ（ソルブ・ピックアップの結果）
    let th = os["thickness"].as_array().unwrap();
    for (j, t) in th.iter().enumerate().skip(1) {
        let t = t.as_f64().unwrap();
        let d = (s.surfaces[j].t - t).abs();
        assert!(
            d < 1e-12 * (1.0 + t.abs()),
            "面 {j} の厚さ {} OS {t}",
            s.surfaces[j].t
        );
    }
    let (px, py) = pupil();
    let mut worst = 0.0f64;
    for r in os["rays"].as_array().unwrap() {
        let f = r["field"].as_u64().unwrap() as usize;
        let w = r["wave"].as_u64().unwrap() as usize;
        let i = r["ray"].as_u64().unwrap() as usize;
        let t = s
            .try_trace(
                FieldSel::Index(f),
                &px[i..=i],
                &py[i..=i],
                Some(w),
                None,
                false,
            )
            .unwrap();
        let ray = &t.rays[0];
        assert_eq!(ray.err, 0);
        assert_eq!(r["err"].as_i64().unwrap(), 0);
        for (k, key) in ["x", "y", "z"].iter().enumerate() {
            let d = (ray.xyz[k] - r[key].as_f64().unwrap()).abs();
            worst = worst.max(d);
            assert!(
                d < 1e-12,
                "視野 {f} 波長 {w} 光線 {i} {key}: {} OS {}",
                ray.xyz[k],
                r[key]
            );
        }
        for (k, key) in ["l", "m", "n"].iter().enumerate() {
            let d = (ray.lmn[k] - r[key].as_f64().unwrap()).abs();
            assert!(
                d < 1e-12,
                "視野 {f} 波長 {w} 光線 {i} {key}: {} OS {}",
                ray.lmn[k],
                r[key]
            );
        }
    }
    println!("像面の位置の最大の差 {worst:e}");
}
