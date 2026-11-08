//! OpticStudio 照合: くさびのウィンドウ（面の tilt）・偏光子・波長板 2 枚（ゼロ次と 3 次）の系
//! （tests/projects/A3/project.json）を .zmx に書き出し（system.zmx）、OpticStudio 26.1 で追った像面の光線
//! （os_rays.json、全視野 × 全波長 × 29 点）と偏光の電場（os_pol.json、Normalized Polarized、入力 x・y）と比べる。
//!
//! 偏光子・波長板は .zmx では JONESMAT 面（設計波長での 1 つの行列）になる。OpticStudio の JONESMAT は波長によらない
//! ので、書き出した .zmx を開いた系とは全波長で合う。プロジェクトの系（波長ごとの Jones）と合うのは設計波長
//! （0.6328 µm）だけで、他の波長では波長板の遅延の違いだけずれる。
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::{zmx, zmx_write};
use lensbench_project::{expand, Env, Project};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/projects/A3")
}

fn glass_dir() -> PathBuf {
    dir().join("../../glass")
}

fn project_system() -> System {
    let txt = std::fs::read_to_string(dir().join("project.json")).unwrap();
    let env = Env {
        base: dir(),
        glass_dirs: vec![glass_dir()],
        catalog: None,
    };
    expand(&Project::from_json(&txt).unwrap(), &env).unwrap()
}

fn zmx_system() -> System {
    let opt = zmx::LoadOptions {
        catalog_dirs: vec![glass_dir()],
        ..Default::default()
    };
    zmx::load_with(&dir().join("system.zmx"), &opt).unwrap()
}

fn json(name: &str) -> serde_json::Value {
    let txt = std::fs::read_to_string(dir().join(name)).unwrap();
    serde_json::from_str(&txt).unwrap()
}

/// system.zmx を書き直す（照合データを取り直すとき）: cargo test -p lensbench-project --test os_a3 -- --ignored
#[test]
#[ignore]
fn write_system_zmx() {
    zmx_write::save(&dir().join("system.zmx"), &project_system()).unwrap();
}

#[test]
fn system_zmx_is_up_to_date() {
    let want = zmx_write::write(&project_system()).unwrap();
    let have = lensbench_core::glass::read_text(&dir().join("system.zmx")).unwrap();
    // 数は相対 1e-12 で比べる（Jones 行列の三角関数は OS・libm で最後の桁が違う）
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
                (Ok(u), Ok(v)) => {
                    assert!(
                        u == v || (u - v).abs() <= 1e-12 * u.abs().max(v.abs()),
                        "{a} | {b}"
                    )
                }
                _ => assert_eq!(x, y, "{a} | {b}"),
            }
        }
    }
}

/// 像面の光線（くさびで軸上でも主光線が曲がる）: プロジェクトの系・.zmx を開いた系とも OpticStudio と合う
#[test]
fn rays_match_opticstudio() {
    let os = json("os_rays.json");
    let pupil: Vec<[f64; 2]> = os["pupil"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()])
        .collect();
    let px: Vec<f64> = pupil.iter().map(|p| p[0]).collect();
    let py: Vec<f64> = pupil.iter().map(|p| p[1]).collect();
    for s in [project_system(), zmx_system()] {
        let (mut dp, mut dd) = (0.0f64, 0.0f64);
        for tr in os["traces"].as_array().unwrap() {
            let f = tr["field"].as_u64().unwrap() as usize - 1;
            let w = tr["wave"].as_u64().unwrap() as usize - 1;
            let t = s.trace(FieldSel::Index(f), &px, &py, Some(w), None, false);
            for (i, r) in tr["rays"].as_array().unwrap().iter().enumerate() {
                let v: Vec<f64> = r
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_f64().unwrap())
                    .collect();
                assert_eq!(v[0], 0.0);
                let ray = &t.rays[i];
                assert_eq!(ray.err, 0, "視野 {f} 波長 {w} 光線 {i}");
                for k in 0..3 {
                    dp = dp.max((ray.xyz[k] - v[2 + k]).abs());
                    dd = dd.max((ray.lmn[k] - v[5 + k]).abs());
                }
            }
        }
        println!("位置の差 {dp:.1e} mm、方向の差 {dd:.1e}");
        assert!(dp < 1e-9 && dd < 1e-10, "{dp:e} {dd:e}");
    }
}

/// 像面の電場の差を波長ごとに: (光線ごとの複素の比 s を除いた残りの最大, |s| − 1 の最大)。
///
/// 比 s を除くのは 2 つの約束の違いのため: (1) 無限遠の物体の光線の位相を、lensbench は入射の平面波に垂直な面から、
/// OpticStudio は光軸に垂直な面から数える（軸外の視野で exp(−i k (px·hx + py·hy)·R·sinθ) だけ違う。R は入射瞳の半径）、
/// (2) 公開側のテスト用ガラス（tests/glass）は内部透過率がなく、OpticStudio の N-BK7 の吸収（7 mm で強度 0.16%）がない
fn pol_diff(s: &System) -> Vec<(f64, f64)> {
    let os = json("os_pol.json");
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
                .try_trace_pol(FieldSel::Norm(hx, hy), &px, &py, Some(w), &pols, &coat)
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
                    assert!(d.is_finite());
                    wv.0 = wv.0.max(d);
                }
                wv.1 = wv.1.max((sc.norm() - 1.0).abs());
            }
        }
    }
    worst
}

/// 書き出した .zmx（JONESMAT は設計波長の行列）を開いた系は、全波長で OpticStudio と同じ偏光になる。
///
/// 残りの差（最大 2e-4）は、くさびで曲がった光線が偏光子（非ユニタリの Jones 面）を通るときだけ出る。
/// くさびがなければ（斜めの視野でも）、また Jones 面がユニタリ（波長板）なら 1e-15 で合う（非公開側の切り分け）。
/// 原因は調査中
#[test]
fn zmx_polarization_matches_opticstudio() {
    let d = pol_diff(&zmx_system());
    println!(".zmx の系（波長ごとの 比を除いた残り, |s| − 1） {d:?}");
    assert!(d.iter().all(|&(r, a)| r < 5e-4 && a < 2e-3), "{d:?}");
}

/// プロジェクトの系（波長ごとの Jones）は設計波長では .zmx と同じに合い、他の波長では波長板の遅延の分だけ違う
#[test]
fn project_polarization_matches_opticstudio_at_the_design_wavelength() {
    let d = pol_diff(&project_system());
    println!("プロジェクトの系（波長ごとの 比を除いた残り, |s| − 1） {d:?}");
    assert!(d[1].0 < 5e-4 && d[1].1 < 2e-3, "{d:?}");
    assert!(d[0].0 > 1e-2 && d[2].0 > 1e-2, "{d:?}");
}
