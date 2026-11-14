//! serve の `analyze`: 解析の登録簿の解析と膜の境界を、読み込んだ系で実行する

use std::path::Path;

use lensbench_analysis::{Param, ParamTy};
use serde_json::{json, Map, Value};

use crate::session::{RpcErr, Session};

/// serve の analyze: 解析の登録簿の解析を、読み込んだ系で実行する。`save` は結果を書き出すファイル、ほかの鍵は解析の設定
pub(crate) fn run(
    sess: &Session,
    def: &dyn lensbench_analysis::DynAnalysis,
    m: &Map<String, Value>,
) -> Result<Value, RpcErr> {
    let sys = sess.current_system()?;
    let ps = def.params();
    let mut params = Map::new();
    let mut save = None;
    for (k, v) in m {
        // 鍵はハイフンでも（CLI の旗と同じ書き方）
        let k = k.replace('-', "_");
        if let Some(n) = lensbench_analysis::registry::renamed_param(&k) {
            return Err(RpcErr::params(format!("{k} は {n} に名前を変えた")));
        }
        match k.as_str() {
            // json は CLI の --json に当たる（serve の結果はいつも JSON）
            "cmd" | "json" => {}
            "file" | "cfg" | "state" | "path" | "beam_path" | "glass_dir" | "glass_dirs"
            | "lens_catalog" | "lens_catalogs" => {
                return Err(RpcErr::params(format!(
                    "{k} は analyze では変えられない（load で選ぶ）"
                )))
            }
            "save" => {
                let p = v
                    .as_str()
                    .ok_or_else(|| RpcErr::params("save はファイルのパス"))?;
                save = Some(sess.files.check_path(Path::new(p), true)?);
            }
            _ => {
                let v = match ps.iter().find(|p| p.name == k) {
                    Some(p) => loose(p, v),
                    None => v.clone(),
                };
                params.insert(k, v);
            }
        }
    }
    let mut params = Value::Object(params);
    // 設定の誤り（知らない鍵・型違い）は要求の誤り
    def.check_params(&params)
        .map_err(|e| RpcErr::params(e.to_string()))?;
    // 設定の中のファイル（ghost の膜表・書き出し先など）も要求で与えたパスなので確かめる
    for (v, write) in paths_mut(def, &mut params) {
        let p = v.as_str().unwrap_or_default().to_string();
        *v = Value::from(
            sess.files
                .check_path(Path::new(&p), write)?
                .display()
                .to_string(),
        );
    }
    let (out, to_save) = crate::session::with_source(sess, |src| {
        let cx = lensbench_analysis::Cx {
            system: &sys,
            source: Some(src),
        };
        def.run_save(&cx, &params, save.as_deref())
    })?
    .map_err(analysis_err)?;
    if let (Some(p), Some(v)) = (&save, &to_save) {
        // 書き出しの provenance には今の系（編集も入る）の hash を刻む
        crate::provenance::set(&sys);
        crate::load::save_value(p, v)?;
    }
    Ok(out)
}

/// 解析の誤り。値の誤り（Invalid）は文だけ（CLI の解析の頃と同じ。系を読んだ所の「不正: 」も重ねない）
fn analysis_err(e: lensbench_core::Error) -> RpcErr {
    match e {
        lensbench_core::Error::Invalid(m) => {
            RpcErr::from(m.strip_prefix("不正: ").unwrap_or(&m).to_string()).with_kind("invalid")
        }
        e => RpcErr::from(e),
    }
}

/// 前の受け方（引数の文字列を経た頃）の値も受ける: 数の文字列（"2"）、数の並びのカンマ区切りの文字列（"10,20"）や
/// 1 つの数、パスの並びの 1 つのパス。読めない値はそのまま渡し、設定を読むところで誤りにする
fn loose(p: &Param, v: &Value) -> Value {
    let num = |t: &str| -> Option<Value> {
        let t = t.trim();
        t.parse::<i64>()
            .map(Value::from)
            .ok()
            .or_else(|| t.parse::<f64>().ok().map(Value::from))
    };
    match (p.ty, v) {
        (ParamTy::Count | ParamTy::Int | ParamTy::Num, Value::String(t)) => {
            num(t).unwrap_or_else(|| v.clone())
        }
        (ParamTy::Nums, Value::String(t)) => t
            .split(',')
            .filter(|x| !x.trim().is_empty())
            .map(num)
            .collect::<Option<Vec<_>>>()
            .map(Value::from)
            .unwrap_or_else(|| v.clone()),
        (ParamTy::Nums, Value::Number(_)) => Value::from(vec![v.clone()]),
        (ParamTy::Paths, Value::String(_)) => Value::from(vec![v.clone()]),
        (ParamTy::Json, Value::String(t)) => serde_json::from_str(t).unwrap_or_else(|_| v.clone()),
        _ => v.clone(),
    }
}

/// analyze の coatings: 読み込んだ系（今の構成・状態）の膜の境界（CLI の coatings の 1 構成分。.zmx でなくてもよい）。
/// `table` で実測表・モデル（LBCT・LBCM）を足せる
pub(crate) fn coatings_loaded(sess: &Session, m: &Map<String, Value>) -> Result<Value, RpcErr> {
    let s = sess.current_system()?;
    let mut tables = Vec::new();
    match m.get("table") {
        None | Some(Value::Null) => {}
        Some(Value::String(t)) => tables.push(sess.files.check_path(Path::new(t), false)?),
        Some(Value::Array(a)) => {
            for t in a {
                let t = t
                    .as_str()
                    .ok_or_else(|| RpcErr::params("table はパスの並び"))?;
                tables.push(sess.files.check_path(Path::new(t), false)?);
            }
        }
        Some(_) => return Err(RpcErr::params("table はパスの並び")),
    }
    if let Some(k) = m
        .keys()
        .find(|k| !["cmd", "table", "json"].contains(&k.as_str()))
    {
        return Err(RpcErr::params(format!(
            "coatings の引数 {k} は無い（table だけ）"
        )));
    }
    let c = crate::load::coat_interfaces(&s, &tables)?;
    Ok(json!({
        "coating_file_path": s.coating_file.as_ref().map(|p| p.display().to_string()),
        "wavelengths_um": s.wavelengths,
        "interfaces": c.list,
        "needs_table": c.needs,
        "auto_coatings": c.auto.iter().map(|(c, p, n)| json!({ "coat": c, "path": p, "note": n })).collect::<Vec<_>>(),
    }))
}

/// 設定の中のファイルのパス（書き出しか）。場所を確かめて置き換える
fn paths_mut<'a>(
    a: &dyn lensbench_analysis::DynAnalysis,
    params: &'a mut Value,
) -> Vec<(&'a mut Value, bool)> {
    let ps = a.params();
    let Some(o) = params.as_object_mut() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, v) in o.iter_mut() {
        let Some(p) = ps.iter().find(|p| &p.name == k) else {
            continue;
        };
        match (p.ty, v) {
            (ParamTy::Path, v @ Value::String(_)) => out.push((v, p.write)),
            (ParamTy::Paths, Value::Array(xs)) => {
                out.extend(xs.iter_mut().map(|x| (x, p.write)));
            }
            _ => {}
        }
    }
    out
}
