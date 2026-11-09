//! 超広角の系の光線エイミング: 近軸の入射瞳から狙うと絞りまで届かない光線を、継続法で解き直す。
//!
//! 基準は tests/aim_wide/W2（独自処方の超広角の系: 強い負のメニスカス + 正の玉 + 絞り + 正の玉、視野 y 0・40・60°・
//! (10°, 78°)、近軸の光線エイミング、2 波長。OpticStudio 2026 R1.01 で開いて保存した system.zmx と BatchRayTrace の
//! rays.npz（定義した視野と 2 つの正規化視野 × 11×11 の瞳）・ops.json）。
//! OpticStudio が通した光線は lensbench もすべて通り、像面の交点がエイミングの収束の範囲で一致する。
//! OpticStudio が止める縁の光線を lensbench が通すことはある（解き直しで絞りに届く光線が見つかるもの）。

use std::path::PathBuf;

use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::FieldSel;
use ndarray::{Array2, Array4};
use ndarray_npy::NpzReader;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/aim_wide/W2")
}

#[test]
fn wide_angle_rays_are_aimed_like_opticstudio() {
    let s = zmx::load_with(
        &dir().join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let mut npz = NpzReader::new(std::fs::File::open(dir().join("rays.npz")).unwrap()).unwrap();
    let hxy: Array2<f64> = npz.by_name("hx_hy").unwrap();
    let pp: Array2<f64> = npz.by_name("px_py").unwrap();
    let img: Array4<f64> = npz.by_name("img").unwrap();
    let px: Vec<f64> = pp.column(0).to_vec();
    let py: Vec<f64> = pp.column(1).to_vec();
    let (mut dp, mut n, mut extra) = (0.0f64, 0, 0);
    // 定義した 4 つの視野
    for h in 0..4 {
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
                let os_ok = img[[h, w, i, 8]] == 0.0;
                if os_ok {
                    assert_eq!(r.err, 0, "視野 {h} 波長 {w} 光線 {i}: OpticStudio は通る");
                    for j in 0..3 {
                        dp = dp.max((r.xyz[j] - img[[h, w, i, j]]).abs());
                    }
                    n += 1;
                } else if r.err == 0 {
                    extra += 1;
                }
            }
        }
    }
    println!("光線 {n} 本、像面の交点の差 {dp:.1e} mm、OpticStudio が止めて lensbench が通す光線 {extra} 本");
    assert!(n > 600, "{n}");
    assert!(dp < 1e-7, "{dp:e}");
}

/// 90° を超える視野（物体側の後ろから来る光）: 光線は向きどおり（n < 0）に進み、面 1 の球（頂点側の半球）に
/// 下から当たる（OpticStudio 付属の 210° 広角レンズの 105° の視野と同じ根の選び方。非公開側で OpticStudio と 1.7e-9 mm）
#[test]
fn field_beyond_90_degrees_hits_the_vertex_side_cap() {
    let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 4\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 0\nYFLN 0 105\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  TYPE STANDARD\n  CURV 0.002777777777777778\n  DISZ 20\n\
SURF 2\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let s = zmx::build(
        &zmx::parse_text(txt).unwrap(),
        &LoadOptions {
            cfg: 1,
            ..Default::default()
        },
        &[],
        None,
    )
    .unwrap();
    let t = s
        .try_trace(FieldSel::Index(1), &[0.0], &[0.0], None, Some(1), false)
        .unwrap();
    let r = &t.rays[0];
    assert_eq!(r.err, 0);
    // 向き: y 0.966、z −0.259
    assert!(
        (r.lmn[1] - 105f64.to_radians().sin()).abs() < 1e-12,
        "{:?}",
        r.lmn
    );
    assert!(
        (r.lmn[2] - 105f64.to_radians().cos()).abs() < 1e-12,
        "{:?}",
        r.lmn
    );
    // 半径 360 の球の頂点側（z < R）で、y < 0 の側に当たる
    let (y, z) = (r.xyz[1], r.xyz[2]);
    assert!(z < 360.0 && y < 0.0, "{:?}", r.xyz);
    assert!(((360.0 - z).powi(2) + y * y - 360.0 * 360.0).abs() < 1e-6);
}
