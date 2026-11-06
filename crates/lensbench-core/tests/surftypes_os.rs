//! 面タイプ Tilted（TILTSURF）・Odd Asphere（ODDASPHE）・Irregular（IRREGULA）を OpticStudio と比べる。
//!
//! 基準は tests/surftypes_os（独自処方の合成系。OpticStudio 2026 R1.01 で開いて保存した system.zmx と、
//! BatchRayTrace の光線 rays.npz（定義した視野と 2 つの正規化視野 × 11×11 の瞳、全波長）・系の値 ops.json）。
//! 面 2 が試す面: レンズの前面（ガラスはモデルガラス）、視野 (0,0)・(0,5)・(3,−4)°。
//! - T1: Tilted（tan 0.05・−0.03）
//! - O1: Odd Asphere（R 50、k −0.5、β1〜β5。β1 は円錐の頂点）、O3: β1 = 0
//! - I4: Irregular（R 50、Za・Zc、角度 30°）、I7: Irregular（偏心・傾き・Zs・Za・Zc・角度 25°）
//!
//! 像面の交点・方向（全光線）と EFFL を見る。OPD は O1（β1 の円錐の頂点）の射出瞳の求め方が OpticStudio と違うので見ない。

use std::path::PathBuf;

use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{FieldSel, System};
use ndarray::{Array2, Array4};
use ndarray_npy::NpzReader;
use serde_json::Value;

fn dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/surftypes_os")
        .join(name)
}

fn load(name: &str) -> System {
    zmx::load_with(
        &dir(name).join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn rays_and_efl_match_opticstudio() {
    for name in ["T1", "O1", "O3", "I4", "I7"] {
        let s = load(name);
        let ops: Value =
            serde_json::from_str(&std::fs::read_to_string(dir(name).join("ops.json")).unwrap())
                .unwrap();
        let mut npz =
            NpzReader::new(std::fs::File::open(dir(name).join("rays.npz")).unwrap()).unwrap();
        let hxy: Array2<f64> = npz.by_name("hx_hy").unwrap();
        let pp: Array2<f64> = npz.by_name("px_py").unwrap();
        let img: Array4<f64> = npz.by_name("img").unwrap();
        let efl = ops["ops"]["EFFL"].as_f64().unwrap();
        let par = s.paraxial().efl;
        assert!(
            (par - efl).abs() < 1e-11 * efl.abs(),
            "{name}: EFFL {par} {efl}"
        );
        let px: Vec<f64> = pp.column(0).to_vec();
        let py: Vec<f64> = pp.column(1).to_vec();
        let (mut dp, mut dd, mut n) = (0.0f64, 0.0f64, 0);
        for h in 0..hxy.nrows() {
            for w in 0..img.shape()[1] {
                let t = s.trace(
                    FieldSel::Norm(hxy[[h, 0]], hxy[[h, 1]]),
                    &px,
                    &py,
                    Some(w),
                    None,
                    false,
                );
                for (i, r) in t.rays.iter().enumerate() {
                    // 列は x y z l m n opd opd_chief err vig
                    let os_err = img[[h, w, i, 8]];
                    assert_eq!(os_err == 0.0, r.err == 0, "{name}: 視野 {h} 光線 {i}");
                    if r.err != 0 {
                        continue;
                    }
                    for j in 0..3 {
                        dp = dp.max((r.xyz[j] - img[[h, w, i, j]]).abs());
                        dd = dd.max((r.lmn[j] - img[[h, w, i, 3 + j]]).abs());
                    }
                    n += 1;
                }
            }
        }
        println!("{name}: 光線 {n} 本、位置 {dp:.1e} mm、方向 {dd:.1e}");
        assert!(n > 0);
        assert!(dp < 1e-11 && dd < 1e-12, "{name}: 位置 {dp:e} 方向 {dd:e}");
    }
}
