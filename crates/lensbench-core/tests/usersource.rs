//! 利用者定義の光源: 物体の座標系の光線が瞳座標の追跡と同じ光線になること、約束の検査、拡張の表への登録

use crate::common::root;
use lensbench_core::ext::Ext;
use lensbench_core::plugin::{Config, Value};
use lensbench_core::system::{FieldSel, Launch};
use lensbench_core::usersource::{check_source, SourceInstance, SourceModel, SourceRays};
use lensbench_core::System;
use lensbench_formats::zmx::{self, LoadOptions};
use std::sync::Arc;

fn load(name: &str) -> System {
    zmx::load_with(
        &root().join("tests/refs").join(name).join("system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap()
}

/// 決めた光線を返す光源
struct Fixed(SourceRays);

impl SourceInstance for Fixed {
    fn rays(&self, _wl: f64) -> lensbench_core::Result<SourceRays> {
        Ok(self.0.clone())
    }
}

struct FixedModel;

impl SourceModel for FixedModel {
    fn name(&self) -> &str {
        "test_fixed"
    }
    fn prepare(&self, c: &Config) -> lensbench_core::Result<Arc<dyn SourceInstance>> {
        let w = match c.get("w") {
            Some(Value::Num(w)) => *w,
            _ => 1.0,
        };
        Ok(Arc::new(Fixed(SourceRays {
            p: vec![[0.0, 1.0, 0.0]; 2],
            k: vec![[0.0, 0.0, 1.0], [0.0, 0.1, 1.0]],
            w: vec![w; 2],
        })))
    }
}

/// 瞳座標の追跡の始点・方向を物体の座標系に直して光源から返すと、大域に戻した光線で同じ追跡になる
#[test]
fn source_rays_reproduce_the_pupil_trace() {
    for name in ["A0", "B1"] {
        let s = load(name);
        let px: Vec<f64> = (-4..=4).map(|i| i as f64 / 4.0).collect();
        let py: Vec<f64> = px.iter().map(|x| 0.5 * x).collect();
        let w = s.primary;
        let t = s
            .try_trace(
                FieldSel::Index(s.fields.len() - 1),
                &px,
                &py,
                Some(w),
                None,
                false,
            )
            .unwrap();
        let f = s.object_frame();
        // 大域 → 物体の座標系（回転の転置）
        let inv = |v: [f64; 3]| [0, 1, 2].map(|c| (0..3).map(|r| f.r[r][c] * v[r]).sum::<f64>());
        let src = Fixed(SourceRays {
            p: t.p0
                .iter()
                .map(|p| inv([p[0] - f.o[0], p[1] - f.o[1], p[2] - f.o[2]]))
                .collect(),
            k: t.k0.iter().map(|&k| inv(k)).collect(),
            w: vec![1.0; px.len()],
        });
        let r = s.source_rays(&src, w).unwrap();
        for i in 0..px.len() {
            for c in 0..3 {
                assert!((r.p[i][c] - t.p0[i][c]).abs() < 1e-9, "{name} {i}");
                assert!((r.k[i][c] - t.k0[i][c]).abs() < 1e-12, "{name} {i}");
            }
        }
        let u = s.trace_launch(&Launch::Rays { p: &r.p, k: &r.k }, w, None, false);
        for (i, (a, b)) in t.rays.iter().zip(&u.rays).enumerate() {
            assert_eq!(a.status, b.status, "{name} {i}");
            if a.status.surf().is_none() {
                for c in 0..3 {
                    assert!((a.xyz[c] - b.xyz[c]).abs() < 1e-9, "{name} {i}");
                }
            }
        }
        assert!(s.source_rays(&src, s.wavelengths.len()).is_err());
    }
}

/// 約束を守らない光線（負の重み・長さの違い・0 の方向）は誤り
#[test]
fn the_check_rejects_broken_sources() {
    let m = FixedModel;
    check_source(&m, &Config::new()).unwrap();
    let neg = Config::from([("w".to_string(), Value::Num(-1.0))]);
    assert!(check_source(&m, &neg).is_err());
    let ok = SourceRays {
        p: vec![[0.0; 3]],
        k: vec![[0.0, 0.0, 1.0]],
        w: vec![1.0],
    };
    ok.check().unwrap();
    let mut short = ok.clone();
    short.w.clear();
    assert!(short.check().is_err());
    let mut zero = ok.clone();
    zero.k[0] = [0.0; 3];
    assert!(zero.check().is_err());
    let mut nan = ok;
    nan.p[0][1] = f64::NAN;
    assert!(nan.check().is_err());
}

#[test]
fn a_source_model_is_registered_by_name() {
    let mut e = Ext::default();
    e.add_source_model(Arc::new(FixedModel));
    assert_eq!(e.source_models(), ["test_fixed"]);
    assert!(e.source_model("test_fixed").is_ok());
    let mut f = Ext::default();
    f.merge(&e);
    assert!(f.source_model("test_fixed").is_ok());
    assert!(f.remove_source_model("test_fixed"));
    let msg = f.source_model("test_fixed").err().unwrap().to_string();
    assert!(msg.contains("test_fixed"), "{msg}");
}

/// trace_source は光源の重みを追跡の結果に持たせ、光線は Launch::Rays の追跡とビットまで同じ。重みの無い追跡は None
#[test]
fn trace_source_carries_the_weights() {
    let s = load("A0");
    let w = s.primary;
    let src = Fixed(SourceRays {
        p: vec![[0.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.3, -0.2, 0.0]],
        k: vec![[0.0, 0.0, 1.0], [0.0, 0.01, 1.0], [0.005, -0.003, 1.0]],
        w: vec![0.5, 2.0, 1.25],
    });
    let t = s.trace_source(&src, w, None, false).unwrap();
    assert_eq!(t.w.as_deref(), Some(&[0.5, 2.0, 1.25][..]));
    let r = s.source_rays(&src, w).unwrap();
    let u = s.trace_launch(&Launch::Rays { p: &r.p, k: &r.k }, w, None, false);
    assert!(u.w.is_none());
    assert_eq!(u.weight(1), 1.0);
    for (a, b) in t.rays.iter().zip(&u.rays) {
        assert_eq!(a.status, b.status);
        assert_eq!(a.xyz.map(f64::to_bits), b.xyz.map(f64::to_bits));
        assert_eq!(a.opl.to_bits(), b.opl.to_bits());
    }
    let arrived: f64 = (0..3).filter(|&i| t.ok(i)).map(|i| t.weight(i)).sum();
    assert_eq!(t.arrived_weight(), arrived);
    // 重みの長さ・値の誤り
    assert!(u.clone().with_weights(vec![1.0; 2]).is_err());
    assert!(u.with_weights(vec![1.0, -1.0, 1.0]).is_err());
}

/// 光線のパワーは全パワー × w / Σw（和が全パワー）
#[test]
fn powers_split_the_total_power_by_weight() {
    let r = SourceRays {
        p: vec![[0.0; 3]; 3],
        k: vec![[0.0, 0.0, 1.0]; 3],
        w: vec![1.0, 3.0, 0.0],
    };
    assert_eq!(r.powers(2.0).unwrap(), vec![0.5, 1.5, 0.0]);
    let sum: f64 = r.powers(1.0).unwrap().iter().sum();
    assert!((sum - 1.0).abs() < 1e-15);
    let zero = SourceRays {
        w: vec![0.0; 3],
        ..r.clone()
    };
    assert!(zero.powers(1.0).is_err());
    assert!(r.powers(-1.0).is_err());
}
