//! 系のファイルの読み方（.zmx・プロジェクト・系ファイル）と、結果の書き出し・プラグインの一覧・膜の境界

use std::path::PathBuf;

use lensbench_core::catalog::LensCatalog;
use lensbench_core::coating::{self, Coatings};
use lensbench_core::{glass, zmx, System};
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
            // 機構部品の詳しい形の STEP は取った STEP のキャッシュから（PRJ-21）
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
    if c.cfg == 0 {
        return Err(lensbench_core::Error::Invalid(format!(
            "{} must be 1 or more",
            c.names.cfg
        )));
    }
    load_source_notes(&Source::read(&c.file)?, c)
}

/// 読んだファイル src から、c の構成・状態・経路の系を読む。注意（プロジェクトの展開・分散式・マクロ・
/// 取り込み）は返す（CLI は標準エラーへ、serve は load の返事と解析の notes へ、GUI は履歴へ。CLI-10）
pub fn load_source_notes(
    src: &Source,
    c: &SysSpec,
) -> Result<(System, Vec<String>), lensbench_core::Error> {
    let o = src.open(&c.open_spec())?;
    let mut notes = o.warnings;
    notes.extend(system_notes(&o.system));
    Ok((o.system, notes))
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
        .ok()
        .and_then(|s| {
            s.project().map(|p| {
                (
                    lensbench_design::tolerance::catalog_vendors(p),
                    lensbench_design::tolerance::reversed_lenses(p),
                )
            })
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
    let kind = FileKind::of(&c.file);
    let is_json = kind != FileKind::Zmx;
    if kind == FileKind::SystemFile {
        return Err("a system file can't draw overlaid configurations (--cfgs, --states and --beam-paths are only for .zmx and projects)".into());
    }
    if let Some(v) = &l.cfgs {
        if is_json {
            return Err("--cfgs is only for .zmx (projects use --states and --beam-paths)".into());
        }
        let opt = zmx::LoadOptions {
            catalog_dirs: c.glass_dir.clone(),
            ..Default::default()
        };
        let txt = glass::read_text(&c.file).map_err(|x| format!("{}: {x}", c.file.display()))?;
        let n = zmx::parse_text(&txt)?.configurations();
        let ks: Vec<usize> = index_list(v, n, "configuration")?
            .into_iter()
            .map(|k| k + 1)
            .collect();
        let ss = zmx::load_configs(&c.file, &opt, &ks)?;
        return Ok((
            ks.iter().map(|k| format!("C{k}")).zip(ss).collect(),
            Vec::new(),
        ));
    }
    if !is_json {
        return Err(
            "--states and --beam-paths are only for project files (.json); .zmx uses --cfgs".into(),
        );
    }
    let src = Source::read(&c.file)?;
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
        // 機構部品の詳しい形の STEP は取った STEP のキャッシュから（PRJ-21）
        step_files: Some(&lensbench_cad::cached_step),
        base: c.file.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
        glass_dirs: c.glass_dir.clone(),
        catalog: Some(&look),
    };
    let mut out = Vec::new();
    let mut notes = Vec::new();
    for st in &states {
        let ps = match &st {
            Some(n) => p.with_state(n)?,
            None => p.clone(),
        };
        let paths: Vec<Option<String>> = match &l.paths {
            None => vec![c.path.clone()],
            Some(v) if v.eq_ignore_ascii_case("all") => {
                let v = lensbench_project::paths(&ps, &env)?;
                if v.is_empty() {
                    return Err("the project has no beam splitters".into());
                }
                v.into_iter().map(Some).collect()
            }
            Some(v) => v.split(',').map(|x| Some(x.trim().to_string())).collect(),
        };
        for pa in &paths {
            let (s, warn) = lensbench_project::expand(
                &ps,
                &env,
                &lensbench_project::ExpandOpts {
                    path: pa.as_deref(),
                    ..Default::default()
                },
            )
            .map(|x| (x.system, x.notes))
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
            let mut name: Vec<&str> = Vec::new();
            if l.states.is_some() {
                name.extend(st.as_deref());
            }
            if l.paths.is_some() {
                name.extend(pa.as_deref());
            }
            out.push((name.join("/"), s));
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
