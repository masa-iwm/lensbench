//! 組合せの系の設定のうち、開いたファイル（.zmx・プロジェクト）から来ることが多いもの
//! （視野の正規化・大域座標の基準の面・温度と気圧・像面の形と傾き）。正規化と温度・気圧は設定の欄で変えられ、基準の面は持つだけ。ケラレ係数は視野の点（[`crate::sysset::FieldPt`]）が持つ

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use lensbench_core::system::{GlassTemplate, VigInterp};
use lensbench_project::Environment;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct SysExtra {
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
    /// 光線エイミングの瞳のずれ・圧縮（プロジェクトの aim_pupil をそのまま）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aim_pupil: Option<Value>,
    /// 系の入射偏光（プロジェクトの input_pol をそのまま）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_pol: Option<Value>,
    /// 光束の組（プロジェクトの beams をそのまま）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beams: Option<Value>,
    /// 面の仕上げ（プロジェクトの finishes をそのまま。非順次の追跡の散乱・反射率）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finishes: Option<Value>,
    /// 定義した視野以外の点のケラレ係数の求め方（プロジェクトの vig_interp。既定の連続は書かない）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vig_interp: Option<VigInterp>,
    /// 硝材置換のテンプレート（プロジェクトの glass_template、.zmx の GSTD。既定は書かない）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glass_template: Option<GlassTemplate>,
}

impl SysExtra {
    pub(crate) fn is_empty(&self) -> bool {
        *self == SysExtra::default()
    }

    /// プロジェクトの値から（fields は norm だけを見る）
    pub(crate) fn read(v: &Value) -> Result<SysExtra, String> {
        let environment = match v.get("environment") {
            Some(x) => Some(serde_json::from_value(x.clone()).map_err(|e| e.to_string())?),
            None => None,
        }
        .filter(|e| *e != Environment::default());
        let vig_interp = match v.get("vig_interp").filter(|g| !g.is_null()) {
            Some(x) => Some(serde_json::from_value(x.clone()).map_err(|e| e.to_string())?),
            None => None,
        }
        .filter(|x| *x != VigInterp::Continuous);
        let glass_template = match v.get("glass_template").filter(|g| !g.is_null()) {
            Some(x) => Some(
                serde_json::from_value::<GlassTemplate>(x.clone()).map_err(|e| e.to_string())?,
            ),
            None => None,
        }
        .filter(|g| !g.is_default());
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
            aim_pupil: v
                .get("aim_pupil")
                .filter(|g| !g.is_null() && **g != json!({}))
                .cloned(),
            input_pol: v
                .get("input_pol")
                .filter(|g| !g.is_null() && **g != json!({}))
                .cloned(),
            beams: v
                .get("beams")
                .filter(|g| !g.is_null() && **g != json!([]))
                .cloned(),
            finishes: v
                .get("finishes")
                .filter(|g| !g.is_null() && **g != json!([]))
                .cloned(),
            vig_interp,
            glass_template,
        })
    }

    /// プロジェクトの系の値に足す（fields は先に書いておく）
    pub(crate) fn put(&self, v: &mut Value) {
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
        if let Some(g) = &self.aim_pupil {
            v["aim_pupil"] = g.clone();
        }
        if let Some(g) = &self.input_pol {
            v["input_pol"] = g.clone();
        }
        if let Some(g) = &self.beams {
            v["beams"] = g.clone();
        }
        if let Some(g) = &self.finishes {
            v["finishes"] = g.clone();
        }
        if let Some(g) = self.vig_interp {
            v["vig_interp"] = json!(g);
        }
        if let Some(g) = self.glass_template.filter(|g| !g.is_default()) {
            v["glass_template"] = json!(g);
        }
    }
}
