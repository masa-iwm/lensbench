//! 相対照度の縁の光線の解き方: 像面が焦点から大きく外れた二回通過の系（tests/double_pass/P4、けられ無し）で、縁の
//! 光線を像点に着けるには視野を何倍も動かす必要がある。前は中心から 1 歩で解けなかった半径をけられの境目として、
//! 曲線が所々 0.2〜3 % 落ちた。けられが無いので、相対照度は視野によらずほぼ 1 で、なめらかに変わる

use lensbench_analysis::illum::{relative_illumination, RIM_POINTS};
use lensbench_core::system::FieldSel;
use lensbench_formats::zmx::{self, LoadOptions};
use std::path::PathBuf;

#[test]
fn double_pass_relative_illumination_is_smooth() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let s = zmx::load_with(
        &root.join("tests/double_pass/P4_double_pass.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root.join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap();
    let w = s.primary;
    // 前に落ちた視野（0.1・0.3・0.55・0.8・0.95・1）を含む
    let hs = [0.1, 0.3, 0.55, 0.8, 0.95, 1.0];
    let ri: Vec<f64> = hs
        .iter()
        .map(|&h| relative_illumination(&s, FieldSel::Norm(0.0, h), w, RIM_POINTS).unwrap())
        .collect();
    for (i, (&h, &r)) in hs.iter().zip(&ri).enumerate() {
        assert!((r - 1.0).abs() < 5e-4, "h {h}: {r}");
        if i > 0 {
            // 視野とともに少しずつ増える（h 0.05 あたり 1e-5 ほど）
            assert!(
                r >= ri[i - 1] && r - ri[i - 1] < 1e-4,
                "h {h}: {r} after {}",
                ri[i - 1]
            );
        }
    }
}
