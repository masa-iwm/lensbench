//! 解析の登録簿（[`lensbench_analysis::Analyses`]）のサブコマンド: 引数は設定のスキーマから作り、
//! 解釈した引数を設定の JSON にして [`lensbench_analysis::DynAnalysis::run_json`] に渡す。

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

/// 既定が真の真偽の項目
fn on_by_default(p: &Param) -> bool {
    p.ty == ParamTy::Flag && p.default == Value::Bool(true)
}

/// 設定の 1 項目の旗
fn arg_of(p: &Param) -> Arg {
    let mut help = p.doc.clone();
    if !p.unit.is_empty() && !help.contains(&p.unit) {
        help = format!("{e} [{unit}]", e = help, unit = p.unit);
    }
    let mut a = Arg::new(leak(p.name.clone()))
        .long(leak(flag_of(&p.name)))
        .help(help);
    // 書き出し先は手書きのサブコマンドと同じく -o でも
    if p.name == "out" {
        a = a.short('o');
    }
    a = match p.ty {
        // 既定が真の旗は値なしで真、`=false` で切る（値を取らない旗では既定の真を切れない）。使い方は --help の
        // `--旗[=<true|false>]` で示し、説明の文は訳せるよう p.doc のまま
        ParamTy::Flag if on_by_default(p) => a
            .value_parser(clap::value_parser!(bool))
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("true")
            .value_name("true|false")
            .action(ArgAction::Set),
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
    // 数の並びで既定が空で省けないもの（スルーフォーカスの周波数など）は必須（設定のファイルで渡すときは省ける）
    if p.required() {
        a = a.required_unless_present(PARAMS_FILE);
    }
    a
}

/// 設定のファイルの旗の id（解析の設定の鍵と重ならないよう、鍵に使わない形）
const PARAMS_FILE: &str = "--params-file";

/// 設定を JSON のファイルから読む旗
fn params_file_arg() -> Arg {
    Arg::new(PARAMS_FILE)
        .long("params-file")
        .value_name("JSON")
        .value_parser(clap::value_parser!(PathBuf))
        .action(ArgAction::Set)
        .help(
            "Settings from a JSON file: an object with the same keys as the flags (underscores, as in the GUI \
             and Python). Flags given on the command line take precedence; relative paths in the file are \
             relative to the file",
        )
}

/// 設定のファイルを読む（JSON のオブジェクト。知らない鍵は解析が断る）。パスの鍵の相対パスはファイルの場所から
fn read_params_file(
    a: &dyn DynAnalysis,
    path: &std::path::Path,
) -> lensbench_core::Result<Map<String, Value>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("--params-file {} can't be read: {e}", path.display()))?;
    let Value::Object(mut o) = serde_json::from_str::<Value>(&text)
        .map_err(|e| format!("--params-file {} JSON can't be read: {e}", path.display()))?
    else {
        return Err(format!("--params-file {} must be a JSON object", path.display()).into());
    };
    let base = path.parent().unwrap_or(std::path::Path::new(""));
    let rel = |v: &mut Value| {
        if let Value::String(s) = v {
            let q = std::path::Path::new(s.as_str());
            if q.is_relative() && !s.is_empty() {
                *s = base.join(q).display().to_string();
            }
        }
    };
    for p in a.params() {
        match (p.ty, o.get_mut(&p.name)) {
            (ParamTy::Path, Some(v)) => rel(v),
            (ParamTy::Paths, Some(Value::Array(vs))) => vs.iter_mut().for_each(rel),
            _ => {}
        }
    }
    Ok(o)
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
    c.arg(params_file_arg())
}

fn nums(s: &str, p: &Param) -> lensbench_core::Result<Value> {
    let v: Vec<f64> = s
        .split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            t.trim()
                .parse::<f64>()
                .map_err(|_| format!("--{} number can't be read ({t})", flag_of(&p.name)))
        })
        .collect::<Result<_, _>>()?;
    if let Some(n) = p.len {
        if v.len() != n {
            return Err(format!(
                "--{} is {n} comma-separated numbers ({s})",
                flag_of(&p.name)
            )
            .into());
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
pub fn parse(a: &dyn DynAnalysis, m: &ArgMatches) -> lensbench_core::Result<AnalysisCmd> {
    let c = Common::from_arg_matches(m).map_err(|e| e.to_string())?;
    let mut o = match m.get_one::<PathBuf>(PARAMS_FILE) {
        Some(f) => read_params_file(a, f)?,
        None => Map::new(),
    };
    for p in a.params() {
        let id = p.name.as_str();
        // 設定のファイルにある鍵は、コマンド行で明示した旗だけが上書きする（旗の既定値では上書きしない）
        if o.contains_key(id) && m.value_source(id) != Some(clap::parser::ValueSource::CommandLine)
        {
            continue;
        }
        let v = match p.ty {
            ParamTy::Flag if on_by_default(&p) => m.get_one::<bool>(id).map(|x| Value::Bool(*x)),
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
                        .map_err(|e| format!("--{} JSON can't be read: {e}", flag_of(&p.name)))
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
