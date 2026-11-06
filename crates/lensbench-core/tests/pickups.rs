//! ピックアップの追従: 元の面のパラメータを変えると、ピックアップ先も解決し直す。
use lensbench_core::elements;
use lensbench_core::param::{Kind, Param, PickKind};
use lensbench_core::system::{FieldSel, System};
use lensbench_core::{zmx, zmx_write};

const HEAD: &str =
    "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 1\nYFLN 0 2\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n";

/// 面 4〜6 が面 1〜3 をピックアップする系（往復の系の戻りの面のように、倍率 −1 のものを含む）
fn text() -> String {
    format!(
        "{HEAD}SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n\
SURF 2\n  TYPE EVENASPH\n  CURV -0.01\n  CONI -0.5\n  PARM 1 0\n  PARM 2 1e-6\n  PARM 3 -2e-9\n  DISZ 3\n\
SURF 3\n  TYPE COORDBRK\n  PARM 1 0.2\n  PARM 2 0\n  PARM 3 0\n  PARM 4 1.5\n  PARM 5 0\n  PARM 6 0\n  DISZ 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.01 4 1 -1 0 \"\"\n  CONI 0\n  PKUP 2 -1 0\n  PZUP 1 1.5 0.2 0\n  DISZ 0\n  GLAS ___BLANK 1 0 1.6 0 0 0 0 0 0\n\
SURF 5\n  TYPE EVENASPH\n  CURV -0.015\n  PPAR 2 2 -1 0 0\n  PPAR 3 2 2 1e-9 0\n  PZUP 2 2 1 0\n  DISZ 0\n\
SURF 6\n  TYPE COORDBRK\n  PPAR 1 3 -1 0 0\n  PPAR 4 3 -1 0 0\n  DISZ 20\n\
SURF 7\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 15\n\
SURF 8\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n"
    )
}

fn build(txt: &str) -> System {
    let raw = zmx::parse_text(txt).unwrap();
    zmx::build(&raw, &Default::default(), &[], None).unwrap()
}

fn p(s: &System, n: &str) -> Param {
    s.param(n).unwrap()
}

fn bits(s: &System) -> Vec<u64> {
    let px = [0.0, 0.5, -0.3, 0.2];
    let py = [0.0, 0.1, 0.6, -0.7];
    let mut v = Vec::new();
    for f in 0..s.fields.len() {
        let t = s.trace(FieldSel::Index(f), &px, &py, None, None, false);
        for r in &t.rays {
            assert_eq!(r.err, 0);
            v.extend([r.xyz[0], r.xyz[1], r.lmn[0], r.opl].map(f64::to_bits));
        }
    }
    v
}

#[test]
fn loaded_values_and_links() {
    let s = build(&text());
    let sf = &s.surfaces;
    // 保存された曲率（0.01）は読込で元の面に合わせ直す（OpticStudio と同じ）
    assert_eq!(sf[4].c, -0.02);
    assert_eq!(sf[4].k, 0.5);
    assert_eq!(sf[4].t, 6.2);
    assert_eq!(sf[5].t, 7.0);
    assert_eq!(sf[5].asph[1], -1e-6);
    assert_eq!(sf[5].asph[2], 2.0 * -2e-9 + 1e-9);
    assert_eq!(sf[6].cb[0], -0.2);
    assert_eq!(sf[6].cb[3], -1.5);
    assert_eq!(s.pickups.len(), 8);
    assert!(s.pickup_of(p(&s, "4.t")).is_some());
    assert!(s.pickup_of(p(&s, "6.ty")).is_some());
    assert!(s.pickup_of(p(&s, "6.dy")).is_none());
    assert!(s.pickup_of(p(&s, "1.t")).is_none());
}

/// 元を変えた系 = 同じ値を書いた .zmx を読んだ系（追跡もビット一致）
#[test]
fn follows_source() {
    let s = build(&text());
    let ch = [
        (p(&s, "1.t"), 5.0),
        (p(&s, "2.k"), -0.8),
        (p(&s, "2.a4"), 3e-6),
        (p(&s, "3.dx"), -0.1),
        (p(&s, "3.ty"), 2.5),
        (p(&s, "2.t"), 2.5),
        (p(&s, "1.r"), 40.0),
    ];
    let a = s.with_params(&ch).unwrap();
    let txt = text()
        .replace("  CURV 0.02\n  DISZ 4\n", "  CURV 0.025\n  DISZ 5\n")
        .replace("CONI -0.5", "CONI -0.8")
        .replace("PARM 2 1e-6", "PARM 2 3e-6")
        .replace("PARM 1 0.2", "PARM 1 -0.1")
        .replace("PARM 4 1.5", "PARM 4 2.5")
        .replace("PARM 3 -2e-9\n  DISZ 3", "PARM 3 -2e-9\n  DISZ 2.5");
    let b = build(&txt);
    assert_eq!(a.surfaces[4].t, 7.7);
    assert_eq!(a.surfaces[4].k, 0.8);
    assert_eq!(a.surfaces[4].c, -0.025);
    assert_eq!(a.surfaces[5].t, 6.0);
    assert_eq!(a.surfaces[5].asph[1], -3e-6);
    assert_eq!(a.surfaces[6].cb[0], 0.1);
    assert_eq!(a.surfaces[6].cb[3], -2.5);
    for j in 0..a.surfaces.len() {
        let (x, y) = (&a.surfaces[j], &b.surfaces[j]);
        assert_eq!(
            (x.c, x.k, x.t, &x.asph, x.cb),
            (y.c, y.k, y.t, &y.asph, y.cb),
            "面 {j}"
        );
    }
    assert_eq!(bits(&a), bits(&b));
    // set_params も同じ
    let mut c = s.clone();
    c.set_params(&ch).unwrap();
    assert_eq!(bits(&a), bits(&c));
}

/// ピックアップ先は変えられない（系も変わらない）
#[test]
fn destination_is_rejected() {
    let s = build(&text());
    for n in ["4.t", "4.k", "5.a4", "6.dx", "4.r", "4.c"] {
        let e = s.with_params(&[(p(&s, n), 1.0)]);
        assert!(e.unwrap_err().to_string().contains("ピックアップ"), "{n}");
    }
    assert!(s.with_params(&[(p(&s, "5.c"), 0.01)]).is_ok());
    let mut c = s.clone();
    assert!(c
        .set_params(&[(p(&s, "1.t"), 9.0), (p(&s, "4.t"), 1.0)])
        .is_err());
    assert_eq!(c.surfaces[1].t, 4.0);
}

/// 書き出して読み戻してもピックアップが残る
#[test]
fn round_trip_keeps_links() {
    let s = build(&text());
    let b = build(&zmx_write::write(&s).unwrap());
    assert_eq!(s.pickups, b.pickups);
    assert_eq!(bits(&s), bits(&b));
    let ch = [(p(&s, "1.t"), 3.0), (p(&s, "3.ty"), -1.0)];
    assert_eq!(
        bits(&s.with_params(&ch).unwrap()),
        bits(&b.with_params(&ch).unwrap())
    );
}

/// 玉の並びを組み直した系はピックアップを引き継がない
#[test]
fn assemble_drops_links() {
    let s = build(&text());
    let els = elements::split(&s).unwrap();
    let a = elements::assemble(&s, &els).unwrap();
    assert!(a.pickups.is_empty());
    assert_eq!(bits(&s), bits(&a));
}

/// 循環するピックアップは読込でエラー
#[test]
fn cycle_is_error() {
    let t = text().replace(
        "  CURV 0.02\n  DISZ 4\n",
        "  CURV 0.02\n  PZUP 4 1 1 0\n  DISZ 4\n",
    );
    let raw = zmx::parse_text(&t).unwrap();
    assert!(zmx::build(&raw, &Default::default(), &[], None).is_err());
}

#[test]
fn kinds() {
    let s = build(&text());
    let k: Vec<PickKind> = s.pickups.iter().map(|q| q.what).collect();
    assert!(k.contains(&PickKind::Parm(4)));
    assert_eq!(Param::new(6, Kind::Ty), p(&s, "6.ty"));
}
