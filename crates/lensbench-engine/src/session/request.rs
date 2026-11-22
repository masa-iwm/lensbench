//! 要求（[`Request`]）: method と、その params を型で。要求を運ぶ道（serve・Python）は JSON の params を
//! [`Request::parse`] で型に読み、同じプロセスの呼び手（GUI）は型を作って [`super::Session::request`] で渡す
//! （JSON を通さない）。型をまだ付けていない method は [`Request::Json`] で JSON のまま渡す

use super::args::{known_keys, opt_usize, str_list};
use super::catalog::{
    BrowseReq, CadStepReq, CatalogAddReq, CatalogEditReq, CatalogFillReq, SearchReq,
};
use super::layout::{LayoutExportReq, LayoutReq};
use super::optim::{SwapReq, TolReq};
use super::trace::{PluginReq, TraceReq};
use super::{LoadSpec, RpcErr, Session};
use lensbench_design::dls_spec::{OptimizeParams, ZoomParams};
use lensbench_project::edit_spec::{EditOp, Ref};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// load・reload の段の扱い（`history`）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadHistory {
    /// 段を空にする（既定）
    #[default]
    Clear,
    /// 前の系を 1 段として積む（開く・読み直すを元に戻せる）
    Push,
    /// 段はそのまま（GUI の組合せの作り直しなど、段を別に持つとき）
    Keep,
}

/// `load` の params
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LoadReq {
    pub file: PathBuf,
    pub glass_dirs: Vec<PathBuf>,
    pub lens_catalogs: Vec<PathBuf>,
    /// 多重構成の番号（1 始まり、既定 1）
    pub cfg: Option<usize>,
    /// プロジェクトの状態の名前
    pub state: Option<String>,
    /// BS の経路（"TR" など）
    pub beam_path: Option<String>,
    pub history: LoadHistory,
    pub strict_macros: bool,
}

/// `get` の params: 1 つ（`param`）か並び（`params`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GetReq {
    One(String),
    Many(Vec<String>),
}

/// `analyze` の params: 解析の名前と、その設定（鍵は解析の設定の名前。`save` は結果を書き出すファイル）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnalyzeReq {
    pub cmd: String,
    pub args: Map<String, Value>,
}

impl AnalyzeReq {
    /// 解析 cmd を設定 args で
    pub fn new(cmd: impl Into<String>, args: Map<String, Value>) -> AnalyzeReq {
        AnalyzeReq {
            cmd: cmd.into(),
            args,
        }
    }

    /// serve の params（`cmd` と設定を 1 つのオブジェクトに）
    pub(crate) fn parse(p: &Value) -> Result<AnalyzeReq, RpcErr> {
        let m = p
            .as_object()
            .ok_or_else(|| RpcErr::params("params must be an object"))?;
        let cmd = m
            .get("cmd")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcErr::params("cmd (the analysis name) is needed"))?;
        let mut args = m.clone();
        args.remove("cmd");
        Ok(AnalyzeReq::new(cmd, args))
    }
}

/// `write` の熱の構成（[`lensbench_core::thermal::ThermalSpec`]）: 足す構成の温度 [°C] と気圧 [atm]（空なら今の気圧）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThermalReq {
    pub temps: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pressures: Vec<f64>,
}

/// 1 つの要求
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    Load(Box<LoadReq>),
    /// 同じ読み方でファイルを読み直す
    Reload {
        history: LoadHistory,
    },
    /// 読み込んだ系の概要
    System,
    /// 元に戻す・やり直す（段の数、1 以上）
    Undo {
        steps: usize,
    },
    Redo {
        steps: usize,
    },
    History,
    Get(GetReq),
    /// パラメータの値を変える（名前と値、順に）
    Set {
        values: Vec<(String, f64)>,
    },
    /// 系を .zmx に書き出す（thermal を与えれば、熱の構成を作った系を）
    Write {
        path: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thermal: Option<ThermalReq>,
    },
    /// 系の編集の操作を順に当てる
    Edit {
        ops: Vec<EditOp>,
    },
    /// 全部の面の値（`surface` で面を指定しないとき）
    Surfaces,
    /// 面のデータを全部の項目で（面の番号か名前）
    SurfaceData {
        surface: Ref,
    },
    /// 系の設定（波長・視野・開口など）
    SystemData,
    /// 多重構成の表
    ConfigTable,
    /// レイアウト図に重ねられる構成
    Configs,
    /// 面の操作をプロジェクトのブロックへ書き戻した結果（保存しない）
    WriteBack {
        ops: Vec<EditOp>,
    },
    /// 系ファイルに保存する
    Save {
        path: PathBuf,
    },
    /// 面 from〜to だけを系ファイルに書き出す
    SaveRange {
        path: PathBuf,
        from: usize,
        to: usize,
    },
    /// 解析を回す
    Analyze(Box<AnalyzeReq>),
    /// レイアウト図
    Layout(Box<LayoutReq>),
    /// レイアウトをファイルに書き出す
    LayoutExport(Box<LayoutExportReq>),
    /// 系をファイルから読んでレイアウト図を作る（読み込んだ系は変えない）。同じプロセスの呼び手だけ（serve の
    /// method には無い。GUI の組合せの図の下書き）
    Preview {
        load: Box<LoadReq>,
        layout: Box<LayoutReq>,
    },
    /// 正規化瞳座標の光線を追う
    Trace(Box<TraceReq>),
    /// 解析・検出器のプラグインを 1 回呼ぶ
    Plugin(Box<PluginReq>),
    /// 減衰最小二乗の最適化
    Optimize(Box<OptimizeParams>),
    /// カタログレンズの差し替え探索
    Swap(Box<SwapReq>),
    /// モンテカルロ公差
    Tolerance(Box<TolReq>),
    /// ズーム軌跡
    Zoom(Box<ZoomParams>),
    /// 版
    Ping,
    /// 版・プロトコル・method・解析の名前
    Describe,
    /// serve を終える
    Shutdown,
    /// 1 つの面の値（面の番号か名前）
    Surface {
        surface: Ref,
    },
    /// .zmx の多重構成を表にする edit の操作
    ConfigDiff,
    /// 読み込んだプロジェクトに読み込んだ後の変更を書き戻して保存する
    SaveProject {
        path: PathBuf,
    },
    /// 解析・検出器のプラグイン
    Plugins,
    /// analyze の解析の設定の項目（解析の名前）
    AnalysisArgs {
        cmd: String,
    },
    /// ガラスの一覧
    Glasses(BrowseReq),
    /// レンズの一覧
    Lenses(BrowseReq),
    /// カタログを探す
    CatalogSearch(Box<SearchReq>),
    /// 読み込んだプロジェクトのブロックをユーザーカタログの項目に
    CatalogAdd(Box<CatalogAddReq>),
    /// ユーザーカタログの項目の一覧
    CatalogItems {
        catalog: String,
    },
    /// ユーザーカタログの項目の名前・説明の変更と削除
    CatalogEdit(Box<CatalogEditReq>),
    /// カタログの未完の品を補って補足のカタログに書く
    CatalogFill(Box<CatalogFillReq>),
    /// 品の STEP を取ってキャッシュに置く
    CadStep(Box<CadStepReq>),
    /// 型をまだ付けていない method（params は serve の JSON）
    Json {
        method: String,
        params: Value,
    },
}

impl Request {
    /// 型をまだ付けていない method の要求
    pub fn json(method: impl Into<String>, params: Value) -> Request {
        Request::Json {
            method: method.into(),
            params,
        }
    }

    /// serve の method の名前
    pub fn method(&self) -> &str {
        match self {
            Request::Load(_) => "load",
            Request::Reload { .. } => "reload",
            Request::System => "system",
            Request::Undo { .. } => "undo",
            Request::Redo { .. } => "redo",
            Request::History => "history",
            Request::Get(_) => "get",
            Request::Set { .. } => "set",
            Request::Write { .. } => "write",
            Request::Edit { .. } => "edit",
            Request::Surfaces => "surface",
            Request::SurfaceData { .. } => "surface_data",
            Request::SystemData => "system_data",
            Request::ConfigTable => "config_table",
            Request::Configs => "configs",
            Request::WriteBack { .. } => "write_back",
            Request::Save { .. } => "save",
            Request::SaveRange { .. } => "save_range",
            Request::Analyze(_) => "analyze",
            Request::Layout(_) => "layout",
            Request::LayoutExport(_) => "layout_export",
            Request::Preview { .. } => "preview",
            Request::Trace(_) => "trace",
            Request::Plugin(_) => "plugin",
            Request::Optimize(_) => "optimize",
            Request::Swap(_) => "swap",
            Request::Tolerance(_) => "tolerance",
            Request::Zoom(_) => "zoom",
            Request::Ping => "ping",
            Request::Describe => "describe",
            Request::Shutdown => "shutdown",
            Request::Surface { .. } => "surface",
            Request::ConfigDiff => "config_diff",
            Request::SaveProject { .. } => "save_project",
            Request::Plugins => "plugins",
            Request::AnalysisArgs { .. } => "analysis_args",
            Request::Glasses(_) => "glasses",
            Request::Lenses(_) => "lenses",
            Request::CatalogSearch(_) => "catalog_search",
            Request::CatalogAdd(_) => "catalog_add",
            Request::CatalogItems { .. } => "catalog_items",
            Request::CatalogEdit(_) => "catalog_edit",
            Request::CatalogFill(_) => "catalog_fill",
            Request::CadStep(_) => "cad_step",
            Request::Json { method, .. } => method,
        }
    }

    /// serve の method と JSON の params を型の要求に（読めなければ serve と同じ誤り。読む所の panic も誤り）。
    /// 型を付けていない method は [`Request::Json`]
    pub fn from_json(method: &str, p: &Value) -> Result<Request, super::CallError> {
        super::catch_panic(|| Request::parse(method, p)).map_err(super::CallError::from)
    }

    /// serve の JSON の params を型に読む（誤りの文は前と同じ）。型を付けていない method は [`Request::Json`]
    pub(crate) fn parse(method: &str, p: &Value) -> Result<Request, RpcErr> {
        Ok(match method {
            "load" => Request::Load(Box::new(LoadReq::parse(p)?)),
            "reload" => Request::Reload {
                history: load_history(p)?,
            },
            "system" => Request::System,
            "undo" => Request::Undo { steps: steps(p)? },
            "redo" => Request::Redo { steps: steps(p)? },
            "history" => Request::History,
            "get" => Request::Get(match p.get("param").and_then(Value::as_str) {
                Some(one) => GetReq::One(one.to_string()),
                None => {
                    let list = p["params"]
                        .as_array()
                        .ok_or_else(|| RpcErr::params("get needs param or params (a list)"))?;
                    let names = list
                        .iter()
                        .map(|x| {
                            x.as_str()
                                .map(str::to_string)
                                .ok_or_else(|| RpcErr::params("params must be a list of strings"))
                        })
                        .collect::<Result<_, _>>()?;
                    GetReq::Many(names)
                }
            }),
            "set" => Request::Set {
                values: set_values(p)?,
            },
            "write" => Request::Write {
                path: path_of(p, "write needs path")?,
                thermal: match &p["thermal"] {
                    Value::Null => None,
                    v => Some(
                        serde_json::from_value(v.clone())
                            .map_err(|e| RpcErr::params(format!("thermal: {e}")))?,
                    ),
                },
            },
            "edit" => Request::Edit {
                ops: lensbench_project::edit_ops::parse(&p["ops"]).map_err(RpcErr::params)?,
            },
            "surface" if p.get("surface").is_none_or(Value::is_null) => Request::Surfaces,
            "surface" => Request::Surface {
                surface: surface_ref(p, "surface")?,
            },
            "surface_data" => Request::SurfaceData {
                surface: surface_ref(p, "surface_data")?,
            },
            "system_data" => Request::SystemData,
            "config_table" => Request::ConfigTable,
            "configs" => Request::Configs,
            "write_back" => Request::WriteBack {
                ops: lensbench_project::edit_ops::parse(&p["ops"])?,
            },
            "save" => Request::Save {
                path: path_of(p, "save needs path")?,
            },
            "save_range" => {
                let path = path_of(p, "save_range needs path")?;
                let num = |k: &str| {
                    p[k].as_u64().map(|x| x as usize).ok_or_else(|| {
                        RpcErr::params(format!("save_range needs {k} (a surface number)"))
                    })
                };
                Request::SaveRange {
                    path,
                    from: num("from")?,
                    to: num("to")?,
                }
            }
            "analyze" => Request::Analyze(Box::new(AnalyzeReq::parse(p)?)),
            "layout" => Request::Layout(Box::new(LayoutReq::parse(p)?)),
            "layout_export" => Request::LayoutExport(Box::new(LayoutExportReq::parse(p)?)),
            "trace" => Request::Trace(Box::new(TraceReq::parse(p)?)),
            "plugin" => Request::Plugin(Box::new(PluginReq::parse(p)?)),
            "optimize" => Request::Optimize(Box::new(
                lensbench_design::dls_spec::read(p)
                    .map_err(|e| RpcErr::params(format!("optimize: {e}")))?,
            )),
            "swap" => Request::Swap(Box::new(SwapReq::parse(p)?)),
            "tolerance" => Request::Tolerance(Box::new(TolReq::parse(p)?)),
            "zoom" => Request::Zoom(Box::new(
                lensbench_design::dls_spec::read(p)
                    .map_err(|e| RpcErr::params(format!("zoom: {e}")))?,
            )),
            "ping" => Request::Ping,
            "describe" => Request::Describe,
            "shutdown" => Request::Shutdown,
            "config_diff" => Request::ConfigDiff,
            "save_project" => Request::SaveProject {
                path: path_of(p, "save_project needs path")?,
            },
            "plugins" => Request::Plugins,
            "analysis_args" => Request::AnalysisArgs {
                cmd: p["cmd"]
                    .as_str()
                    .ok_or_else(|| RpcErr::params("analysis_args needs cmd"))?
                    .to_string(),
            },
            "glasses" | "lenses" => {
                let r = BrowseReq::parse(method, p)?;
                if method == "glasses" {
                    Request::Glasses(r)
                } else {
                    Request::Lenses(r)
                }
            }
            "catalog_search" => Request::CatalogSearch(Box::new(SearchReq::parse(p)?)),
            "catalog_add" => Request::CatalogAdd(Box::new(CatalogAddReq::parse(p)?)),
            "catalog_items" => Request::CatalogItems {
                catalog: super::catalog::catalog_of("catalog_items", p)?,
            },
            "catalog_edit" => Request::CatalogEdit(Box::new(CatalogEditReq::parse(p)?)),
            "catalog_fill" => Request::CatalogFill(Box::new(CatalogFillReq::parse(p)?)),
            "cad_step" => Request::CadStep(Box::new(CadStepReq::parse(p)?)),
            m => Request::json(m, p.clone()),
        })
    }
}

impl LoadReq {
    /// `load` の params（名前は Python と同じ）
    pub(crate) fn parse(p: &Value) -> Result<LoadReq, RpcErr> {
        let history = load_history(p)?;
        let file = p["file"]
            .as_str()
            .ok_or_else(|| RpcErr::params("load needs file"))?;
        let keys = [
            "file",
            "glass_dirs",
            "lens_catalogs",
            "cfg",
            "state",
            "beam_path",
            "history",
            "strict_macros",
        ];
        known_keys(p, &keys)?;
        let beam_path = match &p["beam_path"] {
            Value::Null => None,
            v => Some(
                v.as_str()
                    .ok_or_else(|| {
                        RpcErr::params("beam_path must be a string (the BS path, e.g. \"TR\")")
                    })?
                    .to_string(),
            ),
        };
        Ok(LoadReq {
            file: PathBuf::from(file),
            glass_dirs: str_list(&p["glass_dirs"], "glass_dirs")?,
            lens_catalogs: str_list(&p["lens_catalogs"], "lens_catalogs")?,
            cfg: opt_usize(p, "cfg")?,
            state: p["state"].as_str().map(str::to_string),
            beam_path,
            history,
            strict_macros: match &p["strict_macros"] {
                Value::Null => false,
                v => v
                    .as_bool()
                    .ok_or_else(|| RpcErr::params("strict_macros must be true or false"))?,
            },
        })
    }

    /// 系の読み方（パスは場の決まりで確かめる）
    pub(crate) fn spec(&self, sess: &Session) -> Result<LoadSpec, RpcErr> {
        let read_all = |v: &[PathBuf]| -> Result<Vec<PathBuf>, RpcErr> {
            v.iter().map(|x| sess.files.check_path(x, false)).collect()
        };
        Ok(LoadSpec {
            file: sess.files.check_path(Path::new(&self.file), false)?,
            glass_dir: read_all(&self.glass_dirs)?,
            lens_catalog: read_all(&self.lens_catalogs)?,
            cfg: self.cfg.unwrap_or(1),
            state: self.state.clone(),
            path: self.beam_path.clone(),
            names: lensbench_project::open::ArgNames::SERVE,
            strict_macros: self.strict_macros,
        })
    }
}

fn load_history(p: &Value) -> Result<LoadHistory, RpcErr> {
    Ok(match p.get("history").and_then(Value::as_str) {
        None | Some("clear") => LoadHistory::Clear,
        Some("push") => LoadHistory::Push,
        Some("keep") => LoadHistory::Keep,
        Some(x) => {
            return Err(RpcErr::params(format!(
                "history must be clear, push or keep ({x})"
            )))
        }
    })
}

fn steps(p: &Value) -> Result<usize, RpcErr> {
    Ok(opt_usize(p, "steps")?.unwrap_or(1).max(1))
}

fn set_values(p: &Value) -> Result<Vec<(String, f64)>, RpcErr> {
    let bad = || RpcErr::params("set needs values (\"3.t\": 10.0, or [[\"3.t\", 10.0]])");
    match p.get("values") {
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| Ok((k.clone(), v.as_f64().ok_or_else(bad)?)))
            .collect(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                let k = x.get(0).and_then(Value::as_str).ok_or_else(bad)?;
                let v = x.get(1).and_then(Value::as_f64).ok_or_else(bad)?;
                Ok((k.to_string(), v))
            })
            .collect(),
        _ => Err(bad()),
    }
}

/// 面の番号か名前（`surface`）
fn surface_ref(p: &Value, method: &str) -> Result<Ref, RpcErr> {
    Ok(match p.get("surface") {
        Some(Value::String(t)) => Ref::Name(t.clone()),
        Some(v) => Ref::Index(
            v.as_u64()
                .map(|x| x as usize)
                .ok_or_else(|| RpcErr::params("surface must be a surface number or name"))?,
        ),
        // surface の無い surface は面の一覧（Surfaces）なので、ここに来るのは surface_data
        None => {
            debug_assert_eq!(method, "surface_data");
            return Err(RpcErr::params("surface_data needs surface"));
        }
    })
}

fn path_of(p: &Value, missing: &str) -> Result<PathBuf, RpcErr> {
    p["path"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| RpcErr::params(missing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// serve の params から読んだ要求は、serde で書いて読み直しても同じ（Web 版の Worker へ渡す道）
    #[test]
    fn parsed_requests_survive_serde() {
        let cases = [
            ("glasses", json!({ "glass_dirs": "g" })),
            (
                "lenses",
                json!({ "glass_dirs": ["g"], "lens_catalogs": ["c"] }),
            ),
            (
                "catalog_search",
                json!({ "kinds": ["splitter"], "specs": ["efl>=10"], "fits": 25.4 }),
            ),
            (
                "catalog_add",
                json!({ "catalog": "u.json", "name": " A ", "block": "L1", "replace": true }),
            ),
            ("catalog_items", json!({ "catalog": "u.json" })),
            (
                "catalog_edit",
                json!({ "catalog": "u.json", "name": "A", "rename": " B ", "delete": false }),
            ),
            (
                "catalog_fill",
                json!({ "catalog": "c.json", "out": "o.json", "item": "X", "thickness": 3.0,
                        "diameter": [10, 20], "spec": { "thickness": 3.0 } }),
            ),
            ("cad_step", json!({ "catalog": "c.json", "item": "X" })),
            ("surface", json!({ "surface": 3 })),
            ("analysis_args", json!({ "cmd": "spot" })),
            ("save_project", json!({ "path": "p.json" })),
            (
                "layout",
                json!({ "plane": "3d", "fields": "all", "waves": [1, 2], "fan": "ring",
                        "color": "CFG", "cfgs": [1, 3], "offset": [0, 1, 2] }),
            ),
            (
                "layout_export",
                json!({ "file": "a.svg", "width": 800, "tube": 0.1 }),
            ),
            (
                "trace",
                json!({ "px": [0.0, 0.5], "py": [0.0, 0.0], "field": 2, "global": true }),
            ),
            (
                "plugin",
                json!({ "name": "p", "config": { "a": 1, "b": "x", "c": [1, 2], "d": true },
                        "inputs": { "m": [[1, 2], [3, 4]] } }),
            ),
            ("analyze", json!({ "cmd": "spot", "field": 1 })),
            (
                "optimize",
                json!({ "vars": ["8.t", "7.t=0.5:3"], "merit": "spot", "configs": [1, "B"] }),
            ),
            ("zoom", json!({ "vars": ["G2.shift"], "efl": [50, null] })),
            ("swap", json!({ "slots": [{ "block": "A" }], "top": 3 })),
            ("tolerance", json!({ "configs": [1], "trials": 10 })),
            (
                "load",
                json!({ "file": "a.zmx", "glass_dirs": ["g"], "cfg": 2, "strict_macros": true }),
            ),
            ("reload", json!({})),
            ("undo", json!({ "steps": 2 })),
            ("get", json!({ "param": "3.t" })),
            ("get", json!({ "params": ["3.t", "4.c"] })),
            ("set", json!({ "values": { "3.t": 10.0 } })),
            ("write", json!({ "path": "a.zmx" })),
            (
                "write",
                json!({ "path": "a.zmx", "thermal": { "temps": [-20, 60], "pressures": [1, 0.5] } }),
            ),
            (
                "edit",
                json!({ "ops": [{ "op": "set_stop", "surface": 1 }] }),
            ),
            (
                "write_back",
                json!({ "ops": [{ "op": "set_stop", "surface": 1 }] }),
            ),
            ("save", json!({ "path": "a.json" })),
            (
                "save_range",
                json!({ "path": "a.json", "from": 1, "to": 3 }),
            ),
            ("surface_data", json!({ "surface": 2 })),
        ];
        for (m, p) in cases {
            let r = Request::parse(m, &p).unwrap_or_else(|e| panic!("{m}: {}", e.msg));
            assert_eq!(r.method(), m);
            assert!(!matches!(r, Request::Json { .. }), "{m}");
            let back: Request = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
            assert_eq!(back, r, "{m}");
        }
        match Request::parse(
            "catalog_add",
            &json!({ "catalog": "u.json", "name": " A ", "block": "L1" }),
        ) {
            Ok(Request::CatalogAdd(r)) => {
                assert_eq!(
                    (r.name.as_str(), r.blocks.as_slice()),
                    ("A", &["L1".to_string()][..])
                )
            }
            r => panic!("{:?}", r.map_err(|e| e.msg)),
        }
        match Request::parse("layout", &json!({ "color": "CFG", "cfgs": [1, 3] })) {
            Ok(Request::Layout(r)) => {
                assert_eq!(
                    (r.color.as_deref(), r.cfgs.as_deref()),
                    (Some("cfg"), Some("1,3"))
                )
            }
            r => panic!("{:?}", r.map_err(|e| e.msg)),
        }
        match Request::parse(
            "plugin",
            &json!({ "name": "p", "inputs": { "m": [[1, 2], [3, 4]] } }),
        ) {
            Ok(Request::Plugin(r)) => {
                assert_eq!(
                    (r.inputs[0].shape.as_slice(), r.inputs[0].data.len()),
                    (&[2, 2][..], 4)
                )
            }
            r => panic!("{:?}", r.map_err(|e| e.msg)),
        }
        // 前と同じ誤りの文
        let e = Request::parse("layout", &json!({ "fields": [0.5] })).unwrap_err();
        assert_eq!(e.msg, "fields must be numbers from 1 or \"all\"");
        let e = Request::parse("layout_export", &json!({})).unwrap_err();
        assert_eq!(e.msg, "layout_export needs file");
        let e = Request::parse("analyze", &json!({})).unwrap_err();
        assert_eq!(e.msg, "cmd (the analysis name) is needed");
        let e = Request::parse("catalog_search", &json!({ "kinds": ["nope"] })).unwrap_err();
        assert_eq!(e.msg, "kinds: unknown kind nope");
        let e = Request::parse("catalog_items", &json!({})).unwrap_err();
        assert_eq!(e.msg, "catalog_items needs catalog (the user catalog file)");
        let e = Request::parse("glasses", &json!({ "lens_catalogs": [] })).unwrap_err();
        assert!(e.msg.starts_with("unknown key lens_catalogs"), "{}", e.msg);
    }
}
