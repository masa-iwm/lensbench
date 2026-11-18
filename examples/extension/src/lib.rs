//! lensbench に足すものの雛形: 配列のプラグイン 1 つ、解析 1 つ（GUI の見せ方・訳つき）、面の形 1 つ、Python の関数 1 つ。
//!
//! 足すものはすべて [`extensions`] が 1 つの [`Extensions`] にまとめ、exe・wheel・Web 版の入口がそれを渡す
//! （`src/bin/cli.rs`・`src/bin/gui.rs`・下の `py`・`web`）。

use lensbench_sdk::analysis::{Analysis, Cx, Meta};
use lensbench_sdk::plugin::{
    Array, ArrayReader, Arrays, Config, ConfigReader, Context, Instance, Kind, ParamSpec,
    ParamType, Plugin, Spec, Value,
};
use lensbench_sdk::schemars::JsonSchema;
use lensbench_sdk::serde::{Deserialize, Serialize};
use lensbench_sdk::shape::{ShapeInstance, ShapeModel};
use lensbench_sdk::trace::FieldSel;
use lensbench_sdk::{Extensions, Result};
use std::sync::Arc;

/// 足すもの
pub fn extensions() -> Extensions {
    let e = Extensions::named("example")
        .plugins(|r| r.register(Arc::new(Gain)))
        .analysis(Chief)
        .shape_model(Arc::new(Saddle));
    #[cfg(feature = "gui")]
    let e = gui::add(e);
    #[cfg(feature = "py")]
    let e = py::add(e);
    e
}

// ------------------------------------------------------------------------------------------ プラグイン

/// 配列に数を掛けるプラグイン（名前 example_gain、設定 gain、入力 x、出力 y）
pub struct Gain;

impl Plugin for Gain {
    fn name(&self) -> &str {
        "example_gain"
    }
    fn kind(&self) -> Kind {
        Kind::Analysis
    }
    fn spec(&self) -> Spec {
        Spec {
            doc: "Multiply the array x by gain".into(),
            inputs: vec![("x".into(), "Values".into())],
            ..Spec::default()
        }
    }
    fn params(&self) -> Vec<ParamSpec> {
        vec![ParamSpec::new(
            "gain",
            ParamType::Num,
            Some(Value::Num(1.0)),
            "",
            "Factor",
        )]
    }
    fn prepare(&self, config: &Config) -> Result<Box<dyn Instance>> {
        let gain = ConfigReader::new(config).num("gain", 1.0)?;
        Ok(Box::new(GainRun { gain }))
    }
}

struct GainRun {
    gain: f64,
}

impl Instance for GainRun {
    fn run(&self, _: &Context<'_>, input: &Arrays) -> Result<Arrays> {
        let (shape, x) = ArrayReader::new(input).f64("x")?;
        let y: Vec<f64> = x.iter().map(|v| v * self.gain).collect();
        Ok(Arrays::from([(
            "y".to_string(),
            Array::f64(shape.to_vec(), y)?,
        )]))
    }
}

// ------------------------------------------------------------------------------------------ 面の形

/// 鞍形の形（名前 example_saddle、設定 a [1/mm]）: 面の種類 USERSHAPE の面に、編集の操作
/// （`set_surface_data`）では `"shape": {"model": "example_saddle", "config": {"a": 1e-4}}`、システムファイルでは
/// `"shape": {"user": {"model": "example_saddle", "config": {"a": 1e-4}}}` で付ける。土台の二次曲面に a x y を足す。
/// a は変数にできる（パラメータ `<面>.u_a`）
pub struct Saddle;

impl ShapeModel for Saddle {
    fn name(&self) -> &str {
        "example_saddle"
    }
    fn doc(&self) -> String {
        "Saddle a x y added to the conic".into()
    }
    fn params(&self) -> Vec<ParamSpec> {
        vec![ParamSpec::new(
            "a",
            ParamType::Num,
            Some(Value::Num(1e-4)),
            "1/mm",
            "Saddle coefficient",
        )]
    }
    fn prepare(&self, config: &Config) -> Result<Arc<dyn ShapeInstance>> {
        let a = ConfigReader::new(config).num("a", 0.0)?;
        Ok(Arc::new(SaddleShape { a }))
    }
}

struct SaddleShape {
    a: f64,
}

impl ShapeInstance for SaddleShape {
    /// 土台に足す矢高と勾配 (z, ∂z/∂x, ∂z/∂y)
    fn sag(&self, x: f64, y: f64) -> (f64, f64, f64) {
        (self.a * x * y, self.a * y, self.a * x)
    }
    /// 設定 a による微分（変数・dOPL/dp 用）
    fn dsag(&self, key: &str, x: f64, y: f64) -> Option<[f64; 3]> {
        (key == "a").then_some([x * y, y, x])
    }
}

// ------------------------------------------------------------------------------------------ 解析

/// 主光線が像面に着く位置（CLI のサブコマンド example-chief、serve の analyze の cmd）
pub struct Chief;

/// 設定
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "lensbench_sdk::serde", default, deny_unknown_fields)]
#[schemars(crate = "lensbench_sdk::schemars")]
pub struct ChiefParams {
    /// 視野の番号（1 始まり）
    pub field: Option<usize>,
}

/// 結果
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(crate = "lensbench_sdk::serde")]
#[schemars(crate = "lensbench_sdk::schemars")]
pub struct ChiefOut {
    /// 像面の局所座標の x・y [mm]
    pub x: f64,
    pub y: f64,
}

impl Analysis for Chief {
    type Params = ChiefParams;
    type Output = ChiefOut;
    fn meta(&self) -> Meta {
        Meta {
            name: "example-chief",
            about: "Where the chief ray lands on the image (extension example)",
            help: "",
        }
    }
    fn run(&self, cx: &Cx<'_>, p: &ChiefParams) -> Result<ChiefOut> {
        let f = p.field.unwrap_or(1).max(1) - 1;
        let t = cx
            .system
            .try_trace(FieldSel::Index(f), &[0.0], &[0.0], None, None, false)?;
        let r = t.rays[0];
        Ok(ChiefOut {
            x: r.xyz[0],
            y: r.xyz[1],
        })
    }
}

// ------------------------------------------------------------------------------------------ GUI

#[cfg(feature = "gui")]
mod gui {
    use lensbench_sdk::gui::{egui, fig, Analysis, GuiExtensions};
    use lensbench_sdk::serde_json::Value;
    use lensbench_sdk::Extensions;

    fn figure(ui: &mut egui::Ui, _: u64, v: &Value) -> bool {
        let (Some(x), Some(y)) = (v["x"].as_f64(), v["y"].as_f64()) else {
            return false;
        };
        ui.label(fig::trf(
            "Chief ray at ({}, {}) mm",
            &[&fig::fmt_sig(x, 6), &fig::fmt_sig(y, 6)],
        ));
        true
    }

    pub fn add(e: Extensions) -> Extensions {
        e.gui_analysis(
            Analysis::new("example-chief", "Chief ray on the image (example)")
                .select(true)
                .figure(figure),
        )
        .gui_translations(&[
            ("Chief ray on the image (example)", "像面の主光線（例）"),
            ("Chief ray at ({}, {}) mm", "主光線の位置 ({}, {}) mm"),
        ])
    }
}

// ------------------------------------------------------------------------------------------ Python

#[cfg(feature = "py")]
mod py {
    use lensbench_sdk::py::pyo3::prelude::*;
    use lensbench_sdk::py::PyExtensions;
    use lensbench_sdk::Extensions;

    /// Twice x (extension example)
    #[pyfunction]
    #[pyo3(crate = "lensbench_sdk::py::pyo3")]
    fn example_double(x: f64) -> f64 {
        2.0 * x
    }

    fn module(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_function(wrap_pyfunction!(example_double, m)?)
    }

    pub fn add(e: Extensions) -> Extensions {
        e.py_module(module)
    }

    /// 拡張モジュールの入口（import lensbench。公開の中身に、この拡張の分を足す）。pyo3 は SDK の再輸出なので、
    /// マクロには `crate = "lensbench_sdk::py::pyo3"` を付ける
    #[pymodule]
    #[pyo3(name = "lensbench", crate = "lensbench_sdk::py::pyo3")]
    fn lensbench_example(m: &Bound<'_, PyModule>) -> PyResult<()> {
        lensbench_sdk::py::module(m, super::extensions())
    }
}

// ------------------------------------------------------------------------------------------ Web 版

/// Web 版の入口（wasm32 だけ）。公開の scripts/build_web.sh に `CRATE=lensbench-extension-example
/// MANIFEST=<この Cargo.toml>` を渡すと、画面用（feature gui）・計算の Web Worker 用（worker・threads）の wasm を
/// 作り、公開の Web 版のページと一緒に置く。wasm が読み込まれると（画面・Worker のどれでも）先にこれが動く
#[cfg(target_arch = "wasm32")]
mod web {
    use lensbench_sdk::web::wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen(start, wasm_bindgen = lensbench_sdk::web::wasm_bindgen)]
    pub fn init() {
        // 名前の重なりは組み立ての誤り（tests/check.rs の試験が止める）
        super::extensions()
            .install()
            .expect("no name clashes with the built-in ones");
    }
}
