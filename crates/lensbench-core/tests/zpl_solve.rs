//! ZPL Macro Solve: .zmx の CURV … 12 … "名前" のマクロを探して走らせ、曲率を解き直す。
//! マクロがない・対応していない命令を使うときは、保存された値で固定して警告を残す。

use std::path::{Path, PathBuf};

use lensbench_core::param::{Kind, Param, SolveKind};
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::zmx_write;

const HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 3 1 0 0 0\n\
XFLN 0 0 0\nYFLN 0 14 20\nFWGN 1 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

/// 単レンズ（面 1・2、絞りは面 1）。面 2 の曲率はマクロ name のソルブで、保存された値は saved
fn singlet(name: &str, saved: f64) -> String {
    format!(
        "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5168 64.17 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV {saved:e} 12 0 0 0 \"{name}\"\n  DISZ 45\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    )
}

/// 一時フォルダにマクロ（名前 → 文）を書く
fn macro_dir(tag: &str, macros: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lb_zpl_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    for (n, src) in macros {
        std::fs::write(d.join(format!("{n}.ZPL")), src).unwrap();
    }
    d
}

fn load(txt: &str, dir: &Path) -> lensbench_core::System {
    let raw = zmx::parse_text(txt).unwrap();
    let opt = LoadOptions {
        cfg: 1,
        macro_dirs: vec![dir.to_path_buf()],
        ..Default::default()
    };
    zmx::build(&raw, &opt, &[], None).unwrap()
}

/// 面 2 の曲率 = −面 1 の曲率 × 倍率（倍率は FOR で数える）。保存された値が古くても読込で解き直す
const MIRROR: &str = "! 面 2 を面 1 と対称に\n\
k = 0\n\
FOR i, 1, 3, 1\n  k = k + 1\nNEXT\n\
s = SOSO(0)\n\
IF (k == 3)\n  SOLVERETURN -CURV(s - 1) * k / 3\nELSE\n  SOLVERETURN 0\nENDIF\n";

#[test]
fn macro_curvature_solve_recomputes() {
    let d = macro_dir("mirror", &[("lbMirror", MIRROR)]);
    let s = load(&singlet("LBMIRROR", -0.0123), &d);
    assert!(s.macro_warnings().is_empty(), "{:?}", s.macro_warnings());
    assert_eq!(s.surfaces[2].c, -0.02);
    assert!(s
        .solves
        .iter()
        .any(|q| q.surf == 2 && matches!(q.kind, SolveKind::MacroCurv(_))));
    // 変数を変えると解き直す
    let s2 = s.with_params(&[(Param::new(1, Kind::Curv), 0.03)]).unwrap();
    assert_eq!(s2.surfaces[2].c, -0.03);
    // 書き出して読み戻しても同じ
    let txt = zmx_write::write(&s2).unwrap();
    assert!(txt.contains("12 0 0 0 \"LBMIRROR\""), "{txt}");
    let s3 = load(&txt, &d);
    assert_eq!(s3.surfaces[2].c, -0.03);
    assert!(s3.macro_warnings().is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn missing_or_unsupported_macro_keeps_saved_value() {
    let d = macro_dir(
        "unsup",
        &[("LBGSD", "GETSYSTEMDATA 1\nSOLVERETURN VEC1(2)\n")],
    );
    for (name, why) in [("NOSUCH", "見つからない"), ("LBGSD", "GETSYSTEMDATA")] {
        let s = load(&singlet(name, -0.0123), &d);
        assert_eq!(s.surfaces[2].c, -0.0123, "{name}");
        assert!(s.solves.iter().all(|q| q.surf != 2), "{name}");
        let w = s.macro_warnings();
        assert_eq!(w.len(), 1, "{name}: {w:?}");
        assert!(w[0].contains(name) && w[0].contains(why), "{name}: {w:?}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// 走らせている途中のエラーは OpticStudio 26.1.1 と同じ: それまでの SOLVERETURN の値、なければ 0。
/// 0 で割ると 0。SOLVERETURN しないマクロは値を変えない
#[test]
fn macro_errors_like_opticstudio() {
    let d = macro_dir(
        "err",
        &[
            ("LBDIV", "SOLVERETURN 0.01 + 1 / 0\n"),
            ("LBMID", "SOLVERETURN -0.03\nx = nosuchvar\nSOLVERETURN 1\n"),
            ("LBUND", "x = nosuchvar\nSOLVERETURN 1\n"),
            ("LBNONE", "x = 1\n"),
        ],
    );
    for (name, c) in [
        ("LBDIV", 0.01),
        ("LBMID", -0.03),
        ("LBUND", 0.0),
        ("LBNONE", -0.0123),
    ] {
        let s = load(&singlet(name, -0.0123), &d);
        assert!(
            s.macro_warnings().is_empty(),
            "{name}: {:?}",
            s.macro_warnings()
        );
        assert_eq!(s.surfaces[2].c, c, "{name}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// パラメータのマクロ（PZPL 番号 "名前"）: Jones 面の A の実部・B の実部を面 1 の厚さから決める
#[test]
fn macro_parameter_solve_on_jones_surface() {
    let d = macro_dir(
        "parm",
        &[
            ("LB_JA", "t = THIC(1) * 0.1\nSOLVERETURN COSI(t)\n"),
            ("LB_JB", "t = THIC(1) * 0.1\nSOLVERETURN -SINE(t)\n"),
        ],
    );
    let txt = singlet("X", -0.02)
        .replace("  CURV -2e-2 12 0 0 0 \"X\"\n", "  CURV -2e-2\n")
        .replace(
            "SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
            "SURF 3\n  TYPE JONESMAT\n  CURV 0.0\n  PARM 1 0.5\n  PARM 3 0\n  PARM 7 1\n  DISZ 0\n  PZPL 1 \"LB_JA\"\n  PZPL 3 \"LB_JB\"\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n",
        );
    assert!(txt.contains("PZPL 3"), "{txt}");
    let s = load(&txt, &d);
    assert!(s.macro_warnings().is_empty(), "{:?}", s.macro_warnings());
    assert_eq!(s.macro_parms.len(), 2);
    assert_eq!(s.surfaces[3].jones[0], 0.5f64.cos());
    assert_eq!(s.surfaces[3].jones[2], -(0.5f64.sin()));
    let s = s.with_params(&[(Param::new(1, Kind::Thick), 7.0)]).unwrap();
    assert_eq!(s.surfaces[3].jones[0], (7.0f64 * 0.1).cos());
    let out = zmx_write::write(&s).unwrap();
    assert!(
        out.contains("PZPL 1 \"LB_JA\"") && out.contains("PZPL 3 \"LB_JB\""),
        "{out}"
    );
    let back = load(&out, &d);
    assert_eq!(back.surfaces[3].jones[2], -((7.0f64 * 0.1).sin()));
    let _ = std::fs::remove_dir_all(&d);
}

/// 厚さのマクロ（TZPL "名前"）: 面 2 の厚さ = 面 1 の厚さ × 2 + 面 1 の曲率 × 100（OpticStudio 26.1.1 で
/// 5 → 12、厚さ 7 → 16、さらに半径 25 → 18 になることを確かめた式）
#[test]
fn macro_thickness_solve_follows_variables() {
    let d = macro_dir(
        "dep",
        &[("LB_DEP", "SOLVERETURN THIC(1) * 2 + CURV(1) * 100\n")],
    );
    let txt = singlet("LBMIRROR", -0.02).replace(
        "  CURV -2e-2 12 0 0 0 \"LBMIRROR\"\n  DISZ 45\n",
        "  CURV -2e-2\n  DISZ 33\n  TZPL \"LB_DEP\"\n",
    );
    assert!(txt.contains("TZPL"), "{txt}");
    let s = load(&txt, &d);
    assert!(s.macro_warnings().is_empty(), "{:?}", s.macro_warnings());
    assert_eq!(s.surfaces[2].t, 5.0 * 2.0 + 0.02 * 100.0);
    let s = s.with_params(&[(Param::new(1, Kind::Thick), 7.0)]).unwrap();
    assert_eq!(s.surfaces[2].t, 16.0);
    let s = s
        .with_params(&[(Param::new(1, Kind::Radius), 25.0)])
        .unwrap();
    assert_eq!(s.surfaces[2].t, 18.0);
    let back = load(&zmx_write::write(&s).unwrap(), &d);
    assert_eq!(back.surfaces[2].t, 18.0);
    let _ = std::fs::remove_dir_all(&d);
}

/// 円錐係数（KZPL "名前"）・半直径（DIAM 値 4 0 0 1 "名前"）のマクロも解き直し、書き出して読み戻せる
#[test]
fn conic_and_semi_diameter_macros() {
    let d = macro_dir(
        "other",
        &[
            ("KM", "SOLVERETURN -THIC(1) / 10\n"),
            ("SM", "SOLVERETURN THIC(1) + 1.5\n"),
        ],
    );
    let txt = singlet("X", -0.02)
        .replace(
            "  CURV -2e-2 12 0 0 0 \"X\"\n",
            "  CURV -2e-2\n  CONI -0.123\n  KZPL \"KM\"\n",
        )
        .replacen("  DIAM 6 1 0 0 1 \"\"\n", "  DIAM 9 4 0 0 1 \"SM\"\n", 1);
    let s = load(&txt, &d);
    assert!(s.macro_warnings().is_empty(), "{:?}", s.macro_warnings());
    assert_eq!(s.surfaces[2].k, -0.5);
    assert_eq!(s.surfaces[1].semi, 6.5);
    let s = s.with_params(&[(Param::new(1, Kind::Thick), 7.0)]).unwrap();
    assert_eq!(s.surfaces[2].k, -0.7);
    assert_eq!(s.surfaces[1].semi, 8.5);
    let out = zmx_write::write(&s).unwrap();
    assert!(
        out.contains("KZPL \"KM\"") && out.contains("4 0 0 1 \"SM\""),
        "{out}"
    );
    let back = load(&out, &d);
    assert!(back.macro_warnings().is_empty());
    assert_eq!((back.surfaces[2].k, back.surfaces[1].semi), (-0.7, 8.5));
    let _ = std::fs::remove_dir_all(&d);
}

/// 自分の値を読むマクロ（OpticStudio は更新のたびに決まった回数だけ走らせて値が増えていく）は、収まらないので外す
#[test]
fn self_referencing_macro_is_dropped() {
    let d = macro_dir(
        "self",
        &[("LB_SELF", "SOLVERETURN CURV(2) * 0.5 + 0.001\n")],
    );
    let s = load(&singlet("LB_SELF", -0.0123), &d);
    // c = c/2 + 0.001 は 0.002 に収まる（不動点）ので解ける
    assert!(
        (s.surfaces[2].c - 0.002).abs() < 1e-12,
        "{}",
        s.surfaces[2].c
    );
    let d2 = macro_dir("self2", &[("LB_GROW", "SOLVERETURN CURV(2) + 0.001\n")]);
    let s = load(&singlet("LB_GROW", -0.0123), &d2);
    assert_eq!(s.surfaces[2].c, -0.0123);
    assert!(s.solves.iter().all(|q| q.surf != 2));
    let w = s.macro_warnings();
    assert!(
        w.len() == 1 && w[0].contains("LB_GROW") && w[0].contains("解き直せない"),
        "{w:?}"
    );
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&d2);
}

/// フォルダを書かずに渡した .zmx（今のフォルダにあるファイル）でも、同じフォルダのマクロを探す
#[test]
fn zmx_in_current_dir_finds_macro() {
    let dirs = lensbench_core::zpl_solve::macro_dirs(&[], Some(Path::new("")));
    let here = std::env::current_dir().unwrap();
    assert!(dirs.contains(&here), "{dirs:?}");
    assert!(
        dirs.contains(&here.parent().unwrap().join("Macros")),
        "{dirs:?}"
    );
}

/// RAYTRACE と光線の関数: 面 2 の厚さ = 周辺光線が光軸を横切るまでの距離（面 2 の後の向きから）
#[test]
fn raytrace_marginal_focus_thickness() {
    let d = macro_dir(
        "ray",
        &[(
            "LB_FOC",
            "RAYTRACE 0, 0, 0, 1\nIF RAYE() != 0 THEN END\nSOLVERETURN -RAYY(2) * RAYN(2) / RAYM(2) + RAYZ(2)\n",
        )],
    );
    let txt = singlet("X", -0.02)
        .replace("  CURV -2e-2 12 0 0 0 \"X\"\n", "  CURV -2e-2\n")
        .replace("  DISZ 45\n", "  DISZ 45\n  TZPL \"LB_FOC\"\n");
    let s = load(&txt, &d);
    assert!(s.macro_warnings().is_empty(), "{:?}", s.macro_warnings());
    // 周辺光線が像面で光軸に来る
    let t = s
        .try_trace(
            lensbench_core::system::FieldSel::Norm(0.0, 0.0),
            &[0.0],
            &[1.0],
            None,
            None,
            false,
        )
        .unwrap();
    assert!(
        t.rays[0].xyz[1].abs() < 1e-12,
        "{:?} t = {}",
        t.rays[0].xyz,
        s.surfaces[2].t
    );
    assert!((s.surfaces[2].t - 45.0).abs() > 1.0);
    let _ = std::fs::remove_dir_all(&d);
}

/// 光線の関数の値は lensbench の追跡と同じ。OpticStudio 26.1.1 と同じく、法線は −z 寄り、RAYT・RAYO は前の面からの
/// 長さ、RAYTRACE の前は 0、座標の絶対値が 1 を超える・波長の番号がないとマクロを止めて値を変えない
#[test]
fn ray_functions_match_trace() {
    use lensbench_core::zpl::Program;
    use lensbench_core::zpl_solve::SysHost;
    let d = macro_dir("rayf", &[]);
    let txt = singlet("X", -0.02).replace("  CURV -2e-2 12 0 0 0 \"X\"\n", "  CURV -2e-2\n");
    let s = load(&txt, &d);
    let t = s
        .try_trace(
            lensbench_core::system::FieldSel::Norm(0.0, 1.0),
            &[0.3],
            &[-0.7],
            Some(0),
            None,
            true,
        )
        .unwrap();
    let (r1, r2) = (t.rec_at(1).unwrap(), t.rec_at(2).unwrap());
    let val = |src: &str| {
        Program::parse(src)
            .unwrap()
            .solve(&mut SysHost::new(&s, 2))
            .unwrap()
            .value
    };
    let head = "RAYTRACE 0, 1, 0.3, -0.7, 1\n";
    let v = |e: &str| val(&format!("{head}SOLVERETURN {e}")).unwrap();
    assert_eq!(v("RAYX(2)"), r2.xyz[0][0]);
    assert_eq!(v("RAYM(2)"), r2.lmn[0][1]);
    assert_eq!(v("RAYZ(2)"), r2.xyz[0][2]);
    assert_eq!(v("RAGY(2)"), r2.p[0][1]);
    assert_eq!(v("RAYE() + RAYV()"), 0.0);
    // 面 2 までの長さ・光路長（面 1 の後のガラスの屈折率）
    let ng = s.surfaces[1].n[0];
    let len = lensbench_core::math::norm(lensbench_core::math::sub(r2.p[0], r1.p[0]));
    assert!((v("RAYT(2)") - len).abs() < 1e-12);
    assert!((v("RAYO(2)") - ng * len).abs() < 1e-12);
    assert!((v("OPTH(2) - OPTH(1)") - ng * len).abs() < 1e-12);
    // 法線は −z 寄りで、球の中心を向く
    let n = [v("RANX(1)"), v("RANY(1)"), v("RANZ(1)")];
    assert!(n[2] < 0.0);
    assert!(((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]) - 1.0).abs() < 1e-12);
    // RAYTRACE の前は 0
    assert_eq!(val("SOLVERETURN RAYY(2) + 5"), Some(5.0));
    // 範囲外はマクロを止めて値を変えない（それまでの SOLVERETURN も効かない）
    assert_eq!(
        val("SOLVERETURN 1\nRAYTRACE 0, 0, 0, 1.0000001\nSOLVERETURN 2"),
        None
    );
    assert_eq!(
        val("SOLVERETURN 1\nRAYTRACE 0, 0, 0, 1, 0\nSOLVERETURN 2"),
        None
    );
    assert_eq!(
        val("RAYTRACE 0, 0, 0.9, 0.9, 1.5\nSOLVERETURN 2"),
        Some(2.0)
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// 終わらない繰り返しのマクロは固まらず、ソルブを外して保存された値で固定し警告を残す。深すぎる式は落ちず、
/// ほかの文法の誤りと同じく走らせた時点のエラー（OpticStudio と同じく値は 0）
#[test]
fn runaway_and_deep_macros_do_not_hang_or_crash() {
    let deep = format!(
        "X = {}1{}\nSOLVERETURN X\n",
        "(".repeat(5000),
        ")".repeat(5000)
    );
    let d = macro_dir(
        "runaway",
        &[
            ("LBLOOP", "LABEL 1\nGOTO 1\nSOLVERETURN 0.01\n"),
            ("LBDEEP", deep.as_str()),
        ],
    );
    let t0 = std::time::Instant::now();
    let s = load(&singlet("LBLOOP", -0.0123), &d);
    assert!(t0.elapsed().as_secs_f64() < 10.0);
    assert_eq!(s.surfaces[2].c, -0.0123);
    let w = s.macro_warnings();
    assert!(w.len() == 1 && w[0].contains("超えた"), "{w:?}");
    let s = load(&singlet("LBDEEP", -0.0123), &d);
    assert_eq!(s.surfaces[2].c, 0.0);
    let _ = std::fs::remove_dir_all(&d);
}
