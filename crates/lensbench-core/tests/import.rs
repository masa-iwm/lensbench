//! 読み手が作った文書から系を作る（System::from_import）。.zmx を通さず core だけで確かめる

use lensbench_core::import::{MacroRequest, MacroTarget};
use lensbench_core::param::Kind;
use lensbench_core::pickup::Pickup;
use lensbench_core::surface::Surface;
use lensbench_core::system::{Aiming, ApertureKind, Field, FieldNorm, FieldType};
use lensbench_core::{Error, System};
use std::collections::HashMap;

/// 物体（無限遠）・絞り・ガラスの板（厚さ 2）・空気・像面の文書
fn doc() -> lensbench_core::system::Prescription {
    let surf = |t: f64| Surface {
        t,
        ..Default::default()
    };
    let mut sf = vec![
        surf(f64::INFINITY),
        surf(5.0),
        surf(2.0),
        surf(10.0),
        surf(0.0),
    ];
    sf[1].stop = true;
    sf[2].mat = lensbench_core::material::Material::Index(1.5);
    System::new(
        sf,
        vec![0.5876],
        vec![1.0],
        0,
        vec![Field::xy(0.0, 0.0)],
        FieldType::Angle,
        FieldNorm::Radial,
        (ApertureKind::Epd, 4.0),
        false,
        Aiming::Off,
        HashMap::new(),
    )
    .unwrap()
    .into_prescription()
}

/// ピックアップは仕上げで解く（文書に書いた古い値は元の面に合わせる）
#[test]
fn from_import_resolves_pickups() {
    let mut rx = doc();
    rx.pickups = vec![Pickup::new(3, 2, Kind::Thick, 3.0, 0.5)];
    let s = System::from_import(rx, Vec::new()).unwrap();
    assert_eq!(s.surfaces[3].t, 6.5);
    assert_eq!(s.pickups.len(), 1);
}

/// 見つからないマクロの依頼は、ソルブを付けずに保存された値で固定し、理由を残す
#[test]
fn from_import_fixes_missing_macros() {
    let req = |surf, target| MacroRequest {
        surf,
        target,
        name: "NOPE".into(),
        found: Err(Error::Invalid("no macro NOPE".into())),
    };
    let s = System::from_import(
        doc(),
        vec![
            req(2, MacroTarget::Thick),
            req(2, MacroTarget::Col(Kind::Conic)),
        ],
    )
    .unwrap();
    assert!(s.solves.is_empty() && s.macro_parms.is_empty() && s.macros.is_empty());
    assert_eq!(s.surfaces[2].t, 2.0);
    let w = s.macro_warnings();
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(
        w[0].starts_with("removed the thickness macro solve NOPE of surface 2"),
        "{w:?}"
    );
    assert!(w[0].contains("no macro NOPE"), "{w:?}");
}
