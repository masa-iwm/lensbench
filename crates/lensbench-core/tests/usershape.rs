//! 利用者定義の形（面の種類 USERSHAPE）: 外から登録した形のモデルを公開のテスト光学系 A0 に置き、同じ形の
//! 内蔵の面との追跡・偏光の一致、dOPL/dp・着地点の微分と差分の一致、裏返し・反転、約束の検査、誤りを確かめる

use crate::common::root;
use lensbench_core::coating::{Coatings, C64};
use lensbench_core::ext::{self, Ext};
use lensbench_core::plugin::{Config, ConfigReader, Value};
use lensbench_core::surface::SurfType;
use lensbench_core::system::{FieldSel, PolIn};
use lensbench_core::usershape::{check_shape, ShapeInstance, ShapeModel, UserShape};
use lensbench_core::{Result, System};
use lensbench_formats::zmx::{self, LoadOptions};
use std::sync::Arc;

/// 正規化の半径 [mm]
const R: f64 = 6.0;

/// a (r/R)⁴ + b (x/R)(y/R)²（b の項は x について奇）。wrong は勾配を間違える（約束の検査が見つける）
struct Poly {
    wrong: bool,
}

struct PolyInst {
    a: f64,
    b: f64,
    wrong: bool,
}

impl ShapeModel for Poly {
    fn name(&self) -> &str {
        if self.wrong {
            "lbtest_poly_wrong"
        } else {
            "lbtest_poly"
        }
    }
    fn prepare(&self, config: &Config) -> Result<Arc<dyn ShapeInstance>> {
        let mut c = ConfigReader::new(config);
        Ok(Arc::new(PolyInst {
            a: c.num("a", 0.0)?,
            b: c.num("b", 0.0)?,
            wrong: self.wrong,
        }))
    }
    fn example_config(&self) -> Option<Config> {
        Some(cfg(2e-3, 1e-3))
    }
}

impl ShapeInstance for PolyInst {
    fn sag(&self, x: f64, y: f64) -> (f64, f64, f64) {
        let (u, v) = (x / R, y / R);
        let r2 = u * u + v * v;
        let z = self.a * r2 * r2 + self.b * u * v * v;
        let zx = (4.0 * self.a * r2 * u + self.b * v * v) / R;
        let zy = (4.0 * self.a * r2 * v + 2.0 * self.b * u * v) / R;
        if self.wrong {
            return (z, 1.1 * zx, zy);
        }
        (z, zx, zy)
    }
    fn dsag(&self, key: &str, x: f64, y: f64) -> Option<[f64; 3]> {
        let (u, v) = (x / R, y / R);
        let r2 = u * u + v * v;
        match key {
            "a" => Some([r2 * r2, 4.0 * r2 * u / R, 4.0 * r2 * v / R]),
            "b" => Some([u * v * v, v * v / R, 2.0 * u * v / R]),
            _ => None,
        }
    }
}

fn cfg(a: f64, b: f64) -> Config {
    [("a", a), ("b", b)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), Value::Num(v)))
        .collect()
}

fn table() -> Arc<Ext> {
    let mut e = Ext::default();
    e.add_shape_model(Arc::new(Poly { wrong: false }));
    e.add_shape_model(Arc::new(Poly { wrong: true }));
    Arc::new(e)
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

/// A0 の面 3 を、a (r/R)⁴ + b … の利用者定義の形にした系（土台の c・k は元のまま、k は -0.3）
fn with_shape(s: &System, a: f64, b: f64) -> System {
    let mut rx = s.clone().into_prescription();
    let d = &mut rx.surfaces[3];
    d.set_type(SurfType::UserShape);
    d.k = -0.3;
    *d.user_shape_mut().unwrap() = Some(UserShape::new("lbtest_poly", cfg(a, b)).unwrap());
    let mut s = rx.build_raw().unwrap();
    s.resolve_links().unwrap();
    s
}

/// 同じ形の内蔵の偶数次非球面（a r⁴ / R⁴）
fn with_even(s: &System, a: f64) -> System {
    let mut rx = s.clone().into_prescription();
    let d = &mut rx.surfaces[3];
    d.set_type(SurfType::EvenAsph);
    d.k = -0.3;
    d.set_asph(vec![0.0, a / R.powi(4)]);
    let mut s = rx.build_raw().unwrap();
    s.resolve_links().unwrap();
    s
}

#[test]
fn traces_like_the_same_even_asphere() {
    ext::with(table(), || {
        let a = 2e-3;
        let (u, e) = (with_shape(&a0(), a, 0.0), with_even(&a0(), a));
        let px = [0.0, 0.6, -0.3, 0.2, -0.7, 0.95];
        let py = [0.0, 0.1, 0.5, -0.8, -0.4, 0.0];
        for f in 0..u.fields.len() {
            for w in 0..u.wavelengths.len() {
                let tu = u
                    .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                    .unwrap();
                let te = e
                    .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                    .unwrap();
                for (ru, re) in tu.rays.iter().zip(&te.rays) {
                    assert!(ru.status.is_ok() && re.status.is_ok());
                    for i in 0..3 {
                        assert!((ru.xyz[i] - re.xyz[i]).abs() < 1e-10, "{f} {w}");
                        assert!((ru.lmn[i] - re.lmn[i]).abs() < 1e-12, "{f} {w}");
                    }
                    assert!((ru.opl - re.opl).abs() < 1e-10, "{f} {w}");
                }
            }
        }
        // 近軸（頂点の 2 階は 0）
        let (pu, pe) = (u.paraxial(), e.paraxial());
        assert!((pu.efl - pe.efl).abs() < 1e-9 * pe.efl.abs());
        // 偏光: 形は法線を通してだけ効く
        let (cu, ce) = (
            Coatings::for_system(&u).unwrap(),
            Coatings::for_system(&e).unwrap(),
        );
        let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
        let y = [C64::new(0.0, 0.0), C64::new(1.0, 0.0)];
        let tu = u
            .try_trace_pol(FieldSel::Index(2), &px, &py, None, PolIn::new(&[x, y], &cu))
            .unwrap();
        let te = e
            .try_trace_pol(FieldSel::Index(2), &px, &py, None, PolIn::new(&[x, y], &ce))
            .unwrap();
        for i in 0..px.len() {
            for m in 0..2 {
                for c in 0..3 {
                    let d = (tu.e[i][m][c] - te.e[i][m][c]).norm();
                    // 位相は光路長 / 波長なので、交点の解き方の違い（光路長で 1e-12 mm）が 1e-8 ほど出る
                    assert!(
                        d < 1e-7,
                        "{i} {m} {c}: {} {} {d:e}",
                        tu.e[i][m][c],
                        te.e[i][m][c]
                    );
                }
            }
        }
    });
}

#[test]
fn opl_and_landing_derivatives_match_differences() {
    ext::with(table(), || {
        let s = with_shape(&a0(), 2e-3, 1e-3);
        let names = ["3.u_a", "3.u_b", "3.c", "3.k", "3.t", "1.c"];
        let p: Vec<_> = names.iter().map(|n| s.param(n).unwrap()).collect();
        crate::landing::check("A0+USERSHAPE", &s, 1, s.primary, s.image(), None, &p);
        crate::landing::check("A0+USERSHAPE dz", &s, 3, 0, s.image(), Some(0.3), &p);
        // dOPL/dp は交点を固定した光線で比べるので、像面を焦点から離す
        let mut s = s;
        s.edit_raw(|p| {
            p.solves.clear();
            p.surfaces[7].t = 10.0;
            Ok(())
        })
        .unwrap();
        crate::fermat::check("A0+USERSHAPE", &s, 1, s.primary, s.image(), &p);
        crate::fermat::check("A0+USERSHAPE 視野 3", &s, 3, 0, s.image(), &p);
        // 変数にした値を書き換えた系は、形を作り直している
        let pa = s.param("3.u_a").unwrap();
        let s2 = s.with_params(&[(pa, 3e-3)]).unwrap();
        assert_eq!(s2.get_param(pa).unwrap(), 3e-3);
        let u = s2.surfaces[3].user_shape().unwrap();
        assert_eq!(u.sag(R, 0.0).0, 3e-3);
    });
}

#[test]
fn flips_are_wrapped_by_the_core() {
    ext::with(table(), || {
        let s = with_shape(&a0(), 2e-3, 1e-3);
        let d = &s.surfaces[3];
        let (x, y) = (1.3, -2.1);
        let z0 = d.shape().sag(x, y);
        let mut f = d.clone();
        f.flip_z().unwrap();
        let zf = f.shape().sag(x, y);
        assert!((zf.0 + z0.0).abs() < 1e-15 && (zf.1 + z0.1).abs() < 1e-15);
        let mut m = d.clone();
        m.mirror_x().unwrap();
        let zm = m.shape().sag(-x, y);
        assert!((zm.0 - z0.0).abs() < 1e-15 && (zm.1 + z0.1).abs() < 1e-15);
        assert!((zm.2 - z0.2).abs() < 1e-15);
        // 2 回で元に戻る
        f.flip_z().unwrap();
        m.mirror_x().unwrap();
        assert_eq!(&f, d);
        assert_eq!(&m, d);
        // 玉の反転に入れられる（裏返しは core が包む）
        assert!(lensbench_core::elements::reversible(d));
    });
}

#[test]
fn check_shape_finds_a_wrong_gradient() {
    check_shape(&Poly { wrong: false }, &cfg(2e-3, 1e-3), R).unwrap();
    let e = check_shape(&Poly { wrong: true }, &cfg(2e-3, 1e-3), R).unwrap_err();
    assert!(
        e.to_string().contains("does not match the difference"),
        "{e}"
    );
}

#[test]
fn errors_name_the_model_and_the_setting() {
    // 登録の無いモデル
    let e = UserShape::new("lbtest_none", Config::default()).unwrap_err();
    assert!(
        e.to_string()
            .contains("no user-defined shape model lbtest_none"),
        "{e}"
    );
    ext::with(table(), || {
        let s = with_shape(&a0(), 2e-3, 0.0);
        // 微分の無い項目・無い項目は変数にできない
        assert!(s.param("3.u_zz").is_err());
        assert!(s.param("2.u_a").is_err());
        assert!(lensbench_core::param::Kind::parse("u_a.b").is_err());
        // 数の並びの項目は変数にできない
        let mut rx = s.clone().into_prescription();
        let mut c = cfg(2e-3, 0.0);
        c.insert("v".into(), Value::Nums(vec![1.0]));
        *rx.surfaces[3].user_shape_mut().unwrap() = Some(UserShape::new("lbtest_poly", c).unwrap());
        let s2 = rx.build_raw().unwrap();
        assert!(s2.param("3.u_v").is_err());
        // モデルの無い USERSHAPE の面は組み立てで誤り（二次曲面として追わない）
        let mut rx = s.into_prescription();
        *rx.surfaces[3].user_shape_mut().unwrap() = None;
        let e = rx.build_raw().unwrap_err();
        assert!(e.to_string().contains("has no model (shape)"), "{e}");
    });
}

/// 矢高 a (r/R)⁴ と、Binary 2 と同じ位相 Φ = p1 (r/R)² + p2 (r/R)⁴ [rad]。wrong は位相の勾配を間違える
struct Phased {
    wrong: bool,
}

struct PhasedInst {
    a: f64,
    p1: f64,
    p2: f64,
    wrong: bool,
}

impl ShapeModel for Phased {
    fn name(&self) -> &str {
        if self.wrong {
            "lbtest_phase_wrong"
        } else {
            "lbtest_phase"
        }
    }
    fn prepare(&self, config: &Config) -> Result<Arc<dyn ShapeInstance>> {
        let mut c = ConfigReader::new(config);
        Ok(Arc::new(PhasedInst {
            a: c.num("a", 0.0)?,
            p1: c.num("p1", 0.0)?,
            p2: c.num("p2", 0.0)?,
            wrong: self.wrong,
        }))
    }
}

impl ShapeInstance for PhasedInst {
    fn sag(&self, x: f64, y: f64) -> (f64, f64, f64) {
        let (u, v) = (x / R, y / R);
        let r2 = u * u + v * v;
        (
            self.a * r2 * r2,
            4.0 * self.a * r2 * u / R,
            4.0 * self.a * r2 * v / R,
        )
    }
    fn phase(&self, x: f64, y: f64) -> Option<(f64, f64, f64)> {
        let (u, v) = (x / R, y / R);
        let r2 = u * u + v * v;
        let g = (2.0 * self.p1 + 4.0 * self.p2 * r2) / R;
        let gx = if self.wrong { 1.1 * g * u } else { g * u };
        Some((self.p1 * r2 + self.p2 * r2 * r2, gx, g * v))
    }
}

fn phase_cfg(a: f64, p1: f64, p2: f64) -> Config {
    [("a", a), ("p1", p1), ("p2", p2)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), Value::Num(v)))
        .collect()
}

/// 利用者定義の形の位相は Binary 2 と同じに効く: 同じ矢高・位相の Binary 2 の面と、光線・光路長・近軸の焦点距離・
/// 偏光の場が合う。dOPL/dp は誤り。約束の検査は位相の勾配の誤りを見つける
#[test]
fn a_user_phase_traces_like_binary2() {
    let mut e = Ext::default();
    e.add_shape_model(Arc::new(Phased { wrong: false }));
    e.add_shape_model(Arc::new(Phased { wrong: true }));
    ext::with(Arc::new(e), || {
        let (a, p1, p2) = (2e-3, -40.0, 6.0);
        let mut rx = a0().into_prescription();
        let d = &mut rx.surfaces[3];
        d.set_type(SurfType::UserShape);
        d.k = -0.3;
        *d.user_shape_mut().unwrap() =
            Some(UserShape::new("lbtest_phase", phase_cfg(a, p1, p2)).unwrap());
        let mut u = rx.build_raw().unwrap();
        u.resolve_links().unwrap();
        let mut rx = a0().into_prescription();
        let d = &mut rx.surfaces[3];
        d.set_type(SurfType::Binary2);
        d.k = -0.3;
        d.set_asph(vec![0.0, a / R.powi(4)]);
        *d.bin2_mut().unwrap() = Some(Arc::new(lensbench_core::surface::Binary2 {
            order: 1.0,
            norm: R,
            coef: vec![p1, p2],
        }));
        let mut b = rx.build_raw().unwrap();
        b.resolve_links().unwrap();

        let px = [0.0, 0.6, -0.3, 0.2, -0.7, 0.95];
        let py = [0.0, 0.1, 0.5, -0.8, -0.4, 0.0];
        for f in 0..u.fields.len() {
            for w in 0..u.wavelengths.len() {
                let tu = u
                    .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                    .unwrap();
                let tb = b
                    .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                    .unwrap();
                for (ru, rb) in tu.rays.iter().zip(&tb.rays) {
                    assert_eq!(ru.status.is_ok(), rb.status.is_ok(), "{f} {w}");
                    if !ru.status.is_ok() {
                        continue;
                    }
                    for i in 0..3 {
                        assert!((ru.xyz[i] - rb.xyz[i]).abs() < 1e-10, "{f} {w}");
                        assert!((ru.lmn[i] - rb.lmn[i]).abs() < 1e-12, "{f} {w}");
                    }
                    assert!((ru.opl - rb.opl).abs() < 1e-10, "{f} {w}");
                }
            }
        }
        let (pu, pb) = (u.paraxial(), b.paraxial());
        assert!(
            (pu.efl - pb.efl).abs() < 1e-7 * pb.efl.abs(),
            "{} {}",
            pu.efl,
            pb.efl
        );
        assert!((pu.efl - a0().paraxial().efl).abs() > 1e-3);
        let (cu, cb) = (
            Coatings::for_system(&u).unwrap(),
            Coatings::for_system(&b).unwrap(),
        );
        let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
        let tu = u
            .try_trace_pol(FieldSel::Index(2), &px, &py, None, PolIn::new(&[x], &cu))
            .unwrap();
        let tb = b
            .try_trace_pol(FieldSel::Index(2), &px, &py, None, PolIn::new(&[x], &cb))
            .unwrap();
        for i in 0..px.len() {
            for c in 0..3 {
                assert!((tu.e[i][0][c] - tb.e[i][0][c]).norm() < 1e-7, "{i} {c}");
            }
        }
        // dOPL/dp は Binary 2 と同じく未対応
        let t = u
            .try_trace(FieldSel::Index(1), &[0.3], &[0.2], None, None, true)
            .unwrap();
        let p = u.param("3.t").unwrap();
        assert!(u.opl_grad(&t, u.primary, &[p]).is_err());
        // 約束の検査
        let cfg = phase_cfg(a, p1, p2);
        check_shape(&Phased { wrong: false }, &cfg, R).unwrap();
        let e = check_shape(&Phased { wrong: true }, &cfg, R)
            .unwrap_err()
            .to_string();
        assert!(e.contains("phase gradient"), "{e}");
    });
}

/// 平面の形で Jones 行列を返すモデル: kind 0 は λ/4（速軸 x、[1, 0, 0, i]）、kind 1 は位置で変わる位相 exp(i a x)·I
struct JonesShape;

struct JonesShapeInst {
    kind: f64,
    a: f64,
}

impl ShapeModel for JonesShape {
    fn name(&self) -> &str {
        "lbtest_jones"
    }
    fn prepare(&self, config: &Config) -> Result<Arc<dyn ShapeInstance>> {
        let mut c = ConfigReader::new(config);
        Ok(Arc::new(JonesShapeInst {
            kind: c.num("kind", 0.0)?,
            a: c.num("a", 0.0)?,
        }))
    }
}

impl ShapeInstance for JonesShapeInst {
    fn sag(&self, _: f64, _: f64) -> (f64, f64, f64) {
        (0.0, 0.0, 0.0)
    }
    fn jones(&self, x: f64, _: f64, _: f64) -> Option<[C64; 4]> {
        let (o, z) = (C64::new(1.0, 0.0), C64::new(0.0, 0.0));
        Some(if self.kind == 0.0 {
            [o, z, z, C64::new(0.0, 1.0)]
        } else {
            let p = C64::from_polar(1.0, self.a * x);
            [p, z, z, p]
        })
    }
}

/// 像面を Jones を返す利用者定義の形にすると、同じ行列の Jones 面と同じ場になる（基底は Jones 面と同じ）。位置で変わる
/// 行列は交点ごとに引く（exp(i a x)·I は場をその位相だけ回す）
#[test]
fn a_user_jones_matches_the_jones_surface() {
    let mut e = Ext::default();
    e.add_shape_model(Arc::new(JonesShape));
    ext::with(Arc::new(e), || {
        let s = a0();
        let img = s.image();
        let with = |kind: f64, a: f64| {
            let mut s = s.clone();
            s.edit_raw(|p| {
                p.surfaces[img].set_type(SurfType::UserShape);
                *p.surfaces[img].user_shape_mut().unwrap() = Some(
                    UserShape::new("lbtest_jones", phase_like_cfg(&[("kind", kind), ("a", a)]))
                        .unwrap(),
                );
                Ok(())
            })
            .unwrap();
            s
        };
        let mut j = s.clone();
        j.edit_raw(|p| {
            p.surfaces[img].set_type(SurfType::Jones);
            *p.surfaces[img].jones_mut().unwrap() = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
            Ok(())
        })
        .unwrap();
        let px = [0.0, 0.6, -0.3, 0.2, -0.7];
        let py = [0.0, 0.1, 0.5, -0.8, -0.4];
        let d45 = [C64::new(0.5f64.sqrt(), 0.0), C64::new(0.5f64.sqrt(), 0.0)];
        let x = [C64::new(1.0, 0.0), C64::new(0.0, 0.0)];
        let tr = |s: &System, f: usize| {
            let c = Coatings::for_system(s).unwrap();
            s.try_trace_pol(
                FieldSel::Index(f),
                &px,
                &py,
                None,
                PolIn::new(&[d45, x], &c),
            )
            .unwrap()
        };
        let q = with(0.0, 0.0);
        for f in 0..s.fields.len() {
            let (a, b) = (tr(&q, f), tr(&j, f));
            for i in 0..px.len() {
                for m in 0..2 {
                    for c in 0..3 {
                        let d = (a.e[i][m][c] - b.e[i][m][c]).norm();
                        assert!(d < 1e-12, "{f} {i} {m} {c}: {d:e}");
                    }
                }
            }
        }
        // 位置で変わる位相: 場は exp(i a x) 倍（x は像面の交点）
        let a = 3.0;
        let (p, b) = (with(1.0, a), with(1.0, 0.0));
        let (tp, tb) = (tr(&p, 2), tr(&b, 2));
        for i in 0..px.len() {
            let ph = C64::from_polar(1.0, a * tp.trace.rays[i].xyz[0]);
            for c in 0..3 {
                let d = (tp.e[i][0][c] - ph * tb.e[i][0][c]).norm();
                assert!(d < 1e-12, "{i} {c}: {d:e}");
            }
        }
    });
}

fn phase_like_cfg(kv: &[(&str, f64)]) -> Config {
    kv.iter()
        .map(|(k, v)| (k.to_string(), Value::Num(*v)))
        .collect()
}
