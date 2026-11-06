//! ZPL（OpticStudio のマクロ言語）の小さな解釈系。ZPL Macro Solveと評価関数の ZPLM の項（[`crate::zplm`]）で使う。
//!
//! 対応する範囲:
//! - 数の変数と文字列の変数（名前の最後が `$`）、代入（`x = 式`）。大文字・小文字は区別しない
//! - 式: `+ - * /`、比較 `== != < > <= >=`（真は 1、偽は 0）、論理 `& | ^`（`^` は排他的論理和で、べき乗ではない）、
//!   単項の `-`・`!`、括弧、関数の呼び出し。文字列は `+` でつなぐ
//! - 制御: `IF (式)` … `ELSE` … `ENDIF`（入れ子可）、1 行の `IF 式 THEN 文`、`FOR 変数, 始め, 終わり, 増分` … `NEXT`、
//!   `GOTO 札`・`LABEL 札`、`GOSUB 名前`・`SUB 名前` … `RETURN`、`END`、`SOLVERETURN 式`
//! - 注釈: 行頭の `!` と `REM`、行の途中の `#`（文字列の中は除く）
//! - `PRINT`・`FORMAT` などの出力の命令は何もしない
//! - 数の関数: ABSO ACOS ASIN ATAN COSI SINE TANG EXPE EXPT LOGE LOGT INTE SIGN SQRT POWR（角は rad）
//! - 系のデータを読む関数・命令（CURV・THIC・RAYTRACE など）は [`Host`] から差し込む
//!
//! ここに無い命令・関数を使うと `NotImplemented` を返す（呼ぶ側はソルブの保存された値を残して警告を出す）。OpticStudio に
//! あるかどうか lensbench では分からないので、OpticStudio のエラーの扱い（下）にはしない。
//!
//! ヘルプに書かれていない振る舞い（OpticStudio 26.1.1 で実測）:
//! - 演算子の優先順位（ヘルプは「括弧で優先させる」とだけ書く）: 単項（`-`・`!`）> `* /` > `+ -` >
//!   比較・論理（`== != < > <= >= & | ^`、すべて同じ順位）。同じ順位は左から（`1 | 0 & 0` は 0、`0 & 0 == 0` は 1、
//!   `3 > 2 > 1` は 0）
//! - INTE は小さい方への丸め（INTE(-2.2) は −3）。SIGN(0) は 0。`<>` は使えない（エラー）。行の途中の `!` は注釈ではない
//! - まだ入れていない変数を読むとエラー
//! - SOSO(0) はソルブのある面（ほかの引数は未対応）
//!
//! エラーの扱い（OpticStudio 26.1.1 で実測。[`Program::solve`] は OpticStudio と同じ値を返す）:
//! - 0 での割り算は 0。負の数の SQRT・LOGE・LOGT は引数をそのまま返す。範囲外の ACOS は 0、ASIN は NaN（値が入らない）
//! - 実行中のエラー（まだ入れていない変数、解釈できない行に来たときなど）: そこで止め、それまでの SOLVERETURN の値、
//!   なければ 0 を入れる
//! - FOR の増分の向きが始め→終わりと逆か 0、回数の上限を超えた: 値を変えない
//! - `SUB` に END なしで流れ込むと、そこで終わる
//! - SOLVERETURN が一度もない・値が NaN か無限大: 値を変えない
//!
//! ヘルプに書かれている振る舞い:
//! - FOR の終わりと増分は始めに 1 回だけ評価する。始め < 終わりなら変数が終わりを超えるまで、始め > 終わりなら
//!   終わりを下回るまで回す。始め = 終わりなら 1 回だけ回す
//! - 変数はすべて大域（SUB の中も同じ）
//! - SOLVERETURN は最後に呼んだ値が効く

use std::collections::HashMap;

use crate::error::{Error, Result};

/// 値（数か文字列）
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
}

impl Value {
    fn num(&self, line: usize) -> Result<f64> {
        match self {
            Value::Num(x) => Ok(*x),
            Value::Str(_) => Err(err(line, "数が要るところに文字列")),
        }
    }
    fn str(&self, line: usize) -> Result<&str> {
        match self {
            Value::Str(s) => Ok(s),
            Value::Num(_) => Err(err(line, "文字列が要るところに数")),
        }
    }
}

/// 系のデータを読む関数・命令を差し込む口。知らない名前には `None` を返す
pub trait Host {
    /// 関数（名前は大文字）。例: `CURV(3)`
    fn function(&mut self, name: &str, args: &[Value]) -> Option<Result<Value>>;
    /// 命令（名前は大文字）。例: `RAYTRACE 0, 1, 0, 1`
    fn command(&mut self, name: &str, args: &[Value]) -> Option<Result<Flow>>;
}

/// 命令の後どうするか
#[derive(Debug, Clone, PartialEq)]
pub enum Flow {
    /// 続ける
    Go,
    /// マクロを止め、ソルブの値を変えない（OpticStudio の「does not return a valid value」。理由を添える）
    Abort(String),
}

/// 何も差し込まない [`Host`]
pub struct NoHost;
impl Host for NoHost {
    fn function(&mut self, _: &str, _: &[Value]) -> Option<Result<Value>> {
        None
    }
    fn command(&mut self, _: &str, _: &[Value]) -> Option<Result<Flow>> {
        None
    }
}

fn err(line: usize, msg: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("ZPL の {line} 行目: {msg}"))
}
fn unsupported(line: usize, msg: impl std::fmt::Display) -> Error {
    Error::NotImplemented(format!("ZPL の {line} 行目: {msg}"))
}

// ---------------------------------------------------------------- 字句

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    /// 名前（大文字にしたもの。最後の `$`、先頭の `$` も含む）
    Id(String),
    /// 記号: + - * / ( ) , = == != < > <= >= & | ^ !
    Op(&'static str),
}

const OPS: [&str; 15] = [
    "==", "!=", "<=", ">=", "+", "-", "*", "/", "(", ")", ",", "=", "<", ">", "!",
];

fn lex(src: &str, line: usize) -> Result<Vec<Tok>> {
    let c: Vec<char> = src.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '#' {
            break; // 行の途中の注釈
        } else if ch == '"' {
            let j = (i + 1..c.len())
                .find(|&j| c[j] == '"')
                .ok_or_else(|| err(line, "文字列が閉じていない"))?;
            out.push(Tok::Str(c[i + 1..j].iter().collect()));
            i = j + 1;
        } else if ch.is_ascii_digit()
            || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let mut j = i;
            while j < c.len() && (c[j].is_ascii_digit() || c[j] == '.') {
                j += 1;
            }
            if j < c.len() && (c[j] == 'e' || c[j] == 'E') {
                let k = j + 1 + usize::from(c.get(j + 1).is_some_and(|s| *s == '+' || *s == '-'));
                if c.get(k).is_some_and(|d| d.is_ascii_digit()) {
                    j = k;
                    while j < c.len() && c[j].is_ascii_digit() {
                        j += 1;
                    }
                }
            }
            let s: String = c[i..j].iter().collect();
            out.push(Tok::Num(
                s.parse().map_err(|_| err(line, format!("数 {s}")))?,
            ));
            i = j;
        } else if ch.is_alphabetic() || ch == '_' || ch == '$' {
            let mut j = i + 1;
            while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_') {
                j += 1;
            }
            if j < c.len() && c[j] == '$' {
                j += 1;
            }
            out.push(Tok::Id(c[i..j].iter().collect::<String>().to_uppercase()));
            i = j;
        } else if ch == '&' || ch == '|' || ch == '^' {
            out.push(Tok::Op(match ch {
                '&' => "&",
                '|' => "|",
                _ => "^",
            }));
            i += 1;
        } else {
            let rest: String = c[i..(i + 2).min(c.len())].iter().collect();
            let op = OPS
                .iter()
                .find(|o| rest.starts_with(**o))
                .ok_or_else(|| err(line, format!("字 {ch:?}")))?;
            out.push(Tok::Op(op));
            i += op.len();
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- 式

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Lit(Value),
    Var(String),
    Call(String, Vec<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
}

/// 二項演算子の順位（大きいほど強い）
fn prec(op: &str) -> Option<u8> {
    Some(match op {
        // 比較と論理は同じ順位（OpticStudio 26.1.1 で実測）
        "|" | "^" | "&" | "==" | "!=" | "<" | ">" | "<=" | ">=" => 1,
        "+" | "-" => 2,
        "*" | "/" => 3,
        _ => return None,
    })
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
    line: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(o)) if *o == op)
    }
    fn eat_op(&mut self, op: &str) -> bool {
        let y = self.is_op(op);
        self.i += usize::from(y);
        y
    }
    fn expect_op(&mut self, op: &str) -> Result<()> {
        if self.eat_op(op) {
            Ok(())
        } else {
            Err(err(self.line, format!("{op} が要る")))
        }
    }
    fn done(&self) -> bool {
        self.i >= self.t.len()
    }

    fn expr(&mut self, min: u8) -> Result<Expr> {
        let mut l = self.unary()?;
        while let Some((op, p)) = match self.peek() {
            Some(Tok::Op(o)) => prec(o).filter(|p| *p >= min).map(|p| (*o, p)),
            _ => None,
        } {
            self.i += 1;
            let r = self.expr(p + 1)?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<Expr> {
        if self.eat_op("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat_op("+") {
            return self.unary();
        }
        if self.eat_op("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        let line = self.line;
        match self.t.get(self.i).cloned() {
            Some(Tok::Num(x)) => {
                self.i += 1;
                Ok(Expr::Lit(Value::Num(x)))
            }
            Some(Tok::Str(s)) => {
                self.i += 1;
                Ok(Expr::Lit(Value::Str(s)))
            }
            Some(Tok::Op("(")) => {
                self.i += 1;
                let e = self.expr(0)?;
                self.expect_op(")")?;
                Ok(e)
            }
            Some(Tok::Id(name)) => {
                self.i += 1;
                if self.eat_op("(") {
                    let mut args = vec![];
                    if !self.eat_op(")") {
                        loop {
                            args.push(self.expr(0)?);
                            if self.eat_op(")") {
                                break;
                            }
                            self.expect_op(",")?;
                        }
                    }
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Some(t) => Err(err(line, format!("式の途中に {t:?}"))),
            None => Err(err(line, "式が要る")),
        }
    }

    /// 「,」で区切った式の並び（空でもよい）
    fn list(&mut self) -> Result<Vec<Expr>> {
        let mut v = vec![];
        if self.done() {
            return Ok(v);
        }
        loop {
            v.push(self.expr(0)?);
            if !self.eat_op(",") {
                break;
            }
        }
        Ok(v)
    }

    fn end(&self) -> Result<()> {
        match self.peek() {
            None => Ok(()),
            Some(t) => Err(err(self.line, format!("行の終わりに余分な {t:?}"))),
        }
    }
}

// ---------------------------------------------------------------- 命令

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Assign(String, Expr),
    /// 偽なら飛ぶ
    JumpIfNot(Expr, usize),
    Jump(usize),
    /// FOR の始め（変数、始め、終わり、増分、FOR の番号）。体は次の命令から
    ForInit(String, Expr, Expr, Expr, usize),
    /// NEXT（FOR の番号、体の始め）
    ForNext(usize, usize),
    Gosub(String),
    Return,
    End,
    /// SUB に END なしで流れ込んだ
    FallIntoSub,
    SolveReturn(Expr),
    Command(String, Vec<Expr>),
    Nop,
    /// 解釈できなかった行（そこまで来たらエラー）
    Fail(Error),
}

/// 走らせるのを止めた理由
enum Stop {
    /// エラー（OpticStudio はそこで止め、それまでの SOLVERETURN の値、なければ 0 を入れる）
    Err(Error),
    /// 値を入れずに止まる（OpticStudio は値を変えない。FOR の増分の向きが逆など）
    Keep(Error),
    /// 系の命令が止めた（[`Flow::Abort`]。ソルブでは Keep と同じ）
    Abort(Error),
}

impl From<Error> for Stop {
    fn from(e: Error) -> Stop {
        Stop::Err(e)
    }
}

/// マクロのソルブの結果
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// 入れる値（None は値を変えない）
    pub value: Option<f64>,
    /// 途中で止まったときのエラー（OpticStudio は黙って値を入れる・変えない）
    pub error: Option<String>,
}

/// 解釈した ZPL のマクロ
#[derive(Debug, Clone)]
pub struct Program {
    ops: Vec<Op>,
    /// 命令ごとのもとの行の番号（1 から）
    lines: Vec<usize>,
    labels: HashMap<String, usize>,
    subs: HashMap<String, usize>,
    nfor: usize,
}

/// 何もしない命令（出力・表示）
const IGNORED: [&str; 4] = ["PRINT", "FORMAT", "BEEP", "PAUSE"];

impl Program {
    /// マクロの文を解釈する
    pub fn parse(src: &str) -> Result<Program> {
        Program::parse_inner(src, true)
    }

    /// `resolve` が偽なら GOTO・GOSUB の行き先を確かめない（1 行の IF の THEN の後）
    fn parse_inner(src: &str, resolve: bool) -> Result<Program> {
        let mut p = Program {
            ops: vec![],
            lines: vec![],
            labels: HashMap::new(),
            subs: HashMap::new(),
            nfor: 0,
        };
        // 開いている IF（JumpIfNot の位置、ELSE の後の Jump の位置）と FOR（変数、FOR の番号、体の始め）
        enum Open {
            If(usize, Option<usize>),
            For(String, usize, usize),
        }
        let mut open: Vec<Open> = vec![];
        for (k, raw) in src.lines().enumerate() {
            let line = k + 1;
            let s = raw.trim();
            if s.is_empty() || s.starts_with('!') {
                continue;
            }
            let at0 = p.ops.len();
            let r = (|| -> Result<()> {
                let t = lex(s, line)?;
                let Some(Tok::Id(head)) = t.first().cloned() else {
                    if t.is_empty() {
                        return Ok(());
                    }
                    return Err(err(line, "行の始めは名前でなければならない"));
                };
                if head == "REM" {
                    return Ok(());
                }
                let mut ps = Parser { t: &t, i: 1, line };
                let at = p.ops.len();
                let push = |p: &mut Program, op: Op| {
                    p.ops.push(op);
                    p.lines.push(line);
                };
                if ps.is_op("=") {
                    ps.i += 1;
                    let e = ps.expr(0)?;
                    ps.end()?;
                    push(&mut p, Op::Assign(head, e));
                    return Ok(());
                }
                match head.as_str() {
                    "IF" => {
                        let then = t.iter().position(|x| *x == Tok::Id("THEN".into()));
                        match then {
                            Some(j) => {
                                // 1 行の IF: IF 式 THEN 文
                                let c = Parser {
                                    t: &t[..j],
                                    i: 1,
                                    line,
                                }
                                .whole()?;
                                let body =
                                    &s[then_offset(s).ok_or_else(|| err(line, "THEN の後"))?..];
                                let sub = Program::parse_inner(body, false)
                                    .map_err(|e| e.at(format!("ZPL の {line} 行目の THEN の後")))?;
                                if sub.ops.len() != 1
                                    || !sub.labels.is_empty()
                                    || !sub.subs.is_empty()
                                    || sub.nfor > 0
                                {
                                    return Err(err(line, "THEN の後は 1 つの文"));
                                }
                                push(&mut p, Op::JumpIfNot(c, at + 2));
                                let op = sub.ops.into_iter().next().unwrap();
                                if matches!(op, Op::JumpIfNot(..)) {
                                    return Err(unsupported(line, "THEN の後の IF"));
                                }
                                push(&mut p, op);
                            }
                            None => {
                                let c = ps.expr(0)?;
                                ps.end()?;
                                push(&mut p, Op::JumpIfNot(c, usize::MAX));
                                open.push(Open::If(at, None));
                            }
                        }
                    }
                    "ELSE" => {
                        ps.end()?;
                        match open.last_mut() {
                            Some(Open::If(j, e @ None)) => {
                                let j = *j;
                                *e = Some(at);
                                push(&mut p, Op::Jump(usize::MAX));
                                p.ops[j] = match p.ops[j].clone() {
                                    Op::JumpIfNot(c, _) => Op::JumpIfNot(c, at + 1),
                                    o => o,
                                };
                            }
                            _ => return Err(err(line, "対になる IF のない ELSE")),
                        }
                    }
                    "ENDIF" => {
                        ps.end()?;
                        match open.pop() {
                            Some(Open::If(j, e)) => {
                                match e {
                                    Some(e) => p.ops[e] = Op::Jump(at),
                                    None => {
                                        if let Op::JumpIfNot(c, _) = p.ops[j].clone() {
                                            p.ops[j] = Op::JumpIfNot(c, at);
                                        }
                                    }
                                }
                                push(&mut p, Op::Nop);
                            }
                            _ => return Err(err(line, "対になる IF のない ENDIF")),
                        }
                    }
                    "FOR" => {
                        let v = match ps.t.get(1) {
                            Some(Tok::Id(v)) if !v.ends_with('$') => v.clone(),
                            _ => return Err(err(line, "FOR の後は数の変数")),
                        };
                        ps.i = 2;
                        ps.expect_op(",")?;
                        let a = ps.list()?;
                        ps.end()?;
                        let [a, b, c]: [Expr; 3] = a
                            .try_into()
                            .map_err(|_| err(line, "FOR 変数, 始め, 終わり, 増分"))?;
                        let n = p.nfor;
                        p.nfor += 1;
                        push(&mut p, Op::ForInit(v.clone(), a, b, c, n));
                        open.push(Open::For(v, n, at + 1));
                    }
                    "NEXT" => {
                        // NEXT の後の変数の名前は書いてもよい（照らし合わせる）
                        match open.pop() {
                            Some(Open::For(v, n, body)) => {
                                match ps.t.get(1) {
                                    None => {}
                                    Some(Tok::Id(w)) if *w == v && ps.t.len() == 2 => {}
                                    _ => return Err(err(line, format!("NEXT の後は {v}"))),
                                }
                                push(&mut p, Op::ForNext(n, body));
                            }
                            _ => return Err(err(line, "対になる FOR のない NEXT")),
                        }
                    }
                    "LABEL" | "GOTO" | "GOSUB" | "SUB" => {
                        let name = match (&t.get(1), t.len()) {
                            (Some(Tok::Id(x)), 2) => x.clone(),
                            (Some(Tok::Num(x)), 2) => format!("{x}"),
                            _ => return Err(err(line, format!("{head} の後は名前か番号 1 つ"))),
                        };
                        match head.as_str() {
                            "LABEL" => {
                                if p.labels.insert(name.clone(), at).is_some() {
                                    return Err(err(line, format!("札 {name} が 2 つある")));
                                }
                                push(&mut p, Op::Nop);
                            }
                            "SUB" => {
                                if !open.is_empty() {
                                    return Err(err(line, "IF・FOR の中の SUB"));
                                }
                                if p.subs.insert(name.clone(), at + 1).is_some() {
                                    return Err(err(line, format!("SUB {name} が 2 つある")));
                                }
                                push(&mut p, Op::FallIntoSub);
                            }
                            "GOTO" => push(&mut p, Op::Jump(usize::MAX - 1)),
                            _ => push(&mut p, Op::Gosub(name.clone())),
                        }
                        if head == "GOTO" {
                            // 札は後ろにもありうるので、終わってから解決する
                            p.pending_goto(at, name);
                        }
                    }
                    "RETURN" => {
                        ps.end()?;
                        push(&mut p, Op::Return);
                    }
                    "END" => {
                        ps.end()?;
                        push(&mut p, Op::End);
                    }
                    "SOLVERETURN" => {
                        let e = ps.expr(0)?;
                        ps.end()?;
                        push(&mut p, Op::SolveReturn(e));
                    }
                    h if IGNORED.contains(&h) => push(&mut p, Op::Nop),
                    _ => {
                        let a = ps.list()?;
                        ps.end()?;
                        push(&mut p, Op::Command(head, a));
                    }
                }
                Ok(())
            })();
            if let Err(e) = r {
                // OpticStudio は行ごとに解釈して走らせるので、解釈できない行はそこまで来たときのエラーにする
                p.ops.truncate(at0);
                p.lines.truncate(at0);
                p.ops.push(Op::Fail(e));
                p.lines.push(line);
            }
        }
        // 閉じていない IF・FOR はそのまま（偽の IF は終わりへ飛ぶ。OpticStudio も走らせるまで調べない）
        if resolve {
            p.resolve_gotos()?;
        }
        Ok(p)
    }

    fn pending_goto(&mut self, at: usize, name: String) {
        // 名前は Command に入れておき、resolve_gotos で Jump にする
        self.ops[at] = Op::Command(format!("\u{0}GOTO {name}"), vec![]);
    }

    fn resolve_gotos(&mut self) -> Result<()> {
        for k in 0..self.ops.len() {
            if let Op::Command(c, _) = &self.ops[k] {
                if let Some(name) = c.strip_prefix("\u{0}GOTO ") {
                    self.ops[k] = match self.labels.get(name) {
                        Some(&to) => Op::Jump(to),
                        None => Op::Fail(err(self.lines[k], format!("札 {name} がない"))),
                    };
                }
            }
        }
        for k in 0..self.ops.len() {
            if let Op::Gosub(n) = &self.ops[k] {
                if !self.subs.contains_key(n) {
                    self.ops[k] = Op::Fail(err(self.lines[k], format!("SUB {n} がない")));
                }
            }
        }
        Ok(())
    }

    /// 走らせて、最後の SOLVERETURN の値を返す（呼ばなければ None）。変数は `vars` に残る
    pub fn run(&self, host: &mut dyn Host, vars: &mut Vars) -> Result<Option<f64>> {
        let mut ret = None;
        match self.exec(host, vars, &mut ret) {
            Ok(()) => Ok(ret),
            Err(Stop::Err(e) | Stop::Keep(e) | Stop::Abort(e)) => Err(e),
        }
    }

    /// 評価関数の項として走らせる（OpticStudio 26.1.1 の ZPLM と同じ扱い）: 系の命令が止めた
    /// （[`Flow::Abort`]、範囲外の RAYTRACE など）ときは、そこで黙って終わる（Ok）。ほかのエラーは Err
    pub fn run_term(&self, host: &mut dyn Host, vars: &mut Vars) -> Result<()> {
        let mut ret = None;
        match self.exec(host, vars, &mut ret) {
            Ok(()) | Err(Stop::Abort(_)) => Ok(()),
            Err(Stop::Err(e) | Stop::Keep(e)) => Err(e),
        }
    }

    /// ソルブとして走らせる（OpticStudio 26.1.1 と同じ扱い）: 最後の SOLVERETURN の値を入れる。
    /// 呼ばなければ値を変えない。途中のエラーはそこで止め、それまでの値（なければ 0）を入れる。値が有限でなければ
    /// 変えない。lensbench が対応していない命令・関数（`NotImplemented`）だけはエラーで返す
    pub fn solve(&self, host: &mut dyn Host) -> Result<Outcome> {
        let mut ret = None;
        let r = self.exec(host, &mut Vars::default(), &mut ret);
        let fin = |v: Option<f64>| v.filter(|x| x.is_finite());
        Ok(match r {
            Ok(()) => Outcome {
                value: fin(ret),
                error: None,
            },
            Err(
                Stop::Err(e @ Error::NotImplemented(_))
                | Stop::Keep(e @ Error::NotImplemented(_))
                | Stop::Abort(e @ Error::NotImplemented(_)),
            ) => return Err(e),
            Err(Stop::Err(e)) => Outcome {
                value: fin(Some(ret.unwrap_or(0.0))),
                error: Some(e.to_string()),
            },
            Err(Stop::Keep(e) | Stop::Abort(e)) => Outcome {
                value: None,
                error: Some(e.to_string()),
            },
        })
    }

    fn exec(
        &self,
        host: &mut dyn Host,
        vars: &mut Vars,
        ret: &mut Option<f64>,
    ) -> std::result::Result<(), Stop> {
        const MAX_STEPS: usize = 50_000_000;
        const MAX_DEPTH: usize = 100;
        let mut pc = 0;
        let mut stack: Vec<usize> = vec![];
        // FOR ごとの（終わり、増分、向き）。向き: 1 は増える、-1 は減る、0 は 1 回だけ
        let mut fors: Vec<(f64, f64, i8)> = vec![(0.0, 0.0, 0); self.nfor];
        let mut steps = 0;
        while pc < self.ops.len() {
            steps += 1;
            let line = self.lines[pc];
            if steps > MAX_STEPS {
                return Err(Stop::Keep(err(
                    line,
                    "命令の数が多すぎる（終わらない繰り返し？）",
                )));
            }
            let mut ev = Eval {
                host: &mut *host,
                vars: &*vars,
                line,
            };
            match &self.ops[pc] {
                Op::Assign(name, e) => {
                    let v = ev.expr(e)?;
                    match (name.ends_with('$'), &v) {
                        (true, Value::Str(_)) | (false, Value::Num(_)) => {}
                        _ => return Err(err(line, format!("{name} に入れる値の種類が違う")).into()),
                    }
                    vars.0.insert(name.clone(), v);
                }
                Op::JumpIfNot(c, to) => {
                    if ev.expr(c)?.num(line)? == 0.0 {
                        pc = *to;
                        continue;
                    }
                }
                Op::Jump(to) => {
                    pc = *to;
                    continue;
                }
                Op::ForInit(v, a, b, c, n) => {
                    let a = ev.expr(a)?.num(line)?;
                    let b = ev.expr(b)?.num(line)?;
                    let c = ev.expr(c)?.num(line)?;
                    let dir = if a < b {
                        1
                    } else if a > b {
                        -1
                    } else {
                        0
                    };
                    if (dir == 1 && !(c > 0.0)) || (dir == -1 && !(c < 0.0)) {
                        return Err(Stop::Keep(err(
                            line,
                            format!("FOR の増分 {c} の向きが {a} → {b} と合わない"),
                        )));
                    }
                    fors[*n] = (b, c, dir);
                    vars.0.insert(v.clone(), Value::Num(a));
                }
                Op::ForNext(n, body) => {
                    let (stop, inc, dir) = fors[*n];
                    let Op::ForInit(v, ..) = &self.ops[body - 1] else {
                        unreachable!()
                    };
                    let x = vars.get_num(v) + inc;
                    vars.0.insert(v.clone(), Value::Num(x));
                    let again = match dir {
                        1 => x <= stop,
                        -1 => x >= stop,
                        _ => false,
                    };
                    if again {
                        pc = *body;
                        continue;
                    }
                }
                Op::Gosub(name) => {
                    if stack.len() >= MAX_DEPTH {
                        return Err(err(line, "GOSUB の入れ子が深すぎる").into());
                    }
                    stack.push(pc + 1);
                    pc = self.subs[name];
                    continue;
                }
                Op::Return => match stack.pop() {
                    Some(r) => {
                        pc = r;
                        continue;
                    }
                    None => return Err(err(line, "GOSUB の外の RETURN").into()),
                },
                Op::End => break,
                // SUB に流れ込むと終わる（OpticStudio 26.1.1 で実測: それまでの値が入る）
                Op::FallIntoSub => break,
                Op::SolveReturn(e) => *ret = Some(ev.expr(e)?.num(line)?),
                Op::Command(name, args) => {
                    let a = args
                        .iter()
                        .map(|e| ev.expr(e))
                        .collect::<Result<Vec<_>>>()?;
                    match host.command(name, &a) {
                        Some(r) => {
                            if let Flow::Abort(why) =
                                r.map_err(|e| e.at(format!("ZPL の {line} 行目")))?
                            {
                                return Err(Stop::Abort(err(line, why)));
                            }
                        }
                        None => return Err(unsupported(line, format!("命令 {name}")).into()),
                    }
                }
                Op::Nop => {}
                Op::Fail(e) => return Err(e.clone().into()),
            }
            pc += 1;
        }
        Ok(())
    }
}

/// 1 行の IF の THEN の後の文の始め（文字列の外の最初の THEN）
fn then_offset(s: &str) -> Option<usize> {
    let up = s.to_uppercase();
    let b = up.as_bytes();
    let mut in_str = false;
    for i in 0..b.len() {
        if b[i] == b'"' {
            in_str = !in_str;
        }
        if !in_str
            && up[i..].starts_with("THEN")
            && (i == 0 || !(b[i - 1] as char).is_alphanumeric() && b[i - 1] != b'_')
            && b.get(i + 4)
                .map_or(true, |c| !(*c as char).is_alphanumeric() && *c != b'_')
        {
            return Some(i + 4);
        }
    }
    None
}

impl Parser<'_> {
    /// 残り全部が 1 つの式
    fn whole(mut self) -> Result<Expr> {
        let e = self.expr(0)?;
        self.end()?;
        Ok(e)
    }
}

/// 変数（すべて大域）
#[derive(Debug, Clone, Default)]
pub struct Vars(HashMap<String, Value>);

impl Vars {
    pub fn get_num(&self, name: &str) -> f64 {
        match self.0.get(&name.to_uppercase()) {
            Some(Value::Num(x)) => *x,
            _ => 0.0,
        }
    }
    pub fn set_num(&mut self, name: &str, x: f64) {
        self.0.insert(name.to_uppercase(), Value::Num(x));
    }
}

// ---------------------------------------------------------------- 評価

struct Eval<'a> {
    host: &'a mut dyn Host,
    vars: &'a Vars,
    line: usize,
}

impl Eval<'_> {
    fn expr(&mut self, e: &Expr) -> Result<Value> {
        let line = self.line;
        Ok(match e {
            Expr::Lit(v) => v.clone(),
            Expr::Var(n) => self
                .vars
                .0
                .get(n)
                .cloned()
                .ok_or_else(|| err(line, format!("変数 {n} にまだ値を入れていない")))?,
            Expr::Neg(a) => Value::Num(-self.expr(a)?.num(line)?),
            Expr::Not(a) => Value::Num(b(self.expr(a)?.num(line)? == 0.0)),
            Expr::Bin(op, a, c) => {
                let x = self.expr(a)?;
                let y = self.expr(c)?;
                if let (Value::Str(s), "+") = (&x, *op) {
                    return Ok(Value::Str(format!("{s}{}", y.str(line)?)));
                }
                let (x, y) = (x.num(line)?, y.num(line)?);
                Value::Num(match *op {
                    "+" => x + y,
                    "-" => x - y,
                    "*" => x * y,
                    // 0 で割ると 0（OpticStudio 26.1.1 で実測）
                    "/" => {
                        if y == 0.0 {
                            0.0
                        } else {
                            x / y
                        }
                    }
                    "==" => b(x == y),
                    "!=" => b(x != y),
                    "<" => b(x < y),
                    ">" => b(x > y),
                    "<=" => b(x <= y),
                    ">=" => b(x >= y),
                    "&" => b(x != 0.0 && y != 0.0),
                    "|" => b(x != 0.0 || y != 0.0),
                    "^" => b((x != 0.0) != (y != 0.0)),
                    _ => unreachable!(),
                })
            }
            Expr::Call(name, args) => {
                let a = args
                    .iter()
                    .map(|e| self.expr(e))
                    .collect::<Result<Vec<_>>>()?;
                if let Some(v) = builtin(name, &a, line)? {
                    return Ok(v);
                }
                match self.host.function(name, &a) {
                    Some(r) => r.map_err(|e| e.at(format!("ZPL の {line} 行目")))?,
                    None => return Err(unsupported(line, format!("関数 {name}"))),
                }
            }
        })
    }
}

fn b(x: bool) -> f64 {
    if x {
        1.0
    } else {
        0.0
    }
}

/// 系によらない数の関数（知らない名前は None）
fn builtin(name: &str, a: &[Value], line: usize) -> Result<Option<Value>> {
    let one = |f: fn(f64) -> f64| -> Result<Option<Value>> {
        match a {
            [x] => Ok(Some(Value::Num(f(x.num(line)?)))),
            _ => Err(err(line, format!("{name} の引数は 1 つ"))),
        }
    };
    // 範囲外の引数のときの値（OpticStudio 26.1.1 で実測）: or の値を返す（None は引数をそのまま）
    let dom = |ok: fn(f64) -> bool, f: fn(f64) -> f64, or: Option<f64>| -> Result<Option<Value>> {
        match a {
            [x] => {
                let x = x.num(line)?;
                Ok(Some(Value::Num(if ok(x) { f(x) } else { or.unwrap_or(x) })))
            }
            _ => Err(err(line, format!("{name} の引数は 1 つ"))),
        }
    };
    match name {
        "ABSO" => one(f64::abs),
        // 範囲外の ACOS は 0、ASIN は値なし（NaN。ソルブの値は変わらない）
        "ACOS" => dom(|x| (-1.0..=1.0).contains(&x), f64::acos, Some(0.0)),
        "ASIN" => dom(|x| (-1.0..=1.0).contains(&x), f64::asin, Some(f64::NAN)),
        "ATAN" => one(f64::atan),
        "COSI" => one(f64::cos),
        "SINE" => one(f64::sin),
        "TANG" => one(f64::tan),
        "EXPE" => one(f64::exp),
        "EXPT" => one(|x| 10f64.powf(x)),
        // 0 以下の LOGE・LOGT、負の SQRT は引数をそのまま返す
        "LOGE" => dom(|x| x > 0.0, f64::ln, None),
        "LOGT" => dom(|x| x > 0.0, f64::log10, None),
        "INTE" => one(f64::floor),
        "SIGN" => one(|x| {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }
        }),
        "SQRT" => dom(|x| x >= 0.0, f64::sqrt, None),
        "POWR" => match a {
            [x, y] => Ok(Some(Value::Num(x.num(line)?.abs().powf(y.num(line)?)))),
            _ => Err(err(line, "POWR の引数は 2 つ")),
        },
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Result<Option<f64>> {
        Program::parse(src)?.run(&mut NoHost, &mut Vars::default())
    }
    fn val(src: &str) -> f64 {
        run(src).unwrap().unwrap()
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(val("SOLVERETURN 1 + 2 * 3"), 7.0);
        assert_eq!(val("SOLVERETURN (1 + 2) * 3"), 9.0);
        assert_eq!(val("SOLVERETURN 8 / 4 / 2"), 1.0);
        assert_eq!(val("SOLVERETURN 1 - 2 - 3"), -4.0);
        assert_eq!(val("SOLVERETURN -2 * -3"), 6.0);
        assert_eq!(val("SOLVERETURN 1.5e2 + .5"), 150.5);
        assert_eq!(val("SOLVERETURN 1 + 1 == 2"), 1.0);
        // 比較・論理は同じ順位で左から（OpticStudio 26.1.1 の値）
        assert_eq!(val("SOLVERETURN 1 < 2 & 3 > 4"), 0.0);
        assert_eq!(val("SOLVERETURN 1 | 0 & 0"), 0.0);
        assert_eq!(val("SOLVERETURN 0 & 0 == 0"), 1.0);
        assert_eq!(val("SOLVERETURN 0 == 0 & 0"), 0.0);
        assert_eq!(val("SOLVERETURN 1 | 0 == 0"), 0.0);
        assert_eq!(val("SOLVERETURN 1 | 1 ^ 1"), 0.0);
        assert_eq!(val("SOLVERETURN 1 ^ 1 | 1"), 1.0);
        assert_eq!(val("SOLVERETURN 0 & 2 < 3"), 1.0);
        assert_eq!(val("SOLVERETURN 3 > 2 > 1"), 0.0);
        assert_eq!(val("SOLVERETURN 1 + 1 ^ 1"), 0.0);
        assert_eq!(val("SOLVERETURN 2 == 2 + 1"), 0.0);
        assert_eq!(val("SOLVERETURN !0 + 1"), 2.0);
        assert_eq!(val("SOLVERETURN !1 == 0"), 1.0);
        assert_eq!(val("SOLVERETURN 2 & 3"), 1.0);
        assert_eq!(val("SOLVERETURN 1 ^ 1"), 0.0); // 排他的論理和
        assert_eq!(val("SOLVERETURN 0 | 2"), 1.0);
        assert_eq!(val("SOLVERETURN !0"), 1.0);
        assert!(run("SOLVERETURN 3 <> 3").is_err());
    }

    #[test]
    fn functions() {
        assert_eq!(val("SOLVERETURN SQRT(16) + ABSO(-1)"), 5.0);
        assert_eq!(val("SOLVERETURN POWR(-2, 3)"), 8.0);
        assert_eq!(val("SOLVERETURN INTE(-2.7)"), -3.0);
        assert_eq!(val("SOLVERETURN INTE(2.7)"), 2.0);
        assert_eq!(val("SOLVERETURN SIGN(0) + SIGN(-3) * 10"), -10.0);
        assert_eq!(val("SOLVERETURN ATAN(1) * 4"), std::f64::consts::PI);
        assert_eq!(val("SOLVERETURN EXPT(2)"), 100.0);
        // 範囲外の引数・0 での割り算（OpticStudio 26.1.1 の値）
        assert_eq!(val("SOLVERETURN SQRT(-4) + 10"), 6.0);
        assert_eq!(val("SOLVERETURN LOGE(0) + 10"), 10.0);
        assert_eq!(val("SOLVERETURN LOGT(-1) + 7"), 6.0);
        assert_eq!(val("SOLVERETURN ACOS(2) + 7"), 7.0);
        assert!(val("SOLVERETURN ASIN(2) + 7").is_nan());
        assert_eq!(val("SOLVERETURN 5 + 1 / 0"), 5.0);
        assert!(matches!(
            run("SOLVERETURN FOOB(1)"),
            Err(Error::NotImplemented(_))
        ));
    }

    #[test]
    fn variables_comments_case() {
        let src = "! 注釈\nrem 注釈\nx = 2 # 行の途中の注釈\nY = X * 3\nname$ = \"a#b\" + \"c\"\nSolveReturn y";
        assert_eq!(val(src), 6.0);
        assert!(run("SOLVERETURN undefined + 1").is_err());
        assert!(run("x = 6 ! 注釈ではない\nSOLVERETURN x").is_err());
        assert!(run("x = \"a\"").is_err());
    }

    #[test]
    fn if_blocks() {
        let src = "x = 3\nIF (x > 2)\n  IF x > 5\n    y = 1\n  ELSE\n    y = 2\n  ENDIF\nELSE\n  y = 3\nENDIF\nSOLVERETURN y";
        assert_eq!(val(src), 2.0);
        assert_eq!(val("x = 0\nIF x THEN x = 5\nSOLVERETURN x"), 0.0);
        assert_eq!(
            val("x = 1\nSOLVERETURN x\nIF x == 1 THEN SOLVERETURN 7"),
            7.0
        );
        assert_eq!(val("y = 1\nIF y THEN y = 4\nSOLVERETURN y"), 4.0);
        assert!(run("IF 1\nx = 1").is_ok()); // 閉じていない IF は走らせても困らない
        assert!(run("ENDIF").is_err());
    }

    #[test]
    fn for_loops() {
        assert_eq!(
            val("s = 0\nFOR i, 1, 10, 1\ns = s + i\nNEXT\nSOLVERETURN s"),
            55.0
        );
        assert_eq!(
            val("s = 0\nFOR i, 10, 1, -3\ns = s + i\nNEXT i\nSOLVERETURN s"),
            22.0
        ); // 10 7 4 1
        assert_eq!(
            val("s = 0\nFOR i, 5, 5, -1\ns = s + 1\nNEXT\nSOLVERETURN s"),
            1.0
        );
        // 終わりは始めに 1 回だけ評価する
        assert_eq!(
            val("n = 3\ns = 0\nFOR i, 1, n, 1\nn = 10\ns = s + 1\nNEXT\nSOLVERETURN s"),
            3.0
        );
        assert_eq!(
            val("s = 0\nFOR i, 1, 3, 1\nFOR j, 1, 2, 1\ns = s + 1\nNEXT\nNEXT\nSOLVERETURN s"),
            6.0
        );
        assert_eq!(val("FOR i, 1, 3, 1\nNEXT\nSOLVERETURN i"), 4.0);
        assert_eq!(val("FOR i, 3, 1, -1\nNEXT\nSOLVERETURN i"), 0.0);
        // 増分の向きが逆・0 はエラー（OpticStudio は値が入らない）
        assert!(run("FOR i, 1, 3, -1\nNEXT\nSOLVERETURN 1").is_err());
        assert!(run("FOR i, 3, 1, 1\nNEXT\nSOLVERETURN 1").is_err());
        assert!(run("FOR i, 1, 3, 0\nNEXT\nSOLVERETURN 1").is_err());
    }

    #[test]
    fn goto_gosub_end() {
        assert_eq!(
            val("x = 0\nLABEL 1\nx = x + 1\nIF x < 5 THEN GOTO 1\nSOLVERETURN x"),
            5.0
        );
        assert!(run("IF 1 THEN IF 1 THEN x = 1").is_err());
        let src = "x = 0\nLABEL top\nx = x + 1\nIF (x < 5)\nGOTO top\nENDIF\nGOSUB twice\nSOLVERETURN x\nEND\nSUB twice\nx = x * 2\nRETURN";
        assert_eq!(val(src), 10.0);
        // END なしで SUB に流れ込むと終わる
        assert_eq!(
            val("x = 1\nSOLVERETURN x\nSUB a\nx = 5\nSOLVERETURN x\nRETURN"),
            1.0
        );
        assert!(run("GOTO nowhere").is_err());
        assert_eq!(run("x = 1").unwrap(), None);
        assert_eq!(
            Program::parse("x = 1")
                .unwrap()
                .solve(&mut NoHost)
                .unwrap()
                .value,
            None
        );
    }

    #[test]
    fn last_solvereturn_wins_and_print_ignored() {
        assert_eq!(val("SOLVERETURN 1\nPRINT \"x\", 3\nSOLVERETURN 2"), 2.0);
    }

    #[test]
    fn host_functions_and_commands() {
        struct H(f64);
        impl Host for H {
            fn function(&mut self, name: &str, a: &[Value]) -> Option<Result<Value>> {
                (name == "CURV").then(|| Ok(Value::Num(self.0 * a[0].num(0).unwrap())))
            }
            fn command(&mut self, name: &str, a: &[Value]) -> Option<Result<Flow>> {
                match name {
                    "SETX" => {
                        self.0 = a[0].num(0).unwrap();
                        Some(Ok(Flow::Go))
                    }
                    "STOPNOW" => Some(Ok(Flow::Abort("止める".into()))),
                    _ => None,
                }
            }
        }
        let p = Program::parse("SETX 3\nSOLVERETURN curv(2)").unwrap();
        assert_eq!(p.solve(&mut H(1.0)).unwrap().value, Some(6.0));
        // Abort: それまでの SOLVERETURN があっても値を変えない
        let p = Program::parse("SOLVERETURN 1\nSTOPNOW\nSOLVERETURN 2").unwrap();
        assert_eq!(p.solve(&mut H(1.0)).unwrap().value, None);
        assert!(matches!(
            Program::parse("RAYTRACE 0, 1, 0, 1")
                .unwrap()
                .solve(&mut H(1.0)),
            Err(Error::NotImplemented(_))
        ));
    }

    /// ソルブとしての結果（OpticStudio 26.1.1 と同じ扱い）
    #[test]
    fn solve_outcome_like_opticstudio() {
        let out = |src: &str| {
            Program::parse(src)
                .unwrap()
                .solve(&mut NoHost)
                .unwrap()
                .value
        };
        // 途中のエラー: それまでの値、なければ 0
        assert_eq!(out("SOLVERETURN 7\nx = undefinedvar"), Some(7.0));
        assert_eq!(out("x = undefinedvar\nSOLVERETURN 7"), Some(0.0));
        assert_eq!(out("SOLVERETURN 7\nx = 3 <> 2"), Some(7.0));
        assert_eq!(out("SOLVERETURN 7\nx = 6 ! note"), Some(7.0));
        assert_eq!(out("SOLVERETURN 3\nEND\nSOLVERETURN 4"), Some(3.0));
        // 値を変えない: SOLVERETURN なし、FOR の増分の向きが逆、値が NaN
        assert_eq!(out("x = 1"), None);
        assert_eq!(
            out("s = 0\nFOR i, 1, 3, -1\ns = s + 1\nNEXT\nSOLVERETURN 100 + s"),
            None
        );
        assert_eq!(out("SOLVERETURN ASIN(2) + 7"), None);
        // GOTO で FOR を抜ける
        assert_eq!(
            out("FOR i, 1, 3, 1\nIF i == 2 THEN GOTO 9\nNEXT\nLABEL 9\nSOLVERETURN 100 + i"),
            Some(102.0)
        );
    }

    #[test]
    fn infinite_loop_stops() {
        assert!(run("LABEL 1\nGOTO 1").is_err());
    }
}
