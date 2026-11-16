//! 組合せの系の設定のうち、開いたファイル（.zmx・プロジェクト）から来ることが多いもの
//! （視野の正規化・大域座標の基準の面・温度と気圧）。正規化と温度・気圧は設定の欄で変えられ、基準の面は持つだけ。ケラレ係数は視野の点（[`crate::sysset::FieldPt`]）が持つ

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use lensbench_project::Environment;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SysExtra {
    /// 視野の正規化が矩形（プロジェクトの fields.norm = rect）
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub field_rect: bool,
    /// 大域座標の基準の面（プロジェクトの glrs をそのまま）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glrs: Option<Value>,
    /// 温度・気圧（プロジェクトの environment）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
}

impl SysExtra {
    pub fn is_empty(&self) -> bool {
        *self == SysExtra::default()
    }

    /// プロジェクトの値から（fields は norm だけを見る）
    pub fn read(v: &Value) -> Result<SysExtra, String> {
        let environment = match v.get("environment") {
            Some(x) => Some(serde_json::from_value(x.clone()).map_err(|e| e.to_string())?),
            None => None,
        }
        .filter(|e| *e != Environment::default());
        Ok(SysExtra {
            field_rect: v.pointer("/fields/norm").and_then(Value::as_str) == Some("rect"),
            glrs: v.get("glrs").filter(|g| !g.is_null()).cloned(),
            environment,
        })
    }

    /// プロジェクトの系の値に足す（fields は先に書いておく）
    pub fn put(&self, v: &mut Value) {
        if self.field_rect {
            v["fields"]["norm"] = json!("rect");
        }
        if let Some(g) = &self.glrs {
            v["glrs"] = g.clone();
        }
        if let Some(e) = &self.environment {
            v["environment"] = json!(e);
        }
    }
}
