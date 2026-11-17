//! 系のファイルの読み方（.zmx・プロジェクト・系ファイル）と、結果の書き出し・プラグインの一覧・膜の境界

use std::path::PathBuf;

use lensbench_core::catalog::LensCatalog;
use lensbench_core::coating::{self, Coatings};
use lensbench_core::{zmx, System};
use lensbench_export::table;
use lensbench_project::open::{FileKind, OpenSpec, Source};
use serde_json::{json, Value};

/// 系の読み方（CLI の系の引数、serve の load。reload で同じ読み方を繰り返す）
#[derive(Clone, Debug, Default)]
pub struct SysSpec {
    /// .zmx ファイル、プロジェクトファイル（.json）、系ファイル
    pub file: PathBuf,
    /// ガラスカタログを探すフォルダ
    pub glass_dir: Vec<PathBuf>,
    /// プロジェクトファイルのカタログレンズを探すレンズカタログ
    pub lens_catalog: Vec<PathBuf>,
    /// .zmx の多重構成の番号（1 始まり）
    pub cfg: usize,
    /// プロジェクトファイルの状態の名前
    pub state: Option<String>,
    /// プロジェクトファイルのビームスプリッターの経路
    pub path: Option<String>,
    /// 誤りの文に書く引数の名前（既定は CLI の旗。serve の load は [`lensbench_project::open::ArgNames::SERVE`]）
    pub names: lensbench_project::open::ArgNames,
    /// マクロのソルブを解けなかったら（途中で止まった・外して保存された値で固定した）誤りにする。既定（false）は
    /// OpticStudio と同じ値で読み、注意を出す（11/17 の決定）
    pub strict_macros: bool,
}

impl SysSpec {
    /// 系のファイルの読み方（[`lensbench_project::open::OpenSpec`]）
    pub fn open_spec(&self) -> OpenSpec<'_> {
        OpenSpec {
            glass_dirs: &self.glass_dir,
            lens_catalogs: &self.lens_catalog,
            cfg: self.cfg,
            state: self.state.as_deref(),
            split_path: self.path.as_deref(),
            wavelengths: None,
            names: self.names,
            // 機構部品の詳しい形の STEP は取った STEP のキャッシュから
            step_files: Some(&lensbench_cad::cached_step),
        }
    }
}

/// ファイルから系を読む（読んだときの注意は捨てる。利用者に見せる読み方は [`load_file_notes`]）。
/// ファイルが無いときは種類 io の誤り
pub fn load_file(c: &SysSpec) -> Result<System, lensbench_core::Error> {
    Ok(load_file_notes(c)?.0)
}

/// ファイルから系を読み、読んだときの注意（[`load_source_notes`]）と一緒に返す
pub fn load_file_notes(c: &SysSpec) -> Result<(System, Vec<String>), lensbench_core::Error> {
    let r = read_file_notes(c)?;
    Ok((r.sys, r.notes))
}

/// [`read_file_notes`] で読んだもの
pub struct ReadFile {
    /// 読んだファイル（serve が持っておき、別の状態・経路をそこから引く）
    pub src: Source,
    pub sys: System,
    /// 読んだときの注意（[`load_source_notes`]）
    pub notes: Vec<String>,
    /// プロジェクトの id のあるブロック・群ごとの系の面の範囲（プロジェクトでなければ空）
    pub blocks: Vec<lensbench_project::BlockSpans>,
    /// プロジェクトの展開した系の面ごとの名前（[`lensbench_project::open::Opened::faces`]。プロジェクトでなければ空）
    pub faces: Faces,
}

/// 展開した系の面ごとの名前（[`lensbench_project::SurfOrigin::face`]）
pub type Faces = Vec<Option<lensbench_project::FaceId>>;

/// [`load_file_notes`] で、読んだファイルとブロックの面の範囲も返す
pub fn read_file_notes(c: &SysSpec) -> Result<ReadFile, lensbench_core::Error> {
    if c.cfg == 0 {
        return Err(lensbench_core::Error::Invalid(format!(
            "{} must be 1 or more",
            c.names.cfg
        )));
    }
    let src = Source::read(&c.file)?;
    let o = open_source(&src, c)?;
    Ok(ReadFile {
        src,
        sys: o.system,
        notes: o.warnings,
        blocks: o.blocks,
        faces: o.faces,
    })
}

/// 読んだファイル src から、c の構成・状態・経路の系を読む（注意は捨てる）
pub fn load_source(src: &Source, c: &SysSpec) -> Result<System, lensbench_core::Error> {
    if c.cfg == 0 {
        return Err(lensbench_core::Error::Invalid(format!(
            "{} must be 1 or more",
            c.names.cfg
        )));
    }
    Ok(load_source_notes(src, c)?.0)
}

/// [`load_source`] で、展開した系の面ごとの名前も返す（別の状態・経路へ変更を当て直す）
pub(crate) fn load_source_faces(
    src: &Source,
    c: &SysSpec,
) -> Result<(System, Vec<String>, Faces), lensbench_core::Error> {
    let o = open_source(src, c)?;
    Ok((o.system, o.warnings, o.faces))
}

/// 読んだファイル src から、c の構成・状態・経路の系を読む。注意（プロジェクトの展開・分散式・マクロ・
/// 取り込み）は返す（CLI は標準エラーへ、serve は load の返事と解析の notes へ、GUI は履歴へ）
pub fn load_source_notes(
    src: &Source,
    c: &SysSpec,
) -> Result<(System, Vec<String>), lensbench_core::Error> {
    let o = open_source(src, c)?;
    Ok((o.system, o.warnings))
}

/// [`load_source_notes`] で、ブロックの面の範囲・面の名前も返す（warnings に系そのものの注意も足す）
fn open_source(
    src: &Source,
    c: &SysSpec,
) -> Result<lensbench_project::open::Opened, lensbench_core::Error> {
    let o = src.open(&c.open_spec())?;
    if c.strict_macros {
        // 注意の文は 1 つずつ訳せるよう、最初の 1 つだけを {e} に入れる（数は前に）
        let w = o.system.macro_warnings();
        if let Some(e) = w.first() {
            let n = w.len();
            return Err(lensbench_core::Error::Invalid(if n == 1 {
                format!("a macro solve can't be solved (strict macros): {e}")
            } else {
                format!("{n} macro solves can't be solved (strict macros); the first: {e}")
            }));
        }
    }
    let mut o = o;
    let n = system_notes(&o.system);
    o.warnings.extend(n);
    Ok(o)
}

/// 系そのものが持つ注意: 分散式・マクロ・取り込み（.zmx の読めなかった行・同梱のカタログなど）
pub fn system_notes(s: &System) -> Vec<String> {
    s.dispersion_warnings()
        .into_iter()
        .chain(s.macro_warnings())
        .chain(s.import_notes.iter().cloned())
        .collect()
}

/// [`load_file_notes`] の注意を標準エラーへ出し、結果の出どころ（[`crate::provenance`]）に系を記録する
/// （CLI の解析の系の読み方）
pub fn load_file_noted(c: &SysSpec) -> Result<System, lensbench_core::Error> {
    let (s, notes) = load_file_notes(c)?;
    for w in notes {
        eprintln!("{}", lensbench_core::i18n::warning(&w));
    }
    crate::provenance::set(&s);
    Ok(s)
}

/// プロジェクトファイルのビームスプリッターの経路の一覧（.zmx・系ファイル・BS なしなら空）
pub fn split_paths(c: &SysSpec) -> Result<Vec<String>, lensbench_core::Error> {
    if FileKind::of(&c.file) != FileKind::Project {
        return Ok(Vec::new());
    }
    Source::read(&c.file)?.paths(&c.open_spec())
}

/// プロジェクトファイルの状態の名前（.zmx・系ファイル・読めないプロジェクトなら空）
pub fn state_names(c: &SysSpec) -> Vec<String> {
    if FileKind::of(&c.file) != FileKind::Project {
        return Vec::new();
    }
    Source::read(&c.file)
        .map(|s| s.states())
        .unwrap_or_default()
}

/// 公差のモンテカルロに渡す、プロジェクトファイルのカタログの玉（ブロックの id → メーカー）と裏返して置いた玉の id
/// （.zmx・系ファイル・読めないプロジェクトなら空）
pub fn tolerance_vendors(
    c: &SysSpec,
) -> (
    std::collections::HashMap<String, String>,
    std::collections::HashSet<String>,
) {
    if FileKind::of(&c.file) != FileKind::Project {
        return Default::default();
    }
    Source::read(&c.file)
        .map(|s| tolerance_vendors_in(&s))
        .unwrap_or_default()
}

/// [`tolerance_vendors`] を読んだファイルから
pub fn tolerance_vendors_in(
    src: &Source,
) -> (
    std::collections::HashMap<String, String>,
    std::collections::HashSet<String>,
) {
    src.project()
        .map(|p| {
            (
                lensbench_design::tolerance::catalog_vendors(p),
                lensbench_design::tolerance::reversed_lenses(p),
            )
        })
        .unwrap_or_default()
}

/// 重ね描き・多重構成の書き出しで並べる構成
#[derive(Clone, Default, Debug)]
pub struct Overlay {
    /// .zmx の多重構成（1 始まり、カンマ区切り、all で全部）
    pub cfgs: Option<String>,
    /// プロジェクトの状態（名前のカンマ区切り、all で全部）
    pub states: Option<String>,
    /// ビームスプリッターの経路（T・R の文字列のカンマ区切り、all でとれる経路の全部）
    pub paths: Option<String>,
}

impl Overlay {
    /// どれか選んだか（選んでいなければ今の系 1 つ）
    pub fn any(&self) -> bool {
        self.cfgs.is_some() || self.states.is_some() || self.paths.is_some()
    }
}

/// 重ね描きする構成の系と名前（重ね描きでなければ今の系 1 つ、名前は空）と、読んだときの注意
pub fn overlay_systems(c: &SysSpec, l: &Overlay) -> lensbench_core::Result<Overlaid> {
    if !l.any() {
        let (s, notes) = load_file_notes(c)?;
        crate::provenance::set(&s);
        return Ok((vec![(String::new(), s)], notes));
    }
    let (v, notes) = overlay_in(c, l, None)?;
    Ok((v.into_iter().map(|o| (o.name, o.sys)).collect(), notes))
}

/// [`overlay_in`] の系と、読んだときの注意
pub(crate) type OverlaidIn = (Vec<OverlaidSys>, Vec<String>);

/// [`overlay_in`] の系 1 つ
pub(crate) struct OverlaidSys {
    pub name: String,
    pub sys: System,
    /// 読み込んだ後の変更がもう入っているか
    pub edited: bool,
    /// プロジェクトの展開した系の面ごとの名前（変更を当て直す。ほかは空）
    pub faces: Faces,
}

impl OverlaidSys {
    fn new(name: String, sys: System, edited: bool) -> Self {
        OverlaidSys {
            name,
            sys,
            edited,
            faces: Vec::new(),
        }
    }
}

/// 読み込んだ系（serve）: 読んだときのファイル src と、読み込んだ後の変更の入った系 cur
pub(crate) type Loaded<'a> = (&'a Source, &'a System);

/// [`overlay_systems`] の重ね描きの系。loaded があればファイルを読み直さずに src から引き、.zmx の多重構成の表を
/// 読めていれば cur の表から、今の状態・経路は cur そのものを使う。系ごとの bool はそうして読み込んだ後の
/// 変更がもう入っているか（false なら呼ぶ側が当て直す）
pub(crate) fn overlay_in(
    c: &SysSpec,
    l: &Overlay,
    loaded: Option<Loaded>,
) -> lensbench_core::Result<OverlaidIn> {
    let kind = FileKind::of(&c.file);
    let is_json = kind != FileKind::Zmx;
    if kind == FileKind::SystemFile {
        return Err("a system file can't draw overlaid configurations (--cfgs, --states and --beam-paths are only for .zmx and projects)".into());
    }
    if let Some(v) = &l.cfgs {
        if is_json {
            return Err("--cfgs is only for .zmx (projects use --states and --beam-paths)".into());
        }
        // 多重構成の表を読めていれば、変更の入った系の表から
        if let Some(t) = loaded.and_then(|(_, cur)| cur.configs.as_ref().map(|t| (cur, t.n))) {
            let (cur, n) = t;
            let mut out = Vec::new();
            for k in index_list(v, n, "configuration")? {
                out.push(OverlaidSys::new(
                    format!("C{}", k + 1),
                    cur.config(k + 1)?,
                    true,
                ));
            }
            return Ok((out, Vec::new()));
        }
        let opt = zmx::LoadOptions {
            catalog_dirs: c.glass_dir.clone(),
            ..Default::default()
        };
        // 読み込んだときのファイルの中身から（serve で読み込んでいなければ読む）
        let read;
        let src = match loaded {
            Some((s, _)) => s,
            None => {
                read = Source::read(&c.file)?;
                &read
            }
        };
        let n = src.configurations()?;
        let ks: Vec<usize> = index_list(v, n, "configuration")?
            .into_iter()
            .map(|k| k + 1)
            .collect();
        let ss = src.zmx_configs(&opt, &ks)?;
        return Ok((
            ks.iter()
                .map(|k| format!("C{k}"))
                .zip(ss)
                .map(|(n, s)| OverlaidSys::new(n, s, false))
                .collect(),
            Vec::new(),
        ));
    }
    if !is_json {
        return Err(
            "--states and --beam-paths are only for project files (.json); .zmx uses --cfgs".into(),
        );
    }
    let read;
    let src = match loaded {
        Some((s, _)) => s,
        None => {
            read = Source::read(&c.file)?;
            &read
        }
    };
    let Some(p) = src.project() else {
        return Err("--states and --beam-paths are only for project files (.json)".into());
    };
    let states: Vec<Option<String>> = match &l.states {
        None => vec![c.state.clone()],
        Some(v) if v.eq_ignore_ascii_case("all") => {
            let n: Vec<Option<String>> = p
                .state_names()
                .into_iter()
                .map(|x| Some(x.to_string()))
                .collect();
            if n.is_empty() {
                return Err("the project has no states".into());
            }
            n
        }
        Some(v) => v.split(',').map(|x| Some(x.trim().to_string())).collect(),
    };
    let cat = LensCatalog::load_registered(&c.lens_catalog)?;
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let env = lensbench_project::Env {
        // 機構部品の詳しい形の STEP は取った STEP のキャッシュから
        step_files: Some(&lensbench_cad::cached_step),
        base: c.file.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
        glass_dirs: c.glass_dir.clone(),
        catalog: Some(&look),
    };
    let mut out = Vec::new();
    let mut notes = Vec::new();
    // 状態・経路ごとの展開で、ブロックが読むファイル・カタログの玉を使い回す（読んだときのものを引き継ぐ）
    let cache = match src {
        Source::Project { cache, .. } => cache.clone(),
        _ => Default::default(),
    };
    for st in &states {
        let ps = match &st {
            Some(n) => p.with_state(n)?,
            None => p.clone(),
        };
        let paths: Vec<Option<String>> = match &l.paths {
            None => vec![c.path.clone()],
            Some(v) if v.eq_ignore_ascii_case("all") => {
                let v = lensbench_project::paths_cached(&ps, &env, &cache)?;
                if v.is_empty() {
                    return Err("the project has no beam splitters".into());
                }
                v.into_iter().map(Some).collect()
            }
            Some(v) => v.split(',').map(|x| Some(x.trim().to_string())).collect(),
        };
        for pa in &paths {
            let mut name: Vec<&str> = Vec::new();
            if l.states.is_some() {
                name.extend(st.as_deref());
            }
            if l.paths.is_some() {
                name.extend(pa.as_deref());
            }
            // 今の状態・経路は読み込んだ系そのもの
            if let Some((_, cur)) = loaded.filter(|_| *st == c.state && *pa == c.path) {
                out.push(OverlaidSys::new(name.join("/"), cur.clone(), true));
                continue;
            }
            let (s, warn, map) = lensbench_project::expand(
                &ps,
                &env,
                &lensbench_project::ExpandOpts {
                    path: pa.as_deref(),
                    cache: Some(&cache),
                    ..Default::default()
                },
            )
            .map(|x| (x.system, x.notes, x.map))
            .map_err(|x| match pa {
                Some(q) => format!("path {q}: {x}"),
                None => x.to_string(),
            })?;
            // 状態・経路ごとに同じ注意が出るので 1 度だけ
            for w in warn {
                if !notes.contains(&w) {
                    notes.push(w);
                }
            }
            out.push(OverlaidSys {
                name: name.join("/"),
                sys: s,
                edited: false,
                faces: map.into_iter().map(|o| o.face).collect(),
            });
        }
    }
    Ok((out, notes))
}

/// 重ね描きする系（名前と系）と、読んだときの注意
pub type Overlaid = (Vec<(String, System)>, Vec<String>);

/// "1,3" のような 1 始まりの番号の並びを 0 始まりにする。"all" は 0..n
pub fn index_list(s: &str, n: usize, what: &str) -> lensbench_core::Result<Vec<usize>> {
    if s.eq_ignore_ascii_case("all") {
        return Ok((0..n).collect());
    }
    s.split(',')
        .map(|t| {
            let k: usize = t
                .trim()
                .parse()
                .map_err(|_| lensbench_core::Error::from(format!("{what} number {t}")))?;
            if k == 0 || k > n {
                return Err(format!("no {what} {k} (1 to {n})").into());
            }
            Ok(k - 1)
        })
        .collect()
}

/// --save: 解析の結果を書き出す（json はそのまま、csv・npz は表にして）
pub fn save_value(p: &std::path::Path, v: &Value) -> lensbench_core::Result<()> {
    let json = p
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    if json {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.insert("provenance".into(), crate::provenance::value());
        }
        let s = serde_json::to_string_pretty(&v).expect("a JSON value serializes");
        Ok(lensbench_core::vfs::write(p, s + "\n").map_err(|x| format!("{}: {x}", p.display()))?)
    } else {
        Ok(table::from_json(v).save(p)?)
    }
}

/// 同梱のプラグインを登録した表（exe に組み込む分）に、今の拡張の表
/// （[`lensbench_core::ext::current`]、この CLI を組み込んだ exe が足した分）のプラグインを重ねたもの
pub fn builtin_plugins() -> lensbench_core::plugin::Registry {
    lensbench_design::plugins::builtin()
}

pub fn plugins_json() -> Value {
    let reg = builtin_plugins();
    let pairs = |v: &[(String, String)]| -> Value {
        v.iter()
            .map(|(k, d)| json!({ "name": k, "doc": d }))
            .collect()
    };
    let list: Vec<Value> = reg
        .list()
        .into_iter()
        .filter_map(|(name, kind)| {
            let pl = reg.get(&name).ok()?;
            let s = pl.spec();
            let params: Vec<Value> = pl.params().iter().map(crate::params::param_json).collect();
            Some(json!({
                "name": name, "kind": kind.as_str(), "doc": s.doc, "needs_system": s.needs_system,
                "params": params, "inputs": pairs(&s.inputs), "outputs": s.outputs.iter().map(|o| json!({
                    "name": o.name, "dtype": o.dtype, "shape": o.shape, "unit": o.unit, "doc": o.doc,
                })).collect::<Vec<_>>(),
            }))
        })
        .collect();
    json!({ "plugins": list })
}

/// 膜の名前で自動で読んだ膜（膜, パス, 注記）
pub type AutoCoat = (String, String, String);

/// 系の膜の境界（[`coat_interfaces`]）
pub struct CoatInterfaces {
    /// 境界の並び（surface・key・kind・note）
    pub list: Vec<Value>,
    /// 実測表が要る境界の KEY
    pub needs: Vec<String>,
    /// 膜の名前で自動で読んだ膜
    pub auto: Vec<AutoCoat>,
}

/// 系 s の膜の境界（CLI の coatings の 1 構成分）
pub fn coat_interfaces(s: &System, tables: &[PathBuf]) -> lensbench_core::Result<CoatInterfaces> {
    let e = |e: lensbench_core::Error| e.to_string();
    let mut c = Coatings::for_system(s).map_err(e)?;
    for t in tables {
        c.load_file(t).map_err(e)?;
    }
    let mut auto = Vec::new();
    for (coat, p) in &c.auto {
        let p = p.display().to_string();
        // 注記: モデルの NOTE、近似の多層膜（.lbcs）なら近似の膜であること
        let note = match c.models.get(coat) {
            Some(m) => m.note.clone(),
            None if p.to_ascii_lowercase().ends_with(".lbcs") => {
                "approximate coating (a multilayer fitted to the spectral curve)".to_string()
            }
            None => String::new(),
        };
        auto.push((coat.clone(), p, note));
    }
    let (keys, _) = coating::media_keys(s);
    let mut v = Vec::new();
    let mut need = Vec::new();
    for (j, k) in keys.iter().enumerate() {
        let Some(k) = k else { continue };
        let kind = c.kind(k);
        let mut it = json!({ "surface": j, "key": k, "kind": kind.name() });
        if kind == coating::CoatKind::Model {
            let coat = k.split('|').next().unwrap_or("");
            if let Some(m) = c.models.get(coat).filter(|m| !m.note.is_empty()) {
                it["note"] = json!(m.note);
            }
        }
        v.push(it);
        if kind.needs_table() && !need.contains(k) {
            need.push(k.clone());
        }
    }
    Ok(CoatInterfaces {
        list: v,
        needs: need,
        auto,
    })
}

#[cfg(test)]
mod strict_tests {
    use super::*;

    /// 途中で止まるマクロのソルブ: 既定は注意だけで読み、strict_macros なら誤り
    #[test]
    fn strict_macros_turns_the_macro_warning_into_an_error() {
        let d = std::env::temp_dir().join(format!("lb_strict_macros_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("LBMID.ZPL"),
            "SOLVERETURN -0.03\nx = nosuchvar\nSOLVERETURN 1\n",
        )
        .unwrap();
        let zmx = "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nENPD 10\nFTYP 0 0 3 1 0 0 0\n\
XFLN 0 0 0\nYFLN 0 14 20\nFWGN 1 1 1\nWAVM 1 0.5876 1\nPWAV 1\n\
SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ INFINITY\n\
SURF 1\n  STOP\n  TYPE STANDARD\n  CURV 0.02\n  DISZ 5\n  GLAS ___BLANK 1 0 1.5168 64.17 0 0 0 0 0\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 2\n  TYPE STANDARD\n  CURV -1.23e-2 12 0 0 0 \"LBMID\"\n  DISZ 45\n  DIAM 6 1 0 0 1 \"\"\n\
SURF 3\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n";
        let f = d.join("s.zmx");
        std::fs::write(&f, zmx).unwrap();
        let mut c = SysSpec {
            file: f,
            cfg: 1,
            ..Default::default()
        };
        let (s, notes) = load_file_notes(&c).unwrap();
        assert_eq!(s.surfaces[2].c, -0.03);
        assert!(notes.iter().any(|n| n.contains("LBMID")), "{notes:?}");
        c.strict_macros = true;
        let e = load_file_notes(&c).unwrap_err().to_string();
        assert!(e.contains("strict macros") && e.contains("LBMID"), "{e}");
        // 和訳で外の文と中の注意の両方が訳される（中の注意は「: 」を跨ぐ {e}）
        let ja = lensbench_core::i18n::translate(&e);
        assert!(
            ja.contains("厳密な読込") && ja.contains("面 2 の") && !ja.contains("macro solve"),
            "{ja}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
