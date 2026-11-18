//! MTF の子午・球欠の向き: 既定（視野の向き）では、回転対称な系の子午・球欠の MTF は視野の方位に
//! よらない（y の視野と、同じ大きさの x・斜めの視野で同じ値）。瞳の向き（OpticStudio と同じ）は x の視野で
//! 子午と球欠が入れ替わる

use crate::common::root;
use lensbench_core::analysis::{self, MtfAxes, MtfMethod};
use lensbench_core::surface::Aperture;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::{psf, Waves};
use lensbench_formats::zmx::{self, LoadOptions};

/// A0（回転対称、角度の視野）の視野を、y の角度 h [°] と同じ向きの大きさ（tan の大きさ）で方位 0°（y）・90°（x）・
/// 45°・135°・30°・−120° の 6 つにした系（45° の倍数でない方位では (θx, θy) と (tan θx, tan θy) の向きが違う）
fn rotated_fields(h: f64) -> System {
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    let mut s = zmx::load_with(&root().join("tests/refs/A0/system.zmx"), &opt).unwrap();
    let d = (h.to_radians().tan() / 2f64.sqrt()).atan().to_degrees();
    // 方位 a [°]（y から x の向きに）の視野の角度 (θx, θy)
    let az = |a: f64| {
        let t = h.to_radians().tan();
        let (sn, cs) = a.to_radians().sin_cos();
        ((t * sn).atan().to_degrees(), (t * cs).atan().to_degrees())
    };
    s.edit_raw(|rx| {
        let f0 = rx.fields[0];
        rx.fields = [(0.0, h), (h, 0.0), (d, d), (-d, d), az(30.0), az(-120.0)]
            .iter()
            .map(|&(x, y)| {
                let mut f = f0;
                (f.x, f.y) = (x, y);
                f
            })
            .collect();
        Ok(())
    })
    .unwrap();
    s
}

const FREQS: [f64; 4] = [10.0, 30.0, 60.0, 90.0];

fn mtf(s: &System, i: usize, method: MtfMethod, axes: MtfAxes) -> analysis::Mtf {
    let f = FieldSel::Index(i);
    analysis::mtf_amp(s, f, Waves::All, None, &FREQS, method, None, None, axes).unwrap()
}

fn close(a: &[f64], b: &[f64], tol: f64, what: &str) {
    for (k, (x, y)) in a.iter().zip(b).enumerate() {
        assert!((x - y).abs() < tol, "{what} {}: {x} {y}", FREQS[k]);
    }
}

#[test]
fn field_axes_do_not_depend_on_the_field_azimuth() {
    let s = rotated_fields(14.0);
    for (method, tol) in [
        (MtfMethod::Interp, 1e-5),
        (MtfMethod::Direct, 1e-5),
        (MtfMethod::Fft, 2e-4),
    ] {
        let y = mtf(&s, 0, method, MtfAxes::Field);
        for i in 1..s.fields.len() {
            let m = mtf(&s, i, method, MtfAxes::Field);
            close(
                &m.tan,
                &y.tan,
                tol,
                &format!("{method:?} field {} T", i + 1),
            );
            close(
                &m.sag,
                &y.sag,
                tol,
                &format!("{method:?} field {} S", i + 1),
            );
            let c = (m.cutoff.tan / y.cutoff.tan, m.cutoff.sag / y.cutoff.sag);
            assert!(
                (c.0 - 1.0).abs() < 1e-9 && (c.1 - 1.0).abs() < 1e-9,
                "{c:?}"
            );
        }
        // y の視野は瞳の向きと同じビット
        let p = mtf(&s, 0, method, MtfAxes::Pupil);
        assert_eq!((&p.tan, &p.sag), (&y.tan, &y.sag));
        // 瞳の向きでは x の視野の子午は y の視野の球欠（子午と球欠が入れ替わる）
        let px = mtf(&s, 1, method, MtfAxes::Pupil);
        close(&px.tan, &y.sag, tol, &format!("{method:?} pupil x T"));
        close(&px.sag, &y.tan, tol, &format!("{method:?} pupil x S"));
    }
}

#[test]
fn geometric_mtf_follows_the_field_direction() {
    let s = rotated_fields(14.0);
    let g = |i: usize, axes: MtfAxes| {
        let f = FieldSel::Index(i);
        psf::geometric_mtf_axes(&s, f, Waves::All, 64, &FREQS, false, axes).unwrap()
    };
    let y = g(0, MtfAxes::Field);
    for i in 1..s.fields.len() {
        let m = g(i, MtfAxes::Field);
        close(
            &m.tan,
            &y.tan,
            1e-9,
            &format!("geometric field {} T", i + 1),
        );
        close(
            &m.sag,
            &y.sag,
            1e-9,
            &format!("geometric field {} S", i + 1),
        );
    }
    let px = g(1, MtfAxes::Pupil);
    close(&px.tan, &y.sag, 1e-9, "geometric pupil x T");
}

/// 瞳が遮蔽で切れている（けられの求積の経路）ときも同じ
#[test]
fn clipped_pupils_follow_the_field_direction() {
    let mut s = rotated_fields(14.0);
    s.edit_raw(|rx| {
        let st = rx.surfaces.iter().position(|d| d.stop).unwrap();
        rx.surfaces[st].ap = Aperture::Obsc {
            min: 0.0,
            max: 1.5,
            dx: 0.0,
            dy: 0.0,
        };
        Ok(())
    })
    .unwrap();
    let y = mtf(&s, 0, MtfMethod::Interp, MtfAxes::Field);
    let free = mtf(&rotated_fields(14.0), 0, MtfMethod::Interp, MtfAxes::Field);
    assert!(
        (y.tan[1] - free.tan[1]).abs() > 1e-3,
        "the obscuration must clip the pupil"
    );
    for i in 1..s.fields.len() {
        let m = mtf(&s, i, MtfMethod::Interp, MtfAxes::Field);
        close(&m.tan, &y.tan, 1e-4, &format!("clipped field {} T", i + 1));
        close(&m.sag, &y.sag, 1e-4, &format!("clipped field {} S", i + 1));
    }
}
