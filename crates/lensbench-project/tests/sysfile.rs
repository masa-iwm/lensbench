//! 系ファイル（版 2）: 公開の試験の .zmx すべてと例題を、保存して読み直すと追跡・近軸量がビット一致し、
//! 書き直した文字列も同じになる。hash は同じ系で同じ。版 1 も読める。

use lensbench_core::param::{Kind, Param, PickKind, Pickup};
use lensbench_core::system::FieldSel;
use lensbench_core::zmx::{self, LoadOptions};
use lensbench_core::System;
use lensbench_project::sysfile;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

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

/// 追跡（光線ごとの位置・方向・光路長・状態、面ごとの記録）と近軸量のビットの指紋（全視野・全波長）
fn fingerprint(s: &System) -> u64 {
    let mut h = DefaultHasher::new();
    let mut f = |v: f64| v.to_bits().hash(&mut h);
    let (mut px, mut py) = (Vec::new(), Vec::new());
    for i in 0..5 {
        for j in 0..5 {
            px.push(-1.0 + 0.5 * i as f64);
            py.push(-1.0 + 0.5 * j as f64);
        }
    }
    let p = s.paraxial();
    for v in [
        p.enpp,
        p.epd,
        p.efl,
        p.expp,
        p.expd,
        p.stop_radius,
        p.wfno,
        p.isfn,
        p.pmag,
    ] {
        f(v);
    }
    for v in p
        .marginal
        .0
        .iter()
        .chain(&p.marginal.1)
        .chain(&p.chief.0)
        .chain(&p.chief.1)
    {
        f(*v);
    }
    let mut out = Vec::new();
    for fi in 0..s.fields.len() {
        for w in 0..s.wavelengths.len() {
            match s.try_trace(FieldSel::Index(fi), &px, &py, Some(w), None, true) {
                Ok(t) => {
                    for (i, r) in t.rays.iter().enumerate() {
                        out.push(r.status.code() as u64);
                        out.push(r.vig as u64);
                        out.extend(r.xyz.iter().chain(&r.lmn).map(|x| x.to_bits()));
                        out.push(r.opl.to_bits());
                        for rs in &t.rec {
                            out.extend(rs.xyz[i].iter().chain(&rs.lmn[i]).map(|x| x.to_bits()));
                            out.push(rs.opl[i].to_bits());
                        }
                    }
                }
                Err(e) => out.push(e.to_string().len() as u64 | 1 << 63),
            }
        }
    }
    for x in out {
        f(f64::from_bits(x));
    }
    for (j, d) in s.surfaces.iter().enumerate() {
        for v in s.n(j).iter().chain([&d.t, &d.c]) {
            f(*v);
        }
        // 自動の半直径は光線から求めた値（光線が通らない面では求め直せず、読込の前の値は残らない）
        if !d.semi_auto {
            f(d.semi);
        }
    }
    h.finish()
}

/// tests の .zmx すべて（フォルダの順）
fn zmx_files(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        let mut v: Vec<_> = std::fs::read_dir(p)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        v.sort();
        for e in v {
            zmx_files(&e, out);
        }
    } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zmx")) {
        out.push(p.to_path_buf());
    }
}

/// 系ファイルのスキーマ（docs/schema/system.schema.json と同じもの）
fn validator() -> &'static jsonschema::Validator {
    static V: std::sync::OnceLock<jsonschema::Validator> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        jsonschema::validator_for(&lensbench_project::schema::schema("system").unwrap()).unwrap()
    })
}

/// 保存 → 読込 → 保存: 追跡がビット一致、2 回目の文字列も同じ、注意なし、hash も同じ、スキーマに合う
fn roundtrip(name: &str, s: &System, dirs: &[PathBuf]) {
    let txt = sysfile::to_string(s, &[]).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(sysfile::is_system_json(&txt));
    let errs: Vec<String> = validator()
        .iter_errors(&serde_json::from_str(&txt).unwrap())
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errs.is_empty(), "{name}: {}", errs.join("\n"));
    let (t, meta, warn) =
        sysfile::from_str_with(&txt, dirs).unwrap_or_else(|e| panic!("{name}: {e}\n{txt}"));
    assert!(warn.is_empty(), "{name}: {warn:?}");
    assert_eq!(fingerprint(s), fingerprint(&t), "{name}");
    assert_eq!(s.apodization, t.apodization, "{name}");
    for (j, (a, b)) in s.surfaces.iter().zip(&t.surfaces).enumerate() {
        assert_eq!(
            a.semi.to_bits(),
            b.semi.to_bits(),
            "{name}: surface {j} semi"
        );
    }
    assert_eq!(meta.hash, sysfile::system_hash(s).unwrap(), "{name}");
    assert_eq!(meta.lensbench, lensbench_core::VERSION);
    let txt2 = sysfile::to_string(&t, &[]).unwrap();
    assert_eq!(txt2, txt, "{name}");
    for c in 2..=s.configs().map_or(1, |c| c.n) {
        let (a, b) = (s.config(c).unwrap(), t.config(c).unwrap());
        assert_eq!(fingerprint(&a), fingerprint(&b), "{name} configuration {c}");
    }
}

/// 公開の試験の .zmx すべて（111 個）。tests/bs_configs は試験の SCHOTT を LBSCH として引く
#[test]
fn roundtrip_all_zmx() {
    let tests = root().join("tests");
    let lbsch = std::env::temp_dir().join(format!("lb_sysfile_lbsch_{}", std::process::id()));
    std::fs::create_dir_all(&lbsch).unwrap();
    std::fs::copy(tests.join("glass/SCHOTT.AGF"), lbsch.join("LBSCH.AGF")).unwrap();
    let mut files = Vec::new();
    zmx_files(&tests, &mut files);
    assert!(files.len() >= 111, "{}", files.len());
    for f in &files {
        let dir = f.parent().unwrap().to_path_buf();
        let dirs = vec![
            dir.clone(),
            dir.parent().unwrap().to_path_buf(),
            tests.join("glass"),
            lbsch.clone(),
        ];
        let opt = LoadOptions {
            cfg: 1,
            catalog_dirs: dirs.clone(),
            ..Default::default()
        };
        let name = f.strip_prefix(&tests).unwrap().display().to_string();
        let s = zmx::load_with(f, &opt).unwrap_or_else(|e| panic!("{name}: {e}"));
        roundtrip(&name, &s, &dirs);
    }
    std::fs::remove_dir_all(&lbsch).ok();
}

/// 例題: examples/lenses の .zmx とプロジェクト（カタログのレンズも引く）
#[test]
fn roundtrip_examples() {
    let glass = vec![root().join("tests/glass")];
    let mut n = 0;
    let mut files = Vec::new();
    zmx_files(&root().join("examples"), &mut files);
    let lens = [
        root().join("examples/lenses/LB"),
        root().join("examples/lenses/LBP"),
    ];
    for dir in ["examples/projects", "examples/combos"] {
        let mut v: Vec<_> = std::fs::read_dir(root().join(dir))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        v.sort();
        files.extend(v);
    }
    for f in &files {
        let o = lensbench_project::open::open(
            f,
            &lensbench_project::open::OpenSpec {
                glass_dirs: &glass,
                lens_catalogs: &lens,
                cfg: 1,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        roundtrip(&f.display().to_string(), &o.system, &glass);
        n += 1;
    }
    assert!(n >= 26, "{n}");
}

/// 版 2 の形: 内部の値（屈折率・ガラスの式・自動の半直径の値）は書かず、曲率は c、ガラスは名前とカタログの hash
/// 瞳のアポダイゼーション（ANA-05）も系ファイル v2 に残る（一様なら書かない）
#[test]
fn apodization_roundtrip() {
    use lensbench_core::pupil::Apodization;
    let dirs = vec![root().join("tests/glass")];
    for a in [
        Apodization::Uniform,
        Apodization::Gaussian(1.5),
        Apodization::Gaussian(0.0),
        Apodization::CosineCubed,
    ] {
        let mut s = load("A0");
        s.edit_raw(|rx| {
            rx.apodization = a;
            Ok(())
        })
        .unwrap();
        roundtrip(&format!("A0 {a:?}"), &s, &dirs);
        let txt = sysfile::to_string(&s, &[]).unwrap();
        assert_eq!(
            txt.contains("apodization"),
            a != Apodization::Uniform,
            "{a:?}"
        );
    }
}

#[test]
fn v2_shape() {
    let s = load("A0");
    let txt = sysfile::to_string(&s, &[]).unwrap();
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    assert_eq!(v["format_version"], 2);
    let sys = &v["system"];
    assert!(sys.get("glasses").is_none());
    for k in ["n", "n_base", "typ", "semi_auto", "asph"] {
        assert!(!txt.contains(&format!("\"{k}\"")), "{k}: {txt}");
    }
    let cats = sys["catalogs"].as_array().unwrap();
    assert!(!cats.is_empty());
    for c in cats {
        assert!(c["hash"].as_str().unwrap().starts_with("sha256:"), "{c}");
    }
    let rows = sys["surfaces"].as_array().unwrap();
    assert_eq!(rows[0]["object"], true);
    assert_eq!(rows.last().unwrap()["image"], true);
    assert!(rows.iter().any(|r| r["shape"]["conic"]["c"].is_number()));
    assert!(rows.iter().any(|r| r["medium"].is_string()));
    assert!(txt.len() < 8000, "{}", txt.len());
}

/// カタログのファイルが保存時と違えば注意を返し、値は今のカタログで求める。見つからなければ誤り
#[test]
fn catalog_hash_note() {
    let s = load("A0");
    let txt = sysfile::to_string(&s, &[]).unwrap();
    let dir = std::env::temp_dir().join(format!("lb_sysfile_cat_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    for c in v["system"]["catalogs"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let body = std::fs::read_to_string(root().join(format!("tests/glass/{name}.AGF"))).unwrap();
        std::fs::write(
            dir.join(format!("{name}.AGF")),
            format!("{body}\n! changed\n"),
        )
        .unwrap();
    }
    let (t, _, warn) = sysfile::from_str_with(&txt, std::slice::from_ref(&dir)).unwrap();
    assert!(
        warn.iter()
            .any(|w| w.contains("differs from when the file was saved")),
        "{warn:?}"
    );
    // 中身の値は同じカタログなので追跡は同じ
    assert_eq!(fingerprint(&s), fingerprint(&t));
    // 見つからないカタログは誤り
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    v["system"]["catalogs"][0]["name"] = serde_json::json!("NO_SUCH_CATALOG");
    let e = sysfile::from_str(&v.to_string()).unwrap_err();
    assert!(e.to_string().contains("NO_SUCH_CATALOG"), "{e}");
    // 符号はガラスが見つからない（.zmx の読込と同じ。GUI はカタログのフォルダを足す案内を出す）
    assert_eq!(
        e.code(),
        lensbench_core::error::code::GLASS_NOT_FOUND,
        "{e}"
    );
    // 系ファイルが無いのは入出力の誤り
    let e = sysfile::load(&dir.join("no_such.json")).unwrap_err();
    assert_eq!(e.code(), lensbench_core::error::code::FILE_NOT_FOUND, "{e}");
    std::fs::remove_dir_all(&dir).ok();
}

/// 系のガラスと値が同じカタログのファイルが無ければ、書くときに注意を返して hash を空にし、読むときに値を確かめられない
/// と注意を返す（.zmx を元のカタログが無いところ・違うところで系ファイルにしたとき）
#[test]
fn catalog_without_hash() {
    let mut s = load("A0");
    assert!(sysfile::save_notes(&s).is_empty());
    let name = s
        .surfaces
        .iter()
        .map(|d| d.mat.name())
        .find(|m| s.glasses.contains_key(m))
        .unwrap();
    let mut g = (*s.glasses).clone();
    let x = g.get_mut(&name).unwrap();
    x.c[0] *= 1.001;
    let cat = x.catalog.clone();
    s.edit_raw(|rx| {
        rx.glasses = std::sync::Arc::new(g);
        Ok(())
    })
    .unwrap();
    let notes = sysfile::save_notes(&s);
    assert!(
        notes.len() == 1 && notes[0].contains(&cat) && notes[0].contains(&name),
        "{notes:?}"
    );
    let txt = sysfile::to_string(&s, &[]).unwrap();
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let r = v["system"]["catalogs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == cat.as_str())
        .unwrap();
    // 空の hash は書かない
    assert!(r.get("hash").is_none(), "{r}");
    let (_, _, warn) = sysfile::from_str_with(&txt, &[root().join("tests/glass")]).unwrap();
    assert!(
        warn.iter()
            .any(|w| w.contains(&cat) && w.contains("no hash in the system file")),
        "{warn:?}"
    );
}

/// 実数の欄を整数の書き方（`3`）で書いても読める（Python・手で書いた系ファイル）。誤りには欄の道が付く（IOO-32）。
/// 知らないキーは誤り
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
        let before = v.to_string();
        ints(&mut v["system"]);
        let txt2 = v.to_string();
        assert_ne!(txt2, before, "{name}: 整数に書き直せていない");
        let (t, meta, warn) = sysfile::from_str(&txt2).unwrap();
        assert!(warn.is_empty(), "{name}: {warn:?}");
        assert_eq!(meta.hash, sysfile::system_hash(&s).unwrap(), "{name}");
        assert_eq!(fingerprint(&s), fingerprint(&t), "{name}");
    }
    let s = load("A0");
    let txt = sysfile::to_string(&s, &[]).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    v["system"]["surfaces"][1]["t"] = serde_json::json!("x");
    let e = sysfile::from_str(&v.to_string()).unwrap_err();
    let m = e.message().unwrap_or_default();
    assert!(m.contains("surfaces[1].t"), "{e}");
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    v["system"]["surfaces"][1]["n"] = serde_json::json!([1.5]);
    let e = sysfile::from_str(&v.to_string()).unwrap_err();
    let m = e.message().unwrap_or_default();
    assert!(m.contains("surfaces[1]") && m.contains("`n`"), "{e}");
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
    assert!(txt.contains("\"dn\": 0.0001"), "{txt}");
    let (t, meta, _) = sysfile::from_str(&txt).unwrap();
    assert_eq!(meta.coatings, vec![PathBuf::from("coat.json")]);
    assert_eq!(fingerprint(&s), fingerprint(&t));
    assert_eq!(t.pickups, s.pickups);
    assert_eq!(t.solves, s.solves);
    assert!(!s.solves.is_empty());
    assert_eq!(t.configs(), s.configs());
    assert_eq!(t.units.len(), s.units.len());
    assert!(t.find_unit("G").is_some());
    assert_eq!(
        fingerprint(&s.config(2).unwrap()),
        fingerprint(&t.config(2).unwrap())
    );
    for (j, (a, b)) in s.surfaces.iter().zip(&t.surfaces).enumerate() {
        assert_eq!((a.dn, &a.mat, s.n(j)), (b.dn, &b.mat, t.n(j)));
    }
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
    // 鏡に媒質は書けない、座標ブレークに形は書けない
    let mut v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    v["system"]["surfaces"][1]["action"] = serde_json::json!("reflect");
    v["system"]["surfaces"][1]["medium"] = serde_json::json!("N-BK7");
    assert!(sysfile::from_str(&v.to_string()).is_err());
    let dir = std::env::temp_dir().join(format!("lb_sysfile_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("b0.json");
    sysfile::save(&p, &s, &[PathBuf::from("c.json")]).unwrap();
    assert!(sysfile::is_system_file(&p));
    let (t, meta, _) = sysfile::load(&p).unwrap();
    assert_eq!(meta.coatings, vec![dir.join("c.json")]);
    assert_eq!(fingerprint(&s), fingerprint(&t));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 版 1（core の System の serde をそのまま書いた形）も読める
#[test]
fn reads_version_1() {
    fn json(v: ciborium::value::Value) -> serde_json::Value {
        use ciborium::value::Value as C;
        match v {
            C::Null => serde_json::Value::Null,
            C::Bool(b) => b.into(),
            C::Integer(i) => {
                let i: i128 = i.into();
                serde_json::json!(i as i64)
            }
            C::Float(f) if f.is_finite() => f.into(),
            C::Float(f) => serde_json::json!({"$f": if f > 0.0 { "inf" } else { "-inf" }}),
            C::Text(s) => s.into(),
            C::Array(a) => a.into_iter().map(json).collect(),
            C::Map(m) => m
                .into_iter()
                .map(|(k, v)| (k.into_text().unwrap(), json(v)))
                .collect::<serde_json::Map<_, _>>()
                .into(),
            C::Tag(_, v) => json(*v),
            _ => panic!(),
        }
    }
    for name in ["A0", "D0"] {
        let s = load(name);
        let sys = json(ciborium::value::Value::serialized(&s).unwrap());
        let v1 =
            serde_json::json!({"format": "lensbench-system", "format_version": 1, "system": sys});
        let (t, _, warn) = sysfile::from_str(&v1.to_string()).unwrap();
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(fingerprint(&s), fingerprint(&t), "{name}");
        // 書くのは版 2
        assert!(sysfile::to_string(&t, &[])
            .unwrap()
            .contains("\"format_version\": 2"));
    }
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
    // モデルガラスは値で書く
    assert!(j.contains("\"nd\": 1.5168"), "{j}");
    std::fs::remove_dir_all(&d).unwrap();
    let (back, _, _) = sysfile::from_str(&j).unwrap();
    let back = back
        .with_params(&[(Param::new(1, Kind::Thick), 7.0)])
        .unwrap();
    assert_eq!(back.surfaces[2].t, 16.0);
}
