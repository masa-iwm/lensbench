//! 減衰最小二乗: 評価関数が解析の RMS と一致すること、解析的なヤコビアンが差分と合うこと、
//! ずらした系が元の性能まで戻ること。

use crate::common::root;
use lensbench_core::pupil::Apodization;
use lensbench_core::sweep::{rms, RmsOpts, RmsRef};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::Waves;
use lensbench_core::{FieldSel, System};
use lensbench_optim::{dls, DefaultMerit, DlsOpts, JacMode, MtfDir, Problem, Term, TermKind, Var};

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

/// 像面の手前の面（その厚さが焦点合わせの変数。手前の厚さがソルブで決まる系でも、その後ろでずらせる）
fn focus(s: &System) -> usize {
    s.image() - 1
}

fn stop_of(s: &System) -> usize {
    s.surfaces.iter().position(|d| d.stop).unwrap()
}

/// 絞りの後（解析的）と前（開始の光線が変わるので差分）の変数
fn vars(s: &System) -> Vec<String> {
    let st = stop_of(s);
    let f = focus(s);
    let v: Vec<String> = vec![
        format!("{}.c", st + 1),
        format!("{}.t", st + 1),
        format!("{}.dy", st + 1),
        format!("{}.tx", st + 2),
        format!("{}.t", f),
        "1.c".into(),
        "1.t".into(),
    ];
    let mut out: Vec<String> = Vec::new();
    for x in v {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

const SYSTEMS: [&str; 6] = ["A0", "A3", "B0", "B2", "C0", "E0"];

#[test]
fn merit_matches_rms() {
    // 系のアポダイゼーション（ガウス）も解析と同じ重みになる
    let apods = [Apodization::Uniform, Apodization::Gaussian(1.5)];
    for (name, apod) in SYSTEMS.iter().flat_map(|n| apods.map(|a| (*n, a))) {
        let mut s = load(name);
        s.edit_raw(|rx| {
            rx.apodization = apod;
            Ok(())
        })
        .unwrap();
        for reference in [RmsRef::Centroid, RmsRef::Chief] {
            for fi in 0..s.fields.len() {
                for wave in [None, Some(s.primary)] {
                    let o = RmsOpts {
                        rings: 4,
                        reference,
                        ..Default::default()
                    };
                    let want = rms(&s, FieldSel::Index(fi), Waves::all_or(wave), o).unwrap();
                    for (kind, v) in [
                        (
                            TermKind::Spot {
                                field: FieldSel::Index(fi),
                                wave,
                                rings: 4,
                                reference,
                                quad: Default::default(),
                                axis: Default::default(),
                            },
                            want.spot,
                        ),
                        (
                            TermKind::Wavefront {
                                field: FieldSel::Index(fi),
                                wave,
                                rings: 4,
                                reference,
                                quad: Default::default(),
                            },
                            want.wfe,
                        ),
                    ] {
                        let mut p = Problem::single(s.clone());
                        p.terms.push(Term {
                            side: Default::default(),
                            config: 0,
                            weight: 2.0,
                            kind: kind.clone(),
                        });
                        let m = p.merit(&[]).unwrap();
                        assert!(
                            (m - v).abs() <= 1e-12 * v.max(1e-9),
                            "{name} {apod:?} 視野 {fi} {wave:?} {reference:?} {kind:?}: {m} {v}"
                        );
                    }
                }
            }
        }
    }
}

fn check_jac(name: &str, s: &System, kind: TermKind) {
    // 回転対称な系の射出瞳は近軸、そうでなければ実光線で求める（OpticStudio と同じ）ので、評価関数は偏心 0 で
    // わずかに不連続。偏心を少し与えた系で比べる
    let pd = s.param(&format!("{}.dy", stop_of(s) + 1)).unwrap();
    let s = &s.with_params(&[(pd, 0.01)]).unwrap();
    let mut p = Problem::single(s.clone());
    // 列が少なくても解析的に求める（解析的な微分を確かめる）
    p.analytic_min = 0;
    for v in vars(s) {
        p.add_var(&v).unwrap();
    }
    p.terms.push(Term {
        side: Default::default(),
        config: 0,
        weight: 1.0,
        kind: kind.clone(),
    });
    let x = p.x0().unwrap();
    let (ra, ja) = p.jacobian(&x).unwrap();
    p.jac = JacMode::FiniteDiff;
    let (rf, jf) = p.jacobian(&x).unwrap();
    assert_eq!(ra, rf);
    let nv = p.vars.len();
    let gsc = jf.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    for q in 0..nv {
        let col = |j: &[f64]| (0..ra.len()).map(|i| j[i * nv + q]).collect::<Vec<_>>();
        let (a, f) = (col(&ja), col(&jf));
        let sc = f.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let e = a
            .iter()
            .zip(&f)
            .fold(0.0f64, |m, (x, y)| m.max((x - y).abs()));
        assert!(
            e <= 2e-5 * sc + 1e-8 * gsc,
            "{name} {kind:?} 変数 {}: 差 {e:e} / 大きさ {sc:e}",
            vars(s)[q]
        );
    }
    // 前進差分: 残差は同じ、ヤコビアンは中心差分と刻みの 1 次の差
    p.jac = JacMode::Forward;
    let (rw, jw) = p.jacobian(&x).unwrap();
    assert_eq!(ra, rw);
    for q in 0..nv {
        let col = |j: &[f64]| (0..ra.len()).map(|i| j[i * nv + q]).collect::<Vec<_>>();
        let (w, f) = (col(&jw), col(&jf));
        let sc = f.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let e = w
            .iter()
            .zip(&f)
            .fold(0.0f64, |m, (x, y)| m.max((x - y).abs()));
        if std::env::var_os("LB_PRINT_FWD").is_some() {
            eprintln!("fwd {name} 変数 {}: {:.2e}", vars(s)[q], e / (sc + 1e-300));
        }
        assert!(
            e <= 1e-3 * sc + 1e-6 * gsc,
            "前進差分 {name} {kind:?} 変数 {}: 差 {e:e} / 大きさ {sc:e}",
            vars(s)[q]
        );
    }
}

#[test]
fn analytic_jacobian_matches_fd() {
    for name in SYSTEMS {
        let mut s = load(name);
        // 重みがアポダイゼーションで変わっても解析的な微分は合う（一部の系だけ）
        if matches!(name, "A3" | "C0") {
            s.edit_raw(|rx| {
                rx.apodization = Apodization::Gaussian(1.5);
                Ok(())
            })
            .unwrap();
        }
        let last = s.fields.len() - 1;
        for reference in [RmsRef::Centroid, RmsRef::Chief] {
            for wave in [None, Some(0)] {
                let field = FieldSel::Index(last);
                check_jac(
                    name,
                    &s,
                    TermKind::Spot {
                        field,
                        wave,
                        rings: 3,
                        reference,
                        quad: Default::default(),
                        axis: Default::default(),
                    },
                );
                check_jac(
                    name,
                    &s,
                    TermKind::Wavefront {
                        field,
                        wave,
                        rings: 3,
                        reference,
                        quad: Default::default(),
                    },
                );
            }
        }
    }
}

/// 像面の手前の厚さと絞りの後の傾き・偏心をずらした系を、元の評価関数の値以下まで戻す
#[test]
fn dls_recovers_perturbed() {
    for name in ["A0", "A3", "B0"] {
        for kind in [DefaultMerit::Spot, DefaultMerit::Wavefront] {
            let s = load(name);
            let st = stop_of(&s);
            let specs = [
                format!("{}.t", focus(&s)),
                format!("{}.tx", st + 1),
                format!("{}.dy", st + 1),
            ];
            let mut base = Problem::single(s.clone());
            base.add_default(kind, 3, RmsRef::Centroid);
            let m_orig = base.merit(&[]).unwrap();
            // ずらした系から始める
            let ps: Vec<_> = specs.iter().map(|v| s.param(v).unwrap()).collect();
            let d = [0.3, 0.2, 0.05];
            let ch: Vec<_> = ps
                .iter()
                .zip(d)
                .map(|(&p, d)| (p, s.get_param(p).unwrap() + d))
                .collect();
            let s2 = s.with_params(&ch).unwrap();
            let mut p = Problem::single(s2);
            for v in &specs {
                p.add_var(v).unwrap();
            }
            p.add_default(kind, 3, RmsRef::Centroid);
            let r = dls(&p, &DlsOpts::default()).unwrap();
            assert!(
                r.merit <= m_orig * (1.0 + 1e-6),
                "{name} {kind:?}: {} → {}（元 {m_orig}）{:?}",
                r.merit0,
                r.merit,
                r.stop
            );
            assert!(
                r.merit0 > 1.2 * m_orig,
                "{name} {kind:?}: {} {m_orig}",
                r.merit0
            );
            // 返す系の評価関数は結果の値と同じ
            let mut q = Problem::single(r.systems[0].clone());
            q.add_default(kind, 3, RmsRef::Centroid);
            assert!((q.merit(&[]).unwrap() - r.merit).abs() <= 1e-12 * r.merit);
        }
    }
}

/// 境界を守る。複数の構成で、構成ごとの変数と共通の変数
#[test]
fn bounds_and_configs() {
    let s = load("A0");
    let pf = s.param(&format!("{}.t", focus(&s))).unwrap();
    let t0 = s.get_param(pf).unwrap();
    let mut p = Problem::single(s.with_params(&[(pf, t0 + 0.5)]).unwrap());
    p.vars.push(Var::new(pf).bounds(t0 + 0.2, t0 + 1.0));
    p.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    let r = dls(&p, &DlsOpts::default()).unwrap();
    assert!(
        (r.x[0] - (t0 + 0.2)).abs() < 1e-12,
        "{} {}",
        r.x[0],
        t0 + 0.2
    );

    // 2 構成: 構成 1 だけ厚さをずらし、焦点は構成ごと、2 面の曲率は共通
    let a = s.clone();
    let b = s
        .with_params(&[(
            s.param("6.t").unwrap(),
            s.get_param(s.param("6.t").unwrap()).unwrap() + 0.5,
        )])
        .unwrap();
    let mut p = Problem::new(vec![a, b]);
    p.vars.push(Var::new(pf).in_configs(vec![0]));
    p.vars.push(Var::new(pf).in_configs(vec![1]));
    p.add_var("6.c").unwrap();
    p.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    p.check().unwrap();
    let r = dls(&p, &DlsOpts::default()).unwrap();
    assert!(
        r.merit < r.merit0,
        "{} {} {:?} {:?} {:?}",
        r.merit0,
        r.merit,
        r.stop,
        r.x0,
        r.x
    );
    assert_ne!(r.x[0], r.x[1]);
    assert_eq!(r.systems[0].get_param(pf).unwrap(), r.x[0]);
    assert_eq!(r.systems[1].get_param(pf).unwrap(), r.x[1]);
}

/// MTF の項（差分のヤコビアン）で最適化できる
#[test]
fn mtf_term() {
    let s = load("A0");
    let pf = s.param(&format!("{}.t", focus(&s))).unwrap();
    let t0 = s.get_param(pf).unwrap();
    let mut p = Problem::single(s.with_params(&[(pf, t0 + 0.2)]).unwrap());
    p.vars.push(Var::new(pf));
    p.terms.push(Term {
        side: Default::default(),
        config: 0,
        weight: 1.0,
        kind: TermKind::Mtf {
            field: FieldSel::Index(0),
            wave: None,
            n: Some(64),
            freq: 10.0,
            dir: MtfDir::Both,
            target: 1.0,
        },
    });
    let r = dls(&p, &DlsOpts::default()).unwrap();
    assert!(
        r.merit < 0.5 * r.merit0,
        "{} → {} {:?} {:?} {:?} {t0}",
        r.merit0,
        r.merit,
        r.stop,
        r.x,
        r.history
    );
}

/// スレッド数によらず同じ結果
#[test]
fn threads_independent() {
    let s = load("A3");
    let run = |th: Option<usize>| {
        let mut p = Problem::single(s.clone());
        for v in ["8.t", "6.c", "1.c", "6.tx"] {
            p.add_var(v).unwrap();
        }
        p.add_default(DefaultMerit::Wavefront, 3, RmsRef::Centroid);
        lensbench_core::with_threads(th, || dls(&p, &DlsOpts::default())).unwrap()
    };
    let a = run(Some(1));
    let b = run(None);
    assert_eq!(a.x, b.x);
    assert_eq!(a.history, b.history);
}

/// アフォーカル像空間（波面は平面波基準）の系でも、波面の解析的なヤコビアンが差分と合う
#[test]
fn afocal_wavefront_jacobian() {
    // 球面の単レンズ 2 枚のケプラー型望遠鏡（物体は無限遠、視野 0・0.5°、FTYP の 7 番目が 1）
    let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 1\n\
XFLN 0 0\nYFLN 0 0.5\nFWGN 1 1\nWAVM 1 0.55 1\nPWAV 1\nSURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.04\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 73\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.08\n  DISZ 3\n  GLAS ___BLANK 1 0 1.5 0 0 0 0 0 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 20\nSURF 5\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let s = zmx::build(
        &zmx::parse_text(txt).unwrap(),
        &Default::default(),
        &[],
        None,
    )
    .unwrap();
    assert!(s.afocal);
    for base in [
        s.clone(),
        s.with_params(&[(s.param("3.dy").unwrap(), 0.01)]).unwrap(),
    ] {
        for reference in [RmsRef::Centroid, RmsRef::Chief] {
            let mut p = Problem::single(base.clone());
            p.analytic_min = 0;
            for v in ["2.t", "3.c", "3.t", "3.dy", "4.tx"] {
                p.add_var(v).unwrap();
            }
            p.terms.push(Term {
                side: Default::default(),
                config: 0,
                weight: 1.0,
                kind: TermKind::Wavefront {
                    field: FieldSel::Index(1),
                    wave: None,
                    rings: 3,
                    reference,
                    quad: Default::default(),
                },
            });
            // 評価関数は rms と同じ
            let want = rms(
                &base,
                FieldSel::Index(1),
                Waves::All,
                RmsOpts {
                    rings: 3,
                    reference,
                    ..Default::default()
                },
            )
            .unwrap();
            let x = p.x0().unwrap();
            let m = p.merit(&x).unwrap();
            assert!((m - want.wfe).abs() <= 1e-12 * want.wfe, "{m} {}", want.wfe);
            let (_, ja) = p.jacobian(&x).unwrap();
            p.jac = JacMode::FiniteDiff;
            // 波面の値が大きく（約 4 波）、刻み 1e-6 では差分が丸めに負けるので 1e-5。回転対称でない系は射出瞳を
            // 実光線の外挿で求めるので揺れが大きく、差分（相対 1e-3）の精度が 2e-5 程度で頭打ちになる
            p.fd_rel = 1e-5;
            let tol = if base.symmetric() { 2e-5 } else { 5e-5 };
            let (_, jf) = p.jacobian(&x).unwrap();
            let nv = p.vars.len();
            let gsc = jf.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            for q in 0..nv {
                let sc = (0..ja.len() / nv).fold(0.0f64, |m, i| m.max(jf[i * nv + q].abs()));
                let e = (0..ja.len() / nv).fold(0.0f64, |m, i| {
                    m.max((ja[i * nv + q] - jf[i * nv + q]).abs())
                });
                assert!(
                    e <= tol * sc + 1e-8 * gsc,
                    "{reference:?} 変数 {q}: {e:e} / {sc:e}"
                );
            }
        }
    }
}

/// auto でも解析的に求める列が analytic_min より少なければ中心差分（fd と同じ値）
#[test]
fn auto_uses_fd_for_few_columns() {
    let s = load("C0");
    let mut p = Problem::single(s.clone());
    for v in vars(&s) {
        p.add_var(&v).unwrap();
    }
    p.add_default(DefaultMerit::Spot, 4, RmsRef::Centroid);
    let x = p.x0().unwrap();
    assert!(p.vars.len() < p.analytic_min);
    let (ra, ja) = p.jacobian(&x).unwrap();
    p.jac = JacMode::FiniteDiff;
    let (rf, jf) = p.jacobian(&x).unwrap();
    assert_eq!(ra, rf);
    assert_eq!(ja, jf);
    // 0 なら解析的（差分とは丸めの桁で違う）
    p.jac = JacMode::Auto;
    p.analytic_min = 0;
    let (_, jz) = p.jacobian(&x).unwrap();
    assert_ne!(jz, jf);
}

/// 評価関数に効かない変数（前後とも空気の絞り面の曲率・円錐係数）は動かさない。以前は差分の雑音の列で大きく
/// 動き（R = −4.9 mm など）、そのまま系に書かれた
#[test]
fn optimizer_leaves_ineffective_vars() {
    let s = load("A0");
    let stop = s.stop();
    assert!(s.surfaces[stop].mat.is_air() && s.surfaces[stop - 1].mat.is_air());
    let mut p = Problem::single(s.clone());
    p.add_default(DefaultMerit::Wavefront, 3, RmsRef::Centroid);
    for name in [format!("{stop}.c"), format!("{stop}.k"), "2.t".to_string()] {
        p.add_var(&name).unwrap();
    }
    let r = dls(&p, &DlsOpts::default()).unwrap();
    assert_eq!(r.x[0], r.x0[0], "{}.c", stop);
    assert_eq!(r.x[1], r.x0[1], "{}.k", stop);
    assert!(r.inactive.contains(&0) && r.inactive.contains(&1));
    assert!(r.merit <= r.merit0);
}

/// 一歩の解き方: QR（正規方程式を作らない）は Cholesky と同じ解に着く。勾配の判定・減衰を強めて一歩が
/// 小さくなったときの止まり方
#[test]
fn dls_qr_and_stops() {
    use lensbench_optim::Stop;
    let s = load("A0");
    let mut p = Problem::single(s.clone());
    p.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    for v in vars(&s) {
        p.add_var(&v).unwrap();
    }
    let o = DlsOpts {
        max_iter: 30,
        ..Default::default()
    };
    let a = dls(&p, &o).unwrap();
    let b = dls(&p, &DlsOpts { qr: true, ..o }).unwrap();
    assert!(b.merit < b.merit0);
    assert!(
        (a.merit - b.merit).abs() < 1e-6 * a.merit,
        "{} {}",
        a.merit,
        b.merit
    );
    // 余弦は 1 以下なので gtol = 1.1 は最初の反復で止まり、変数は動かない
    let c = dls(&p, &DlsOpts { gtol: 1.1, ..o }).unwrap();
    assert_eq!(c.stop, Stop::SmallGradient);
    assert_eq!(c.x, c.x0);
    assert_eq!(c.stop.as_str(), "small_gradient");
    // 止まる理由が SmallStep なのは、減衰を強めずに求めた一歩が小さいときだけ（強めて小さくなったのは NoProgress）
    let d = dls(&p, &DlsOpts { xtol: 1e300, ..o }).unwrap();
    assert_eq!(d.stop, Stop::SmallStep);
    assert_eq!(d.x, d.x0);
}

/// 有効制約法: 範囲を出る変数を境界に固定して残りを解き直す。境界を守り、評価関数は減る
#[test]
fn dls_active_set_bounds() {
    let s = load("A0");
    let pf = s.param(&format!("{}.t", focus(&s))).unwrap();
    let t0 = s.get_param(pf).unwrap();
    let mut p = Problem::single(s.with_params(&[(pf, t0 + 0.5)]).unwrap());
    p.vars.push(Var::new(pf).bounds(t0 + 0.2, t0 + 1.0));
    let st = stop_of(&s);
    p.add_var(&format!("{}.c", st + 1)).unwrap();
    p.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    for qr in [false, true] {
        let o = DlsOpts {
            active_set: true,
            qr,
            ..Default::default()
        };
        let r = dls(&p, &o).unwrap();
        assert!(r.merit < r.merit0, "{qr}");
        assert!(r.x[0] >= t0 + 0.2 && r.x[0] <= t0 + 1.0, "{qr} {}", r.x[0]);
        // 焦点は下限に張り付く（自由ならもっと手前）
        assert!((r.x[0] - (t0 + 0.2)).abs() < 1e-12, "{qr} {}", r.x[0]);
    }
}

/// 境界にある変数の差分は範囲の内側の片側差分: 下限なら前進、上限なら後退の差分と同じ列
#[test]
fn fd_at_bounds_is_one_sided() {
    let s = load("A0");
    let pf = s.param(&format!("{}.t", focus(&s))).unwrap();
    let t0 = s.get_param(pf).unwrap();
    for (lo, hi, sign) in [(t0, t0 + 1.0, 1.0), (t0 - 1.0, t0, -1.0)] {
        let mut p = Problem::single(s.clone());
        p.jac = JacMode::FiniteDiff;
        p.vars.push(Var::new(pf).bounds(lo, hi));
        p.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
        let (r, j) = p.jacobian(&[t0]).unwrap();
        let h = p.fd_rel * t0.abs().max(1.0);
        let rh = p.residuals(&[t0 + sign * h]).unwrap();
        for i in 0..r.len() {
            let want = sign * (rh[i] - r[i]) / h;
            assert!(
                (j[i] - want).abs() <= 1e-9 * (1.0 + want.abs()),
                "{sign} {i}: {} {want}",
                j[i]
            );
        }
    }
}

/// 縮退した問題: 同じパラメータの変数 2 つは誤り、残差が初めから 0 の問題は動かずに止まる
#[test]
fn degenerate_problems() {
    use lensbench_optim::{ParaxQ, Stop};
    let s = load("A0");
    let mut p = Problem::single(s.clone());
    p.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    p.add_var("2.c").unwrap();
    p.add_var("2.c").unwrap();
    assert!(p.check().is_err());
    assert!(dls(&p, &DlsOpts::default()).is_err());
    // 焦点距離の目標が今の値: 評価関数 0 から減らせない
    let mut p = Problem::single(s.clone());
    p.terms.push(Term {
        side: Default::default(),
        config: 0,
        weight: 1.0,
        kind: TermKind::Paraxial {
            q: ParaxQ::Efl,
            wave: None,
            target: s.paraxial().efl,
        },
    });
    p.add_var("2.c").unwrap();
    let r = dls(&p, &DlsOpts::default()).unwrap();
    assert_eq!(r.merit0, 0.0);
    assert_eq!(r.merit, 0.0);
    assert_eq!(r.x, r.x0);
    assert!(
        matches!(r.stop, Stop::NoProgress | Stop::SmallStep),
        "{:?}",
        r.stop
    );
}

/// 項が使う構成に追跡できない板（複屈折材の Mode 1・3、異常光の道）があれば、どの構成かを言って断る
#[test]
fn a_configuration_with_an_extraordinary_plate_is_named() {
    let dir = root().join("tests/birefringent");
    let b = std::fs::read(dir.join("b1.zmx")).unwrap();
    let u: Vec<u16> = b[2..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let t = String::from_utf16(&u).unwrap();
    assert_eq!(t.matches("  PARM 1 2\r\n").count(), 1);
    let t = t.replacen("  PARM 1 2\r\n", "  PARM 1 1\r\n", 1);
    let f = std::env::temp_dir().join(format!("lb_optim_m1_{}.zmx", std::process::id()));
    let mut out = vec![0xff, 0xfe];
    out.extend(t.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&f, out).unwrap();
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root().join("tests/glass")],
        ..Default::default()
    };
    let m1 = zmx::load_with(&f, &opt).unwrap();
    std::fs::remove_file(&f).ok();
    let a = zmx::load_with(&dir.join("b1.zmx"), &opt).unwrap();
    let mut p = Problem::new(vec![a, m1]);
    p.add_default(DefaultMerit::Spot, 3, RmsRef::Centroid);
    let e = p.check().unwrap_err().to_string();
    assert!(e.contains("configuration 2") && e.contains("Mode 1"), "{e}");
}
