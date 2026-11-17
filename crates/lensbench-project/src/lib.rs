//! ブロックで組む系（プロジェクトファイル、JSON）と、その順次の面の列への展開。
//!
//! ユーザーが編集するのはブロック（玉・空気・絞り・鏡・面の並び・.zmx から取り込んだ系・群）で、計算の直前に
//! [`expand`] で OpticStudio と同じ順次の面の列（[`System`]）にする。
//!
//! - 玉の向きと面のデータは「光の進む向き」で書く。鏡のあと（光が −z へ進む区間）では、展開で曲率・非球面・厚さの
//!   符号を変える（OpticStudio の約束: 鏡のあとは厚さが負）。
//! - 偏心・傾きは座標ブレークの組で入れる。ブロックの前で傾け、ブロックの後で元の軸へ戻す（戻る点は元の軸上で
//!   ブロックの長さだけ進んだ点）。鏡の傾き（折り曲げ）は戻さない。
//! - ON/OFF: OFF のブロックは「詰める」（長さごと除く）か「空気で残す」（同じ長さの空気にする）。
//! - 群（ズーム群・フォーカス群など）: 連続したブロック（.zmx の取り込みでは面の範囲）に名前と役割を付ける。
//!   移動 `shift` は前の間隔に足して後の間隔から引くので、群の外の合計長は変わらない（物体が無限遠で群が
//!   最初にあるときは、前の間隔は表さないので、後ろの間隔が縮むだけ）。
//! - ユーザーカタログ（[`UserCatalog`]、JSON）: 玉モードで得た玉や組んだブロックを名前で保存し、`"type": "user"` の
//!   ブロックで他のプロジェクトから使う。
//! - 検出器（[`DetectorBlock`]、`"type"` は登録した種類の名前）: 最後のブロックに置く。像面に置き、面は足さない。
//!   種類と項目の確かめ方は [`detectors`] に登録する（非公開版などの拡張が足す）。
//! - 状態（多重構成に当たる）: ブロックの値（ON/OFF、間隔、群の移動、偏心・傾き、.zmx の構成番号など）と
//!   系の値（物体距離など）の変更の組に名前を付ける。[`expand`] に [`ExpandOpts::state`] を渡してその状態の系を展開する。

#![warn(unreachable_pub)]
// 入力次第で panic しないように、ライブラリでは unwrap・panic を使わない。前に確かめた形は expect に理由を書く（試験は対象外）

pub mod catalog_spec;
pub mod detectors;
pub mod edit_ops;
pub mod edit_spec;
pub mod filter;
pub mod lenient;
pub mod open;
pub mod retarget;
pub mod schema;
mod splitter;
pub mod surface_ops;
pub mod sysfile;
pub mod sysfile_v2;
pub mod writeback;
pub use edit_spec::SurfaceOp;
use edit_spec::{BlockRef, EditOp, Ref};
pub use splitter::{Arm, PbsSpec, SplitFace, SplitForm, SplitterSpec};
use splitter::{Level, SplitState};

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use lensbench_core::elements::{self, Element, Kind};
use lensbench_core::glass::{self, Glass};
use lensbench_core::math::{mv, rot, M3};
use lensbench_core::pickup::Pickup;
use lensbench_core::solves::Solve;
use lensbench_core::surface::{After, Aperture, Material, SurfType, Surface};
use lensbench_core::system::{
    AfocalUnit, Aiming, ApertureKind, Field, FieldCond, FieldNorm, FieldType, System,
};
use lensbench_core::zmx;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod defaults;
use defaults::{default_catalogs, is_yes, one, one_f64, wave_weight, yes};

mod back;
mod blocks;
mod catalog;
mod expand;
mod faces;
mod model;
mod zmx_wrap;
pub use back::*;
pub use blocks::*;
pub use catalog::*;
pub use expand::*;
use faces::*;
pub use model::*;
pub use zmx_wrap::{same_system, zmx_project, ZMX_ID};

#[cfg(test)]
mod retarder_spec_tests {
    use super::*;

    /// 偏光子・波長板の面を書き戻すと同じ書き方に。遅延 0 の波長板は波長板のまま（設計波長を残す）
    #[test]
    fn surface_spec_round_trip() {
        for q in [
            SurfSpec {
                waveplate: Some(WaveplateSpec {
                    retardance: 0.0,
                    wavelength_um: 0.55,
                    axis_deg: 10.0,
                    order: 0,
                    transmittance: 1.0,
                    material: None,
                    thickness_mm: None,
                    plates: Vec::new(),
                }),
                ..Default::default()
            },
            // 複屈折材の板（1 枚・2 枚の組）は厚さのまま戻る
            SurfSpec {
                waveplate: Some(WaveplateSpec {
                    retardance: 0.25,
                    wavelength_um: 0.633,
                    axis_deg: 45.0,
                    order: 0,
                    transmittance: 1.0,
                    material: Some("QUARTZ".into()),
                    thickness_mm: Some(0.5),
                    plates: Vec::new(),
                }),
                ..Default::default()
            },
            SurfSpec {
                waveplate: Some(WaveplateSpec {
                    retardance: 0.0,
                    wavelength_um: 0.633,
                    axis_deg: 0.0,
                    order: 0,
                    transmittance: 1.0,
                    material: None,
                    thickness_mm: None,
                    plates: vec![
                        PlateSpec {
                            material: "QUARTZ".into(),
                            thickness_mm: 0.4,
                            axis_deg: 0.0,
                        },
                        PlateSpec {
                            material: "MGF2".into(),
                            thickness_mm: 0.3,
                            axis_deg: 90.0,
                        },
                    ],
                }),
                ..Default::default()
            },
            SurfSpec {
                polarizer: Some(PolarizerSpec {
                    axis_deg: 30.0,
                    extinction_ratio: 500.0,
                    transmittance: 0.9,
                }),
                ..Default::default()
            },
        ] {
            let s = surf(&q).unwrap();
            let back = surface_spec(&s, false).unwrap();
            assert_eq!(back.waveplate, q.waveplate);
            if let (Some(a), Some(b)) = (back.polarizer, q.polarizer) {
                assert!((a.extinction_ratio - b.extinction_ratio).abs() < 1e-9);
                assert!((a.transmittance - b.transmittance).abs() < 1e-12);
                assert_eq!(a.axis_deg, b.axis_deg);
            } else {
                assert_eq!(back.polarizer, q.polarizer);
            }
            assert!(back.jones.is_none());
        }
        // 面の tilt（くさび）は tilt のまま戻る
        let q = SurfSpec {
            tilt: Some([0.5, -0.25]),
            semi: Semi::Value(5.0),
            ..Default::default()
        };
        let back = surface_spec(&surf(&q).unwrap(), true).unwrap();
        assert_eq!(back.tilt, Some([0.5, -0.25]));
        // 順序 1（ty → tx）の傾きは、両方 0 でないと tilt では書けない（片方だけなら同じ向き）
        let mut s = surf(&q).unwrap();
        s.before.as_mut().unwrap().order = 1;
        assert!(surface_spec(&s, true).is_err());
        let q1 = SurfSpec {
            tilt: Some([0.0, -0.25]),
            ..Default::default()
        };
        let mut s = surf(&q1).unwrap();
        s.before.as_mut().unwrap().order = 1;
        assert_eq!(surface_spec(&s, true).unwrap().tilt, Some([0.0, -0.25]));
        // 座標ブレークと一緒は誤り
        let bad = SurfSpec {
            tilt: Some([1.0, 0.0]),
            coordbrk: Some([0.0; 5]),
            ..Default::default()
        };
        assert!(surf(&bad).is_err());
    }
}

#[cfg(test)]
mod surface_spec_all_tests {
    use super::*;

    /// 小さな擬似乱数（試験を毎回同じにする）
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn val(&mut self) -> f64 {
            (self.pick(2001) as f64 - 1000.0) / 37.0
        }
    }

    /// surf が読める面はすべて surface_spec で書け、書いたものから同じ面に戻る
    #[test]
    fn every_readable_surface_round_trips() {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        let mut kinds = [0usize; 8];
        for _ in 0..4000 {
            let shape = [
                SurfShape::Standard,
                SurfShape::OddAsph,
                SurfShape::Toroidal,
                SurfShape::Fresnel,
            ][r.pick(4) as usize];
            let special = r.pick(6);
            let flat = special != 0;
            let mut q = SurfSpec {
                shape: if flat { SurfShape::Standard } else { shape },
                t: r.val(),
                semi: if r.pick(2) == 0 {
                    Semi::Auto(AutoWord::Auto)
                } else {
                    Semi::Value(r.val().abs() + 0.1)
                },
                coat: ["", "AR"][r.pick(2) as usize].into(),
                comment: ["", "c"][r.pick(2) as usize].into(),
                ..Default::default()
            };
            if !flat {
                q.radius = (r.pick(3) != 0).then(|| r.val()).filter(|v| *v != 0.0);
                q.conic = if r.pick(2) == 0 { 0.0 } else { r.val() };
                q.asph = (0..r.pick(4)).map(|_| r.val() * 1e-5).collect();
                if shape == SurfShape::Toroidal {
                    q.radius_x = (r.pick(2) == 0).then(|| r.val()).filter(|v| *v != 0.0);
                }
                q.glass = [
                    None,
                    Some("N-BK7"),
                    Some("1.6"),
                    Some("1.6,40"),
                    Some("1.6,40,0.01"),
                ][r.pick(5) as usize]
                    .map(String::from);
                if r.pick(3) == 0 {
                    q.tilt = Some([r.val(), if r.pick(2) == 0 { 0.0 } else { r.val() }]);
                }
                if r.pick(3) == 0 {
                    q.aperture = Some(SurfAperture::Rect {
                        hx: 2.0,
                        hy: 1.0,
                        dx: r.val(),
                        dy: 0.0,
                    });
                }
            }
            match special {
                1 => q.jones = Some([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, r.val(), 0.0]),
                2 => {
                    q.polarizer = Some(PolarizerSpec {
                        axis_deg: r.val(),
                        extinction_ratio: 100.0,
                        transmittance: 0.9,
                    })
                }
                3 => {
                    q.coordbrk = Some([r.val(), r.val(), r.val(), r.val(), r.val()]);
                    q.order = r.pick(2) as u8;
                }
                _ => {}
            }
            kinds[q.shape as usize + 4 * usize::from(flat)] += 1;
            let s = surf(&q).unwrap_or_else(|e| panic!("{q:?}: {e}"));
            let back = surface_spec(&s, false).unwrap_or_else(|e| panic!("{q:?}: {e}"));
            let s2 = surf(&back).unwrap_or_else(|e| panic!("{back:?}: {e}"));
            assert_eq!(format!("{s:?}"), format!("{s2:?}"), "{q:?}");
            // 書き戻しは変わらない（2 回目も同じ）
            let again = surface_spec(&s2, false).unwrap();
            assert_eq!(
                serde_json::to_value(&back).unwrap(),
                serde_json::to_value(&again).unwrap()
            );
        }
        assert!(kinds.iter().take(5).all(|n| *n > 0), "{kinds:?}");
    }
}

#[cfg(test)]
mod surface_row_tests {
    use super::*;
    use lensbench_core::surface::SurfType;

    /// SurfSpec の欄で書けない面（Zernike・Grid Sag・Irregular・Tilted・Binary 2・理想レンズ・GRIN など）は
    /// システムファイル（版 2）の行で書け、読んで書き直しても同じ
    #[test]
    fn special_surfaces_round_trip_as_rows() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        for d in ["tests/freeform", "tests/surftypes", "tests/grin"] {
            for e in std::fs::read_dir(root.join(d)).unwrap().flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "zmx") {
                    files.push(p);
                } else if p.join("system.zmx").exists() {
                    files.push(p.join("system.zmx"));
                }
            }
        }
        let mut types = std::collections::BTreeSet::new();
        let mut grin = 0;
        for f in &files {
            let opt = lensbench_core::zmx::LoadOptions {
                cfg: 1,
                catalog_dirs: vec![
                    root.join("tests/glass"),
                    f.parent().unwrap().to_path_buf(),
                    root.join("tests/grin"),
                ],
                ..Default::default()
            };
            let Ok(s) = lensbench_core::zmx::load_with(f, &opt) else {
                continue;
            };
            let n = s.surfaces.len();
            for (j, a) in s.surfaces.iter().enumerate().take(n - 1).skip(1) {
                let Ok(q) = surface_spec(a, false) else {
                    assert!(
                        a.is_mirror()
                            || a.reflect.is_some()
                            || a.typ == SurfType::CoordBrk
                            || a.before.is_some(),
                        "{} surface {j}: {:?}",
                        f.display(),
                        surface_spec(a, false).err()
                    );
                    continue;
                };
                if q.row.is_none() {
                    continue;
                }
                types.insert(format!("{:?}", a.typ));
                grin += usize::from(a.grin.is_some());
                let at = || format!("{} surface {j}", f.display());
                let s1 = surf(&q).unwrap_or_else(|e| panic!("{}: {e}", at()));
                assert_eq!(s1.typ, a.typ, "{}", at());
                let q2 = surface_spec(&s1, false).unwrap();
                let s2 = surf(&q2).unwrap();
                assert_eq!(format!("{s1:?}"), format!("{s2:?}"), "{}", at());
                assert_eq!(
                    serde_json::to_value(&q).unwrap(),
                    serde_json::to_value(&q2).unwrap(),
                    "{}",
                    at()
                );
                // 玉の最後の面: 厚さ・媒質は書かない
                let last = surface_spec(a, true).unwrap();
                let r = last.row.as_ref().unwrap();
                assert!(r.t == 0.0 && r.medium.is_none(), "{}", at());
            }
        }
        for t in ["ZernikeStd", "ZernikeFringe", "GridSag"] {
            assert!(types.contains(t), "{t} not in {types:?}");
        }
        assert!(grin > 0, "{types:?}");
        // ほかの欄と一緒には書けない
        let mut q = surface_spec(
            &lensbench_core::surface::Surface {
                typ: SurfType::Paraxial,
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert!(q.row.is_some());
        q.conic = 1.0;
        assert!(surf(&q).is_err());
    }
}

#[cfg(test)]
mod read_text_tests {
    use super::*;
    use std::io::Write;

    /// gzip の展開は上限まで（展開爆弾はエラー）、上限以内ならそのまま読む
    #[test]
    fn gzip_expansion_is_limited() {
        let dir = std::env::temp_dir().join(format!("lbgzlim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("big.json.gz");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&vec![b' '; 1 << 20]).unwrap();
        std::fs::write(&f, gz.finish().unwrap()).unwrap();
        let e = read_text_limit(&f, 1000).unwrap_err();
        assert!(
            e.message().unwrap_or_default().contains("when inflated"),
            "{e}"
        );
        assert_eq!(read_text_limit(&f, 1 << 20).unwrap().len(), 1 << 20);
        // .json を指すと .json.gz を読む
        assert_eq!(read_text(&dir.join("big.json")).unwrap().len(), 1 << 20);
        // ユーザーカタログ: .gz の名前へは圧縮して保存、.json.gz だけあれば load_or_new はそれを読む
        let c = UserCatalog::default();
        let g = dir.join("cat.json.gz");
        c.save(&g).unwrap();
        assert!(std::fs::read(&g).unwrap().starts_with(&[0x1f, 0x8b]));
        assert_eq!(
            UserCatalog::load_or_new(&dir.join("cat.json"))
                .unwrap()
                .to_json(),
            c.to_json()
        );
        assert!(UserCatalog::load_or_new(&dir.join("none.json")).is_ok());
        // load_shared は同じファイルなら覚えたものを返し、書き換えれば読み直す（.json.gz だけでも読める）
        assert_eq!(
            UserCatalog::load_shared(&dir.join("cat.json"))
                .unwrap()
                .to_json(),
            c.to_json()
        );
        let p = dir.join("shared.json");
        let mut a = UserCatalog {
            vendor: "A".into(),
            ..Default::default()
        };
        a.save(&p).unwrap();
        let x = UserCatalog::load_shared(&p).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &x,
            &UserCatalog::load_shared(&p).unwrap()
        ));
        a.vendor = "BB".into();
        a.save(&p).unwrap();
        assert_eq!(UserCatalog::load_shared(&p).unwrap().vendor, "BB");
        std::fs::remove_dir_all(&dir).ok();
    }
}
