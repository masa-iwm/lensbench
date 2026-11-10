//! 評価関数の項の追加: 光線・OPD・近軸量・Zernike・結合効率・利用者の項の値と、解析的なヤコビアン
//! （光線・OPD）が差分と合うこと、利用者の項の差分がまとめて 1 回で評価されること。

use lensbench_core::analysis::{opd, zernike, ZernKind};
use lensbench_core::fiber::FiberOpts;
use lensbench_core::optim::{
    dls, DlsOpts, JacMode, ParaxQ, Problem, RayQ, Term, TermKind, UserTerm, UserTermRef,
};
use lensbench_core::param::Param;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::{FieldSel, Result, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

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

fn stop_of(s: &System) -> usize {
    s.surfaces.iter().position(|d| d.stop).unwrap()
}

/// 絞りの後（解析的）と前（開始の光線が変わるので差分）の変数
fn vars(s: &System) -> Vec<String> {
    let st = stop_of(s);
    let f = s.image() - 1;
    let mut out: Vec<String> = Vec::new();
    for x in [
        format!("{}.c", st + 1),
        format!("{}.t", st + 1),
        format!("{}.dy", st + 1),
        format!("{}.tx", st + 2),
        format!("{}.t", f),
        "1.c".into(),
        "1.t".into(),
    ] {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

const SYSTEMS: [&str; 6] = ["A0", "A3", "B0", "B2", "C0", "E0"];

fn one(s: &System, kind: TermKind) -> Problem {
    let mut p = Problem::single(s.clone());
    // 列が少なくても解析的に求める（解析的な微分を確かめる）
    p.analytic_min = 0;
    p.terms.push(Term {
        side: Default::default(),
        config: 0,
        weight: 1.0,
        kind,
    });
    p
}

/// 解析的なヤコビアンが差分と合い、少なくとも 1 列は解析的に求めていること（差分の刻みを変えても変わらない列）
fn check_jac(name: &str, s: &System, kind: TermKind) {
    let pd = s.param(&format!("{}.dy", stop_of(s) + 1)).unwrap();
    let s = &s.with_params(&[(pd, 0.01)]).unwrap();
    let mut p = one(s, kind.clone());
    for v in vars(s) {
        p.add_var(&v).unwrap();
    }
    let x = p.x0().unwrap();
    let (ra, ja) = p.jacobian(&x).unwrap();
    p.fd_rel = 1e-4;
    let (_, jb) = p.jacobian(&x).unwrap();
    p.fd_rel = 1e-6;
    p.jac = JacMode::FiniteDiff;
    let (rf, jf) = p.jacobian(&x).unwrap();
    assert_eq!(ra, rf);
    let nv = p.vars.len();
    let gsc = jf.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let mut analytic = 0;
    for q in 0..nv {
        let col = |j: &[f64]| (0..ra.len()).map(|i| j[i * nv + q]).collect::<Vec<_>>();
        let (a, b, f) = (col(&ja), col(&jb), col(&jf));
        if a == b && a.iter().any(|v| *v != 0.0) {
            analytic += 1;
        }
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
    // E0 は絞りが像面の 2 面前で、絞りの後の変数が方向余弦に効かない
    let dir_e0 = name == "E0"
        && matches!(
            kind,
            TermKind::Ray {
                q: RayQ::L | RayQ::M | RayQ::N,
                ..
            }
        );
    assert!(analytic > 0 || dir_e0, "{name} {kind:?}: 解析的な列がない");
}

#[test]
fn ray_and_opd_values() {
    for name in SYSTEMS {
        let s = load(name);
        let fi = s.fields.len() - 1;
        let field = FieldSel::Index(fi);
        let w = s.wavelengths.len() - 1;
        for &(px, py) in &[(0.0, 0.0), (0.3, -0.7), (0.0, 1.0)] {
            // 像面と途中の面
            for surf in [None, Some(stop_of(&s) + 1)] {
                let last = surf.unwrap_or(s.image());
                let t = s
                    .try_trace(field, &[px], &[py], Some(w), Some(last), false)
                    .unwrap();
                let r = &t.rays[0];
                for (q, v) in [
                    (RayQ::X, r.xyz[0]),
                    (RayQ::Y, r.xyz[1]),
                    (RayQ::Z, r.xyz[2]),
                    (RayQ::L, r.lmn[0]),
                    (RayQ::M, r.lmn[1]),
                    (RayQ::N, r.lmn[2]),
                ] {
                    let p = one(
                        &s,
                        TermKind::Ray {
                            field,
                            wave: Some(w),
                            px,
                            py,
                            surf,
                            q,
                            target: 0.0,
                        },
                    );
                    assert_eq!(p.term_values(&[]).unwrap()[0], v, "{name} {q:?}");
                }
            }
            let o = opd(&s, field, &[px], &[py], Some(w), true).unwrap();
            let p = one(
                &s,
                TermKind::Opd {
                    field,
                    wave: Some(w),
                    px,
                    py,
                    target: 0.0,
                },
            );
            let v = p.term_values(&[]).unwrap()[0];
            assert!((v - o.opd[0]).abs() <= 1e-12, "{name} OPD {v} {}", o.opd[0]);
        }
    }
}

#[test]
fn ray_and_opd_jacobian() {
    for name in SYSTEMS {
        let s = load(name);
        let field = FieldSel::Index(s.fields.len() - 1);
        for q in [RayQ::X, RayQ::Y, RayQ::L, RayQ::M, RayQ::N] {
            for surf in [None, Some(s.image() - 1)] {
                check_jac(
                    name,
                    &s,
                    TermKind::Ray {
                        field,
                        wave: None,
                        px: 0.3,
                        py: 0.8,
                        surf,
                        q,
                        target: 0.0,
                    },
                );
            }
        }
        for wave in [None, Some(0)] {
            check_jac(
                name,
                &s,
                TermKind::Opd {
                    field,
                    wave,
                    px: 0.3,
                    py: 0.8,
                    target: 0.0,
                },
            );
        }
    }
}

#[test]
fn paraxial_zernike_coupling_values() {
    let s = load("A0");
    let d = lensbench_core::report::system_data(&s);
    for (q, v) in [
        (ParaxQ::Efl, d.effl),
        (ParaxQ::Pmag, d.pmag),
        (ParaxQ::Enpp, d.enpp),
        (ParaxQ::Epd, d.epdi),
        (ParaxQ::Expp, d.expp),
        (ParaxQ::Expd, d.expd),
        (ParaxQ::Wfno, d.wfno),
        (ParaxQ::Pwfn, d.pwfn),
        (ParaxQ::Isfn, d.isfn),
        (ParaxQ::Pimh, d.pimh),
        (ParaxQ::Totr, d.totr),
        (ParaxQ::Obna, d.obna),
        (ParaxQ::Isna, d.isna),
    ] {
        assert_eq!(ParaxQ::parse(q.name()).unwrap(), q);
        let p = one(&s, TermKind::Paraxial { q, target: 0.0 });
        let got = p.term_values(&[]).unwrap()[0];
        assert!(got == v || (got.is_nan() && v.is_nan()), "{q:?} {got} {v}");
    }
    let field = FieldSel::Index(s.fields.len() - 1);
    let z = zernike(&s, field, None, ZernKind::Fringe, 9, 32, false).unwrap();
    let p = one(
        &s,
        TermKind::Zernike {
            field,
            wave: None,
            kind: ZernKind::Fringe,
            term: 9,
            n: 32,
            target: 0.0,
        },
    );
    assert_eq!(p.term_values(&[]).unwrap()[0], z.coef[8]);
}

#[test]
fn coupling_term() {
    // 有限物体の系
    let s = ["A0", "A3", "B0", "B2", "C0", "E0"]
        .iter()
        .map(|n| load(n))
        .find(|s| !s.infinite());
    let s = s.expect("有限物体の系がない");
    let o = FiberOpts {
        grid: 31,
        ..Default::default()
    };
    let c = lensbench_core::fiber::fiber_coupling(&s, s.primary, &o, None).unwrap();
    let p = one(
        &s,
        TermKind::Coupling {
            wave: None,
            opts: o,
            target: 1.0,
        },
    );
    assert_eq!(p.residuals(&[]).unwrap()[0], c.eta - 1.0);
}

/// 像面の y と、評価した系の数を数える利用者の項
struct UserY {
    calls: AtomicUsize,
    systems: AtomicUsize,
    analytic: bool,
}

impl UserTerm for UserY {
    fn name(&self) -> &str {
        "y"
    }
    fn rows(&self) -> usize {
        2
    }
    fn eval(&self, systems: &[&System]) -> Result<Vec<Vec<f64>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.systems.fetch_add(systems.len(), Ordering::SeqCst);
        systems
            .iter()
            .map(|s| {
                let t = s.try_trace(
                    FieldSel::Index(0),
                    &[0.0, 0.0],
                    &[0.5, -0.5],
                    None,
                    None,
                    false,
                )?;
                Ok(vec![t.rays[0].xyz[1], t.rays[1].xyz[1]])
            })
            .collect()
    }
    fn grad(&self, s: &System, ps: &[Param]) -> Result<Option<Vec<f64>>> {
        if !self.analytic {
            return Ok(None);
        }
        let t = s.try_trace(
            FieldSel::Index(0),
            &[0.0, 0.0],
            &[0.5, -0.5],
            None,
            None,
            true,
        )?;
        let g = s.land_grad(&t, s.primary, ps, None)?;
        let np = ps.len();
        let mut j = vec![0.0; 2 * np];
        for i in 0..2 {
            for q in 0..np {
                j[i * np + q] = g[(i * np + q) * 4 + 1];
            }
        }
        Ok(Some(j))
    }
}

#[test]
fn user_term_batches_fd() {
    let s = load("A0");
    let st = stop_of(&s);
    let specs = [format!("{}.c", st + 1), format!("{}.t", s.image() - 1)];
    for analytic in [false, true] {
        let u = Arc::new(UserY {
            calls: AtomicUsize::new(0),
            systems: AtomicUsize::new(0),
            analytic,
        });
        let mut p = one(&s, TermKind::User(UserTermRef(u.clone())));
        for v in &specs {
            p.add_var(v).unwrap();
        }
        let x = p.x0().unwrap();
        let (r, j) = p.jacobian(&x).unwrap();
        // 中心の系 1 回 + 差分の系 2×2 を 1 回で
        if analytic {
            assert_eq!(u.calls.load(Ordering::SeqCst), 1);
        } else {
            assert_eq!(u.calls.load(Ordering::SeqCst), 2);
            assert_eq!(u.systems.load(Ordering::SeqCst), 5);
        }
        // 同じ量を光線の項で
        let mut q = Problem::single(s.clone());
        q.analytic_min = 0;
        for py in [0.5, -0.5] {
            q.terms.push(Term {
                side: Default::default(),
                config: 0,
                weight: 1.0,
                kind: TermKind::Ray {
                    field: FieldSel::Index(0),
                    wave: None,
                    px: 0.0,
                    py,
                    surf: None,
                    q: RayQ::Y,
                    target: 0.0,
                },
            });
        }
        for v in &specs {
            q.add_var(v).unwrap();
        }
        let (r2, j2) = q.jacobian(&x).unwrap();
        assert_eq!(r, r2);
        for (a, b) in j.iter().zip(&j2) {
            assert!((a - b).abs() <= 1e-6 * b.abs().max(1e-3), "{a} {b}");
        }
    }
}

/// 光線の目標で焦点合わせ: 縁の光線の y を主光線の y に（ピント位置の変数 1 つ）
#[test]
fn dls_with_ray_targets() {
    let s = load("A0");
    let f = s.param(&format!("{}.t", s.image() - 1)).unwrap();
    let s0 = s
        .with_params(&[(f, s.get_param(f).unwrap() + 0.5)])
        .unwrap();
    let mut p = Problem::single(s0);
    p.add_var(&format!("{}.t", s.image() - 1)).unwrap();
    for (px, py) in [(0.0, 0.7), (0.7, 0.0)] {
        p.terms.push(Term {
            side: Default::default(),
            config: 0,
            weight: 1.0,
            kind: TermKind::Ray {
                field: FieldSel::Index(0),
                wave: None,
                px,
                py,
                surf: None,
                q: if px == 0.0 { RayQ::Y } else { RayQ::X },
                target: 0.0,
            },
        });
    }
    let r = dls(&p, &DlsOpts::default()).unwrap();
    assert!(
        r.merit < 1e-9 * r.merit0.max(1e-3),
        "{} → {}",
        r.merit0,
        r.merit
    );
}
