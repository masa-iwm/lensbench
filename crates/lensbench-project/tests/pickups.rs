//! zmx ブロックで取り込んだ面のピックアップを展開した系に残す。
use lensbench_core::system::{FieldSel, System};
use lensbench_core::zmx;
use lensbench_project::{expand_report, Env, Project};
use std::path::PathBuf;

/// 面 4〜6 が面 1〜3 をピックアップする雛形（曲率・円錐・厚さ・非球面・座標ブレーク）
const ZMX: &str = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 1\nYFLN 0 2\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n\
SURF 2\n  TYPE EVENASPH\n  CURV -0.01\n  CONI -0.5\n  PARM 1 0\n  PARM 2 1e-6\n  DISZ 3\n\
SURF 3\n  TYPE COORDBRK\n  PARM 1 0.2\n  PARM 4 1.5\n  DISZ 0\n\
SURF 4\n  TYPE STANDARD\n  CURV 0.01 4 1 -1 0 \"\"\n  PKUP 2 -1 0\n  PZUP 1 1.5 0.2 0\n  DISZ 0\n  GLAS ___BLANK 1 0 1.6 0 0 0 0 0 0\n\
SURF 5\n  TYPE EVENASPH\n  CURV -0.015\n  PPAR 2 2 -1 1e-7 0\n  PZUP 4 0.5 1 0\n  DISZ 0\n\
SURF 6\n  TYPE COORDBRK\n  PPAR 1 3 -1 0 0\n  PPAR 4 3 -1 0 0\n  DISZ 20\n\
SURF 7\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 15\n\
SURF 8\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("lb_pickups_{}", std::process::id()));
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("t.zmx"), ZMX).unwrap();
    });
    d
}

const HEAD: &str = r#""glass_catalogs": [],
  "wavelengths": [{"um": 0.5876}], "primary": 1,
  "aperture": {"type": "epd", "value": 8},
  "fields": {"type": "angle", "points": [[0, 0], [1, 2]]},"#;

fn project(blocks: &str) -> (System, Vec<String>) {
    let txt = format!("{{ {HEAD} \"blocks\": {blocks} }}");
    let p = Project::from_json(&txt).unwrap();
    let env = Env {
        base: dir(),
        glass_dirs: vec![],
        catalog: None,
    };
    expand_report(&p, &env).unwrap()
}

fn bits(s: &System) -> Vec<u64> {
    let px = [0.0, 0.5, -0.3];
    let py = [0.0, 0.1, 0.6];
    let mut v = Vec::new();
    for f in 0..s.fields.len() {
        let t = s
            .try_trace(FieldSel::Index(f), &px, &py, None, None, false)
            .unwrap();
        for r in &t.rays {
            assert_eq!(r.err, 0);
            v.extend([r.xyz[0], r.xyz[1], r.opl].map(f64::to_bits));
        }
    }
    v
}

/// 全体を取り込むと、ピックアップは読み込んだ .zmx と同じに追従する
#[test]
fn whole_zmx_keeps_pickups() {
    let (s, warn) = project(r#"[{"type": "zmx", "path": "t.zmx"}]"#);
    assert!(warn.is_empty(), "{warn:?}");
    let raw = zmx::parse_text(ZMX).unwrap();
    let z = zmx::build(&raw, &Default::default(), &[], None).unwrap();
    assert_eq!(s.pickups.len(), z.pickups.len());
    assert_eq!(s.surfaces.len(), z.surfaces.len());
    for ch in [
        vec![("1.t", 5.0), ("2.k", -0.8), ("2.a4", 3e-6), ("3.dx", -0.1)],
        vec![("1.r", 40.0), ("3.ty", 2.5), ("2.t", 2.5)],
    ] {
        let a: Vec<_> = ch.iter().map(|(n, v)| (s.param(n).unwrap(), *v)).collect();
        let b: Vec<_> = ch.iter().map(|(n, v)| (z.param(n).unwrap(), *v)).collect();
        let (sa, zb) = (s.with_params(&a).unwrap(), z.with_params(&b).unwrap());
        for j in 0..sa.surfaces.len() {
            let (x, y) = (&sa.surfaces[j], &zb.surfaces[j]);
            assert_eq!(
                (x.c, x.k, x.t, &x.asph, x.cb),
                (y.c, y.k, y.t, &y.asph, y.cb),
                "面 {j}"
            );
        }
        assert_eq!(bits(&sa), bits(&zb));
    }
    assert!(s.with_params(&[(s.param("4.t").unwrap(), 1.0)]).is_err());
}

/// 範囲の外を参照するピックアップは値で固定して説明を返す。範囲の中どうしは残る
#[test]
fn out_of_range_is_fixed_with_a_warning() {
    // 面 3〜7（座標ブレーク 3、玉 4〜5、座標ブレーク 6、面 7）: 面 4・5 のピックアップ（元は面 1・2）は範囲の外
    let (s, warn) = project(
        r#"[{"type": "stop", "semi": 4}, {"type": "air", "t": 1}, {"type": "zmx", "path": "t.zmx", "from": 3, "to": 7}]"#,
    );
    assert_eq!(warn.len(), 4, "{warn:?}");
    assert!(warn.iter().all(|w| w.contains("範囲の外")));
    // 座標ブレーク 6 ← 3 と、厚さ 5 ← 4 は残り、追従する
    assert_eq!(s.pickups.len(), 3, "{:?}", s.pickups);
    let q = s
        .pickups
        .iter()
        .find(|q| q.what == lensbench_core::param::PickKind::Parm(1))
        .unwrap();
    let s2 = s
        .with_params(&[(s.param(&format!("{}.dx", q.src)).unwrap(), -0.4)])
        .unwrap();
    assert_eq!(s2.surfaces[q.dst].cb[0], 0.4);
}

/// 鏡のあと（光が −z へ進む）では厚さの符号が変わるので、オフセットの符号を変えて残す
#[test]
fn after_a_mirror_the_offset_flips() {
    let (s, warn) = project(
        r#"[{"type": "stop", "semi": 4}, {"type": "air", "t": 30}, {"type": "mirror", "tilt": [0, 0]}, {"type": "air", "t": 2},
           {"type": "zmx", "path": "t.zmx", "from": 3, "to": 7}]"#,
    );
    assert_eq!(warn.len(), 4, "{warn:?}");
    let th = s
        .pickups
        .iter()
        .find(|q| q.what == lensbench_core::param::PickKind::Thick)
        .unwrap();
    assert_eq!((th.scale, th.offset), (0.5, -1.0));
    let t4 = s.surfaces[th.src].t;
    assert!(t4 < 0.0);
    let s2 = s
        .with_params(&[(s.param(&format!("{}.t", th.src)).unwrap(), t4 - 1.0)])
        .unwrap();
    assert_eq!(s2.surfaces[th.dst].t, 0.5 * (t4 - 1.0) - 1.0);
}

/// 群を動かして厚さの関係が崩れたピックアップは値で固定する
#[test]
fn broken_relation_is_fixed() {
    // 群（面 4〜5 の玉）を 0.7 ずらすと、前の空気（面 3 の厚さ）と後ろの空気（面 5 の厚さ）が変わり、
    // 面 5 ← 4 の厚さの関係が崩れる
    let (s, warn) = project(
        r#"[{"type": "zmx", "path": "t.zmx", "groups": [{"name": "G", "from": 4, "to": 5, "shift": 0.7}]}]"#,
    );
    assert!(warn.iter().any(|w| w.contains("値の関係")), "{warn:?}");
    for q in &s.pickups {
        assert!(q.holds(&s));
    }
}

/// 往路を戻る傾いた鏡（戻りの座標ブレークが入る）の後の zmx ブロックでも、ピックアップが同じ面どうしに付く
#[test]
fn after_a_tilted_return_mirror() {
    let (s, warn) = project(
        r#"[{"type": "stop", "semi": 4}, {"type": "air", "t": 10}, {"type": "mirror", "back": true, "tilt": [0.5, -0.3]},
           {"type": "air", "t": 2}, {"type": "zmx", "path": "t.zmx", "from": 3, "to": 7}]"#,
    );
    assert_eq!(warn.len(), 4, "{warn:?}");
    assert!(warn.iter().all(|w| w.contains("範囲の外")), "{warn:?}");
    assert_eq!(s.pickups.len(), 3, "{:?}", s.pickups);
    use lensbench_core::param::PickKind;
    use lensbench_core::surface::SurfType;
    for q in &s.pickups {
        assert!(q.holds(&s));
        // 座標ブレーク 6 ← 3 と、厚さ 5 ← 4（玉の後ろの面 ← 玉の前の面）
        match q.what {
            PickKind::Parm(_) => {
                assert_eq!(s.surfaces[q.src].typ, SurfType::CoordBrk);
                assert_eq!(s.surfaces[q.dst].typ, SurfType::CoordBrk);
                assert_eq!(q.dst, q.src + 3);
            }
            PickKind::Thick => assert_eq!(q.dst, q.src + 1),
            w => panic!("{w:?}"),
        }
    }
}

/// zmx ブロックの stop: false: 取り込んだ面の絞りを外し、系の絞りはほかのブロックにする
#[test]
fn zmx_block_without_its_stop() {
    let (s, _) = project(
        r#"[{"type": "stop", "semi": 4}, {"type": "air", "t": 5}, {"type": "zmx", "path": "t.zmx", "stop": false}]"#,
    );
    assert_eq!(s.stop(), 1);
    let (s, _) = project(r#"[{"type": "zmx", "path": "t.zmx"}]"#);
    assert_eq!(s.stop(), 1);
    let txt = format!(
        "{{ {HEAD} \"blocks\": [{{\"type\": \"zmx\", \"path\": \"t.zmx\", \"stop\": false}}] }}"
    );
    let p = Project::from_json(&txt).unwrap();
    let env = Env {
        base: dir(),
        glass_dirs: vec![],
        catalog: None,
    };
    let e = expand_report(&p, &env).unwrap_err();
    assert!(e.contains("絞りが 0 個"), "{e}");
}

/// 鏡のあとで、符号の変わる量（厚さ）から変わらない量（円錐）への別の列のピックアップは、倍率の符号を返して残す
#[test]
fn after_a_mirror_cross_column_scale_flips() {
    const Z: &str =
        "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 8\nFTYP 0 0 2 1 0 0 0\n\
XFLN 0 1\nYFLN 0 2\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 4\n  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.01\n  CONI 0.4\n  PKUP 1 0.1 2\n  DISZ 20\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    std::fs::write(dir().join("xc.zmx"), Z).unwrap();
    let (s, warn) = project(
        r#"[{"type": "stop", "semi": 4}, {"type": "air", "t": 30}, {"type": "mirror", "tilt": [0, 0]}, {"type": "air", "t": 2},
           {"type": "zmx", "path": "xc.zmx", "from": 1, "to": 2, "stop": false}]"#,
    );
    let k = s
        .pickups
        .iter()
        .find(|q| q.what == lensbench_core::param::PickKind::Conic)
        .unwrap_or_else(|| panic!("円錐のピックアップが残らない: {warn:?}"));
    assert_eq!(k.scale, -0.1);
    let t = s.surfaces[k.src].t;
    assert!(t < 0.0);
    assert!((s.surfaces[k.dst].k - 0.4).abs() < 1e-12);
    let s2 = s
        .with_params(&[(s.param(&format!("{}.t", k.src)).unwrap(), t - 1.0)])
        .unwrap();
    assert!((s2.surfaces[k.dst].k - 0.5).abs() < 1e-12);
}
