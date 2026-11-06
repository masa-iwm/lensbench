//! 評価関数の項としての ZPL マクロ（OpticStudio の ZPLM オペランド）。
//!
//! マクロは系を読み（[`crate::zpl_solve::SysHost`] と同じ関数・命令: CURV・THIC・RAYTRACE・RAYX など）、
//! `OPTRETURN n, 値`（n は 0〜50）で値を返す。項の値は `OPTRETURN data` の値 − target。
//! `PVHX()`・`PVHY()`・`PVPX()`・`PVPY()` は項の hx・hy・px・py を返す（OpticStudio で ZPLM の行の Hx・Hy・Px・Py を読む関数）。
//!
//! 止まったときなどは OpticStudio 26.1.1 の実測に合わせる:
//! - `OPTRETURN data` を呼ばなかった番号（data が 50 を超えるときも）は 0
//! - 範囲外の RAYTRACE など系の命令が止めたときは、そこで黙って終わり、それまでの値を使う
//! - 実行時のエラー（未定義の変数など）・値が有限でないときは評価できない（エラー）。OpticStudio は評価関数の
//!   全体を 9.0E+09 にする
//! - lensbench が対応していない命令・関数はエラー（OpticStudio は知らない命令をそこで黙って止めるが、
//!   lensbench には OpticStudio の命令で未対応のものとの区別がつかない）
//! - 文法のエラーは読むときにエラー（OpticStudio は何も走らせず全部 0 にする）

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::optim::UserTerm;
use crate::system::System;
use crate::zpl::{Flow, Host, Program, Value, Vars};
use crate::zpl_solve::{Macro, SysHost};

/// OPTRETURN の番号の上限（OpticStudio と同じ 0〜50）
pub const OPTRETURN_MAX: usize = 50;

/// ZPLM の項
pub struct ZplmTerm {
    /// 表示の名前（ZPLM マクロ名 data）
    pub label: String,
    /// マクロの名前
    pub macro_name: String,
    prog: Arc<Program>,
    /// OPTRETURN の番号
    pub data: usize,
    /// PVHX・PVHY・PVPX・PVPY の値
    pub pv: [f64; 4],
    pub target: f64,
}

impl ZplmTerm {
    pub fn new(m: &Macro, data: usize, pv: [f64; 4], target: f64) -> Result<ZplmTerm> {
        Ok(ZplmTerm {
            label: format!("ZPLM {} {data}", m.name),
            macro_name: m.name.clone(),
            prog: m.program()?,
            data,
            pv,
            target,
        })
    }

    /// マクロを系 s で走らせ、OPTRETURN の値（呼ばれなかった番号は None）を返す
    pub fn run(&self, s: &System) -> Result<Vec<Option<f64>>> {
        let mut h = ZplmHost {
            inner: SysHost::new(s, 0),
            pv: self.pv,
            out: vec![None; OPTRETURN_MAX + 1],
        };
        self.prog
            .run_term(&mut h, &mut Vars::default())
            .map_err(|e| e.at(format!("ZPLM のマクロ {}", self.macro_name)))?;
        Ok(h.out)
    }

    /// 系 s での値（OPTRETURN data、呼ばれなかったら 0。目標は引かない）
    pub fn value(&self, s: &System) -> Result<f64> {
        let v = self
            .run(s)?
            .get(self.data)
            .copied()
            .flatten()
            .unwrap_or(0.0);
        if !v.is_finite() {
            crate::invalid!(
                "ZPLM のマクロ {}: OPTRETURN {} の値 {v}",
                self.macro_name,
                self.data
            );
        }
        Ok(v)
    }
}

impl UserTerm for ZplmTerm {
    fn name(&self) -> &str {
        &self.label
    }
    fn rows(&self) -> usize {
        1
    }
    fn eval(&self, systems: &[&System]) -> Result<Vec<Vec<f64>>> {
        use rayon::prelude::*;
        systems
            .par_iter()
            .map(|s| Ok(vec![self.value(s)? - self.target]))
            .collect()
    }
}

struct ZplmHost<'a> {
    inner: SysHost<'a>,
    pv: [f64; 4],
    out: Vec<Option<f64>>,
}

impl Host for ZplmHost<'_> {
    fn function(&mut self, name: &str, a: &[Value]) -> Option<Result<Value>> {
        let k = match name {
            "PVHX" => 0,
            "PVHY" => 1,
            "PVPX" => 2,
            "PVPY" => 3,
            _ => return self.inner.function(name, a),
        };
        Some(if a.is_empty() {
            Ok(Value::Num(self.pv[k]))
        } else {
            Err(Error::Invalid(format!("{name} は引数なし")))
        })
    }

    fn command(&mut self, name: &str, a: &[Value]) -> Option<Result<Flow>> {
        if name != "OPTRETURN" {
            return self.inner.command(name, a);
        }
        Some(match a {
            [Value::Num(n), Value::Num(v)] => {
                let n = n.trunc();
                if n >= 0.0 && n as usize <= OPTRETURN_MAX {
                    self.out[n as usize] = Some(*v);
                    Ok(Flow::Go)
                } else {
                    Err(Error::Invalid(format!(
                        "OPTRETURN の番号 {n}（0〜{OPTRETURN_MAX}）"
                    )))
                }
            }
            _ => Err(Error::Invalid("OPTRETURN の引数は (番号, 値)".into())),
        })
    }
}
