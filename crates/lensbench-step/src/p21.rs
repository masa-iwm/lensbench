//! ISO 10303-21（STEP の文字の形）の読み手。品の寸法の測り（[`crate::geom`]）と、書き出す STEP への写し
//! （lensbench-export の `step_embed`）が同じ表を使う（PRJ-06。以前は読み手が 2 つあった。三角形にするのは truck）。
//!
//! - 注釈 `/* … */` と文字 `'…'`（`''` は引用符 1 つ）の中は読まない（注釈の中の `#5=…` で実体を上書きしない、
//!   文字の中の `;`・`#`・括弧を数えない）
//! - DATA 部は HEADER の終わり（最初の `ENDSEC;`）の後から探す（HEADER の文字の中の `DATA;` を拾わない）
//! - 括弧の入れ子は再帰でなく明示のスタックで読み、[`MAX_DEPTH`] より深い所は値を読まずに飛ばす（深い入れ子で
//!   スタックがあふれない）
//! - 実体の名前と列挙（`.T.`）は大文字にして比べる
use std::collections::{HashMap, HashSet};

/// STEP の 1 つの値
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Ref(u64),
    Num(f64),
    List(Vec<Arg>),
    /// .T.・.F.
    Bool(bool),
    /// 文字・ほかの列挙・`$`・`*`・型つきの値（`NAME(…)`）・[`MAX_DEPTH`] より深い括弧
    Other,
}

impl Arg {
    pub fn r(&self) -> Option<u64> {
        match self {
            Arg::Ref(r) => Some(*r),
            _ => None,
        }
    }
    pub fn num(&self) -> Option<f64> {
        match self {
            Arg::Num(x) => Some(*x),
            _ => None,
        }
    }
    pub fn list(&self) -> Option<&[Arg]> {
        match self {
            Arg::List(v) => Some(v),
            _ => None,
        }
    }
}

/// 括弧の入れ子の深さの上限（これより深い所は読み飛ばす）
pub const MAX_DEPTH: usize = 64;

/// 実体の部分（名前と値の並び）。単純な実体は部分 1 つ、複合の実体 `(A(…) B(…))` は 2 つ以上
pub type Part = (String, Vec<Arg>);

/// 1 つの実体
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// `=` の後ろ・`;` の前の文字（前後の空白と改行を除き、ほかは元のまま。書き出しへの写しに使う）
    pub body: String,
    /// 読んだ部分
    pub parts: Vec<Part>,
}

/// 読んだ STEP（実体の番号 → 実体）と長さの単位
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepFile {
    ents: HashMap<u64, Entity>,
    unit_mm: f64,
}

impl StepFile {
    /// STEP の文字を読む。DATA 部が無ければ誤り
    pub fn parse(text: &str) -> Result<StepFile, String> {
        let text = strip_comments(text);
        let from = match (text.find("HEADER;"), text.find("ENDSEC;")) {
            (Some(h), Some(e)) if h < e => e + 7,
            _ => 0,
        };
        let Some(data) = text[from..].find("DATA;").map(|k| k + from) else {
            return Err("no DATA section in the STEP".into());
        };
        let unit_mm = unit_mm(&text);
        let body = &text[data + 5..];
        let b = body.as_bytes();
        let mut ents = HashMap::new();
        let mut i = 0;
        while i < b.len() {
            // #n = 本体 ;
            if b[i] != b'#' {
                i += 1;
                continue;
            }
            let st = i + 1;
            let mut j = st;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let Some(id) = body[st..j].parse::<u64>().ok() else {
                i = j.max(i + 1);
                continue;
            };
            skip_ws(b, &mut j);
            if j >= b.len() || b[j] != b'=' {
                i = j;
                continue;
            }
            let b0 = j + 1;
            let end = record_end(b, b0);
            let text = body[b0..end].trim().replace(['\r', '\n'], "");
            let parts = parse_parts(text.as_bytes());
            if !parts.is_empty() || !text.is_empty() {
                ents.insert(id, Entity { body: text, parts });
            }
            i = end + 1;
        }
        Ok(StepFile { ents, unit_mm })
    }

    /// 長さの単位の倍率（mm へ）
    pub fn unit_mm(&self) -> f64 {
        self.unit_mm
    }

    pub fn get(&self, id: u64) -> Option<&Entity> {
        self.ents.get(&id)
    }

    /// 実体の部分（無ければ None）
    pub fn parts(&self, id: u64) -> Option<&[Part]> {
        self.ents.get(&id).map(|e| e.parts.as_slice())
    }

    pub fn contains(&self, id: u64) -> bool {
        self.ents.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.ents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ents.is_empty()
    }

    /// 実体の番号（並びは決まらない）
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.ents.keys().copied()
    }

    /// (番号, 部分) の並び（並びは決まらない）
    pub fn iter(&self) -> impl Iterator<Item = (u64, &[Part])> + '_ {
        self.ents.iter().map(|(k, e)| (*k, e.parts.as_slice()))
    }

    /// roots から参照で辿れる実体だけの写し（書き出しへの写しに要る分だけを持つ。PRJ-21）
    pub fn subset(&self, roots: impl IntoIterator<Item = u64>) -> StepFile {
        let mut seen = HashSet::new();
        let mut stack: Vec<u64> = roots.into_iter().collect();
        let mut ents = HashMap::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(e) = self.ents.get(&id) else {
                continue;
            };
            stack.extend(refs(&e.body).into_iter().filter(|r| !seen.contains(r)));
            ents.insert(id, e.clone());
        }
        StepFile {
            ents,
            unit_mm: self.unit_mm,
        }
    }
}

/// 本体の参照（文字の外の #番号）
pub fn refs(body: &str) -> Vec<u64> {
    let b = body.as_bytes();
    let mut v = Vec::new();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' => in_str = !in_str,
            b'#' if !in_str => {
                let st = i + 1;
                let mut j = st;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if let Ok(n) = body[st..j].parse() {
                    v.push(n);
                }
                i = j;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    v
}

/// 単純な実体の型と、上の段の引数（文字のまま）。複合の実体（`(` で始まる）は None
pub fn split(body: &str) -> Option<(&str, Vec<&str>)> {
    let open = body.find('(')?;
    let ty = body[..open].trim();
    if ty.is_empty() || !body.ends_with(')') {
        return None;
    }
    let inner = &body[open + 1..body.len() - 1];
    let b = inner.as_bytes();
    let mut args = Vec::new();
    let (mut depth, mut in_str, mut st) = (0i64, false, 0);
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'\'' => in_str = !in_str,
            b'(' if !in_str => depth += 1,
            b')' if !in_str => depth -= 1,
            b',' if !in_str && depth == 0 => {
                args.push(inner[st..i].trim());
                st = i + 1;
            }
            _ => {}
        }
    }
    args.push(inner[st..].trim());
    Some((ty, args))
}

/// 長さの単位（mm へ）。`'INCH'`・SI の mm・cm・m（無ければ mm）
fn unit_mm(text: &str) -> f64 {
    let upper: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if upper.contains("'INCH'") {
        25.4
    } else if upper.contains("SI_UNIT(.MILLI.,.METRE.)") {
        1.0
    } else if upper.contains("SI_UNIT(.CENTI.,.METRE.)") {
        10.0
    } else if upper.contains("SI_UNIT($,.METRE.)") {
        1000.0
    } else {
        1.0
    }
}

fn skip_ws(b: &[u8], j: &mut usize) {
    while *j < b.len() && b[*j].is_ascii_whitespace() {
        *j += 1;
    }
}

/// 文字の外の最初の `;` の位置（無ければ末尾）
fn record_end(b: &[u8], from: usize) -> usize {
    let mut q = from;
    let mut in_str = false;
    while q < b.len() {
        match b[q] {
            b'\'' => in_str = !in_str,
            b';' if !in_str => return q,
            _ => {}
        }
        q += 1;
    }
    b.len()
}

/// 注釈 `/* … */` を空白 1 つに（文字 '…' の中は残す）
fn strip_comments(t: &str) -> std::borrow::Cow<'_, str> {
    if !t.contains("/*") {
        return std::borrow::Cow::Borrowed(t);
    }
    let b = t.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut quoted = false;
    while i < b.len() {
        if quoted {
            out.push(b[i]);
            if b[i] == b'\'' {
                if b.get(i + 1) == Some(&b'\'') {
                    out.push(b'\'');
                    i += 1;
                } else {
                    quoted = false;
                }
            }
            i += 1;
        } else if b[i] == b'\'' {
            quoted = true;
            out.push(b[i]);
            i += 1;
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i += 2;
            out.push(b' ');
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    std::borrow::Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

/// 名前（英数字と _）を読み、大文字にする
fn read_name(b: &[u8], j: &mut usize) -> String {
    let ns = *j;
    while *j < b.len() && (b[*j].is_ascii_alphanumeric() || b[*j] == b'_') {
        *j += 1;
    }
    String::from_utf8_lossy(&b[ns..*j]).to_ascii_uppercase()
}

/// 本体（`NAME(…)` か `(NAME(…) NAME(…) …)`）の部分
fn parse_parts(b: &[u8]) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut j = 0;
    skip_ws(b, &mut j);
    let compound = j < b.len() && b[j] == b'(';
    if compound {
        j += 1;
    }
    loop {
        skip_ws(b, &mut j);
        if j >= b.len() || b[j] == b')' {
            break;
        }
        let name = read_name(b, &mut j);
        skip_ws(b, &mut j);
        if j < b.len() && b[j] == b'(' {
            let args = parse_args(b, &mut j);
            if !name.is_empty() {
                parts.push((name, args));
            }
        } else if name.is_empty() {
            break;
        }
        if !compound {
            break;
        }
    }
    parts
}

/// s[*i] の '(' に対応する ')' の後へ進む（文字の中の括弧は数えない）
fn skip_group(s: &[u8], i: &mut usize) {
    let mut depth = 0usize;
    while *i < s.len() {
        match s[*i] {
            b'(' => depth += 1,
            b')' => {
                if depth <= 1 {
                    *i += 1;
                    return;
                }
                depth -= 1;
            }
            b'\'' => skip_string(s, i),
            _ => {}
        }
        *i += 1;
    }
}

/// s[*i] の `'` から、閉じる `'` へ進む（`''` は引用符 1 つ）
fn skip_string(s: &[u8], i: &mut usize) {
    *i += 1;
    while *i < s.len() {
        if s[*i] == b'\'' {
            if s.get(*i + 1) == Some(&b'\'') {
                *i += 2;
                continue;
            }
            break;
        }
        *i += 1;
    }
}

/// `(a,b,(c,d))` の中身を値の並びに。s[*i] は '('。入れ子は明示のスタックで読む
pub fn parse_args(s: &[u8], i: &mut usize) -> Vec<Arg> {
    // (値の並び, 型つきの値の中か)。型つきの値の中身は読んで捨てる
    let mut stack: Vec<(Vec<Arg>, bool)> = vec![(Vec::new(), false)];
    *i += 1;
    loop {
        while *i < s.len() && (s[*i].is_ascii_whitespace() || s[*i] == b',') {
            *i += 1;
        }
        if *i >= s.len() {
            // 閉じていない: 読めた所まで
            while stack.len() > 1 {
                let (v, typed) = stack.pop().expect("1 段以上");
                let top = &mut stack.last_mut().expect("1 段以上").0;
                top.push(if typed { Arg::Other } else { Arg::List(v) });
            }
            return stack.pop().map(|x| x.0).unwrap_or_default();
        }
        match s[*i] {
            b')' => {
                *i += 1;
                let (v, typed) = stack.pop().expect("1 段以上");
                match stack.last_mut() {
                    None => return v,
                    Some(top) => top.0.push(if typed { Arg::Other } else { Arg::List(v) }),
                }
            }
            b'(' => {
                if stack.len() > MAX_DEPTH {
                    skip_group(s, i);
                    stack.last_mut().expect("1 段以上").0.push(Arg::Other);
                } else {
                    *i += 1;
                    stack.push((Vec::new(), false));
                }
            }
            b'\'' => {
                skip_string(s, i);
                *i += 1;
                stack.last_mut().expect("1 段以上").0.push(Arg::Other);
            }
            b'#' => {
                *i += 1;
                let st = *i;
                while *i < s.len() && s[*i].is_ascii_digit() {
                    *i += 1;
                }
                let n = std::str::from_utf8(&s[st..*i])
                    .ok()
                    .and_then(|x| x.parse().ok());
                stack
                    .last_mut()
                    .expect("1 段以上")
                    .0
                    .push(n.map_or(Arg::Other, Arg::Ref));
            }
            c if c == b'-'
                || c == b'+'
                || c == b'.' && s.get(*i + 1).is_some_and(u8::is_ascii_digit)
                || c.is_ascii_digit() =>
            {
                let st = *i;
                *i += 1;
                while *i < s.len()
                    && (s[*i].is_ascii_digit() || matches!(s[*i], b'.' | b'e' | b'E' | b'-' | b'+'))
                {
                    *i += 1;
                }
                let x = std::str::from_utf8(&s[st..*i])
                    .ok()
                    .and_then(|x| x.parse::<f64>().ok());
                stack
                    .last_mut()
                    .expect("1 段以上")
                    .0
                    .push(x.map_or(Arg::Other, Arg::Num));
            }
            _ => {
                // 列挙（.T.）・$・* ・型つきの値（名前(...)）
                let st = *i;
                while *i < s.len() && !matches!(s[*i], b',' | b')' | b'(') {
                    *i += 1;
                }
                let tok = std::str::from_utf8(&s[st..*i])
                    .unwrap_or("")
                    .trim()
                    .to_ascii_uppercase();
                if *i < s.len() && s[*i] == b'(' {
                    if stack.len() > MAX_DEPTH {
                        skip_group(s, i);
                        stack.last_mut().expect("1 段以上").0.push(Arg::Other);
                    } else {
                        *i += 1;
                        stack.push((Vec::new(), true));
                    }
                } else {
                    stack
                        .last_mut()
                        .expect("1 段以上")
                        .0
                        .push(match tok.as_str() {
                            ".T." => Arg::Bool(true),
                            ".F." => Arg::Bool(false),
                            _ => Arg::Other,
                        });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_split_refs() {
        let t = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=CARTESIAN_POINT('a;b',(1.,2.,\n3.));\n#2 = circle('',#3,2.5);\n#3=(A()B(#1));\nENDSEC;";
        let f = StepFile::parse(t).unwrap();
        assert_eq!(f.get(1).unwrap().body, "CARTESIAN_POINT('a;b',(1.,2.,3.))");
        assert_eq!(
            f.parts(1).unwrap()[0],
            (
                "CARTESIAN_POINT".into(),
                vec![
                    Arg::Other,
                    Arg::List(vec![Arg::Num(1.0), Arg::Num(2.0), Arg::Num(3.0)])
                ]
            )
        );
        // 名前は大文字にする
        assert_eq!(f.parts(2).unwrap()[0].0, "CIRCLE");
        assert_eq!(
            split(&f.get(2).unwrap().body).unwrap(),
            ("circle", vec!["''", "#3", "2.5"])
        );
        assert!(split(&f.get(3).unwrap().body).is_none());
        assert_eq!(refs(&f.get(3).unwrap().body), vec![1]);
        let p3: Vec<&str> = f.parts(3).unwrap().iter().map(|p| p.0.as_str()).collect();
        assert_eq!(p3, ["A", "B"]);
        // 辿れる実体だけ
        let sub = f.subset([3]);
        assert!(sub.contains(1) && sub.contains(3) && !sub.contains(2));
    }

    /// 深い括弧の入れ子でスタックがあふれず、注釈の中の実体（#5 の上書き）を読まない。文字の中の注釈・;・# は文字
    #[test]
    fn deep_nesting_comments_and_strings() {
        let deep = format!(
            "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=X({}1{});\n#2=CARTESIAN_POINT('',(1.,2.,3.));\nENDSEC;\n",
            "(".repeat(300_000),
            ")".repeat(300_000)
        );
        let f = StepFile::parse(&deep).unwrap();
        assert!(f.contains(1) && f.contains(2));
        let c = "ISO-10303-21;\nHEADER;\nFILE_NAME('DATA;','',(''),(''),'','','');\nENDSEC;\nDATA;\n\
#5=CIRCLE('',#4,12.7);\n/* #5=CIRCLE('',#4,99.); */\n#6=X('/* not a comment */');\n#7=Y('it''s;#9',#5);\nENDSEC;\n";
        let f = StepFile::parse(c).unwrap();
        assert!(matches!(f.parts(5).unwrap()[0].1[2], Arg::Num(r) if r == 12.7));
        assert_eq!(f.get(6).unwrap().body, "X('/* not a comment */')");
        assert_eq!(refs(&f.get(7).unwrap().body), vec![5]);
        assert_eq!(f.parts(7).unwrap()[0].1, vec![Arg::Other, Arg::Ref(5)]);
        assert!(StepFile::parse("not a step").is_err());
    }

    #[test]
    fn typed_values_enums_and_units() {
        let t = "DATA;\n#1=A(.t.,.F.,$,*,LENGTH_MEASURE(2.5),#2,(#3,(4.)));\n#2=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\nENDSEC;";
        let f = StepFile::parse(t).unwrap();
        assert_eq!(
            f.parts(1).unwrap()[0].1,
            vec![
                Arg::Bool(true),
                Arg::Bool(false),
                Arg::Other,
                Arg::Other,
                Arg::Other,
                Arg::Ref(2),
                Arg::List(vec![Arg::Ref(3), Arg::List(vec![Arg::Num(4.0)])]),
            ]
        );
        assert_eq!(f.unit_mm(), 1.0);
        let inch = "DATA;\n#1=CONVERSION_BASED_UNIT( 'INCH' ,#2);\nENDSEC;";
        assert_eq!(StepFile::parse(inch).unwrap().unit_mm(), 25.4);
    }
}
