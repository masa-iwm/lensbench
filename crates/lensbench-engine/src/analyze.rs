//! serve の `analyze`: 解析の登録簿の解析と膜の境界を、読み込んだ系で実行する

use std::path::Path;

use lensbench_analysis::ParamTy;
use serde_json::{json, Map, Value};

use crate::session::{Answer, RpcErr, Session};

/// serve の analyze: 解析の登録簿の解析を、読み込んだ系で実行する。`save` は結果を書き出すファイル、ほかの鍵は解析の設定
pub(crate) fn run(
    sess: &Session,
    def: &dyn lensbench_analysis::DynAnalysis,
    m: &Map<String, Value>,
) -> Result<Answer, RpcErr> {
    let sys = sess.current_system()?;
    let mut params = Map::new();
    let mut save = None;
    for (k, v) in m {
        // 鍵はハイフンでも（CLI の旗と同じ書き方）
        let k = k.replace('-', "_");
        match k.as_str() {
            // json は CLI の --json に当たる（serve の結果はいつも JSON）
            "cmd" | "json" => {}
            "file" | "cfg" | "state" | "beam_path" | "glass_dirs" | "lens_catalogs" => {
                return Err(RpcErr::params(format!(
                    "{k} can't be changed in analyze (choose it in load)"
                )))
            }
            "save" => {
                let p = v
                    .as_str()
                    .ok_or_else(|| RpcErr::params("save must be a file path"))?;
                save = Some(sess.files.check_path(Path::new(p), true)?);
            }
            _ => {
                params.insert(k, v.clone());
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
    // 書き出さないなら結果は型のまま（同じプロセスの呼び手は JSON を通さずに読む）
    if save.is_none() {
        let t = crate::session::with_source(sess, |src| {
            let cx = lensbench_analysis::Cx {
                system: &sys,
                source: Some(src),
            };
            def.run_typed(&cx, &params)
        })?
        .map_err(analysis_err)?;
        return Ok(Answer::Analysis(Box::new(t)));
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
    Ok(Answer::Json(out))
}

/// 解析の誤り。値の誤り（Invalid）は文だけ（CLI の解析の頃と同じ。系を読んだ所の「不正: 」も重ねない）
fn analysis_err(e: lensbench_core::Error) -> RpcErr {
    match e.base() {
        lensbench_core::Error::Invalid(m) => RpcErr::from(lensbench_core::i18n::tr(
            lensbench_core::error::strip_invalid(m),
        ))
        .with_core(&e),
        _ => RpcErr::from(e),
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
                    .ok_or_else(|| RpcErr::params("table must be a list of paths"))?;
                tables.push(sess.files.check_path(Path::new(t), false)?);
            }
        }
        Some(_) => return Err(RpcErr::params("table must be a list of paths")),
    }
    if let Some(k) = m
        .keys()
        .find(|k| !["cmd", "table", "json"].contains(&k.as_str()))
    {
        return Err(RpcErr::params(format!(
            "no coatings argument {k} (only table)"
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
