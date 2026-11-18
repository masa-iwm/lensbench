//! .zmx の評価関数の行（面の後、公差 TOL・多重構成 MNUM の前に、演算子の名前を鍵として字下げせずに書かれる行）。
//!
//! 行の欄は `名前 P1 P2 P3 P4 P5 P6 目標 重み P7 P8`（P1〜P8 は Merit Function Editor の Param1〜Param8 の列。
//! OpticStudio 26.1 で実測: 整数の列は整数、実数の列は 17 桁で書き、P7・P8 は目標・重みの後ろ）。BLNK の行は残りの文字が
//! コメント（ほかの行のコメントは保存されない）。Optimization Wizard の行の前には引数のない `DMFS` の行が入る（Wizard の
//! 設定は .zmx に残らない）。CONF n の後の行は構成 n で計算する。
//! 系を作るとき（[`super::build`]）は使わない。項への写し方は lensbench-design の `merit_zmx`。

use super::*;

/// 評価関数の 1 行
#[derive(Debug, Clone, PartialEq)]
pub struct MeritRow {
    /// 評価関数の行の番号（1 始まり、OpticStudio の Merit Function Editor の行の番号。OPGT・SUMM などが参照する番号）
    pub row: usize,
    /// ファイルの行番号（1 始まり。注意の文に使う）
    pub line: usize,
    /// 演算子の名前（4 文字、大文字）
    pub op: String,
    /// 名前の後の欄（数のまま、文字列）
    pub fields: Vec<String>,
    /// BLNK の行のコメント（名前の後の文字。前後の空白は除く）
    pub comment: String,
}

impl MeritRow {
    /// 1 行を読む。読めない行（名前が 4 文字の大文字・数字でない）も op にその語を入れて残す（[`Self::readable`] が偽。
    /// 項に写すときに注意にする）
    pub(super) fn parse(row: usize, line: usize, text: &str) -> MeritRow {
        let text = text.trim_end();
        let op = text.split_whitespace().next().unwrap_or("").to_string();
        let rest = text[op.len()..].trim();
        if op == "BLNK" {
            return MeritRow {
                row,
                line,
                op,
                fields: vec![],
                comment: rest.to_string(),
            };
        }
        MeritRow {
            row,
            line,
            op,
            fields: rest.split_whitespace().map(str::to_string).collect(),
            comment: String::new(),
        }
    }

    /// 名前が演算子の形（4 文字の大文字・数字）
    pub fn readable(&self) -> bool {
        self.op.len() == 4
            && self
                .op
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    }

    /// i 番目（0 始まり）の欄の数（無ければ 0）
    pub fn num(&self, i: usize) -> Result<f64> {
        match self.fields.get(i) {
            None => Ok(0.0),
            Some(v) => fv(v).map_err(|e| e.at(format!("zmx line {} ({})", self.line, self.op))),
        }
    }

    /// i 番目の欄の整数（Int1・Int2 など。小数は誤り）
    pub fn int(&self, i: usize) -> Result<i64> {
        let v = self.num(i)?;
        if v.fract() != 0.0 || !v.is_finite() || v.abs() > 1e15 {
            invalid!(
                "zmx line {} ({}): field {} must be an integer ({v})",
                self.line,
                self.op,
                i + 1
            );
        }
        Ok(v as i64)
    }
}

impl MeritRow {
    /// 書き出す行（`名前 P1 P2 P3 P4 P5 P6 目標 重み P7 P8`）。row・line は 0（書くときに決まる）
    pub fn new(op: &str, p: [f64; 8], target: f64, weight: f64) -> MeritRow {
        let v = [
            p[0], p[1], p[2], p[3], p[4], p[5], target, weight, p[6], p[7],
        ];
        MeritRow {
            row: 0,
            line: 0,
            op: op.to_string(),
            fields: v.iter().map(|&x| num_text(x)).collect(),
            comment: String::new(),
        }
    }

    /// コメントの行（BLNK）。改行は空白にする
    pub fn blank(comment: &str) -> MeritRow {
        MeritRow {
            row: 0,
            line: 0,
            op: "BLNK".into(),
            fields: vec![],
            comment: comment.replace(['\r', '\n'], " ").trim().to_string(),
        }
    }

    /// .zmx の 1 行（改行なし）
    pub fn text(&self) -> String {
        if self.op == "BLNK" {
            return format!("BLNK {}", self.comment);
        }
        let mut t = self.op.clone();
        for f in &self.fields {
            t.push(' ');
            t.push_str(f);
        }
        t
    }
}

/// 数を読み戻しで同じ値になる最短の表記で（整数は小数点なし）
fn num_text(v: f64) -> String {
    let s = format!("{v:?}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// .zmx の評価関数（行とレンズ単位）
#[derive(Debug, Clone, PartialEq)]
pub struct MeritFunction {
    pub rows: Vec<MeritRow>,
    /// レンズ単位の長さ [mm]（UNIT。長さの演算子の目標を mm に直すのに使う）
    pub lens_unit_mm: f64,
}

/// .zmx の評価関数（ファイルを読む）
pub fn merit_function(path: &Path) -> Result<MeritFunction> {
    merit_function_text(&read_zmx(path)?)
}

/// [`merit_function`] で、.zmx の中身をもう読んであるとき
pub fn merit_function_text(txt: &str) -> Result<MeritFunction> {
    let raw = parse_text(txt)?;
    let unit = sys_str(&raw, "UNIT", 0).unwrap_or("MM");
    let lens_unit_mm = match unit.to_ascii_uppercase().as_str() {
        "MM" => 1.0,
        "CM" => 10.0,
        "IN" => 25.4,
        "METER" | "M" => 1000.0,
        u => notimpl!("lens unit {u} (mm, cm, in, m)"),
    };
    Ok(MeritFunction {
        rows: raw.merit,
        lens_unit_mm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 面の後に評価関数（OpticStudio 26.1 の書き方）
    const ZMX: &[&str] = &[
        "VERS 260327 42",
        "MODE SEQ",
        "UNIT MM X W X CM MR CPMM",
        "ENPD 10",
        "FTYP 0 0 1 1 0 0 0",
        "XFLN 0",
        "YFLN 0",
        "FWGN 1",
        "WAVM 1 0.55 1",
        "PWAV 1",
        "SURF 0",
        "  TYPE STANDARD",
        "  CURV 0.0",
        "  DISZ INFINITY",
        "SURF 1",
        "  STOP",
        "  TYPE STANDARD",
        "  CURV 0.02",
        "  DISZ 5",
        "  GLAS N-BK7 0 0 1.5 40 0 0 0 0 0 0",
        "SURF 2",
        "  TYPE STANDARD",
        "  CURV 0.0",
        "  DISZ 45",
        "SURF 3",
        "  TYPE STANDARD",
        "  CURV 0.0",
        "  DISZ 0",
        "DMFS 0 0 0 0 0 0 0 0 0 0",
        "BLNK hello  merit ",
        "EFFL 0 1 0 0 0 0 100 1 0 0",
        "ZERN 4 1 3 1 2 0.29999999999999999 0.20000000000000001 1.5 1 0",
        "CONF 2 0 0 0 0 0 0 0 0 0",
        "TOL TOFF   0   0 0.0000000000000000E+00 0.0000000000000000E+00   0 0 0 0 0",
        "MNUM 1 1",
    ];

    fn zmx() -> String {
        ZMX.join("\n")
    }

    #[test]
    fn rows_after_the_surfaces() {
        let raw = parse_text(&zmx()).unwrap();
        let ops: Vec<&str> = raw.merit.iter().map(|r| r.op.as_str()).collect();
        assert_eq!(ops, ["DMFS", "BLNK", "EFFL", "ZERN", "CONF"]);
        // 系の行（読まずに飛ばした注意）には入らない
        assert!(!raw.sysl.contains_key("EFFL") && raw.sysl.contains_key("TOL"));
        let r = &raw.merit;
        assert_eq!((r[0].row, r[1].row, r[4].row), (1, 2, 5));
        assert_eq!(r[1].comment, "hello  merit");
        assert_eq!(
            r[2].line,
            zmx().lines().position(|l| l.starts_with("EFFL")).unwrap() + 1
        );
        // ZERN: Term 4, Wave 1, Samp 3, Field 1, Type 2, Epsilon 0.3, 目標 0.2, 重み 1.5, Vertex 1
        assert_eq!((r[3].int(0).unwrap(), r[3].int(4).unwrap()), (4, 2));
        assert_eq!(
            (
                r[3].num(6).unwrap(),
                r[3].num(7).unwrap(),
                r[3].int(8).unwrap()
            ),
            (0.2, 1.5, 1)
        );
        assert!(r[3].int(5).is_err());
        let mf = merit_function_text(&zmx().replace("UNIT MM", "UNIT IN")).unwrap();
        assert_eq!((mf.rows.len(), mf.lens_unit_mm), (5, 25.4));
        // 系は評価関数があっても同じに読める
        let s = build(
            &raw,
            &LoadOptions {
                cfg: 1,
                ..Default::default()
            },
            &[],
            None,
        )
        .unwrap();
        assert!(
            s.import_notes.iter().all(|n| !n.contains("EFFL")),
            "{:?}",
            s.import_notes
        );
        // 読めない行も番号を保って残す（項に写すときに注意にする）
        let raw = parse_text(&zmx().replace("CONF 2", "conf 2")).unwrap();
        let r = raw.merit.last().unwrap();
        assert_eq!((r.row, r.op.as_str(), r.readable()), (5, "conf", false));
    }
}
