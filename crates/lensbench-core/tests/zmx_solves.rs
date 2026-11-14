//! 読込での厚さのソルブの計算し直し: MAZH（次の面での近軸マージナル光線の高さ）
use lensbench_core::system::System;
use lensbench_core::zmx;

const HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 0\nYFLN 0 5\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

fn build(txt: &str) -> lensbench_core::Result<System> {
    let raw = zmx::parse_text(txt)?;
    zmx::build(
        &raw,
        &zmx::LoadOptions {
            cfg: 1,
            ..Default::default()
        },
        &[],
        None,
    )
}

/// 両凸の単玉（絞りは面 1）→ 空気 → 平面の窓 → 像面。`back` は面 2 の行（DISZ とソルブ）、`win` は面 3 の行
fn lens(back: &str, win: &str) -> String {
    format!(
        "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.01\n{back}\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 2\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n{win}\
SURF 5\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    )
}

fn marg_at(s: &System, j: usize) -> f64 {
    s.paraxial().marginal.0[j]
}

#[test]
fn stale_disz_is_resolved_to_the_paraxial_image() {
    // 像面の手前の厚さを、保存値 50（古い値）から近軸像面に解き直す
    let s = build(&lens("  DISZ 30\n", "  DISZ 50\n  MAZH 0 0\n")).unwrap();
    let t = s.surfaces[4].t;
    assert!((t - 50.0).abs() > 1.0, "解き直していない: {t}");
    assert!(marg_at(&s, 5).abs() < 1e-12, "{}", marg_at(&s, 5));
    // 解いた値を DISZ に書いた系と同じ（ソルブなし）
    let s2 = build(&lens("  DISZ 30\n", &format!("  DISZ {t:e}\n"))).unwrap();
    assert_eq!(s2.surfaces[4].t, t);
}

#[test]
fn height_target_and_chained_solves() {
    // 面 2 で窓の上の高さ 2、面 4 で像面の高さ 0。後ろのソルブは前のソルブの結果を使う
    let s = build(&lens("  DISZ 1\n  MAZH 2 0\n", "  DISZ 1\n  MAZH 0 0\n")).unwrap();
    assert!((marg_at(&s, 3) - 2.0).abs() < 1e-12, "{}", marg_at(&s, 3));
    assert!(marg_at(&s, 5).abs() < 1e-12, "{}", marg_at(&s, 5));
}

#[test]
fn pickup_follows_a_solved_thickness() {
    // 窓の厚さ（面 3）は面 2 の厚さの 0.1 倍のピックアップ
    let txt = lens("  DISZ 1\n  MAZH 2 0\n", "  DISZ 1\n  MAZH 0 0\n")
        .replace("  DISZ 2\n  GLAS", "  DISZ 2\n  PZUP 2 0.1 0\n  GLAS");
    let s = build(&txt).unwrap();
    assert!((s.surfaces[3].t - 0.1 * s.surfaces[2].t).abs() < 1e-12);
    assert!((marg_at(&s, 3) - 2.0).abs() < 1e-12);
    assert!(marg_at(&s, 5).abs() < 1e-12);
}

#[test]
fn solve_before_the_stop_converges() {
    // 絞りを窓（面 3）に移し、有限物体（物体距離 200）で入射瞳が厚さで動く系
    let txt = lens("  DISZ 30\n  MAZH 3 0\n", "  DISZ 40\n  MAZH 0 0\n")
        .replace("  DISZ INFINITY\n", "  DISZ 200\n")
        .replace("SURF 1\n  STOP\n", "SURF 1\n")
        .replace("SURF 3\n", "SURF 3\n  STOP\n")
        .replace("FTYP 0 0 2", "FTYP 1 0 2");
    let s = build(&txt).unwrap();
    assert!((marg_at(&s, 3) - 3.0).abs() < 1e-12, "{}", marg_at(&s, 3));
    assert!(marg_at(&s, 5).abs() < 1e-12);
}

/// 瞳の帯 ≠ 0 は実光線（軸上、瞳 (0, 帯)）の高さ
#[test]
fn real_ray_zone() {
    let s = build(&lens("  DISZ 30\n", "  DISZ 50\n  MAZH 0 0.7\n")).unwrap();
    let t = s
        .try_trace(
            lensbench_core::system::FieldSel::Norm(0.0, 0.0),
            &[0.0],
            &[0.7],
            None,
            None,
            false,
        )
        .unwrap();
    assert!(t.rays[0].xyz[1].abs() < 1e-12, "{}", t.rays[0].xyz[1]);
}

fn opd_at(s: &System, zone: f64) -> f64 {
    lensbench_core::analysis::opd(
        s,
        lensbench_core::system::FieldSel::Norm(0.0, 0.0),
        &[0.0],
        &[zone],
        None,
        true,
    )
    .unwrap()
    .opd[0]
}

/// OPD の厚さのソルブ（OPDZ OPD 帯）: 像面の OPD（軸上・主波長・瞳 (0, 帯)）が目標になる。書き出して
/// 読み戻すと同じ厚さ（OPD の丸めの揺れの中）、パラメータを変えると解き直す
#[test]
fn opd_solve() {
    let s = build(&lens("  DISZ 30\n", "  DISZ 50\n  OPDZ 0.25 0.7\n")).unwrap();
    assert!((s.surfaces[4].t - 50.0).abs() > 1.0);
    assert!((opd_at(&s, 0.7) - 0.25).abs() < 1e-9, "{}", opd_at(&s, 0.7));
    let w = lensbench_core::zmx_write::write(&s).unwrap();
    assert!(w.contains("  OPDZ 0.25 0.7"), "{w}");
    let r = build(&w).unwrap();
    assert!((r.surfaces[4].t - s.surfaces[4].t).abs() < 1e-9);
    let a = s.with_params(&[(s.param("1.c").unwrap(), 0.021)]).unwrap();
    assert!((a.surfaces[4].t - s.surfaces[4].t).abs() > 1e-3);
    assert!((opd_at(&a, 0.7) - 0.25).abs() < 1e-9);
    // 目標 0 の帯 0.7 は近軸の焦点（MAZH 0 0）と違う（球面収差の分）
    let z = build(&lens("  DISZ 30\n", "  DISZ 50\n  OPDZ 0 0.7\n")).unwrap();
    let m = build(&lens("  DISZ 30\n", "  DISZ 50\n  MAZH 0 0\n")).unwrap();
    assert!(opd_at(&z, 0.7).abs() < 1e-9);
    assert!((z.surfaces[4].t - m.surfaces[4].t).abs() > 1e-4);
}

/// 後ろに焦点のソルブ（MAZH）がある面の OPD のソルブは、焦点を解き直しながら解く（両方を同時に満たす）。
/// 玉の厚さ（面 1）で、近軸の焦点での帯 0.7 の OPD（球面収差）を決める
#[test]
fn opd_solve_with_a_later_focus_solve() {
    let focus = lens("  DISZ 30\n", "  DISZ 50\n  MAZH 0 0\n");
    let w0 = opd_at(&build(&focus).unwrap(), 0.7);
    let target = w0 - 0.008;
    for start in ["4", "7"] {
        let txt = focus.replace(
            "  DISZ 4\n  GLAS",
            &format!("  DISZ {start}\n  OPDZ {target} 0.7\n  GLAS"),
        );
        let s = build(&txt).unwrap();
        assert!((s.surfaces[1].t - 4.0).abs() > 1e-3, "{}", s.surfaces[1].t);
        assert!(marg_at(&s, 5).abs() < 1e-12, "{}", marg_at(&s, 5));
        assert!(
            (opd_at(&s, 0.7) - target).abs() < 1e-9,
            "{}",
            opd_at(&s, 0.7)
        );
    }
}

/// パラメータを変えるとソルブを解き直す。書き出して読み戻した系（MAZH の行も書く）と同じ
#[test]
fn set_param_resolves_and_round_trips() {
    let txt = lens("  DISZ 1\n  MAZH 2 0\n", "  DISZ 1\n  MAZH 0 0\n")
        .replace("  DISZ 2\n  GLAS", "  DISZ 2\n  PZUP 2 0.1 0\n  GLAS");
    let s = build(&txt).unwrap();
    assert_eq!(s.solves.len(), 2);
    let p = s.param("1.c").unwrap();
    let a = s.with_params(&[(p, 0.025)]).unwrap();
    assert!((a.surfaces[2].t - s.surfaces[2].t).abs() > 1e-3);
    assert!((marg_at(&a, 3) - 2.0).abs() < 1e-12, "{}", marg_at(&a, 3));
    assert!(marg_at(&a, 5).abs() < 1e-12);
    assert_eq!(a.surfaces[3].t, 0.1 * a.surfaces[2].t);
    let w = lensbench_core::zmx_write::write(&a).unwrap();
    assert_eq!(w.matches("MAZH").count(), 2);
    let r = build(&w).unwrap();
    for j in 0..a.surfaces.len() {
        assert_eq!(a.surfaces[j].t, r.surfaces[j].t, "面 {j}");
    }
    // 関係ないパラメータ（傾き）では厚さは変わらない
    let b = s.with_params(&[(s.param("1.tx").unwrap(), 0.3)]).unwrap();
    for j in 0..s.surfaces.len() {
        assert_eq!(b.surfaces[j].t, s.surfaces[j].t, "面 {j}");
    }
    // ソルブで決まる厚さは変えられない
    let e = s
        .with_params(&[(s.param("4.t").unwrap(), 3.0)])
        .unwrap_err();
    assert!(e.to_string().contains("ソルブ"), "{e}");
}

fn build_cfg(txt: &str, cfg: usize) -> lensbench_core::Result<System> {
    let raw = zmx::parse_text(txt)?;
    zmx::build(
        &raw,
        &zmx::LoadOptions {
            cfg,
            ..Default::default()
        },
        &[],
        None,
    )
}

/// 位置ソルブ（.zmx の `TOLE 基準の面 長さ`）: 面 2 から面 4 までの厚さの和を 40 にする（OpticStudio の書式は実測）
#[test]
fn position_solve_keeps_the_length() {
    let txt = lens("  DISZ 30\n", "  DISZ 50\n  TOLE 2 40\n");
    let s = build(&txt).unwrap();
    assert_eq!(s.solves.len(), 1);
    assert_eq!(s.surfaces[4].t, 8.0);
    // 前の厚さを変えると追従する。書き出して読み戻しても同じ
    let a = s.with_params(&[(s.param("2.t").unwrap(), 25.0)]).unwrap();
    assert_eq!(a.surfaces[4].t, 13.0);
    let w = lensbench_core::zmx_write::write(&a).unwrap();
    assert!(w.contains("  TOLE 2 40"), "{w}");
    let r = build(&w).unwrap();
    for j in 0..a.surfaces.len() {
        assert_eq!(a.surfaces[j].t, r.surfaces[j].t, "面 {j}");
    }
    let e = s
        .with_params(&[(s.param("4.t").unwrap(), 3.0)])
        .unwrap_err();
    assert!(e.to_string().contains("ソルブ"), "{e}");
    // 多重構成で前の厚さが変わる（THIC）ときも、コンフィグごとに解き直す
    let mc = format!(
        "{txt}MNUM 2 1\nTHIC 2 1 30 0 0 0 1 1 1 0 0 \"\" 0\nTHIC 2 2 21.5 0 0 0 1 1 1 0 0 \"\" 0\n"
    );
    assert_eq!(build_cfg(&mc, 1).unwrap().surfaces[4].t, 8.0);
    assert_eq!(build_cfg(&mc, 2).unwrap().surfaces[4].t, 16.5);
}

/// 位置ソルブと近軸マージナル光線の高さのソルブを一緒に: 面 2 は窓の上の高さ 2、面 4 は面 2 からの長さ 40
#[test]
fn position_solve_after_a_height_solve() {
    let s = build(&lens("  DISZ 1\n  MAZH 2 0\n", "  DISZ 50\n  TOLE 2 40\n")).unwrap();
    assert!((marg_at(&s, 3) - 2.0).abs() < 1e-12, "{}", marg_at(&s, 3));
    let sum: f64 = s.surfaces[2..=4].iter().map(|d| d.t).sum();
    assert!((sum - 40.0).abs() < 1e-12, "{sum}");
    let e = build(&lens("  DISZ 30\n", "  DISZ 50\n  TOLE 9 40\n")).unwrap_err();
    assert!(e.to_string().contains("基準の面"), "{e}");
}

/// 基準の面が後ろの位置ソルブ: 面 2 から面 4 までの距離 t2 + t3 を 35 にする（ZMF のレンズにある書き方。
/// 保存された DISZ と同じ値になる）
#[test]
fn position_solve_with_the_reference_after() {
    let s = build(&lens("  DISZ 30\n  TOLE 4 35\n", "  DISZ 50\n")).unwrap();
    assert_eq!(s.surfaces[2].t, 33.0);
    let a = s.with_params(&[(s.param("3.t").unwrap(), 4.5)]).unwrap();
    assert_eq!(a.surfaces[2].t, 30.5);
    let r = build(&lensbench_core::zmx_write::write(&a).unwrap()).unwrap();
    assert_eq!(r.surfaces[2].t, 30.5);
}
