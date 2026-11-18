//! 計算の行（SUMM・OSUM・DIFF・PROD・DIVI・ABSO・SQRT・RECI・CONS）を calc の項にする。参照する行は項の引数に入れ子にする
//! （OPGT・OPLT と同じく、参照する行の重みによらない）。OpticStudio 26.1 の実測に合わせる: 行の番号 0 は行 1、範囲の外・
//! 自分より後ろの行・BLNK の値は 0、CONS の値は目標、OSUM(a, b) は a > b なら 0。片側の条件の行（CTGT・MNCA・OPGT など）は
//! OpticStudio が値を目標に寄せるので参照できない（注意にして飛ばす）。0 での割り算は OpticStudio は 1e13・1e20 にするが、
//! lensbench は項の誤りにする。

use super::{kind_of, zern_standard_fit, INT1, INT2, TARGET};
use lensbench_formats::zmx::merit::MeritRow;
use serde_json::{json, Map, Value};

/// 計算の行の演算子
pub(super) const OPS: &[&str] = &[
    "SUMM", "OSUM", "DIFF", "PROD", "DIVI", "ABSO", "SQRT", "RECI", "CONS", "MAXX", "MINN", "QSUM",
    "SINE", "COSI", "TANG", "ASIN", "ACOS", "ATAN", "OPVA", "ABGT", "ABLT",
];

/// 1 行の項に入れ子にする行の数の上限（前の計算の行を 2 つずつ参照すると倍々に増える）
const MAX_NODES: usize = 1000;

/// 写すときの共通の値
pub(super) struct Rows<'a> {
    pub(super) rows: &'a [MeritRow],
    pub(super) row_config: &'a [i64],
    pub(super) unit: f64,
    pub(super) primary: usize,
    pub(super) image: usize,
    pub(super) sys: &'a super::ZmxSys,
    /// 今の行に入れ子にした行の数
    pub(super) nodes: std::cell::Cell<usize>,
}

impl Rows<'_> {
    /// 行 k（0 始まり）の項の種類のキー。計算の行は calc、Standard の ZERN は fit も入れる
    pub(super) fn kind(&self, k: usize) -> Result<Map<String, Value>, String> {
        self.nodes.set(0);
        self.kind_at(k, 0)
    }

    fn kind_at(&self, k: usize, depth: usize) -> Result<Map<String, Value>, String> {
        let r = &self.rows[k];
        if !OPS.contains(&r.op.as_str()) {
            let mut m = kind_of(r, self.unit, self.primary, self.image, self.sys)?;
            if r.op == "ZERN" && m.get("kind").and_then(Value::as_str) == Some("standard") {
                m.insert("fit".into(), json!(zern_standard_fit(self.rows, k)));
            }
            return Ok(m);
        }
        if self.unit != 1.0 {
            return Err("calc rows are read only in mm lens units".into());
        }
        self.nodes.set(self.nodes.get() + 1);
        if depth > 64 || self.nodes.get() > MAX_NODES {
            return Err("calc rows nested too deeply".into());
        }
        let int = |i: usize| r.int(i).map_err(|e| e.to_string());
        let target = r.num(TARGET).map_err(|e| e.to_string())?;
        let arg = |n: i64| self.arg(k, n, depth);
        let (op, args) = match r.op.as_str() {
            "SUMM" => ("sum", vec![arg(int(INT1)?)?, arg(int(INT2)?)?]),
            "DIFF" => ("diff", vec![arg(int(INT1)?)?, arg(int(INT2)?)?]),
            "PROD" => ("prod", vec![arg(int(INT1)?)?, arg(int(INT2)?)?]),
            "DIVI" => ("div", vec![arg(int(INT1)?)?, arg(int(INT2)?)?]),
            "ABSO" => ("abs", vec![arg(int(INT1)?)?]),
            "SQRT" => ("sqrt", vec![arg(int(INT1)?)?]),
            "RECI" => ("reci", vec![arg(int(INT1)?)?]),
            "OSUM" | "QSUM" => {
                let (a, b) = (int(INT1)?.max(1), int(INT2)?);
                let mut v = vec![json!(0.0)];
                for n in a..=b.min(self.rows.len() as i64) {
                    v.push(arg(n)?);
                }
                (if r.op == "OSUM" { "sum" } else { "qsum" }, v)
            }
            // 行 a から b の最大・最小（a > b なら行 a だけ）
            "MAXX" | "MINN" => {
                let (a, b) = (int(INT1)?.max(1), int(INT2)?);
                let mut v = vec![arg(a)?];
                for n in a + 1..=b.min(self.rows.len() as i64) {
                    v.push(arg(n)?);
                }
                (if r.op == "MAXX" { "max" } else { "min" }, v)
            }
            // Flag 1 は度（SINE・COSI・TANG の引数、ASIN・ACOS・ATAN の値）
            "SINE" | "COSI" | "TANG" | "ASIN" | "ACOS" | "ATAN" => {
                let deg = int(INT2)? == 1;
                let x = arg(int(INT1)?)?;
                let name = match r.op.as_str() {
                    "SINE" => "sin",
                    "COSI" => "cos",
                    "TANG" => "tan",
                    "ASIN" => "asin",
                    "ACOS" => "acos",
                    _ => "atan",
                };
                let calc = |op: &str, args: Vec<Value>| json!({"type": "calc", "op": op, "args": args, "target": 0.0});
                let k = std::f64::consts::PI / 180.0;
                match (deg, name.starts_with('a')) {
                    (false, _) => (name, vec![x]),
                    (true, false) => (name, vec![calc("prod", vec![x, json!(k)])]),
                    (true, true) => ("prod", vec![calc(name, vec![x]), json!(1.0 / k)]),
                }
            }
            "OPVA" | "ABGT" | "ABLT" => {
                let x = arg(int(INT1)?)?;
                (if r.op == "OPVA" { "sum" } else { "abs" }, vec![x])
            }
            _ => ("sum", vec![json!(target)]),
        };
        let mut m = Map::new();
        m.insert("type".into(), json!("calc"));
        m.insert("op".into(), json!(op));
        m.insert("args".into(), Value::Array(args));
        // ABGT・ABLT: |行の値| ≥・≤ 目標
        let key = match r.op.as_str() {
            "ABGT" => "min",
            "ABLT" => "max",
            _ => "target",
        };
        m.insert(key.into(), json!(target));
        Ok(m)
    }

    /// 行 k の計算の行が参照する行 n（1 始まり）の値の引数
    fn arg(&self, k: usize, n: i64, depth: usize) -> Result<Value, String> {
        let n = n.max(1) as usize;
        if n > self.rows.len() || n > k {
            return Ok(json!(0.0));
        }
        let a = &self.rows[n - 1];
        match a.op.as_str() {
            "BLNK" | "CONF" | "DMFS" => return Ok(json!(0.0)),
            "CONS" => return a.num(TARGET).map(|t| json!(t)).map_err(|e| e.to_string()),
            _ => {}
        }
        if self.row_config[n - 1] != self.row_config[k] {
            return Err(format!(
                "the referenced row {} is in another configuration",
                a.row
            ));
        }
        let m = self
            .kind_at(n - 1, depth + 1)
            .map_err(|e| format!("the referenced row {} ({}) is {e}", a.row, a.op))?;
        if m.contains_key("min") || m.contains_key("max") || a.op == "OPGT" || a.op == "OPLT" {
            return Err(format!(
                "the referenced row {} ({}) is a bound, whose value OpticStudio moves to its target",
                a.row, a.op
            ));
        }
        if !m.contains_key("target") {
            return Err(format!(
                "the referenced row {} ({}) has no single value",
                a.row, a.op
            ));
        }
        Ok(Value::Object(m))
    }
}
