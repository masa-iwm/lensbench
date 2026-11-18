//! 利用者定義の形（USERSHAPE）を外から登録し、編集の操作で面に付け、系ファイルに書いて読み直す

use lensbench_core::ext::{self, Ext};
use lensbench_core::plugin::{Config, ConfigReader};
use lensbench_core::usershape::{ShapeInstance, ShapeModel};
use lensbench_core::{zmx, FieldSel, Result};
use lensbench_project::{edit_ops, sysfile};
use serde_json::json;
use std::sync::Arc;

/// a x³（x について奇なので、映すと向きが変わる）
struct Cubic;
struct CubicInst(f64);

impl ShapeModel for Cubic {
    fn name(&self) -> &str {
        "lbtest_cubic"
    }
    fn prepare(&self, config: &Config) -> Result<Arc<dyn ShapeInstance>> {
        Ok(Arc::new(CubicInst(
            ConfigReader::new(config).num("a", 0.0)?,
        )))
    }
}

impl ShapeInstance for CubicInst {
    fn sag(&self, x: f64, _y: f64) -> (f64, f64, f64) {
        (self.0 * x * x * x, 3.0 * self.0 * x * x, 0.0)
    }
}

const ZMX: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 4\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 5\nYFLN 0 0\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";

#[test]
fn edit_and_system_file_round_trip() {
    let mut e = Ext::default();
    e.add_shape_model(Arc::new(Cubic));
    ext::with(Arc::new(e), || {
        let raw = zmx::parse_text(ZMX).unwrap();
        let mut s = zmx::build(&raw, &zmx::LoadOptions::default(), &[], None).unwrap();
        // 形のデータ（shape）が無いうちは面タイプを変えられない
        assert!(edit_ops::apply_json(
            &mut s,
            &json!([{"op": "set_surface_type", "surface": 1, "type": "USERSHAPE"}])
        )
        .is_err());
        edit_ops::apply_json(
            &mut s,
            &json!([
                {"op": "set_surface_data", "surface": 1,
                 "data": {"typ": "UserShape", "shape": {"model": "lbtest_cubic", "config": {"a": 2e-5}, "mirror_x": true}}},
            ]),
        )
        .unwrap();
        let u = s.surfaces[1].user_shape().unwrap();
        assert!(u.mirror_x && !u.flip_z);
        // 映したので x > 0 で矢高が負
        assert!(s.surfaces[1].shape().sag(1.0, 0.0).0 < 0.02 / 2.0);
        let d = edit_ops::surface_data(&s, 1).unwrap();
        assert_eq!(d["data"]["shape"]["model"], json!("lbtest_cubic"));
        assert_eq!(d["data"]["shape"]["mirror_x"], json!(true));
        // 系ファイルに書いて読み直すと同じ追跡
        let txt = sysfile::to_string(&s, &[]).unwrap();
        assert!(txt.contains("\"mirror_x\": true"), "{txt}");
        let (s2, _, _) = sysfile::from_str(&txt).unwrap();
        let (px, py) = ([0.0, 0.7, -0.4], [0.0, -0.5, 0.6]);
        let t1 = s
            .try_trace(FieldSel::Index(1), &px, &py, None, None, false)
            .unwrap();
        let t2 = s2
            .try_trace(FieldSel::Index(1), &px, &py, None, None, false)
            .unwrap();
        for (a, b) in t1.rays.iter().zip(&t2.rays) {
            assert_eq!((a.xyz, a.lmn, a.opl), (b.xyz, b.lmn, b.opl));
        }
        // .zmx には書けない
        assert!(lensbench_formats::zmx_write::write(&s).is_err());
        // 知らないモデルは誤り
        let bad = txt.replace("lbtest_cubic", "no_such_shape");
        assert!(sysfile::from_str(&bad).is_err());
    });
}
