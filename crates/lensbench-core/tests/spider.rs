//! スパイダー開口（.zmx の SPID）: OpticStudio 2026 R1.01 の遮り（tests/refs/spider、合成系 8 ケース）と、書き出しの往復。
//!
//! 基準の系: 物体無限遠、EPD 20、軸上、面 1 = 絞り（平面、空気、厚さ 10）にスパイダー、面 2 = 近軸レンズ f 50。
//! 面 1 の点 (x, y) = (−9.9 + 0.3i, −9.9 + 0.3j)（i, j = 0..66）を通る光線を追い、遮れば #。ファイルの行は j = 66 から
use lensbench_core::surface::Aperture;
use lensbench_core::system::{FieldSel, System};
use lensbench_core::{zmx, zmx_write};

fn build(txt: &str) -> System {
    let raw = zmx::parse_text(txt).unwrap();
    zmx::build(
        &raw,
        &zmx::LoadOptions {
            cfg: 1,
            ..Default::default()
        },
        &[],
        None,
    )
    .unwrap()
}

/// 面 1 の開口の行（SPID・OBDC・DIAM）を与えた基準の系
fn sys(lines: &str) -> String {
    format!(
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 20\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.55 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 10\n{lines}\
SURF 2\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 50\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    )
}

/// 格子の遮り（行は j = 66 から、# 遮り）
fn bitmap(s: &System) -> Vec<String> {
    let c = |i: usize| (3.0 * i as f64 - 99.0) / 10.0;
    let mut out = Vec::new();
    for j in (0..67).rev() {
        let px: Vec<f64> = (0..67).map(|i| c(i) / 10.0).collect();
        let py = vec![c(j) / 10.0; 67];
        let t = s.trace(FieldSel::Index(0), &px, &py, None, None, false);
        out.push(
            t.rays
                .iter()
                .map(|r| {
                    assert_eq!(r.err, 0);
                    assert!(r.vig == 0 || r.vig == 1);
                    if r.vig == 1 {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect(),
        );
    }
    out
}

fn reference(case: u32) -> Vec<String> {
    let p = format!(
        "{}/tests/refs/spider/case{case}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

const CASES: &[(u32, &str)] = &[
    (
        1,
        "  DIAM 10 0 0 0 1 \"\"\n  SPID 0.76200000000000001 4 0\n",
    ),
    (2, "  DIAM 10 0 0 0 1 \"\"\n  SPID 2 4 0\n"),
    (3, "  DIAM 10 0 0 0 1 \"\"\n  SPID 2 1 0\n"),
    (4, "  DIAM 10 0 0 0 1 \"\"\n  SPID 2 3 0\n"),
    (
        5,
        "  DIAM 10 0 0 0 1 \"\"\n  SPID 1.5 5 0\n  OBDC 1.000000000000E+00 -5.000000000000E-01\n",
    ),
    // 半直径を固定 5 にしても、スパイダーは r > 5 を切らない（ケース 4 と同じ）
    (
        4,
        "  DIAM 5 1 0 0 1 \"\"\n  MEMA 5 0 0 0 1 \"\"\n  SPID 2 3 0\n",
    ),
    (7, "  DIAM 10 0 0 0 1 \"\"\n  SPID 2 2 0\n"),
    (8, "  DIAM 10 0 0 0 1 \"\"\n  SPID 0 4 0\n"),
];

#[test]
fn spider_matches_opticstudio() {
    for (case, lines) in CASES {
        let s = build(&sys(lines));
        let got = bitmap(&s);
        let want = reference(*case);
        for (j, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "ケース {case}（{lines:?}）の行 {}", 66 - j);
        }
    }
}

#[test]
fn spider_parse_and_round_trip() {
    let s = build(&sys(CASES[4].1));
    assert_eq!(
        s.surfaces[1].ap,
        Aperture::Spider {
            width: 1.5,
            arms: 5,
            dx: 1.0,
            dy: -0.5
        }
    );
    let txt = zmx_write::write(&s).unwrap();
    assert!(txt.contains("  SPID 1.5 5 0"), "{txt}");
    let b = build(&txt);
    assert_eq!(s.surfaces[1].ap, b.surfaces[1].ap);
    assert_eq!(bitmap(&b), reference(5));
    // 開口のピックアップ（3 番目の欄が元の面）でも、書かれた値と偏心を使う
    let p = build(&sys("  SPID 1.5 5 1\n  OBDC 1 -0.5\n"));
    assert_eq!(p.surfaces[1].ap, s.surfaces[1].ap);
    // 本数が整数でない・幅が負は読まない
    for bad in ["  SPID 1 2.5 0\n", "  SPID -1 4 0\n"] {
        let raw = zmx::parse_text(&sys(bad));
        assert!(raw.is_err(), "{bad}");
    }
}
