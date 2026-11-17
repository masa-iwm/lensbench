//! BS（ビームスプリッター）だけが持つ欄（[`Bs`]）と splitter ブロック（種類ごとの欄を分ける）

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{ElemKind, Pick};
use crate::i18n::tr;

/// BS の形・分け方・膜（[`Pick::bs`]。BS の要素だけが持つ）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bs {
    /// 形
    pub form: BsForm,
    /// キューブの一辺・平板の厚さ [mm]
    pub size: f64,
    /// キューブ・平板の硝材
    pub glass: String,
    /// 透過率
    pub ratio: f64,
    /// 分ける面の向きを逆に（反射を反対側へ。キューブの分ける面の傾き −45°）
    pub bs_flip: bool,
    /// 平板: くさび。裏面を表面に対して傾ける角 x・y [deg]
    pub wedge: [f64; 2],
    /// 平板: 裏の面で分ける
    pub split_back: bool,
    /// 平板・理想面: 分ける面の傾き x・y [deg]。反射を逆向きにすると x の符号が変わる（キューブは 45° のまま）
    pub bs_angle: [f64; 2],
    /// 分ける面の分け方（None は前の保存: 膜があれば膜、無ければ分岐比）
    pub split: Option<BsSplit>,
    /// 理想の偏光 BS の p・s の強度の透過率
    pub pbs: [f64; 2],
    /// 分ける面以外の面の膜の既定（空なら膜なし）
    pub ar: String,
    /// 面ごとの膜（キューブは a〜d、平板は front・back）
    pub faces: std::collections::BTreeMap<String, String>,
}

impl Bs {
    /// 既定（BS でない要素が読むときもこの値）
    pub const DEFAULT: Bs = Bs {
        form: BsForm::Cube,
        size: 12.7,
        glass: String::new(),
        ratio: 0.5,
        bs_flip: false,
        wedge: [0.0; 2],
        split_back: false,
        bs_angle: [45.0, 0.0],
        split: None,
        pbs: [1.0, 0.0],
        ar: String::new(),
        faces: std::collections::BTreeMap::new(),
    };
}

impl Default for Bs {
    fn default() -> Self {
        Bs::DEFAULT
    }
}

/// BS の形
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BsForm {
    /// キューブ（斜面が 45°）
    #[default]
    Cube,
    /// 平板（表裏の 2 面、くさびも可）
    Plate,
    /// 厚さのない理想の面
    Ideal,
}

impl BsForm {
    pub fn label(self) -> &'static str {
        match self {
            BsForm::Cube => tr("Cube"),
            BsForm::Plate => tr("Plate"),
            BsForm::Ideal => tr("ideal surface"),
        }
    }
    pub(super) fn key(self) -> &'static str {
        match self {
            BsForm::Cube => "cube",
            BsForm::Plate => "plate",
            BsForm::Ideal => "ideal",
        }
    }
    /// 面ごとの膜を付ける面（分ける面を除く）
    pub fn faces(self) -> &'static [&'static str] {
        match self {
            BsForm::Cube => &["a", "b", "c", "d"],
            BsForm::Plate => &["front", "back"],
            BsForm::Ideal => &[],
        }
    }
}

/// BS の分ける面の分け方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BsSplit {
    /// 偏光によらない理想の分岐比
    Ratio,
    /// 膜（膜ファイル・膜モデル）
    Coat,
    /// 理想の偏光 BS
    Pbs,
}

impl BsSplit {
    pub fn label(self) -> &'static str {
        match self {
            BsSplit::Ratio => tr("Split ratio"),
            BsSplit::Coat => tr("Coating"),
            BsSplit::Pbs => tr("Ideal PBS"),
        }
    }
}

impl Pick {
    /// BS の欄（BS でなければ既定）
    pub fn bs(&self) -> &Bs {
        static DEFAULT: Bs = Bs::DEFAULT;
        self.bs.as_deref().unwrap_or(&DEFAULT)
    }

    /// BS の欄を変える（無ければ作る。BS の要素だけで呼ぶ）
    pub fn bs_mut(&mut self) -> &mut Bs {
        self.bs.get_or_insert_with(Box::default)
    }

    /// BS の分ける面の分け方
    pub fn split_mode(&self) -> BsSplit {
        self.bs().split.unwrap_or(if self.coat.trim().is_empty() {
            BsSplit::Ratio
        } else {
            BsSplit::Coat
        })
    }

    /// BS の分ける面の傾き（反射を逆向きにすると x の符号が変わる）
    pub fn split_angle(&self) -> [f64; 2] {
        let a = match self.bs().form {
            BsForm::Cube => [45.0, 0.0],
            _ => self.bs().bs_angle,
        };
        if self.bs().bs_flip {
            [-a[0], a[1]]
        } else {
            a
        }
    }

    /// splitter ブロック（分岐の t・r・d は [`Combo::project`] が足す）
    pub(super) fn splitter_block(&self, id: &str) -> Value {
        let mut b = json!({
            "type": "splitter", "id": id, "form": self.bs().form.key(),
            "arm": if self.arm_r { "r" } else { "t" },
            "angle": self.split_angle(),
        });
        match self.bs().form {
            BsForm::Cube => {
                b["size"] = json!(self.bs().size);
                b["glass"] = json!(self.bs().glass);
            }
            BsForm::Plate => {
                b["thickness"] = json!(self.bs().size);
                b["glass"] = json!(self.bs().glass);
                b["semi"] = json!(self.semi);
                if self.bs().wedge != [0.0; 2] {
                    b["wedge"] = json!(self.bs().wedge);
                }
                if self.bs().split_back {
                    b["split_face"] = json!("back");
                }
            }
            BsForm::Ideal => {
                b["semi"] = json!(self.semi);
            }
        }
        match self.split_mode() {
            BsSplit::Coat if !self.coat.trim().is_empty() => b["coat"] = json!(self.coat.trim()),
            BsSplit::Pbs => b["pbs"] = json!({ "tp": self.bs().pbs[0], "ts": self.bs().pbs[1] }),
            _ => b["ratio"] = json!(self.bs().ratio),
        }
        if self.bs().form != BsForm::Ideal {
            if !self.bs().ar.trim().is_empty() {
                b["ar"] = json!(self.bs().ar.trim());
            }
            let faces: serde_json::Map<String, Value> = self
                .bs()
                .faces
                .iter()
                .filter(|(k, v)| {
                    !v.trim().is_empty() && self.bs().form.faces().contains(&k.as_str())
                })
                .map(|(k, v)| (k.clone(), json!(v.trim())))
                .collect();
            if !faces.is_empty() {
                b["faces"] = Value::Object(faces);
            }
        }
        b
    }

    /// カタログの BS の中身（[`Pick::extra`]）のうち、欄のある項目を欄へ移す
    pub fn absorb_extra(&mut self) {
        if self.kind != ElemKind::Splitter {
            return;
        }
        // extra を外して読み、残りがあれば戻す（欄へ移すとき self を借りるため）
        let mut extra = std::mem::take(&mut self.extra);
        let Some(m) = extra.as_object_mut() else {
            self.extra = extra;
            return;
        };
        let num2 =
            |v: &Value| -> Option<[f64; 2]> { Some([v.get(0)?.as_f64()?, v.get(1)?.as_f64()?]) };
        if let Some(f) = m.remove("form") {
            self.bs_mut().form = match f.as_str() {
                Some("plate") => BsForm::Plate,
                Some("ideal") => BsForm::Ideal,
                _ => BsForm::Cube,
            };
        }
        if let Some(x) = m.remove("thickness").and_then(|v| v.as_f64()) {
            if self.bs_mut().form == BsForm::Plate {
                self.bs_mut().size = x;
            }
        }
        if let Some(x) = m.remove("size").and_then(|v| v.as_f64()) {
            if self.bs_mut().form == BsForm::Cube && x > 0.0 {
                self.bs_mut().size = x;
            }
        }
        if let Some(x) = m.remove("semi").and_then(|v| v.as_f64()) {
            self.semi = x;
        }
        if let Some(x) = m.remove("wedge").as_ref().and_then(num2) {
            self.bs_mut().wedge = x;
        }
        if let Some(f) = m.remove("split_face") {
            self.bs_mut().split_back = f == "back";
        }
        if let Some(a) = m.remove("angle").as_ref().and_then(num2) {
            self.bs_mut().bs_flip = a[0] < 0.0;
            self.bs_mut().bs_angle = [a[0].abs(), a[1]];
        }
        if let Some(a) = m.remove("arm") {
            self.arm_r = a == "r";
        }
        if let Some(p) = m.remove("pbs").filter(|p| p.is_object()) {
            self.bs_mut().pbs = [
                p["tp"].as_f64().unwrap_or(1.0),
                p["ts"].as_f64().unwrap_or(0.0),
            ];
            self.bs_mut().split = Some(BsSplit::Pbs);
        }
        if let Some(x) = m.remove("ratio").and_then(|v| v.as_f64()) {
            self.bs_mut().ratio = x;
        }
        if let Some(c) = m
            .remove("coat")
            .and_then(|v| v.as_str().map(str::to_string))
        {
            self.coat = c;
        }
        if let Some(a) = m.remove("ar").and_then(|v| v.as_str().map(str::to_string)) {
            self.bs_mut().ar = a;
        }
        if let Some(f) = m.remove("faces").and_then(|v| v.as_object().cloned()) {
            for (k, v) in f {
                if let Some(v) = v.as_str() {
                    self.bs_mut().faces.insert(k, v.to_string());
                }
            }
        }
        if let Some(g) = m
            .remove("glass")
            .and_then(|v| v.as_str().map(str::to_string))
        {
            self.bs_mut().glass = g;
        }
        for k in ["type", "id"] {
            m.remove(k);
        }
        if !m.is_empty() {
            self.extra = extra;
        }
    }
}
