//! 利用者定義の面: 画角 → 像高を与える理想レンズ `ideal_mapping` を、f 50 の理想レンズの系に置いて追う

use lensbench_core::plugin::{Config, Value};
use lensbench_core::surface::SurfType;
use lensbench_core::usersurf::{self, SurfaceInstance, SurfaceModel, UserSurf};
use lensbench_core::Waves;
use lensbench_core::{analysis, zmx, FieldSel, System};
use std::sync::Arc;

const ZMX: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 4\nFTYP 0 0 5 1 0 0 0\n\
XFLN 0 0 0 0 0\nYFLN 0 10 20 30 40\nFWGN 1 1 1 1 1\nWAVM 1 0.55 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 50\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";

fn paraxial() -> System {
    let raw = zmx::parse_text(ZMX).unwrap();
    zmx::build(&raw, &zmx::LoadOptions::default(), &[], None).unwrap()
}

fn with_user(kv: &[(&str, Value)]) -> System {
    let mut s = paraxial();
    let cfg: Config = kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    s.surfaces[1].user = Some(UserSurf::new("ideal_mapping", cfg).unwrap());
    s.surfaces[1].typ = SurfType::User;
    s.changed().unwrap();
    s
}

fn f50(m: &str) -> System {
    with_user(&[("f", Value::Num(50.0)), ("mapping", Value::Str(m.into()))])
}

#[test]
fn tan_traces_like_the_paraxial_lens() {
    let (a, b) = (paraxial(), f50("tan"));
    let px = [0.0, 0.3, -0.7, 0.5];
    let py = [0.0, -0.2, 0.4, 0.9];
    for f in 0..5 {
        let ta = a.trace(FieldSel::Index(f), &px, &py, None, None, false);
        let tb = b.trace(FieldSel::Index(f), &px, &py, None, None, false);
        for (ra, rb) in ta.rays.iter().zip(&tb.rays) {
            assert_eq!(
                (ra.xyz, ra.lmn, ra.opl, ra.lens_opd),
                (rb.xyz, rb.lmn, rb.opl, rb.lens_opd)
            );
        }
    }
    let (pa, pb) = (a.paraxial(), b.paraxial());
    assert_eq!(pa.efl, pb.efl);
}

#[test]
fn image_height_follows_the_mapping_without_aberration() {
    for (m, h) in [
        ("theta", (|t: f64| 50.0 * t) as fn(f64) -> f64),
        ("sin", |t: f64| 50.0 * t.sin()),
        ("equisolid", |t: f64| 100.0 * (t / 2.0).sin()),
        ("stereographic", |t: f64| 100.0 * (t / 2.0).tan()),
    ] {
        let s = f50(m);
        assert!((s.paraxial().efl - 50.0).abs() < 1e-12, "{m}");
        for (i, deg) in [0.0f64, 10.0, 20.0, 30.0, 40.0].iter().enumerate() {
            let t = s.trace(FieldSel::Index(i), &[0.0], &[0.0], None, None, false);
            let want = h(deg.to_radians());
            assert!(
                (t.rays[0].xyz[1] - want).abs() < 1e-9,
                "{m} {deg}: {} {want}",
                t.rays[0].xyz[1]
            );
            // 同じ方向の光線は 1 点に集まり（スポット 0）、OPD も 0
            let sp = analysis::spot(&s, FieldSel::Index(i), Waves::All, 6).unwrap();
            assert!(sp.rms_chief < 1e-9, "{m} {deg}: {}", sp.rms_chief);
            let px: Vec<f64> = (0..9).map(|k| -0.8 + 0.2 * k as f64).collect();
            let py: Vec<f64> = px.iter().rev().cloned().collect();
            let o = analysis::opd(&s, FieldSel::Index(i), &px, &py, None, false).unwrap();
            let mx = o
                .opd
                .iter()
                .filter(|v| v.is_finite())
                .fold(0.0f64, |a, v| a.max(v.abs()));
            assert!(mx < 1e-6, "{m} {deg}: OPD {mx}");
        }
    }
}

#[test]
fn polarization_and_errors() {
    let s = f50("theta");
    // 偏光追跡も通る（理想レンズと同じく振幅は変えない）
    let c = lensbench_core::coating::Coatings::default();
    let one = lensbench_core::coating::C64::new(1.0, 0.0);
    let zero = lensbench_core::coating::C64::new(0.0, 0.0);
    let t = s
        .try_trace_pol(FieldSel::Index(3), &[0.2], &[0.5], None, &[[one, zero]], &c)
        .unwrap();
    let p: f64 = t.e[0][0].iter().map(|z| z.norm_sqr()).sum();
    assert!((p - 1.0).abs() < 1e-12, "{p}");
    // dOPL/dp は未対応
    let tr = s
        .try_trace(FieldSel::Index(1), &[0.0], &[0.5], None, None, true)
        .unwrap();
    let prm = lensbench_core::param::Param::new(2, lensbench_core::param::Kind::Thick);
    assert!(s.opl_grad(&tr, 0, &[prm]).is_err());
    // 空気以外の中は未対応
    let mut g = f50("theta");
    g.surfaces[1].n = vec![1.5];
    assert!(g.validate().is_err());
    // .zmx には書けない
    assert!(lensbench_core::zmx_write::write(&s).is_err());
}

/// 利用者が登録するモデル（光線を曲げずに、決まった位相だけ足す）
struct Plate;
struct PlateInst(f64);
impl SurfaceModel for Plate {
    fn name(&self) -> &str {
        "test_plate"
    }
    fn prepare(&self, c: &Config) -> lensbench_core::Result<Arc<dyn SurfaceInstance>> {
        let mut r = lensbench_core::plugin::ConfigReader::new(c);
        let a = r.num("a", 0.0)?;
        r.finish()?;
        Ok(Arc::new(PlateInst(a)))
    }
}
impl SurfaceInstance for PlateInst {
    fn deflect(&self, p: [f64; 3], k: [f64; 3], _: f64, _: f64, _: f64) -> Option<([f64; 3], f64)> {
        Some((k, self.0 * (p[0] * p[0] + p[1] * p[1])))
    }
}

#[test]
fn user_registered_model() {
    usersurf::register(Arc::new(Plate));
    assert!(usersurf::models().contains(&"test_plate".to_string()));
    let mut s = paraxial();
    // 理想レンズの直後に位相板（間隔 0）を足す代わりに、像面の手前の面を位相板にする
    s.surfaces[2].typ = SurfType::User;
    s.surfaces[2].user =
        Some(UserSurf::new("test_plate", Config::from([("a".into(), Value::Num(1e-3))])).unwrap());
    s.changed().unwrap();
    let t = s.trace(FieldSel::Index(0), &[0.0], &[1.0], None, None, false);
    let t0 = paraxial().trace(FieldSel::Index(0), &[0.0], &[1.0], None, None, false);
    assert_eq!(t.rays[0].xyz, t0.rays[0].xyz);
    assert!(
        (t.rays[0].lens_opd - t0.rays[0].lens_opd - 1e-3 * t0.rays[0].xyz[1].powi(2)).abs() < 1e-15
    );
    assert!(usersurf::unregister("test_plate"));
}
