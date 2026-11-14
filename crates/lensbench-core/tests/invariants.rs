//! 物理的に同じ系は同じ結果になる、という不変条件の試験（ビット一致で実装を固定する試験とは別に）。
//! - 厚さ 0 の無視（IGNR）の面をどこに挟んでも近軸量・波面収差は変わらない
//! - 回転対称でない系で視野を軸上 1 点だけにしても、その視野の射出瞳・波面収差は変わらない
//! - 標準面に非球面の係数を残した系は誤り（交点と法線で解釈が食い違わない）
//! - 間隔の分割・恒等の座標ブレークで結果は変わらない、追跡できた ⇔ 有限、スレッド数で変わらない、入力で落ちない

use lensbench_core::analysis;
use lensbench_core::report::system_data;
use lensbench_core::surface::Surface;
use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use std::path::PathBuf;

fn load(name: &str) -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join(format!("tests/refs/{name}/system.zmx")), &opt).unwrap()
}

/// 比べる値: 近軸量と各視野の波面収差の P-V・RMS
fn values(s: &System) -> Vec<(String, f64)> {
    let d = system_data(s);
    let mut v = vec![
        ("effl".into(), d.effl),
        ("epdi".into(), d.epdi),
        ("enpp".into(), d.enpp),
        ("expd".into(), d.expd),
        ("expp".into(), d.expp),
        ("wfno".into(), d.wfno),
        ("pwfn".into(), d.pwfn),
        ("isfn".into(), d.isfn),
        ("pimh".into(), d.pimh),
        ("pmag".into(), d.pmag),
        ("totr".into(), d.totr),
        ("isna".into(), d.isna),
    ];
    for f in 0..s.fields.len() {
        let w = analysis::wavefront(s, FieldSel::Index(f), None, 32, false).unwrap();
        v.push((format!("pv{f}"), w.pv));
        v.push((format!("rms{f}"), w.rms));
    }
    v
}

fn close(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || a == b || (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn ignored_surface_anywhere() {
    for name in ["A0", "C0"] {
        let s0 = load(name);
        let v0 = values(&s0);
        for (k, x) in &v0 {
            assert!(x.is_finite(), "{name} {k} = {x}");
        }
        for at in 1..=s0.image() {
            let mut s = s0.clone();
            // 前の面の後の媒質のまま、厚さ 0 の平面を無視にして挟む
            let p = &s0.surfaces[at - 1];
            let d = Surface {
                mat: if p.mirror {
                    String::new()
                } else {
                    p.mat.clone()
                },
                n: if p.mirror { Vec::new() } else { p.n.clone() },
                ignore: true,
                ..Default::default()
            };
            if p.mirror {
                continue;
            }
            s.insert_surface(at, d).unwrap();
            let v = values(&s);
            for ((k, a), (_, b)) in v0.iter().zip(&v) {
                assert!(close(*a, *b), "{name} 面 {at} に無視の面: {k} {a} → {b}");
            }
        }
    }
}

/// 回転対称でない系（C0）で視野を軸上 1 点だけにしても、射出瞳と軸上の視野の波面収差は同じ（以前は射出瞳が
/// 0/0 で NaN になり、OPD の基準が黙って平面に落ちた）
#[test]
fn on_axis_only_field() {
    let s0 = load("C0");
    assert!(!s0.symmetric());
    let k = (0..s0.fields.len())
        .find(|&i| s0.fields[i].x == 0.0 && s0.fields[i].y == 0.0)
        .expect("C0 に軸上の視野");
    let mut s = s0.clone();
    s.fields = vec![s0.fields[k]];
    s.changed().unwrap();
    let (e0, e1) = (s0.exit_pupil(), s.exit_pupil());
    assert!(e1.is_finite(), "{e1}");
    // C0 の最大視野は 20（正規化の値）、軸上だけの系は 10 の相当で外挿する。外挿の差は 1e-5 mm ほど
    assert!((e0 - e1).abs() < 1e-6 * e0.abs().max(1.0), "{e0} vs {e1}");
    let w0 = analysis::wavefront(&s0, FieldSel::Index(k), None, 32, false).unwrap();
    let w1 = analysis::wavefront(&s, FieldSel::Index(0), None, 32, false).unwrap();
    assert!(
        (w0.pv - w1.pv).abs() < 1e-6 * w0.pv,
        "{} vs {}",
        w0.pv,
        w1.pv
    );
    assert!(
        (w0.rms - w1.rms).abs() < 1e-6 * w0.rms,
        "{} vs {}",
        w0.rms,
        w1.rms
    );
}

/// 非球面の項を使わない型（標準面）に係数が残っていれば誤り
#[test]
fn standard_with_asph_is_error() {
    let mut s = load("A0");
    let j = (1..s.image())
        .find(|&j| {
            s.surfaces[j].typ == lensbench_core::surface::SurfType::Standard
                && s.surfaces[j].c != 0.0
        })
        .unwrap();
    s.surfaces[j].asph = vec![0.0, 2e-5];
    assert!(s.changed().is_err());
}

/// 評価関数に効かない変数（前後とも空気の絞り面の曲率・円錐係数）は動かさない。以前は差分の雑音の列で大きく
/// 動き（R = −4.9 mm など）、そのまま系に書かれた
#[test]
fn optimizer_leaves_ineffective_vars() {
    use lensbench_core::optim::{self, DefaultMerit, DlsOpts, Problem};
    use lensbench_core::sweep::RmsRef;
    let s = load("A0");
    let stop = s.stop();
    assert!(s.surfaces[stop].mat.is_empty() && s.surfaces[stop - 1].mat.is_empty());
    let mut p = Problem::single(s.clone());
    p.add_default(DefaultMerit::Wavefront, 3, RmsRef::Centroid);
    for name in [format!("{stop}.c"), format!("{stop}.k"), "2.t".to_string()] {
        p.add_var(&name).unwrap();
    }
    let r = optim::dls(&p, &DlsOpts::default()).unwrap();
    assert_eq!(r.x[0], r.x0[0], "{}.c", stop);
    assert_eq!(r.x[1], r.x0[1], "{}.k", stop);
    assert!(r.inactive.contains(&0) && r.inactive.contains(&1));
    assert!(r.merit <= r.merit0);
}

/// 面 at の前に、前の面の後の媒質のまま面 d（無視しない）を挟んだ系。前の面が鏡なら None
fn with_surface(s0: &System, at: usize, mut d: Surface) -> Option<System> {
    let p = &s0.surfaces[at - 1];
    if p.mirror {
        return None;
    }
    // 座標ブレークは材料を持たない（前の媒質が続く）
    if d.typ != lensbench_core::surface::SurfType::CoordBrk {
        d.mat = p.mat.clone();
        d.n = p.n.clone();
    }
    let mut s = s0.clone();
    s.insert_surface(at, d).unwrap();
    Some(s)
}

fn near(a: f64, b: f64, rel: f64) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() <= rel * a.abs().max(b.abs()).max(1.0)
}

/// 空気・ガラスの間隔を厚さ 0 でない平面で 2 つに分けても（面の分割）、近軸量・波面収差は丸め誤差まで同じ
#[test]
fn split_gap_anywhere() {
    for name in ["A0", "C0"] {
        let s0 = load(name);
        let v0 = values(&s0);
        for at in 2..s0.image() {
            let t = s0.surfaces[at - 1].t;
            if !(t.is_finite() && t.abs() > 1e-3) || s0.solves.iter().any(|q| q.surf == at - 1) {
                continue;
            }
            let Some(mut s) = with_surface(&s0, at, Surface::default()) else {
                continue;
            };
            s.surfaces[at - 1].t = 0.4 * t;
            s.surfaces[at].t = t - 0.4 * t;
            s.changed().unwrap();
            let v = values(&s);
            for ((k, a), (_, b)) in v0.iter().zip(&v) {
                assert!(
                    near(*a, *b, 1e-9),
                    "{name} 面 {at} の前の間隔を分けた: {k} {a} → {b}"
                );
            }
        }
    }
}

/// 値がすべて 0 の座標ブレーク（恒等）をどこに挟んでも同じ
#[test]
fn identity_coordinate_break_anywhere() {
    use lensbench_core::surface::SurfType;
    for name in ["A0", "C0"] {
        let s0 = load(name);
        let v0 = values(&s0);
        for at in 1..=s0.image() {
            let cb = Surface {
                typ: SurfType::CoordBrk,
                ..Default::default()
            };
            let Some(s) = with_surface(&s0, at, cb) else {
                continue;
            };
            let v = values(&s);
            for ((k, a), (_, b)) in v0.iter().zip(&v) {
                assert!(
                    near(*a, *b, 1e-9),
                    "{name} 面 {at} に恒等の座標ブレーク: {k} {a} → {b}"
                );
            }
        }
    }
}

/// 追跡できた光線（traced）⇔ 位置・方向・光路長が全部有限。瞳の外の届かない光線を含む格子で
#[test]
fn traced_iff_finite() {
    for name in ["A0", "A2", "C0"] {
        let s = load(name);
        let n = 21;
        let (mut px, mut py) = (Vec::new(), Vec::new());
        for i in 0..n {
            for j in 0..n {
                px.push(3.0 * (2.0 * i as f64 / (n - 1) as f64 - 1.0));
                py.push(3.0 * (2.0 * j as f64 / (n - 1) as f64 - 1.0));
            }
        }
        for f in 0..s.fields.len() {
            let t = s
                .try_trace(FieldSel::Index(f), &px, &py, None, None, false)
                .unwrap();
            for (i, r) in t.rays.iter().enumerate() {
                let fin = r.xyz.iter().chain(&r.lmn).all(|v| v.is_finite()) && r.opl.is_finite();
                assert_eq!(t.traced(i), fin, "{name} 視野 {f} 光線 {i}: {r:?}");
            }
        }
    }
}

/// `--threads 1`（with_threads(Some(1))）の中の既定（None）は逐次のまま、結果はスレッド数によらず同じビット
#[test]
fn threads_default_inherits_sequential() {
    use lensbench_core::par::{current_threads, with_threads};
    assert_eq!(
        with_threads(Some(1), || with_threads(None, current_threads)),
        1
    );
    let s = load("A2");
    let run = || {
        analysis::wavefront(&s, FieldSel::Index(1), None, 64, false)
            .unwrap()
            .rms
    };
    let a = with_threads(Some(1), run);
    let b = with_threads(Some(4), run);
    assert_eq!(a.to_bits(), b.to_bits());
}

/// 入力で落ちない: 範囲外の視野・波長の番号は誤り、上限を超えた Zernike の項・壊れた Grid Sag の点の数も落ちない
#[test]
fn bad_inputs_are_errors() {
    use lensbench_core::freeform::{GridSag, ZernSag};
    let s = load("A0");
    assert!(s
        .try_trace(FieldSel::Index(99), &[0.0], &[0.0], None, None, false)
        .is_err());
    assert!(s
        .try_trace(FieldSel::Index(0), &[0.0], &[0.0], Some(99), None, false)
        .is_err());
    let z = ZernSag::new(false, 10.0, vec![1e-6; 600], true);
    assert!(z.eval(1.0, 2.0).0.is_finite());
    for head in ["-3 4 0.1 0.1 0", "1e9 1e9 0.1 0.1 0", "2.5 2 0.1 0.1 0"] {
        assert!(
            GridSag::from_dat(&format!("{head}\n0 0 0 0 0\n")).is_err(),
            "{head}"
        );
    }
}

/// 多色の MTF で重みが全部 0 なら、NaN でなく誤り（ANA-23。psf・幾何 MTF と同じ）
#[test]
fn mtf_with_all_zero_weights_is_an_error() {
    let mut s = load("A0");
    assert!(s.wavelengths.len() > 1);
    s.weights.iter_mut().for_each(|w| *w = 0.0);
    let r = analysis::mtf(
        &s,
        FieldSel::Index(0),
        lensbench_core::Waves::All,
        None,
        &[10.0],
    );
    let e = r.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("重みのある波長がない"), "{e}");
}
