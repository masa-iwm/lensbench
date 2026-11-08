//! 評価関数の項としてのプラグイン（`{"type": "plugin"}`）。登録したプラグイン（[`crate::plugins::builtin`]）に
//! 系を渡し、出力の配列 `output` を残差（− target）にする。

use std::sync::Arc;

use lensbench_core::optim::UserTerm;
use lensbench_core::plugin::{Array, Arrays, Config, Context, Instance, Value as PValue};
use lensbench_core::{Error, Result, System};
use serde_json::Value;

pub struct PluginTerm {
    label: String,
    inst: Box<dyn Instance>,
    inputs: Arrays,
    output: String,
    index: Option<usize>,
    target: Vec<f64>,
    rows: usize,
}

impl PluginTerm {
    /// プラグイン name を設定 args で準備し、系 s で 1 回評価して残差の数を決める
    pub fn new(
        name: &str,
        args: &Config,
        inputs: Arrays,
        output: &str,
        index: Option<usize>,
        target: Vec<f64>,
        s: &System,
    ) -> Result<PluginTerm> {
        let inst = crate::plugins::builtin().prepare(name, args)?;
        let mut t = PluginTerm {
            label: format!("plugin {name}.{output}"),
            inst,
            inputs,
            output: output.to_string(),
            index,
            target: vec![],
            rows: 0,
        };
        let v = t.values(s)?;
        t.rows = v.len();
        t.target = match target.len() {
            0 => vec![0.0; t.rows],
            1 => vec![target[0]; t.rows],
            n if n == t.rows => target,
            n => {
                return Err(Error::Invalid(format!(
                    "{}: target {n} 個（出力は {} 個）",
                    t.label, t.rows
                )))
            }
        };
        Ok(t)
    }

    /// 系 s での出力の値（index があればその要素だけ）
    pub fn values(&self, s: &System) -> Result<Vec<f64>> {
        let sys = Arc::new(s.clone());
        let out = self.inst.run(&Context::with_system(&sys), &self.inputs)?;
        let a = out.get(&self.output).ok_or_else(|| {
            Error::Invalid(format!(
                "{}: 出力 {} がない（{}）",
                self.label,
                self.output,
                out.keys().cloned().collect::<Vec<_>>().join("・")
            ))
        })?;
        let v = a
            .as_f64()
            .ok_or_else(|| Error::Invalid(format!("{}: 出力が実数でない", self.label)))?;
        Ok(match self.index {
            None => v.to_vec(),
            Some(i) => vec![*v.get(i).ok_or_else(|| {
                Error::Invalid(format!(
                    "{}: index {i}（要素は {} 個）",
                    self.label,
                    v.len()
                ))
            })?],
        })
    }
}

impl UserTerm for PluginTerm {
    fn name(&self) -> &str {
        &self.label
    }
    fn rows(&self) -> usize {
        self.rows
    }
    fn eval(&self, systems: &[&System]) -> Result<Vec<Vec<f64>>> {
        use rayon::prelude::*;
        systems
            .par_iter()
            .map(|s| {
                let v = self.values(s)?;
                if v.len() != self.rows {
                    return Err(Error::Invalid(format!(
                        "{}: 出力 {} 個（{} 個のはず）",
                        self.label,
                        v.len(),
                        self.rows
                    )));
                }
                Ok(v.iter().zip(&self.target).map(|(x, t)| x - t).collect())
            })
            .collect()
    }
}

/// JSON の {"名前": 数・文字列・数の並び} → 設定
pub fn config_of(v: Option<&Value>) -> std::result::Result<Config, String> {
    let mut c = Config::new();
    let Some(v) = v else { return Ok(c) };
    let m = v
        .as_object()
        .ok_or_else(|| format!("args は {{\"名前\": 値}}（{v}）"))?;
    for (k, x) in m {
        let p = match x {
            Value::Number(n) => PValue::Num(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => PValue::Str(s.clone()),
            Value::Array(a) => PValue::Nums(
                a.iter()
                    .map(|y| {
                        y.as_f64()
                            .ok_or_else(|| format!("args の {k} は数の並び（{x}）"))
                    })
                    .collect::<std::result::Result<_, _>>()?,
            ),
            _ => return Err(format!("args の {k} は数・文字列・数の並び（{x}）")),
        };
        c.insert(k.clone(), p);
    }
    Ok(c)
}

/// JSON の {"名前": 数・数の並び} → 入力の配列
pub fn arrays_of(v: Option<&Value>) -> std::result::Result<Arrays, String> {
    let mut a = Arrays::new();
    let Some(v) = v else { return Ok(a) };
    let m = v
        .as_object()
        .ok_or_else(|| format!("inputs は {{\"名前\": 値}}（{v}）"))?;
    for (k, x) in m {
        a.insert(k.clone(), array_of(k, x)?);
    }
    Ok(a)
}

pub fn array_of(k: &str, x: &Value) -> std::result::Result<Array, String> {
    match x {
        Value::Number(n) => Ok(Array::scalar(n.as_f64().unwrap_or(f64::NAN))),
        Value::Array(v) => {
            let d: Vec<f64> = v
                .iter()
                .map(|y| {
                    y.as_f64()
                        .ok_or_else(|| format!("入力 {k} は数の並び（{x}）"))
                })
                .collect::<std::result::Result<_, _>>()?;
            Array::f64(vec![d.len()], d).map_err(|e| e.to_string())
        }
        _ => Err(format!("入力 {k} は数か数の並び（{x}）")),
    }
}
