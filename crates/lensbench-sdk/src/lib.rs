//! lensbench の拡張の SDK: 外のクレートが依存するのはこのクレート 1 つ。
//!
//! 外のクレートは足すもの（プラグイン・解析・検出器・利用者定義の面・カタログの形式・誤りの文の訳、GUI の見せ方、
//! Python の関数）を [`Extensions`] 1 つにまとめる関数を持ち、自分の exe・wheel の入口でそれを渡す:
//!
//! ```ignore
//! fn main() -> std::process::ExitCode {
//!     lensbench_sdk::cli::main(my_ext::extensions())
//! }
//! ```
//!
//! GUI は [`gui::main`]（feature `gui`）、Python の拡張モジュールは [`py::module`]（feature `py`）。
//!
//! **約束の型** はこのクレートが再輸出するものだけ（下のモジュール）。各クレートのほかの pub は中の都合で変わる。
//! 0.x の間はマイナー版で約束の型を壊してよく、壊す変更は CHANGELOG の「拡張の約束」に書く。外は git のタグか
//! rev で参照する。pyo3・egui は境を越える型なので、外は [`py::pyo3`]・[`gui::egui`] だけを使い、自分の Cargo.toml に
//! 直接の依存を書かない（版がずれると型が合わない）。

pub use lensbench_core::{Error, Result};
pub use lensbench_engine::Extensions;
// 解析の設定・結果の型に使う（derive は `#[serde(crate = "lensbench_sdk::serde")]`・
// `#[schemars(crate = "lensbench_sdk::schemars")]` を付ける）
pub use schemars;
pub use serde;
pub use serde_json;

/// 検出器・解析のプラグイン（名前つきの配列を受けて返す）
pub mod plugin {
    pub use lensbench_core::plugin::{
        Array, ArrayReader, Arrays, Config, ConfigReader, Context, Data, Instance, Kind,
        OutputSpec, ParamSpec, ParamType, Plugin, Registry, Spec, Value,
    };
}

/// 解析（CLI のサブコマンド・serve の analyze・GUI の解析）
pub mod analysis {
    pub use lensbench_analysis::{Analysis, Cx, Meta, Param, ParamTy, Saved, Source, Variant};
}

/// 系（読むだけ）と、プロジェクトからの展開
pub mod system {
    pub use lensbench_core::param::Param;
    pub use lensbench_core::System;
    pub use lensbench_project::{expand, BlockAt, Env, ExpandError, ExpandOpts, Project};
}

/// 追跡の要求と結果
pub mod trace {
    pub use lensbench_core::raygrid::RayGrid;
    pub use lensbench_core::system::{
        Cols, FieldSel, Launch, ObjSpec, PolIn, PolTrace, RayBuf, RecSel, Trace, TraceOpts,
    };
    pub use lensbench_core::trace::{RayCounts, RayOut, RayStatus};
}

/// 利用者定義の面のモデル
pub mod surface {
    pub use lensbench_core::usersurf::{check_flip, Flip, SurfaceInstance, SurfaceModel};
}

/// レンズカタログの形式
pub mod catalog {
    pub use lensbench_core::catalog::{CatalogLens, LensCatalogSource};
}

/// プロジェクトの検出器のブロック
pub mod detector {
    pub use lensbench_project::detectors::DetectorKind;
}

/// 足し忘れの検査: GUI の見せ方の無い解析、検出器の欄の無い検出器（feature `gui` のとき）、反転の約束を守らない
/// 利用者定義の面のモデル。見つけたものの説明を返す（空なら問題なし）。雛形と exe の試験で呼ぶ
pub fn check(ext: &Extensions) -> Vec<String> {
    let mut out = Vec::new();
    let table = match ext.ext() {
        Ok(t) => t,
        Err(e) => return vec![e.to_string()],
    };
    #[cfg(feature = "gui")]
    {
        for a in ext.analysis_names() {
            if !lensbench_gui::ext::has_analysis(&table, a) {
                out.push(format!(
                    "analysis {a} has no GUI presentation (gui_analysis)"
                ));
            }
        }
        for d in ext.detector_kinds() {
            if !lensbench_gui::ext::has_detector(&table, d.name) {
                out.push(format!(
                    "detector {} has no GUI fields (gui_detector)",
                    d.name
                ));
            }
        }
    }
    #[cfg(not(feature = "gui"))]
    let _ = &table;
    for m in ext.surface_models() {
        if let Some(c) = m.example_config() {
            if let Err(e) = lensbench_core::usersurf::check_flip(m.as_ref(), &c) {
                out.push(format!("surface model {}: {e}", m.name()));
            }
        }
    }
    out
}

/// CLI の入口（feature `cli`）
#[cfg(feature = "cli")]
pub mod cli {
    /// 拡張を入れて CLI を動かす
    pub fn main(ext: crate::Extensions) -> std::process::ExitCode {
        lensbench_cli::main_with(ext)
    }
}

/// GUI の入口と見せ方（feature `gui`）
#[cfg(feature = "gui")]
pub mod gui {
    pub use egui;
    pub use lensbench_gui::ext::{
        fig, Analysis, Detector, Figure, FigureParams, GuiExtensions, ObjectArg,
    };

    /// 拡張を入れて GUI を起動する
    #[cfg(not(target_arch = "wasm32"))]
    pub fn main(ext: crate::Extensions) -> std::process::ExitCode {
        lensbench_gui::main_with(ext)
    }
}

/// Python の拡張モジュールの入口と関数の足し方（feature `py`）
#[cfg(feature = "py")]
pub mod py {
    pub use lensbench::ext::{
        arrays_to_py, ModuleFn, NativeTask, NativeTaskFrom, PyExtensions, PyPlugin, PySystem,
    };
    pub use pyo3;

    /// 拡張を入れ、同梱の関数・クラスと拡張の分をモジュールに足す（拡張モジュールの `#[pymodule]` から呼ぶ）
    pub fn module(
        m: &pyo3::Bound<'_, pyo3::types::PyModule>,
        ext: crate::Extensions,
    ) -> pyo3::PyResult<()> {
        lensbench::ext::module_with(m, ext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同梱の利用者定義の面（反転の約束を守る）は検査を通る。組み込みと同じ名前の検出器は表を作れずに挙がる
    #[test]
    fn check_passes_a_good_model_and_reports_errors() {
        let m = lensbench_core::ext::current()
            .surface_model("ideal_mapping")
            .unwrap();
        let ok = Extensions::named("t").surface_model(m);
        assert!(check(&ok).is_empty(), "{:?}", check(&ok));
        let bad = lensbench_project::BUILTIN_BLOCKS[0];
        let e = Extensions::named("t").detector(detector::DetectorKind {
            name: bad,
            doc: "",
            check: |_| Ok(()),
            schema: None,
        });
        assert_eq!(check(&e).len(), 1);
    }

    /// GUI の見せ方の無い検出器を挙げ、足せば挙げない
    #[cfg(feature = "gui")]
    #[test]
    fn check_reports_a_detector_without_gui_fields() {
        use gui::GuiExtensions;
        let k = detector::DetectorKind {
            name: "sdk-test-detector",
            doc: "",
            check: |_| Ok(()),
            schema: None,
        };
        let e = Extensions::named("t").detector(k);
        assert_eq!(
            check(&e),
            ["detector sdk-test-detector has no GUI fields (gui_detector)"]
        );
        let e = e.gui_detector(gui::Detector {
            kind: "sdk-test-detector",
            object: gui::ObjectArg {
                arg: "",
                title: "",
                note: "",
                specs: Vec::new,
                groups: &[],
                file_name: "d.json",
            },
        });
        assert!(check(&e).is_empty());
    }
}
