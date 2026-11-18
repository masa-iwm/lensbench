//! 拡張の解析（GUI の側）: この GUI を組み込んだ exe（非公開版など）が足す解析を、解析パネルの一覧・図・設定の欄に
//! 載せる。計算は解析の登録簿に足した拡張の解析（`lensbench_engine::register`）で、ここでは見せ方だけを登録する。
//!
//! 外のクレートは [`GuiExtensions`] で `lensbench_engine::Extensions` に見せ方を足し、[`crate::main_with`] に渡す
//! （計算の登録と同じ値 1 つ）。図を描く部品は [`fig`]（色の図・色の表・視野の選び・干渉の図など）。

use serde_json::{Map, Value};

/// 図を描く関数（ui、図の識別子、結果）。描けない結果なら false
pub type Figure = fn(&mut egui::Ui, u64, &Value) -> bool;

/// 図に要る引数を足す関数（引数の表、図で見せるか）
pub type FigureParams = fn(&mut Map<String, Value>, bool);

/// 拡張の解析（[`Analysis::new`] から作る）
#[derive(Clone, Copy)]
pub struct Analysis {
    /// CLI のサブコマンドの名前（serve の analyze の cmd）
    pub(crate) cmd: &'static str,
    /// 一覧の表示名（英語。日本語は [`GuiExtensions::gui_translations`] で足す）
    pub(crate) label: &'static str,
    /// 視野・波長を選べるか（解析が `--field`・`--wave` を受けるか）
    pub(crate) select: bool,
    /// 図を描く（描けない結果なら false。表で見せる）
    pub(crate) figure: Option<Figure>,
    /// 図に要る引数を足す（figure は図で見せるとき。利用者が与えた引数はそのまま残す）
    pub(crate) figure_params: Option<FigureParams>,
    /// 設定のオブジェクトの欄（センサの設定など）
    pub(crate) object: Option<ObjectArg>,
}

/// 設定のオブジェクトの欄: 項目（入れ子を「.」でつないだ名前）を欄に並べ、値は入れ子の JSON にして引数 `arg`
/// （解析の設定の JSON のオブジェクトの項目）に渡す。JSON のファイルに保存・読込できる（[`ObjectArg::new`] から作る）
#[derive(Clone, Copy)]
pub struct ObjectArg {
    /// 引数の名前（解析の引数の欄からは除く。ファイルで受ける `<arg>_file` の引数も除く）
    pub(crate) arg: &'static str,
    /// 欄の見出し
    pub(crate) title: &'static str,
    /// 欄の下の説明
    pub(crate) note: &'static str,
    /// 項目（`lensbench_engine::params::param_json` と同じ形の JSON の並び）
    pub(crate) specs: fn() -> Vec<Value>,
    /// 項目を選んだプラグインの設定から作るときの、プラグインの名前を受ける解析の引数（[`ObjectArg::specs_from`]）
    pub(crate) specs_from: Option<&'static str>,
    /// 項目の組（名前の頭, 見出し）。並べた順に、頭が合う項目を組に入れる（どれにも合わない項目は最後の組の後に）
    pub(crate) groups: &'static [(&'static str, &'static str)],
    /// 保存するファイルの既定の名前
    pub(crate) file_name: &'static str,
}

impl Analysis {
    /// 解析 cmd（CLI のサブコマンド・serve の analyze の cmd）を一覧に label で出す。視野・波長は選べない、図なし
    pub fn new(cmd: &'static str, label: &'static str) -> Self {
        Analysis {
            cmd,
            label,
            select: false,
            figure: None,
            figure_params: None,
            object: None,
        }
    }
    /// 視野・波長を選べる（解析が `--field`・`--wave` を受ける）
    pub fn select(mut self, yes: bool) -> Self {
        self.select = yes;
        self
    }
    /// 図を描く関数（描けない結果なら false。表で見せる）
    pub fn figure(mut self, f: Figure) -> Self {
        self.figure = Some(f);
        self
    }
    /// 図に要る引数を足す関数
    pub fn figure_params(mut self, f: FigureParams) -> Self {
        self.figure_params = Some(f);
        self
    }
    /// 設定のオブジェクトの欄
    pub fn object(mut self, o: ObjectArg) -> Self {
        self.object = Some(o);
        self
    }
    /// CLI のサブコマンドの名前
    pub fn cmd(&self) -> &'static str {
        self.cmd
    }
    /// 設定のオブジェクトの欄
    pub fn object_arg(&self) -> Option<&ObjectArg> {
        self.object.as_ref()
    }
}

impl ObjectArg {
    /// 引数 arg（解析の引数の欄から除く）の欄を見出し title で。項目は specs が返す
    /// （`lensbench_engine::params::param_json` と同じ形の JSON の並び）
    pub fn new(arg: &'static str, title: &'static str, specs: fn() -> Vec<Value>) -> Self {
        ObjectArg {
            arg,
            title,
            note: "",
            specs,
            specs_from: None,
            groups: &[],
            file_name: "settings.json",
        }
    }
    /// 欄の下の説明
    pub fn note(mut self, note: &'static str) -> Self {
        self.note = note;
        self
    }
    /// 項目の組（名前の頭, 見出し）。並べた順に、頭が合う項目を組に入れる
    pub fn groups(mut self, groups: &'static [(&'static str, &'static str)]) -> Self {
        self.groups = groups;
        self
    }
    /// 保存するファイルの既定の名前
    pub fn file_name(mut self, name: &'static str) -> Self {
        self.file_name = name;
        self
    }
    /// 項目を、解析の引数 arg（値がプラグインの名前）で選んだプラグインの設定の項目にする。引数が空なら解析の
    /// 既定値のプラグイン、プラグインが無ければ new に渡した specs。欄の見出しにはプラグインの名前を添える
    pub fn specs_from(mut self, arg: &'static str) -> Self {
        self.specs_from = Some(arg);
        self
    }
    /// 項目（specs_from のときは選んだプラグインがないときの分）
    pub fn specs(&self) -> Vec<Value> {
        (self.specs)()
    }
    /// 解析の引数の欄 args（解析 cmd）で選んだプラグインの名前（specs_from のときだけ）
    pub fn chosen_plugin(
        &self,
        cmd: &str,
        args: &std::collections::BTreeMap<String, String>,
    ) -> Option<String> {
        let from = self.specs_from?;
        let v = args.get(from).map(|s| s.trim()).filter(|s| !s.is_empty());
        let v = match v {
            Some(v) => v.to_string(),
            None => crate::argform::analysis_specs(cmd)
                .iter()
                .find(|s| s.name == from)
                .map(|s| s.default_text())?,
        };
        (!v.is_empty()).then_some(v)
    }
    /// 項目（specs_from なら、選んだプラグイン `chosen` の設定の項目。無いプラグインなら new に渡した specs）
    pub fn specs_for(&self, chosen: Option<&str>) -> Vec<Value> {
        if self.specs_from.is_some() {
            if let Some(v) = chosen.and_then(lensbench_engine::params::plugin_params) {
                return v;
            }
        }
        (self.specs)()
    }
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
        self.json_for(None, vals, defaults)
    }

    /// [`ObjectArg::json`] の、specs_from で選んだプラグイン `chosen`（[`ObjectArg::chosen_plugin`]）の項目で
    pub fn json_for(
        &self,
        chosen: Option<&str>,
        vals: &std::collections::BTreeMap<String, String>,
        defaults: bool,
    ) -> Result<Value, String> {
        let specs = crate::argform::object_specs(self, chosen);
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

/// GUI の見せ方の表（拡張の表 `lensbench_core::ext::Ext` の欄）
#[derive(Clone, Default)]
pub struct GuiExt {
    analyses: Vec<Analysis>,
    detectors: Vec<Detector>,
}

impl GuiExt {
    /// 解析を足す（同じ cmd は置き換える）
    pub fn add_analysis(&mut self, a: Analysis) {
        self.analyses.retain(|x| x.cmd != a.cmd);
        self.analyses.push(a);
    }

    /// 検出器を足す（同じ kind は置き換える）
    pub fn add_detector(&mut self, d: Detector) {
        self.detectors.retain(|x| x.kind != d.kind);
        self.detectors.push(d);
    }
}

/// `lensbench_engine::Extensions` に GUI の見せ方を足す（表の欄に入れる。install・ext のときに足す）
pub trait GuiExtensions {
    /// 解析の見せ方（同じ cmd は置き換える）
    fn gui_analysis(self, a: Analysis) -> Self;
    /// 検出器の欄（同じ kind は置き換える）
    fn gui_detector(self, d: Detector) -> Self;
    /// 画面の文字の訳（英語, 日本語）。`tr` で引く（同梱の表に無いものだけ使う）
    fn gui_translations(self, pairs: &'static [(&'static str, &'static str)]) -> Self;
}

impl GuiExtensions for lensbench_engine::Extensions {
    fn gui_analysis(self, a: Analysis) -> Self {
        self.with_ext(move |e| e.slot_mut::<GuiExt>().add_analysis(a))
    }
    fn gui_detector(self, d: Detector) -> Self {
        self.with_ext(move |e| e.slot_mut::<GuiExt>().add_detector(d))
    }
    fn gui_translations(self, pairs: &'static [(&'static str, &'static str)]) -> Self {
        self.on_install(move || crate::i18n::add(pairs))
    }
}

/// 表 e に cmd の解析の見せ方があるか（拡張の検査用）
pub fn has_analysis(e: &lensbench_core::ext::Ext, cmd: &str) -> bool {
    e.slot::<GuiExt>()
        .is_some_and(|t| t.analyses.iter().any(|a| a.cmd == cmd))
}

/// 表 e に kind の検出器の欄があるか（拡張の検査用）
pub fn has_detector(e: &lensbench_core::ext::Ext, kind: &str) -> bool {
    e.slot::<GuiExt>()
        .is_some_and(|t| t.detectors.iter().any(|d| d.kind == kind))
}

fn table() -> Option<GuiExt> {
    lensbench_core::ext::current().slot::<GuiExt>().cloned()
}

/// 表を写さずに f で読む（毎フレーム引く `get`・`detector` 用）
fn with_table<R>(f: impl FnOnce(&GuiExt) -> Option<R>) -> Option<R> {
    lensbench_core::ext::current().slot::<GuiExt>().and_then(f)
}

/// 拡張の検出器（組合せの最後に置く検出器のブロック、`lensbench_project::detectors` に登録した種類）の欄
#[derive(Clone, Copy)]
pub struct Detector {
    /// ブロックの `"type"`
    pub(crate) kind: &'static str,
    /// 欄の項目（ブロックの項目。値は入れ子の JSON にしてブロックに入れる）
    pub(crate) object: ObjectArg,
}

impl Detector {
    /// ブロックの `"type"` が kind の検出器の欄
    pub fn new(kind: &'static str, object: ObjectArg) -> Self {
        Detector { kind, object }
    }
    /// 欄の項目
    pub fn object_arg(&self) -> &ObjectArg {
        &self.object
    }
}

/// 足した検出器（足した順）
pub fn detectors() -> Vec<Detector> {
    table().map(|t| t.detectors).unwrap_or_default()
}

/// kind の検出器
pub fn detector(kind: &str) -> Option<Detector> {
    with_table(|t| t.detectors.iter().find(|d| d.kind == kind).copied())
}

/// 足した解析（足した順）
pub fn list() -> Vec<Analysis> {
    table().map(|t| t.analyses).unwrap_or_default()
}

/// cmd の拡張の解析
pub fn get(cmd: &str) -> Option<Analysis> {
    with_table(|t| t.analyses.iter().find(|a| a.cmd == cmd).copied())
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
        let a = Analysis::new("toy-camera", "試しのカメラ")
            .select(true)
            .figure(fig)
            .figure_params(|m, figure| {
                if figure {
                    m.entry("grid").or_insert(json!(true));
                }
            })
            .object(
                ObjectArg::new("sensor", "センサ", || {
                    vec![
                        json!({"name": "a.b", "type": "num", "default": 1.0, "unit": "", "doc": ""}),
                    ]
                })
                .groups(&[("a.", "A")])
                .file_name("s.json"),
            );
        lensbench_engine::Extensions::new()
            .gui_analysis(a)
            .gui_translations(&[("Toy camera", "試しのカメラ")])
            .install()
            .unwrap();
        assert!(crate::layout::analyses()
            .iter()
            .any(|a| a.0 == "toy-camera"));
        assert!(crate::layout::takes_sel("toy-camera"));
        assert!(crate::figures::has_figure("toy-camera"));
        let mut m = Map::new();
        crate::figures::figure_params("toy-camera", &mut m, None, true);
        assert_eq!(m["grid"], json!(true));
        let o = get("toy-camera").unwrap().object.unwrap();
        assert_eq!(crate::argform::object_specs(&o, None)[0].name, "a.b");
        assert_eq!(
            o.json(&Default::default(), true).unwrap(),
            json!({"a": {"b": 1.0}})
        );
        let v = [("a.b".to_string(), "2.5".to_string())].into();
        assert_eq!(o.json(&v, false).unwrap(), json!({"a": {"b": 2.5}}));
    }

    /// specs_from: 欄の項目は引数で選んだプラグインの設定。無いプラグイン・空の引数（既定の無い解析）なら new の specs
    #[test]
    fn object_fields_from_the_chosen_plugin() {
        let o = ObjectArg::new("cfg", "Settings", Vec::new).specs_from("analysis");
        let mut args = std::collections::BTreeMap::new();
        assert_eq!(o.chosen_plugin("no-such-analysis", &args), None);
        args.insert("analysis".into(), " geo_mtf ".into());
        let p = o.chosen_plugin("no-such-analysis", &args);
        assert_eq!(p.as_deref(), Some("geo_mtf"));
        let names: Vec<String> = crate::argform::object_specs(&o, p.as_deref())
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.iter().any(|n| n == "freqs"), "{names:?}");
        assert!(o.specs_for(Some("no-such-plugin")).is_empty());
        // specs_from の無い欄は選んだプラグインに依らない
        let plain = ObjectArg::new("cfg", "Settings", Vec::new);
        assert_eq!(plain.chosen_plugin("x", &args), None);
        assert!(plain.specs_for(Some("geo_mtf")).is_empty());
    }

    #[test]
    fn detector_block_goes_last() {
        let o = ObjectArg::new("", "テストの検出器", || {
            vec![
                json!({"name": "mla.pitch", "type": "num", "default": 0.15, "unit": "mm", "doc": ""}),
                json!({"name": "detector.bits", "type": "int", "default": 12, "unit": "", "doc": ""}),
            ]
        })
        .groups(&[("mla.", "MLA")])
        .file_name("d.json");
        lensbench_engine::Extensions::new()
            .gui_detector(Detector::new("testdet", o))
            .install()
            .unwrap();
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
