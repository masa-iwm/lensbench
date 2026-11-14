//! 系ファイル: 保存して読み直した系の追跡が元とビット一致し、hash が同じ系で同じになる。

use lensbench_core::param::{Kind, Param, PickKind, Pickup};
use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use lensbench_project::sysfile;
use std::path::PathBuf;

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

fn rays(s: &System) -> Vec<u64> {
    let mut px = Vec::new();
    let mut py = Vec::new();
    for i in 0..7 {
        for j in 0..7 {
            let (x, y) = (i as f64 / 3.0 - 1.0, j as f64 / 3.0 - 1.0);
            if x * x + y * y <= 1.0 {
                px.push(x);
                py.push(y);
            }
        }
    }
    let mut out = Vec::new();
    for f in 0..s.fields.len() {
        for w in 0..s.wavelengths.len() {
            let t = s
                .try_trace(FieldSel::Index(f), &px, &py, Some(w), None, false)
                .unwrap();
            for r in &t.rays {
                out.extend(r.xyz.iter().chain(&r.lmn).map(|x| x.to_bits()));
                out.push(r.opl.to_bits());
                out.push(r.err as u64);
                out.push(r.vig as u64);
            }
        }
    }
    out
}

fn names() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root().join("tests/refs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| {
            root()
                .join("tests/refs")
                .join(n)
                .join("system.zmx")
                .exists()
        })
        .collect();
    v.sort();
    v
}

/// 照合系すべて: 書いて読み直すと追跡がビット一致、hash が変わらず、書いた文字列も同じ
#[test]
fn roundtrip_all_refs() {
    let ns = names();
    assert!(ns.len() >= 30);
    for name in ns {
        let s = load(&name);
        let txt = sysfile::to_string(&s, &[]).unwrap();
        assert!(sysfile::is_system_json(&txt));
        let (t, meta, warn) = sysfile::from_str(&txt).unwrap();
        assert!(warn.is_empty(), "{name}: {warn:?}");
        assert_eq!(rays(&s), rays(&t), "{name}");
        assert_eq!(meta.hash, sysfile::system_hash(&s).unwrap(), "{name}");
        assert_eq!(meta.lensbench, lensbench_core::VERSION);
        assert_eq!(sysfile::to_string(&t, &[]).unwrap(), txt, "{name}");
    }
}

/// 実数の欄を整数の書き方（`3`）で書いても読める（Python・手で書いた系ファイル）。誤りには欄の道が付く（IOO-32）
#[test]
fn integers_in_float_fields() {
    fn ints(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Number(n) => {
                if let Some(x) = n.as_f64().filter(|x| x.fract() == 0.0 && x.abs() < 1e15) {
                    *v = serde_json::json!(x as i64);
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(ints),
            serde_json::Value::Object(o) => o.values_mut().for_each(ints),
            _ => {}
        }
    }
    for name in ["A0", "B0"] {
        let s = load(name);
        let txt = sysfile::to_string(&s, &[]).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
        ints(&mut v["system"]);
        let txt2 = v.to_string();
        assert!(
            txt2.contains("\"t\":0,") || txt2.contains("\"t\":0}"),
            "{name}: 整数に書き直せていない"
        );
        let (t, meta, warn) = sysfile::from_str(&txt2).unwrap();
        assert!(warn.is_empty(), "{name}: {warn:?}");
        assert_eq!(meta.hash, sysfile::system_hash(&s).unwrap(), "{name}");
        assert_eq!(rays(&s), rays(&t), "{name}");
    }
    let s = load("A0");
    let mut v: serde_json::Value =
        serde_json::from_str(&sysfile::to_string(&s, &[]).unwrap()).unwrap();
    v["system"]["surfaces"][1]["t"] = serde_json::json!("x");
    let e = sysfile::from_str(&v.to_string()).unwrap_err();
    assert!(e.contains("surfaces[1].t"), "{e}");
}

/// 編集・パラメータ変更・ピックアップ・多重構成の表・玉も残る。違う系は違う hash
#[test]
fn edited_system_roundtrip() {
    let mut s = load("A0");
    let h0 = sysfile::system_hash(&s).unwrap();
    let j = s.image() - 1;
    s.set_param(Param::new(j, Kind::Thick), s.surfaces[j].t + 0.25)
        .unwrap();
    s.set_param(Param::new(2, Kind::Dn), 1e-4).unwrap();
    s.insert_surface(1, lensbench_core::surface::Surface::default())
        .unwrap();
    s.add_pickup(Pickup::new(1, 3, PickKind::Thick, 0.5, 0.0))
        .unwrap();
    let k = s.image() - 1;
    s.add_solve(lensbench_core::param::ThickSolve {
        surf: k - 1,
        height: 0.0,
        kind: lensbench_core::param::SolveKind::ConcentricSurface(k - 2),
    })
    .unwrap();
    s.set_config_count(2).unwrap();
    let t0 = s.surfaces[k].t;
    s.set_config_row(Param::new(k, Kind::Thick), vec![t0, t0 + 1.0])
        .unwrap();
    // 構成のピックアップも保存される
    s.insert_config(3, 2, true).unwrap();
    s.define_unit("G", &[[2, 5]], [0.0, 0.0, 0.0]).unwrap();
    s.set_param(s.param("G.tx").unwrap(), 0.1).unwrap();
    let h1 = sysfile::system_hash(&s).unwrap();
    assert_ne!(h0, h1);
    let txt = sysfile::to_string(&s, &[PathBuf::from("coat.json")]).unwrap();
    let (t, meta, _) = sysfile::from_str(&txt).unwrap();
    assert_eq!(meta.coatings, vec![PathBuf::from("coat.json")]);
    assert_eq!(rays(&s), rays(&t));
    assert_eq!(t.pickups, s.pickups);
    assert_eq!(t.solves, s.solves);
    assert!(!s.solves.is_empty());
    assert_eq!(t.configs(), s.configs());
    assert_eq!(t.units.len(), s.units.len());
    assert!(t.find_unit("G").is_some());
    assert_eq!(rays(&s.config(2).unwrap()), rays(&t.config(2).unwrap()));
    // 読み直した系も編集できる
    let mut t = t;
    t.set_param(t.param("G.tx").unwrap(), 0.0).unwrap();
}

/// 手で直したら注意、新しい版・別の format はエラー、ファイルの読み書き
#[test]
fn errors_and_files() {
    let s = load("B0");
    let txt = sysfile::to_string(&s, &[PathBuf::from("c.json")]).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    v["system"]["name"] = serde_json::json!("changed");
    let (_, _, warn) = sysfile::from_str(&v.to_string()).unwrap();
    assert_eq!(warn.len(), 1, "{warn:?}");
    v["format_version"] = serde_json::json!(99);
    assert!(sysfile::from_str(&v.to_string()).is_err());
    v["format"] = serde_json::json!("other");
    assert!(sysfile::from_str(&v.to_string()).is_err());
    assert!(!sysfile::is_system_json("{\"blocks\": []}"));
    // 屈折率の数が合わないものは読まない
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    v["system"]["surfaces"][1]["n"] = serde_json::json!([1.5]);
    assert!(sysfile::from_str(&v.to_string()).is_err());
    let dir = std::env::temp_dir().join(format!("lb_sysfile_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("b0.json");
    sysfile::save(&p, &s, &[PathBuf::from("c.json")]).unwrap();
    assert!(sysfile::is_system_file(&p));
    let (t, meta, _) = sysfile::load(&p).unwrap();
    assert_eq!(meta.coatings, vec![dir.join("c.json")]);
    assert_eq!(rays(&s), rays(&t));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// ZPL Macro Solve: 系ファイルにはマクロの文を入れ、パスは書かない。読み直すときにファイルは読まず、
/// マクロのファイルを消した後でも解き直せる
#[test]
fn macro_solve_source_is_embedded() {
    let d = std::env::temp_dir().join(format!("lb_sysfile_zpl_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("LB_DEP.ZPL"),
        "SOLVERETURN THIC(1) * 2 + CURV(1) * 100\n",
    )
    .unwrap();
    let txt = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 1 1 0 0 0\n\
XFLN 0\nYFLN 0\nFWGN 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5168 64.17 0 0 0 0 0\n\
SURF 2\n  TYPE STANDARD\n  CURV -0.02\n  DISZ 33\n  TZPL \"LB_DEP\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
    let raw = zmx::parse_text(txt).unwrap();
    let opt = LoadOptions {
        cfg: 1,
        macro_dirs: vec![d.clone()],
        ..Default::default()
    };
    let s = zmx::build(&raw, &opt, &[], None).unwrap();
    assert_eq!(s.surfaces[2].t, 12.0);
    let j = sysfile::to_string(&s, &[]).unwrap();
    assert!(j.contains("SOLVERETURN THIC(1)"), "{j}");
    assert!(!j.contains(&*d.to_string_lossy()), "{j}");
    std::fs::remove_dir_all(&d).unwrap();
    let (back, _, _) = sysfile::from_str(&j).unwrap();
    let back = back
        .with_params(&[(Param::new(1, Kind::Thick), 7.0)])
        .unwrap();
    assert_eq!(back.surfaces[2].t, 16.0);
}
