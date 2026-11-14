//! serve の `analyze` のうち、引数の文字列を経ずに読み込んだ系で実行するもの（解析の登録簿の解析・膜の境界）

use std::path::Path;

use lensbench_analysis::{Param, ParamTy};
use serde_json::{json, Map, Value};

use crate::serve::{check_path, current_system, RpcErr};

/// serve の analyze: 解析の登録簿の解析を、読み込んだ系で実行する。`save` は結果を書き出すファイル、ほかの鍵は解析の設定
pub(crate) fn run(
    def: &dyn lensbench_analysis::DynAnalysis,
    m: &Map<String, Value>,
) -> Result<Value, RpcErr> {
    let sys = current_system()?;
    let ps = def.params();
    let mut params = Map::new();
    let mut save = None;
    for (k, v) in m {
        // 鍵はハイフンでも（CLI の旗と同じ書き方）
        let k = k.replace('-', "_");
        match k.as_str() {
            // json は CLI の --json に当たる（serve の結果はいつも JSON）
            "cmd" | "json" => {}
            "file" | "cfg" | "state" | "path" | "glass_dir" | "lens_catalog" => {
                return Err(RpcErr::params(format!(
                    "{k} は analyze では変えられない（load で選ぶ）"
                )))
            }
            "save" => {
                let p = v
                    .as_str()
                    .ok_or_else(|| RpcErr::params("save はファイルのパス"))?;
                save = Some(check_path(Path::new(p), true)?);
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
    for (v, write) in crate::analysis_cmd::paths_mut(def, &mut params) {
        let p = v.as_str().unwrap_or_default().to_string();
        *v = Value::from(check_path(Path::new(&p), write)?.display().to_string());
    }
    let out = def.run_json(&lensbench_analysis::Cx { system: &sys }, &params)?;
    if let Some(p) = &save {
        // 書き出しの provenance には今の系（編集も入る）の hash を刻む
        crate::provenance::set(&sys);
        crate::save_value(p, &out)?;
    }
    Ok(out)
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
        _ => v.clone(),
    }
}

/// analyze の coatings: 読み込んだ系（今の構成・状態）の膜の境界（CLI の coatings の 1 構成分。.zmx でなくてもよい）。
/// `table` で実測表・モデル（LBCT・LBCM）を足せる
pub(crate) fn coatings_loaded(m: &Map<String, Value>) -> Result<Value, RpcErr> {
    let s = current_system()?;
    let mut tables = Vec::new();
    match m.get("table") {
        None | Some(Value::Null) => {}
        Some(Value::String(t)) => tables.push(check_path(Path::new(t), false)?),
        Some(Value::Array(a)) => {
            for t in a {
                let t = t
                    .as_str()
                    .ok_or_else(|| RpcErr::params("table はパスの並び"))?;
                tables.push(check_path(Path::new(t), false)?);
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
    let c = crate::coat_interfaces(&s, &tables)?;
    Ok(json!({
        "coating_file_path": s.coating_file.as_ref().map(|p| p.display().to_string()),
        "wavelengths_um": s.wavelengths,
        "interfaces": c.list,
        "needs_table": c.needs,
        "auto_coatings": c.auto.iter().map(|(c, p, n)| json!({ "coat": c, "path": p, "note": n })).collect::<Vec<_>>(),
    }))
}
