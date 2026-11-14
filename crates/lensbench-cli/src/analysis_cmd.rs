//! 解析の登録簿（[`lensbench_analysis::Analyses`]）のサブコマンド: 引数は設定のスキーマから作り、
//! 解釈した引数を設定の JSON にして [`lensbench_analysis::DynAnalysis::run_json`] に渡す（見直し CLI-02・XCT-02）。

use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches, Args as _, Command, FromArgMatches as _};
use lensbench_analysis::{DynAnalysis, Param, ParamTy};
use serde_json::{Map, Value};

use crate::Common;

/// 解析のサブコマンドを解釈したもの
#[derive(Clone)]
pub struct AnalysisCmd {
    /// 解析の名前
    pub name: String,
    pub c: Common,
    /// 設定（JSON のオブジェクト。省いた鍵は既定値）
    pub params: Value,
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn flag_of(name: &str) -> String {
    name.replace('_', "-")
}

/// 既定値を `--help` に出す文字
fn default_text(p: &Param) -> Option<String> {
    match &p.default {
        Value::Null | Value::Bool(_) => None,
        Value::Array(a) if a.is_empty() => None,
        Value::Array(a) => Some(
            a.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(","),
        ),
        Value::String(s) => Some(s.clone()),
        v => Some(v.to_string()),
    }
}

/// 設定の 1 項目の旗
fn arg_of(p: &Param) -> Arg {
    let mut help = p.doc.clone();
    if !p.unit.is_empty() && !help.contains(&p.unit) {
        help = format!("{help} [{}]", p.unit);
    }
    let mut a = Arg::new(leak(p.name.clone()))
        .long(leak(flag_of(&p.name)))
        .help(help);
    a = match p.ty {
        ParamTy::Flag => a.action(ArgAction::SetTrue),
        ParamTy::Count => a.value_parser(clap::value_parser!(u64)),
        ParamTy::Int => a.value_parser(clap::value_parser!(i64)),
        ParamTy::Num => a.value_parser(clap::value_parser!(f64)),
        ParamTy::Choice => a.value_parser(clap::builder::PossibleValuesParser::new(
            p.choices
                .iter()
                .map(|c| leak(c.clone()))
                .collect::<Vec<_>>(),
        )),
        ParamTy::Str | ParamTy::Nums | ParamTy::Json => {
            a.allow_hyphen_values(p.ty == ParamTy::Nums)
        }
        ParamTy::Path => a.value_parser(clap::value_parser!(PathBuf)),
        ParamTy::Paths => a
            .value_parser(clap::value_parser!(PathBuf))
            .action(ArgAction::Append),
    };
    if p.ty != ParamTy::Flag && p.ty != ParamTy::Paths {
        a = a.action(ArgAction::Set);
        if let Some(d) = default_text(p) {
            a = a.default_value(leak(d));
        }
    }
    // 数の並びで既定が空で省けないもの（スルーフォーカスの周波数など）は必須
    if p.required() {
        a = a.required(true);
    }
    a
}

/// 解析のサブコマンド（系の読み方の引数＋設定の旗）
pub fn command(a: &dyn DynAnalysis) -> Command {
    let m = a.meta();
    // Common の augment_args は説明を Common の doc で上書きするので、説明は後から付ける
    let mut c = Common::augment_args(Command::new(m.name)).about(m.about);
    if !m.help.is_empty() {
        c = c.long_about(m.help);
    }
    for p in a.params() {
        c = c.arg(arg_of(&p));
    }
    c
}

fn nums(s: &str, p: &Param) -> Result<Value, String> {
    let v: Vec<f64> = s
        .split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            t.trim()
                .parse::<f64>()
                .map_err(|_| format!("--{} の数が読めない（{t}）", flag_of(&p.name)))
        })
        .collect::<Result<_, _>>()?;
    if let Some(n) = p.len {
        if v.len() != n {
            return Err(format!(
                "--{} は数 {n} 個のカンマ区切り（{s}）",
                flag_of(&p.name)
            ));
        }
    }
    // 整数の並び（視野・面の番号）にも渡せるよう、整数の値は整数で（実数の並びは整数でも受ける）
    Ok(Value::from(
        v.into_iter()
            .map(|x| {
                if x.fract() == 0.0 && x.abs() < 9.0e15 {
                    Value::from(x as i64)
                } else {
                    Value::from(x)
                }
            })
            .collect::<Vec<_>>(),
    ))
}

/// 解釈した引数から解析のサブコマンドを作る
pub fn parse(a: &dyn DynAnalysis, m: &ArgMatches) -> Result<AnalysisCmd, String> {
    let c = Common::from_arg_matches(m).map_err(|e| e.to_string())?;
    let mut o = Map::new();
    for p in a.params() {
        let id = p.name.as_str();
        let v = match p.ty {
            ParamTy::Flag => m.get_flag(id).then_some(Value::Bool(true)),
            ParamTy::Count => m.get_one::<u64>(id).map(|x| Value::from(*x)),
            ParamTy::Int => m.get_one::<i64>(id).map(|x| Value::from(*x)),
            ParamTy::Num => m.get_one::<f64>(id).map(|x| Value::from(*x)),
            ParamTy::Choice | ParamTy::Str => {
                m.get_one::<String>(id).map(|x| Value::from(x.clone()))
            }
            ParamTy::Nums => m.get_one::<String>(id).map(|x| nums(x, &p)).transpose()?,
            ParamTy::Json => m
                .get_one::<String>(id)
                .map(|x| {
                    serde_json::from_str::<Value>(x)
                        .map_err(|e| format!("--{} の JSON が読めない: {e}", flag_of(&p.name)))
                })
                .transpose()?,
            ParamTy::Path => m
                .get_one::<PathBuf>(id)
                .map(|x| Value::from(x.display().to_string())),
            ParamTy::Paths => m
                .get_many::<PathBuf>(id)
                .map(|xs| Value::from(xs.map(|x| x.display().to_string()).collect::<Vec<_>>())),
        };
        if let Some(v) = v {
            o.insert(p.name.clone(), v);
        }
    }
    Ok(AnalysisCmd {
        name: a.meta().name.to_string(),
        c,
        params: Value::Object(o),
    })
}
