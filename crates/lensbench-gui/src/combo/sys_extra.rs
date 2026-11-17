//! 組合せの系の設定のうち、開いたファイル（.zmx・プロジェクト）から来ることが多いもの
//! （視野の正規化・大域座標の基準の面・温度と気圧・像面の形と傾き）。正規化と温度・気圧は設定の欄で変えられ、基準の面は持つだけ。ケラレ係数は視野の点（[`crate::sysset::FieldPt`]）が持つ

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
    /// 像面の形・傾き（プロジェクトの image をそのまま）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Value>,
    /// 物体面の形・開口（プロジェクトの object をそのまま）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<Value>,
    /// 瞳の振幅の分布・OPD の基準（プロジェクトの apodization・ropd をそのまま）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apodization: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ropd: Option<Value>,
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
            image: v.get("image").filter(|g| !g.is_null()).cloned(),
            object: v.get("object").filter(|g| !g.is_null()).cloned(),
            apodization: v
                .get("apodization")
                .filter(|g| !g.is_null() && **g != json!("uniform"))
                .cloned(),
            ropd: v
                .get("ropd")
                .filter(|g| !g.is_null() && **g != json!(2))
                .cloned(),
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
        if let Some(g) = &self.image {
            v["image"] = g.clone();
        }
        if let Some(g) = &self.object {
            v["object"] = g.clone();
        }
        if let Some(g) = &self.apodization {
            v["apodization"] = g.clone();
        }
        if let Some(g) = &self.ropd {
            v["ropd"] = g.clone();
        }
    }
}
