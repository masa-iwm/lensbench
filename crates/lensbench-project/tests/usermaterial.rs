//! 利用者定義の材料のカタログを足した系を、系ファイルに書いて読み直す（カタログは名前で残り、hash は空）

use lensbench_core::ext::{self, Ext};
use lensbench_core::usermaterial::MaterialModel;
use lensbench_core::{zmx, FieldSel};
use lensbench_project::sysfile;
use std::sync::Arc;

/// n = 1.6 − 0.01 (λ − 0.55)（温度・気圧によらない）
struct Linear;

impl MaterialModel for Linear {
    fn name(&self) -> &str {
        "LIN16"
    }
    fn index(&self, wl: f64, _t: f64, _p: f64) -> f64 {
        1.6 - 0.01 * (wl - 0.55)
    }
}

const ZMX: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 4\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 0\nYFLN 0 3\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";

#[test]
fn system_file_keeps_the_catalog_name() {
    let mut e = Ext::default();
    e.add_material_catalog("LbMat", vec![Arc::new(Linear)]);
    let e = Arc::new(e);
    let txt = ext::with(e.clone(), || {
        let raw = zmx::parse_text(ZMX).unwrap();
        let mut s = zmx::build(&raw, &zmx::LoadOptions::default(), &[], None).unwrap();
        assert!(s.add_glass_catalogs(&["LbMat".into()], &[]).is_empty());
        s.set_material(1, "LIN16").unwrap();
        assert!((s.n(1)[0] - 1.6).abs() < 1e-15);
        assert!(
            sysfile::save_notes(&s).is_empty(),
            "{:?}",
            sysfile::save_notes(&s)
        );
        let txt = sysfile::to_string(&s, &[]).unwrap();
        let (s2, _, notes) = sysfile::from_str(&txt).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let t1 = s
            .try_trace(
                FieldSel::Index(1),
                &[0.0, 0.5],
                &[0.0, -0.4],
                None,
                None,
                false,
            )
            .unwrap();
        let t2 = s2
            .try_trace(
                FieldSel::Index(1),
                &[0.0, 0.5],
                &[0.0, -0.4],
                None,
                None,
                false,
            )
            .unwrap();
        for (a, b) in t1.rays.iter().zip(&t2.rays) {
            assert_eq!((a.xyz, a.lmn, a.opl), (b.xyz, b.lmn, b.opl));
        }
        txt
    });
    assert!(txt.contains("\"LbMat\""), "{txt}");
    // 同じ名前の AGF がフォルダにあっても登録したカタログが先で、注意を返す
    let dir = std::env::temp_dir().join(format!("lbtest-usermat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::copy(
        root.join("tests/glass/LBCRYSTAL.AGF"),
        dir.join("LbMat.AGF"),
    )
    .unwrap();
    ext::with(e.clone(), || {
        let (s2, _, notes) = sysfile::from_str_with(&txt, std::slice::from_ref(&dir)).unwrap();
        assert!((s2.n(1)[0] - 1.6).abs() < 1e-15);
        assert!(
            notes.len() == 1 && notes[0].contains("is not used"),
            "{notes:?}"
        );
        // .zmx に書くと、名前だけ書いたと注意する
        let n = lensbench_formats::zmx_write::write_notes(&s2);
        assert!(n.iter().any(|x| x.contains("LIN16")), "{n:?}");
    });
    std::fs::remove_dir_all(&dir).ok();
    // 足していないプロセスでは、カタログが無い誤り
    let e = sysfile::from_str(&txt).unwrap_err();
    assert!(
        e.to_string().contains("glass catalog LbMat was not found"),
        "{e}"
    );
}
