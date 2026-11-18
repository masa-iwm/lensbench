//! 要求の答え（[`Answer`]）: 結果の型が決まっている method（面の表・最適化・差し替え探索・公差・ズーム軌跡）は
//! 型のまま返し、同じプロセスの呼び手（GUI）は JSON を通さずに受け取る（[`super::Session::call_answer`]）。
//! 要求を運ぶ道（serve・Python）と [`super::Session::call`] は JSON に書く（[`Answer::into_json`]、形は前と同じ）

use super::views::{ConfigTable, OptimizeView, SurfacesView, ZoomView};
use super::{RpcErr, Session};
use lensbench_analysis::Typed;
use lensbench_design::swap::SwapReply;
use lensbench_design::tolerance::ToleranceReport;
use serde_json::{json, Value};

/// 1 つの要求の答え（型の答えは箱に入れ、どの枝も小さく保つ）
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// 形が method や解析ごとに違う結果（解析・レイアウト・一覧など）
    Json(Value),
    /// `surface`（面を指定しないとき）
    Surfaces(Box<SurfacesView>),
    /// `optimize`
    Optimize(Box<OptimizeView>),
    /// `swap`
    Swap(Box<SwapReply>),
    /// `tolerance`
    Tolerance(Box<ToleranceReport>),
    /// `zoom`
    Zoom(Box<ZoomView>),
    /// `config_table`
    ConfigTable(Box<ConfigTable>),
    /// `analyze` の解析の結果（書き出さないとき）。同梱の解析は結果の型のまま、型を消した拡張は JSON
    Analysis(Box<Typed>),
}

impl Answer {
    /// 要求を運ぶ道の JSON（serve の result と同じ）。誤りは解析の結果を JSON に書けないとき
    pub fn into_json(self) -> lensbench_core::Result<Value> {
        Ok(match self {
            Answer::Json(v) => v,
            Answer::Analysis(t) => t.to_json()?,
            Answer::Surfaces(v) => json!(v),
            Answer::Optimize(v) => json!(v),
            Answer::Swap(v) => json!(v),
            Answer::Tolerance(v) => json!(v),
            Answer::ConfigTable(v) => json!(v),
            // ズーム軌跡の notes は空でも書く（serve の結果の形）
            Answer::Zoom(z) => {
                let notes = z.notes.clone();
                let mut v = json!(*z);
                v["notes"] = json!(notes);
                v
            }
        })
    }

    /// 型の無い答え（JSON）なら、その値
    pub fn as_json(&self) -> Option<&Value> {
        match self {
            Answer::Json(v) => Some(v),
            _ => None,
        }
    }
}

impl From<Value> for Answer {
    fn from(v: Value) -> Self {
        Answer::Json(v)
    }
}

/// 全部の面の値と、玉・群の面の範囲・機構部品（`surface` で面を指定しないとき）
pub(super) fn surfaces_view(sess: &Session) -> Result<SurfacesView, RpcErr> {
    let (s, _) = sess.current()?;
    let one = |j: usize| lensbench_project::edit_ops::surf_fields_json(&s, j).map_err(RpcErr::app);
    // 面ごとのソルブ・ピックアップも（GUI の面の表の印。形は surface_data と同じ）
    let all = (0..s.surfaces.len())
        .map(|j| {
            let mut f = one(j)?;
            let sp = lensbench_project::edit_ops::surface_solves(&s, j);
            if let (Some(o), Value::Object(x)) = (f.as_object_mut(), sp) {
                o.extend(x);
            }
            super::views::read::<super::views::SurfaceView>(&f).map_err(RpcErr::app)
        })
        .collect::<Result<Vec<_>, _>>()?;
    // 玉・群（ブロック）の面の範囲も（GUI の面の表の階層）
    // 玉・群の偏心・傾き（motion: dx dy tx ty tz）・移動 shift・媒質の dn・disp（面ごとに違えば null）
    let units: Vec<super::views::UnitView> = s
        .units
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let get = |k| s.get_param(lensbench_core::param::Param::unit(i, k)).ok();
            super::views::UnitView {
                name: u.name.clone(),
                spans: u.spans.clone(),
                auto: u.auto,
                motion: u.motion,
                shift: u.shift,
                dn: get(lensbench_core::units::UnitKind::Dn),
                disp: get(lensbench_core::units::UnitKind::Disp),
            }
        })
        .collect();
    // 機構部品（系ファイルの mechanics と同じ形、GUI の面の表で編集）
    let mechs = serde_json::to_value(&s.mechanics).unwrap_or_default();
    let v = SurfacesView {
        surfaces: all,
        units,
        mechanics: mechs,
    };
    Ok(v)
}
