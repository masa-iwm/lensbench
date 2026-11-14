//! 系のファイルの読み方（.zmx・プロジェクト・系ファイル）と、結果の書き出し・プラグインの一覧・膜の境界

use std::path::PathBuf;

use lensbench_core::catalog::LensCatalog;
use lensbench_core::coating::{self, Coatings};
use lensbench_core::{glass, zmx, System};
use lensbench_export::table;
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

/// ファイルから系を読む（注意は標準誤差へ）。ファイルが無いときは種類 io の誤り
pub fn load_file(c: &SysSpec) -> Result<System, lensbench_core::Error> {
    let o = lensbench_project::open::open(
        &c.file,
        &lensbench_project::open::OpenSpec {
            glass_dirs: &c.glass_dir,
            lens_catalogs: &c.lens_catalog,
            cfg: c.cfg,
            state: c.state.as_deref(),
            split_path: c.path.as_deref(),
            wavelengths: None,
            names: c.names,
        },
    )?;
    for w in o.warnings {
        eprintln!("注意: {w}");
    }
    Ok(o.system)
}

/// [`load_file`] に、分散式・マクロ・取り込みの注意も標準誤差へ出し、結果の出どころ（[`crate::provenance`]）に
/// 系を記録する（CLI の解析の系の読み方）
pub fn load_file_noted(c: &SysSpec) -> Result<System, lensbench_core::Error> {
    let s = load_file(c)?;
    for w in s
        .dispersion_warnings()
        .into_iter()
        .chain(s.macro_warnings())
        .chain(s.import_notes.iter().cloned())
    {
        eprintln!("注意: {w}");
    }
    crate::provenance::set(&s);
    Ok(s)
}

/// プロジェクトファイルの状態の名前（.zmx なら空）
/// プロジェクトファイルのビームスプリッターの経路の一覧（.zmx・BS なしなら空）
pub fn split_paths(c: &SysSpec) -> Result<Vec<String>, String> {
    if !c
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        || lensbench_project::sysfile::is_system_file(&c.file)
    {
        return Ok(Vec::new());
    }
    let t = lensbench_core::vfs::read_to_string(&c.file)
        .map_err(|x| format!("{}: {x}", c.file.display()))?;
    let mut p = lensbench_project::Project::from_json(&t)?;
    if let Some(st) = &c.state {
        p = p.with_state(st)?;
    }
    let cat = LensCatalog::load_registered(&c.lens_catalog)?;
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let env = lensbench_project::Env {
        base: c.file.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
        glass_dirs: c.glass_dir.clone(),
        catalog: Some(&look),
    };
    lensbench_project::paths(&p, &env)
}

pub fn state_names(c: &SysSpec) -> Vec<String> {
    if !c
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
    {
        return Vec::new();
    }
    lensbench_core::vfs::read_to_string(&c.file)
        .ok()
        .and_then(|t| lensbench_project::Project::from_json(&t).ok())
        .map(|p| p.states.iter().map(|s| s.name.clone()).collect())
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

/// 重ね描きする構成の系と名前。重ね描きでなければ今の系 1 つ（名前は空）
pub fn overlay_systems(c: &SysSpec, l: &Overlay) -> Result<Vec<(String, System)>, String> {
    if !l.any() {
        let s = load_file(c).map_err(|e| e.to_string())?;
        crate::provenance::set(&s);
        return Ok(vec![(String::new(), s)]);
    }
    let is_json = c
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    if let Some(v) = &l.cfgs {
        if is_json {
            return Err("--cfgs は .zmx だけ（プロジェクトは --states・--paths）".into());
        }
        let opt = zmx::LoadOptions {
            catalog_dirs: c.glass_dir.clone(),
            ..Default::default()
        };
        let txt = glass::read_text(&c.file).map_err(|x| format!("{}: {x}", c.file.display()))?;
        let n = zmx::parse_text(&txt)
            .map_err(|x| x.to_string())?
            .configurations();
        let ks: Vec<usize> = index_list(v, n, "構成")?
            .into_iter()
            .map(|k| k + 1)
            .collect();
        let ss = zmx::load_configs(&c.file, &opt, &ks).map_err(|x| x.to_string())?;
        return Ok(ks.iter().map(|k| format!("C{k}")).zip(ss).collect());
    }
    if !is_json {
        return Err(
            "--states・--paths はプロジェクトファイル（.json）だけ（.zmx は --cfgs）".into(),
        );
    }
    let txt = lensbench_core::vfs::read_to_string(&c.file)
        .map_err(|x| format!("{}: {x}", c.file.display()))?;
    let p = lensbench_project::Project::from_json(&txt)?;
    let states: Vec<Option<String>> = match &l.states {
        None => vec![c.state.clone()],
        Some(v) if v.eq_ignore_ascii_case("all") => {
            let n: Vec<Option<String>> = p
                .state_names()
                .into_iter()
                .map(|x| Some(x.to_string()))
                .collect();
            if n.is_empty() {
                return Err("プロジェクトに状態がない".into());
            }
            n
        }
        Some(v) => v.split(',').map(|x| Some(x.trim().to_string())).collect(),
    };
    let cat = LensCatalog::load_registered(&c.lens_catalog)?;
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let env = lensbench_project::Env {
        base: c.file.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
        glass_dirs: c.glass_dir.clone(),
        catalog: Some(&look),
    };
    let mut out = Vec::new();
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
                    return Err("プロジェクトにビームスプリッターがない".into());
                }
                v.into_iter().map(Some).collect()
            }
            Some(v) => v.split(',').map(|x| Some(x.trim().to_string())).collect(),
        };
        for pa in &paths {
            let (s, warn) =
                lensbench_project::expand_path(&ps, &env, pa.as_deref()).map_err(|x| match pa {
                    Some(q) => format!("経路 {q}: {x}"),
                    None => x,
                })?;
            for w in warn {
                eprintln!("注意: {w}");
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
    Ok(out)
}

/// "1,3" のような 1 始まりの番号の並びを 0 始まりにする。"all" は 0..n
pub fn index_list(s: &str, n: usize, what: &str) -> Result<Vec<usize>, String> {
    if s.eq_ignore_ascii_case("all") {
        return Ok((0..n).collect());
    }
    s.split(',')
        .map(|t| {
            let k: usize = t.trim().parse().map_err(|_| format!("{what}の番号 {t}"))?;
            if k == 0 || k > n {
                return Err(format!("{what} {k} はありません（1〜{n}）"));
            }
            Ok(k - 1)
        })
        .collect()
}

/// --save: 解析の結果を書き出す（json はそのまま、csv・npz は表にして）
pub fn save_value(p: &std::path::Path, v: &Value) -> Result<(), String> {
    let json = p
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    if json {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.insert("provenance".into(), crate::provenance::value());
        }
        let s = serde_json::to_string_pretty(&v).unwrap();
        lensbench_core::vfs::write(p, s + "\n").map_err(|x| format!("{}: {x}", p.display()))
    } else {
        table::from_json(v).save(p)
    }
}

/// 同梱のプラグインを登録した表（exe に組み込む分）に、プロセスで共通の登録
/// （[`lensbench_core::plugin::global`]、この CLI を組み込んだ exe が足した分）を重ねたもの
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
                "config": pairs(&s.config), "params": params, "inputs": pairs(&s.inputs), "outputs": pairs(&s.outputs),
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
pub fn coat_interfaces(s: &System, tables: &[PathBuf]) -> Result<CoatInterfaces, String> {
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
                "近似の膜（分光の曲線に合わせた多層膜）".to_string()
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
