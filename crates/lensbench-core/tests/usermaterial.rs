//! 利用者定義の材料: AGF のガラスを包んだモデルをカタログとして足し、公開のテスト光学系 A0 の材料を置き換えると、
//! 元のガラスと同じに追える（系の温度・気圧、屈折率を環境に合わせない場合、内部透過率も）。カタログの名前で引く経路、
//! 足していないときの誤り、約束の検査も確かめる

use crate::common::root;
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::ext::{self, Ext};
use lensbench_core::glass::{self, Glass};
use lensbench_core::system::{FieldSel, PolIn};
use lensbench_core::usermaterial::{check_material, MaterialModel};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use std::sync::Arc;

/// AGF のガラスをそのまま返すモデル（名前は別に付ける）
struct Wrap {
    name: &'static str,
    g: Glass,
}

impl MaterialModel for Wrap {
    fn name(&self) -> &str {
        self.name
    }
    fn index(&self, wl: f64, t: f64, p: f64) -> f64 {
        self.g.n(wl, t, p)
    }
    fn tref(&self) -> f64 {
        self.g.td[6]
    }
    fn range(&self) -> Option<[f64; 2]> {
        self.g.ld
    }
    fn transmittance(&self) -> Vec<[f64; 3]> {
        self.g.it.clone()
    }
}

/// 範囲の外で NaN を返す（約束を守らない）
struct Bad;

impl MaterialModel for Bad {
    fn name(&self) -> &str {
        "BAD"
    }
    fn index(&self, wl: f64, _t: f64, _p: f64) -> f64 {
        if wl > 0.6 {
            f64::NAN
        } else {
            1.5
        }
    }
}

fn a0() -> System {
    zmx::load_with(
        &root().join("tests/refs/A0/system.zmx"),
        &LoadOptions {
            cfg: 1,
            catalog_dirs: vec![root().join("tests/glass")],
            ..Default::default()
        },
    )
    .unwrap()
}

fn table(s: &System) -> Arc<Ext> {
    let mut e = Ext::default();
    e.add_material_catalog(
        "LbTestMat",
        vec![
            Arc::new(Wrap {
                name: "MY-SK16",
                g: s.glasses["N-SK16"].clone(),
            }),
            Arc::new(Wrap {
                name: "MY-F2",
                g: s.glasses["F2"].clone(),
            }),
        ],
    );
    Arc::new(e)
}

/// A0 の材料を、足したカタログの材料に置き換えた系
fn replaced(s: &System) -> System {
    let mut u = s.clone();
    let missing = u.add_glass_catalogs(&["LBTESTMAT".into()], &[]);
    assert!(missing.is_empty(), "{missing:?}");
    for (j, m) in [(1, "MY-SK16"), (3, "MY-F2"), (6, "MY-SK16")] {
        u.set_material(j, m).unwrap();
    }
    u
}

fn same_trace(a: &System, b: &System) {
    let px = [0.0, 0.6, -0.3, 0.2, -0.7];
    let py = [0.0, 0.1, 0.5, -0.8, -0.4];
    for f in 0..a.fields.len() {
        for w in 0..a.wavelengths.len() {
            let ta = a
                .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                .unwrap();
            let tb = b
                .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                .unwrap();
            for (ra, rb) in ta.rays.iter().zip(&tb.rays) {
                assert_eq!((ra.xyz, ra.lmn, ra.opl), (rb.xyz, rb.lmn, rb.opl));
            }
        }
    }
}

#[test]
fn a_wrapped_glass_traces_like_the_glass() {
    let s = a0();
    ext::with(table(&s), || {
        let u = replaced(&s);
        assert_eq!(u.glasses["MY-SK16"].catalog, "LbTestMat");
        same_trace(&s, &u);
        assert_eq!(s.paraxial().efl, u.paraxial().efl);
        // 系の温度・気圧（屈折率を環境に合わせる）と、合わせない場合
        for adjust in [true, false] {
            let env = |x: &System| {
                let mut x = x.clone();
                x.edit(|p| {
                    p.temperature = 35.0;
                    p.pressure = 0.8;
                    p.adjust_index = adjust;
                    Ok(())
                })
                .unwrap();
                x
            };
            same_trace(&env(&s), &env(&u));
        }
        // 内部透過率（偏光の追跡の吸収）
        let (cs, cu) = (
            Coatings::for_system(&s).unwrap(),
            Coatings::for_system(&u).unwrap(),
        );
        let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
        let ts = s
            .try_trace_pol(
                FieldSel::Index(1),
                &[0.3],
                &[0.2],
                None,
                PolIn::new(&[x], &cs),
            )
            .unwrap();
        let tu = u
            .try_trace_pol(
                FieldSel::Index(1),
                &[0.3],
                &[0.2],
                None,
                PolIn::new(&[x], &cu),
            )
            .unwrap();
        assert_eq!(ts.e, tu.e);
    });
}

#[test]
fn catalogs_are_looked_up_by_name() {
    let s = a0();
    ext::with(table(&s), || {
        // 大文字小文字は問わない。名前@カタログでも引ける
        let (g, missing) = glass::load_catalogs(&["lbtestmat".into()], &[]);
        assert!(missing.is_empty());
        assert_eq!(g["MY-F2"].ld, s.glasses["F2"].ld);
        let mut t = std::collections::HashMap::new();
        let left = glass::resolve_qualified(&mut t, ["MY-F2@LbTestMat"], &[]);
        assert!(left.is_empty(), "{left:?}");
        assert!(t.contains_key("MY-F2@LbTestMat"));
    });
    // 足していなければカタログが無い
    let (_, missing) = glass::load_catalogs(&["LBTESTMAT".into()], &[]);
    assert_eq!(missing, ["LBTESTMAT"]);
    // 表の外で作った系をあとで組み直すと、カタログを足す拡張が無いと止まる
    let u = ext::with(table(&s), || replaced(&s));
    let mut u2 = u.clone();
    let e = u2
        .edit(|p| {
            p.temperature = 30.0;
            Ok(())
        })
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("no extension registers that catalog"),
        "{e}"
    );
    // カタログはあってもモデルが無ければ、その名前を言う
    let mut e = Ext::default();
    e.add_material_catalog("lbtestmat", vec![]);
    ext::with(Arc::new(e), || {
        let e = u
            .clone()
            .edit(|p| {
                p.temperature = 30.0;
                Ok(())
            })
            .unwrap_err();
        assert!(e.to_string().contains("has no MY-SK16"), "{e}");
    });
}

#[test]
fn check_material_finds_a_broken_model() {
    let s = a0();
    check_material(&Wrap {
        name: "MY-SK16",
        g: s.glasses["N-SK16"].clone(),
    })
    .unwrap();
    let e = check_material(&Bad).unwrap_err();
    assert!(e.to_string().contains("not a positive number"), "{e}");
}
