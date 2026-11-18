//! lensbench に足すものの雛形: 配列のプラグイン 1 つ、解析 1 つ（GUI の見せ方・訳つき）、Python の関数 1 つ。
//!
//! 足すものはすべて [`extensions`] が 1 つの [`Extensions`] にまとめ、exe・wheel の入口がそれを渡す
//! （`src/bin/cli.rs`・`src/bin/gui.rs`・下の `py`）。

use lensbench_sdk::analysis::{Analysis, Cx, Meta};
use lensbench_sdk::plugin::{
    Array, ArrayReader, Arrays, Config, ConfigReader, Context, Instance, Kind, ParamSpec,
    ParamType, Plugin, Spec, Value,
};
use lensbench_sdk::schemars::JsonSchema;
use lensbench_sdk::serde::{Deserialize, Serialize};
use lensbench_sdk::trace::FieldSel;
use lensbench_sdk::{Extensions, Result};
use std::sync::Arc;

/// 足すもの
pub fn extensions() -> Extensions {
    let e = Extensions::named("example")
        .plugins(|r| r.register(Arc::new(Gain)))
        .analysis(Chief);
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
        e.gui_analysis(Analysis {
            cmd: "example-chief",
            label: "Chief ray on the image (example)",
            select: true,
            figure: Some(figure),
            figure_params: None,
            object: None,
        })
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
