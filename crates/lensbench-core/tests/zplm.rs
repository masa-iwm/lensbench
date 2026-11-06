//! ZPLM の項: マクロの OPTRETURN の値を評価関数の項にする。

use lensbench_core::optim::UserTerm;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::zpl_solve::Macro;
use lensbench_core::zplm::ZplmTerm;
use lensbench_core::FieldSel;

const SINGLET: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 3 1 0 0 0\n\
XFLN 0 0 0\nYFLN 0 14 20\nFWGN 1 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5168 64.17 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.01\n  DISZ 45\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";

fn sys() -> lensbench_core::System {
    let raw = zmx::parse_text(SINGLET).unwrap();
    zmx::build(
        &raw,
        &LoadOptions {
            cfg: 1,
            ..Default::default()
        },
        &[],
        None,
    )
    .unwrap()
}

#[test]
fn optreturn_values_and_pv() {
    let s = sys();
    let m = Macro::from_source(
        "T",
        "x = CURV(1)\nOPTRETURN 0, x * 2\nOPTRETURN 3, PVHY() + PVPX()\n\
         RAYTRACE PVHX(), PVHY(), PVPX(), PVPY()\nOPTRETURN 5, RAYY(NSUR())",
    )
    .unwrap();
    let pv = [0.0, 0.5, 0.0, 1.0];
    let t = ZplmTerm::new(&m, 0, pv, 0.0).unwrap();
    let o = t.run(&s).unwrap();
    assert_eq!(o[0], Some(0.04));
    assert_eq!(o[3], Some(0.5));
    assert!(o[1].is_none());
    // RAYTRACE は lensbench の追跡と同じ光線
    let r = s.trace(FieldSel::Norm(0.0, 0.5), &[0.0], &[1.0], None, None, false);
    assert_eq!(o[5], Some(r.rays[0].xyz[1]));
    // 項の値は OPTRETURN data − target
    let t = ZplmTerm::new(&m, 0, pv, 0.01).unwrap();
    assert_eq!(t.eval(&[&s, &s]).unwrap(), vec![vec![0.04 - 0.01]; 2]);
    // 呼ばれていない番号・50 を超える番号は 0（OpticStudio 26.1.1 で実測）
    for d in [1, 50, 51] {
        assert_eq!(
            ZplmTerm::new(&m, d, pv, 0.0).unwrap().value(&s).unwrap(),
            0.0
        );
    }
}

#[test]
fn errors_stop_the_term() {
    let s = sys();
    for src in [
        "OPTRETURN 0, y",
        "OPTRETURN 0, 1.5\nx = zzz + 1",
        "OPTRETURN 0, EXPE(1000)",
        "OPTRETURN 60, 1",
        "FOOBAR 1\nOPTRETURN 0, 1",
        "OPTRETURN 0, 1, 2",
    ] {
        let m = Macro::from_source("E", src).unwrap();
        let r = ZplmTerm::new(&m, 0, [0.0; 4], 0.0).unwrap().value(&s);
        assert!(r.is_err(), "{src}");
    }
    // 範囲外の RAYTRACE はそこで黙って止まり、それまでの値（OpticStudio 26.1.1 で実測）
    let m = Macro::from_source(
        "D",
        "OPTRETURN 0, 4.5\nRAYTRACE 2, 0, 0, 0, 1\nOPTRETURN 0, 9",
    )
    .unwrap();
    let t = ZplmTerm::new(&m, 0, [0.0; 4], 0.0).unwrap();
    assert_eq!(t.value(&s).unwrap(), 4.5);
    // 0 割りは 0（Macro Solve と同じ解釈系、OpticStudio と同じ）
    let m = Macro::from_source("Z", "OPTRETURN 0, 1 / 0").unwrap();
    assert_eq!(
        ZplmTerm::new(&m, 0, [0.0; 4], 0.0)
            .unwrap()
            .value(&s)
            .unwrap(),
        0.0
    );
}
