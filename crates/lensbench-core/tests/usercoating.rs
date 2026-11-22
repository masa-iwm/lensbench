//! 外から足す膜（膜の一覧・膜のファイル形式）: 引ける・係数が同梱の膜と同じ・同じ名前の優先の順と注意

use lensbench_core::coating::{CoatKind, Coatings, C64};
use lensbench_core::ext::{self, Ext};
use lensbench_core::usercoating::{check_coatings, CoatingSet, CoatingSource};
use lensbench_core::{Result, System};
use lensbench_formats::zmx::{self, LoadOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn a0() -> System {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let opt = LoadOptions {
        cfg: 1,
        catalog_dirs: vec![root.join("tests/glass")],
        ..Default::default()
    };
    zmx::load_with(&root.join("tests/refs/A0/system.zmx"), &opt).unwrap()
}

/// 最初の境界の面に膜 coat を付けた系と、その面・境界の鍵
fn with_coat(coat: &str, coating_file: Option<PathBuf>) -> (System, usize, String) {
    let mut s = a0();
    let w = s.primary;
    let j = (1..s.surfaces.len())
        .find(|&j| {
            !s.surfaces[j].ignore
                && !s.surfaces[j].is_coord_break()
                && s.index_before(j, w) != s.n(j)[w]
        })
        .unwrap();
    s.edit_raw(|p| {
        p.surfaces[j].coat = coat.into();
        p.coating_file = coating_file;
        Ok(())
    })
    .unwrap();
    let key = format!("{coat}|AIR|{}", s.surfaces[j].mat.label());
    (s, j, key)
}

const AR: &str = "MATE MGF2X\n0.4 1.38 0\n0.8 1.38 0\nCOAT EXT_AR\nMGF2X 0.25\n";

fn coef(c: &Coatings, s: &System, j: usize, key: &str) -> [C64; 2] {
    let w = s.primary;
    c.prepare(s, key, s.wavelengths[w], s.index_before(j, w), s.n(j)[w])
        .unwrap()
        .eval(0.2)
        .unwrap()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lensbench-usercoat-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_registered_coating_is_found_and_matches_the_coating_file() {
    let set = CoatingSet::parse_dat(AR).unwrap();
    check_coatings(&set).unwrap();
    let mut e = Ext::default();
    e.add_coating_catalog("Example", set);
    // 系の名前は小文字（引くときは大文字小文字を問わない）
    let (s, j, key) = with_coat("ext_ar", None);
    // 表を渡さない計算では膜が無い
    assert_eq!(
        Coatings::for_system(&s).unwrap().kind(&key),
        CoatKind::Missing
    );
    let got = ext::with(Arc::new(e), || {
        let c = Coatings::for_system(&s).unwrap();
        assert_eq!(c.kind(&key), CoatKind::Stack);
        assert_eq!(
            c.registered,
            [("ext_ar".to_string(), "Example".to_string())]
        );
        assert!(c.notes.is_empty(), "{:?}", c.notes);
        coef(&c, &s, j, &key)
    });
    // 同じ膜を膜ファイル（COATING.DAT の形式）で書いたときと同じ係数
    let d = tmp("same");
    std::fs::write(d.join("COATING.DAT"), AR.replace("EXT_AR", "ext_ar")).unwrap();
    let (s2, _, _) = with_coat("ext_ar", Some(d.join("COATING.DAT")));
    let want = coef(&Coatings::for_system(&s2).unwrap(), &s2, j, &key);
    assert!((got[0] - want[0]).norm() < 1e-14 && (got[1] - want[1]).norm() < 1e-14);
}

#[test]
fn the_registered_catalog_comes_before_the_coating_file_and_the_folder() {
    let mut e = Ext::default();
    e.add_coating_catalog("EXAMPLE", CoatingSet::parse_dat(AR).unwrap());
    let e = Arc::new(e);
    // 膜ファイルに同じ名前の膜: 一覧を使い注意（膜ファイルのほかの膜が使う材料は変えない）
    let d = tmp("order");
    std::fs::write(
        d.join("COATING.DAT"),
        "MATE MGF2X\n0.4 1.5 0\n0.8 1.5 0\nCOAT EXT_AR\nMGF2X 0.1\n",
    )
    .unwrap();
    let (s, _, key) = with_coat("EXT_AR", Some(d.join("COATING.DAT")));
    ext::with(e.clone(), || {
        let c = Coatings::for_system(&s).unwrap();
        assert_eq!(c.kind(&key), CoatKind::Stack);
        assert_eq!(c.registered.len(), 1);
        assert_eq!(c.file.coat["EXT_AR"][0].d, 0.25);
        assert_eq!(c.file.mate["MGF2X"][0][1], 1.5);
        assert_eq!(c.notes.len(), 1, "{:?}", c.notes);
        assert!(
            c.notes[0].contains("in the coating file is not used"),
            "{:?}",
            c.notes
        );
    });
    // 膜ファイルに無く、フォルダに同じ名前のモデル: 一覧を使い注意。フォルダのファイルは読まない（壊れていても止まらない）
    std::fs::write(d.join("COATING.DAT"), "MATE MGF2X\n0.4 1.5 0\n0.8 1.5 0\n").unwrap();
    std::fs::write(d.join("EXT_AR.lbcm"), "broken").unwrap();
    ext::with(e.clone(), || {
        let c = Coatings::for_system(&s).unwrap();
        assert_eq!(c.kind(&key), CoatKind::Stack);
        assert_eq!(c.registered.len(), 1);
        assert!(c.auto.is_empty());
        assert_eq!(c.notes.len(), 1, "{:?}", c.notes);
        assert!(c.notes[0].contains("EXT_AR.lbcm"), "{:?}", c.notes);
        // 膜ファイルの同じ名前の違う材料は変えず、一覧の材料は名前を替えて入れる
        assert_eq!(c.file.mate["MGF2X"][0][1], 1.5);
        assert_eq!(c.file.coat["EXT_AR"][0].mat, "MGF2X@EXAMPLE");
        assert_eq!(c.file.mate["MGF2X@EXAMPLE"][0][1], 1.38);
    });
}

/// 素通しの膜のモデル（LBCM 2）
fn lbcm(name: &str) -> String {
    let mut t = format!("LBCM 2\nCOAT {name}\nD 0.0\nBMAX 1.3\n");
    for w in [0.4, 0.6, 0.8] {
        t += &format!("WAVE {w}\n");
        for i in 0..=13 {
            let b = 0.1 * i as f64;
            t += &format!("{b:?} 1 0 0 0 0 0 1 0 0 1 0 0 0 0 0 1 0 0\n");
        }
    }
    t
}

#[test]
fn an_explicit_model_wins_over_a_registered_stack() {
    let mut e = Ext::default();
    e.add_coating_catalog("EXAMPLE", CoatingSet::parse_dat(AR).unwrap());
    let (s, _, key) = with_coat("EXT_AR", None);
    let d = tmp("explicit");
    std::fs::write(d.join("m.lbcm"), lbcm("EXT_AR")).unwrap();
    std::fs::write(d.join("other.lbcm"), lbcm("OTHER")).unwrap();
    ext::with(Arc::new(e), || {
        let mut c = Coatings::for_system(&s).unwrap();
        assert_eq!(c.kind(&key), CoatKind::Stack);
        // 関係のない膜のモデルを読んでも、一覧の層はそのまま
        c.load_file(&d.join("other.lbcm")).unwrap();
        assert_eq!(c.kind(&key), CoatKind::Stack);
        c.load_file(&d.join("m.lbcm")).unwrap();
        assert_eq!(c.kind(&key), CoatKind::Model);
        // 明示して置き換えた膜は、一覧から入れた記録に残らない
        assert!(c.registered.is_empty());
    });
    // 一覧の同じ名前の膜に層とモデルの両方があると約束の誤り。検査を通さずに足しても、関係のないファイルを読んで
    // 一覧の層が一覧のモデルに替わることはない
    let mut both = CoatingSet::parse_dat(AR).unwrap();
    both.add_models_text(&lbcm("EXT_AR")).unwrap();
    assert!(check_coatings(&both).is_err());
    let mut e = Ext::default();
    e.add_coating_catalog("EXAMPLE", both);
    ext::with(Arc::new(e), || {
        let mut c = Coatings::for_system(&s).unwrap();
        assert_eq!(c.kind(&key), CoatKind::Stack);
        c.load_file(&d.join("other.lbcm")).unwrap();
        assert_eq!(c.kind(&key), CoatKind::Stack);
    });
}

/// 見本の形式: 1 行目 `LBX`、続けて COATING.DAT の形式
struct Lbx;
impl CoatingSource for Lbx {
    fn extensions(&self) -> &[&str] {
        &["lbx"]
    }
    fn load(&self, path: &Path) -> Result<CoatingSet> {
        let t = std::fs::read_to_string(path)
            .map_err(|e| lensbench_core::Error::Io(format!("{}: {e}", path.display())))?;
        let Some(body) = t.strip_prefix("LBX\n") else {
            lensbench_core::invalid!("{}: not an LBX file", path.display());
        };
        CoatingSet::parse_dat(body)
    }
}

#[test]
fn a_registered_format_is_read_from_the_coating_folder_and_explicitly() {
    let mut e = Ext::default();
    e.add_coating_source(Arc::new(Lbx));
    let e = Arc::new(e);
    let d = tmp("format");
    std::fs::write(d.join("COATING.DAT"), "").unwrap();
    std::fs::write(d.join("Ext_Ar.LBX"), format!("LBX\n{AR}")).unwrap();
    let (s, _, key) = with_coat("EXT_AR", Some(d.join("COATING.DAT")));
    // 形式を足していなければ読まない
    assert_eq!(
        Coatings::for_system(&s).unwrap().kind(&key),
        CoatKind::Missing
    );
    ext::with(e.clone(), || {
        let c = Coatings::for_system(&s).unwrap();
        assert_eq!(c.kind(&key), CoatKind::Stack);
        assert_eq!(c.auto.len(), 1);
        assert!(c.auto[0].1.ends_with("Ext_Ar.LBX"));
        // 同梱の形式（.lbcm）があればそちらが先
        std::fs::write(d.join("EXT_AR.lbcm"), lbcm("EXT_AR")).unwrap();
        let c = Coatings::for_system(&s).unwrap();
        assert!(c.auto[0].1.ends_with("EXT_AR.lbcm"), "{:?}", c.auto);
        std::fs::remove_file(d.join("EXT_AR.lbcm")).unwrap();
    });
    // 明示して読むファイルも拡張子で選ぶ
    let (s2, _, key2) = with_coat("EXT_AR", None);
    ext::with(e, || {
        let mut c = Coatings::for_system(&s2).unwrap();
        assert_eq!(c.kind(&key2), CoatKind::Missing);
        assert_eq!(c.load_file(&d.join("Ext_Ar.LBX")).unwrap(), 1);
        assert_eq!(c.kind(&key2), CoatKind::Stack);
    });
}

#[test]
fn the_check_rejects_broken_sets() {
    assert!(check_coatings(&CoatingSet::parse_dat("COAT I.5\nX 0.25\n").unwrap()).is_err());
    assert!(check_coatings(
        &CoatingSet::parse_dat("MATE X\n0.6 1.4 0\n0.5 1.4 0\nCOAT A\nX 0.25\n").unwrap()
    )
    .is_err());
    assert!(check_coatings(
        &CoatingSet::parse_dat("MATE X\n0.5 1.4 0.1\nCOAT A\nX 0.25\n").unwrap()
    )
    .is_err());
    let mut s = CoatingSet::new();
    s.add_layers("A", Vec::new());
    assert!(check_coatings(&s).is_err());
}

#[test]
fn the_table_lists_and_removes_catalogs() {
    let mut e = Ext::default();
    e.add_coating_catalog("B", CoatingSet::parse_dat("COAT Y\nX 0.1\n").unwrap());
    e.add_coating_catalog("a", CoatingSet::parse_dat(AR).unwrap());
    e.add_coating_catalog("A", CoatingSet::parse_dat(AR).unwrap());
    assert_eq!(e.coating_catalogs(), ["A", "B"]);
    assert_eq!(e.find_coating("ext_ar").unwrap().name, "A");
    assert!(e.remove_coating_catalog("a"));
    assert!(e.find_coating("EXT_AR").is_none());
    let mut f = Ext::default();
    f.merge(&e);
    assert_eq!(f.coating_catalogs(), ["B"]);
}

#[test]
fn a_set_reads_model_measured_and_dat_files() {
    let d = tmp("addfile");
    std::fs::write(d.join("a.lbcm"), lbcm("ZEC_A")).unwrap();
    std::fs::write(d.join("b.dat"), AR).unwrap();
    let mut t = String::from("LBCT 1\nKEY ZEC_B|AIR|N-BK7\nWAVE 0.5\n");
    for a in 0..=10 {
        t += &format!("{a}.0 0.9 0 0.9 0\n");
    }
    std::fs::write(d.join("c.lbct"), t).unwrap();
    let mut s = CoatingSet::new();
    assert_eq!(s.add_file(&d.join("a.lbcm")).unwrap(), 1);
    assert_eq!(s.add_file(&d.join("b.dat")).unwrap(), 1);
    assert_eq!(s.add_file(&d.join("c.lbct")).unwrap(), 1);
    assert_eq!(s.names(), ["EXT_AR", "ZEC_A", "ZEC_B"]);
    check_coatings(&s).unwrap();
    let mut e = Ext::default();
    e.add_coating_catalog("X", s);
    e.add_coating_catalog("Y", CoatingSet::parse_dat("COAT ZEC_A\nM 0.1\n").unwrap());
    assert_eq!(e.coating_names(), ["EXT_AR", "ZEC_A", "ZEC_B"]);
}
