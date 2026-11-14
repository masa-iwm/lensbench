//! 拡張の解析（GUI の側）: この GUI を組み込んだ exe（非公開版など）が足す解析を、解析パネルの一覧・図・設定の欄に
//! 載せる。計算は解析の登録簿に足した拡張の解析（`lensbench_engine::register`）で、ここでは見せ方だけを登録する。
//!
//! exe の入口で `lensbench_engine::register` と [`register`] をしてから [`crate::main`] を呼ぶ。図を描く部品は
//! [`fig`]（色の図・色の表・視野の選び・干渉の図など）。

use lensbench_core::RwLockExt;
use std::sync::RwLock;

use serde_json::{Map, Value};

/// 図を描く関数（ui、図の識別子、結果）。描けない結果なら false
pub type Figure = fn(&mut egui::Ui, u64, &Value) -> bool;

/// 図に要る引数を足す関数（引数の表、図で見せるか）
pub type FigureParams = fn(&mut Map<String, Value>, bool);

/// 拡張の解析
#[derive(Clone, Copy)]
pub struct Analysis {
    /// CLI のサブコマンドの名前（serve の analyze の cmd）
    pub cmd: &'static str,
    /// 一覧の表示名（日本語。英語は [`translations`] で足す）
    pub label: &'static str,
    /// 視野・波長を選べるか（解析が `--field`・`--wave` を受けるか）
    pub select: bool,
    /// 図を描く（描けない結果なら false。表で見せる）
    pub figure: Option<Figure>,
    /// 図に要る引数を足す（figure は図で見せるとき。利用者が与えた引数はそのまま残す）
    pub figure_params: Option<FigureParams>,
    /// 設定のオブジェクトの欄（センサの設定など）
    pub object: Option<ObjectArg>,
}

/// 設定のオブジェクトの欄: 項目（入れ子を「.」でつないだ名前）を欄に並べ、値は入れ子の JSON にして引数 `arg`
/// （解析の設定の JSON のオブジェクトの項目）に渡す。JSON のファイルに保存・読込できる
#[derive(Clone, Copy)]
pub struct ObjectArg {
    /// 引数の名前（解析の引数の欄からは除く。ファイルで受ける `<arg>_file` の引数も除く）
    pub arg: &'static str,
    /// 欄の見出し
    pub title: &'static str,
    /// 欄の下の説明
    pub note: &'static str,
    /// 項目（`lensbench_engine::params::param_json` と同じ形の JSON の並び）
    pub specs: fn() -> Vec<Value>,
    /// 項目の組（名前の頭, 見出し）。並べた順に、頭が合う項目を組に入れる（どれにも合わない項目は最後の組の後に）
    pub groups: &'static [(&'static str, &'static str)],
    /// 保存するファイルの既定の名前
    pub file_name: &'static str,
}

impl ObjectArg {
    /// 解析の引数の欄から除く引数か（オブジェクトそのものと、ファイルで受ける `<arg>_file`）
    pub fn covers(&self, name: &str) -> bool {
        name == self.arg || name.strip_suffix("_file") == Some(self.arg)
    }

    /// 欄の文字列（項目の名前 → 欄に書いた文字列、空の欄は省く）から、引数に渡す入れ子の JSON（GUI の欄と同じ変換）。
    /// `defaults` が真なら、書いていない項目も既定値で埋める
    pub fn json(
        &self,
        vals: &std::collections::BTreeMap<String, String>,
        defaults: bool,
    ) -> Result<Value, String> {
        let specs = crate::argform::object_specs(self);
        let mut all = std::collections::BTreeMap::new();
        if defaults {
            for s in &specs {
                let t = s.default_text();
                if !t.is_empty() {
                    all.insert(s.name.clone(), t);
                }
            }
        }
        all.extend(vals.iter().map(|(k, v)| (k.clone(), v.clone())));
        Ok(crate::argform::nest(&crate::argform::to_map(&specs, &all)?))
    }
}

static ANALYSES: RwLock<Vec<Analysis>> = RwLock::new(Vec::new());

/// 拡張の検出器（組合せの最後に置く検出器のブロック、`lensbench_project::detectors` に登録した種類）の欄
#[derive(Clone, Copy)]
pub struct Detector {
    /// ブロックの `"type"`
    pub kind: &'static str,
    /// 欄の項目（ブロックの項目。値は入れ子の JSON にしてブロックに入れる）
    pub object: ObjectArg,
}

static DETECTORS: RwLock<Vec<Detector>> = RwLock::new(Vec::new());

/// 検出器を足す（同じ kind は置き換える）
pub fn register_detector(d: Detector) {
    let mut v = DETECTORS.write_ok();
    v.retain(|x| x.kind != d.kind);
    v.push(d);
}

/// 足した検出器（足した順）
pub fn detectors() -> Vec<Detector> {
    DETECTORS.read_ok().clone()
}

/// kind の検出器
pub fn detector(kind: &str) -> Option<Detector> {
    detectors().into_iter().find(|d| d.kind == kind)
}

/// 解析を足す（同じ cmd は置き換える）
pub fn register(a: Analysis) {
    let mut v = ANALYSES.write_ok();
    v.retain(|x| x.cmd != a.cmd);
    v.push(a);
}

/// 足した解析（足した順）
pub fn list() -> Vec<Analysis> {
    ANALYSES.read_ok().clone()
}

/// cmd の拡張の解析
pub fn get(cmd: &str) -> Option<Analysis> {
    list().into_iter().find(|a| a.cmd == cmd)
}

/// 英語の訳（日本語, 英語）を足す（`tr` で引く。同梱の表に無いものだけ使う）
pub fn translations(pairs: &'static [(&'static str, &'static str)]) {
    crate::i18n::add(pairs);
}

/// 図を描く部品（拡張の図に使う）
pub mod fig {
    pub use crate::figures::{
        cmap, f64s, fig_height, heatmap, interference_grid, interference_view, pick_field, weak,
        Cmap, Grid,
    };
    pub use crate::i18n::{tr, trf};
    pub use crate::plots::fmt_sig;
    pub use crate::theme::series;
    pub use egui;
    pub use egui_plot;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fig(_: &mut egui::Ui, _: u64, v: &Value) -> bool {
        v["ok"] == true
    }

    #[test]
    fn registered_analysis_is_listed_and_drawn() {
        register(Analysis {
            cmd: "toy-camera",
            label: "試しのカメラ",
            select: true,
            figure: Some(fig),
            figure_params: Some(|m, figure| {
                if figure {
                    m.entry("grid").or_insert(json!(true));
                }
            }),
            object: Some(ObjectArg {
                arg: "sensor",
                title: "センサ",
                note: "",
                specs: || {
                    vec![
                        json!({"name": "a.b", "type": "num", "default": 1.0, "unit": "", "doc": ""}),
                    ]
                },
                groups: &[("a.", "A")],
                file_name: "s.json",
            }),
        });
        translations(&[("試しのカメラ", "Toy camera")]);
        assert!(crate::layout::analyses()
            .iter()
            .any(|a| a.0 == "toy-camera"));
        assert!(crate::layout::takes_sel("toy-camera"));
        assert!(crate::figures::has_figure("toy-camera"));
        let mut m = Map::new();
        crate::figures::figure_params("toy-camera", &mut m, None, true);
        assert_eq!(m["grid"], json!(true));
        let o = get("toy-camera").unwrap().object.unwrap();
        assert_eq!(crate::argform::object_specs(&o)[0].name, "a.b");
        assert_eq!(
            o.json(&Default::default(), true).unwrap(),
            json!({"a": {"b": 1.0}})
        );
        let v = [("a.b".to_string(), "2.5".to_string())].into();
        assert_eq!(o.json(&v, false).unwrap(), json!({"a": {"b": 2.5}}));
    }

    #[test]
    fn detector_block_goes_last() {
        register_detector(Detector {
            kind: "testdet",
            object: ObjectArg {
                arg: "",
                title: "テストの検出器",
                note: "",
                specs: || {
                    vec![
                        json!({"name": "mla.pitch", "type": "num", "default": 0.15, "unit": "mm", "doc": ""}),
                        json!({"name": "detector.bits", "type": "int", "default": 12, "unit": "", "doc": ""}),
                    ]
                },
                groups: &[("mla.", "MLA")],
                file_name: "d.json",
            },
        });
        assert!(detector("testdet").is_some());
        let mut c = crate::combo::Combo {
            detector: Some(crate::combo::DetectorPick {
                kind: "testdet".into(),
                vals: [("mla.pitch".to_string(), "0.3".to_string())].into(),
            }),
            ..Default::default()
        };
        let p = c.project();
        let last = p["blocks"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(last, json!({"type": "testdet", "mla": {"pitch": 0.3}}));
        // 読めない値は設定の誤り
        c.detector
            .as_mut()
            .unwrap()
            .vals
            .insert("detector.bits".into(), "x".into());
        assert!(!c.settings_problems().is_empty());
    }
}
