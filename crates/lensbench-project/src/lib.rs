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
//!   系の値（物体距離など）の変更の組に名前を付ける。[`expand_state`] でその状態の系を展開する。

pub mod catalog_fill;
pub mod catalog_spec;
pub mod detectors;
pub mod edit_ops;
pub mod filter;
mod lenient;
pub mod open;
pub mod schema;
mod splitter;
mod step_geom;
pub mod sysfile;
pub use splitter::{Arm, PbsSpec, SplitFace, SplitForm, SplitterSpec};
use splitter::{Level, SplitState};

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use lensbench_core::elements::{self, Element, Kind};
use lensbench_core::glass::{self, Glass};
use lensbench_core::math::{mv, rot, M3};
use lensbench_core::param::{Pickup, ThickSolve};
use lensbench_core::surface::{After, Aperture, SurfType, Surface};
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
mod model;
pub use back::*;
pub use blocks::*;
pub use catalog::*;
pub use expand::*;
pub use model::*;

/// プロジェクトファイルを読んで展開する
pub fn load(
    path: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
) -> Result<System, String> {
    load_state(path, glass_dirs, catalog, None)
}

/// プロジェクトファイルを読んで、状態 state（None なら変更なし）を展開する
pub fn load_state(
    path: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
) -> Result<System, String> {
    load_state_report(path, glass_dirs, catalog, state).map(|r| r.0)
}

/// load_state と同じ。展開で落としたもの（[`expand_report`]）の説明も返す
pub fn load_state_report(
    path: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    load_path_report(path, glass_dirs, catalog, state, None)
}

/// load_state_report と同じ。split_path はビームスプリッターの経路（None ならプロジェクト・状態の path）
pub fn load_path_report(
    path: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
    split_path: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    let txt = lensbench_core::vfs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    expand_json_path(&txt, &base, glass_dirs, catalog, state, split_path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// プロジェクトの JSON（ファイルの中身と同じ形）を展開する。相対パスは base から。
/// 展開で落としたもの（[`expand_report`]）の説明も返す
pub fn expand_json(
    txt: &str,
    base: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    expand_json_path(txt, base, glass_dirs, catalog, state, None)
}

/// expand_json と同じ。split_path はビームスプリッターの経路（None ならプロジェクト・状態の path）
pub fn expand_json_path(
    txt: &str,
    base: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
    split_path: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    let p = Project::from_json(txt)?;
    let p = match state {
        Some(n) => p.with_state(n)?,
        None => p,
    };
    let env = Env {
        base: base.to_path_buf(),
        glass_dirs: glass_dirs.to_vec(),
        catalog,
    };
    expand_path(&p, &env, split_path)
}

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
        assert!(e.contains("超える"), "{e}");
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
