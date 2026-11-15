//! カタログのフォルダの様子（ガラスのフォルダの AGF の名前・見つからないフォルダ）。計算の側（下書きの Worker）で
//! 調べる: Web 版の画面はメーカーのデータを持たない（GAR-11）

use super::*;

/// 調べたフォルダの並びと、その答え
#[derive(Default)]
pub(super) struct Folders {
    /// 頼んだフォルダの並び（ガラス、レンズ）。None は頼み直す
    asked: Option<(Vec<String>, Vec<String>)>,
    /// ガラスのフォルダにある AGF の名前（系の設定の硝材カタログの候補）
    pub agf: Vec<String>,
    /// 見つからないフォルダ
    pub missing: Vec<String>,
}

impl Folders {
    /// 次に調べ直す（カタログを読み直したとき）
    pub fn stale(&mut self) {
        self.asked = None;
    }
}

impl Core {
    /// フォルダの並びが変わっていたら、下書きの Worker に調べ直しを頼む（使う画面を描くたびに呼ぶ）
    pub(super) fn ask_folders(&mut self) {
        let now = (self.s.glass_dirs.clone(), self.s.lens_dirs.clone());
        if self.folders.asked.as_ref() == Some(&now) {
            return;
        }
        self.preview.send(
            Tag::Folders,
            "folders",
            json!({ "glass_dirs": now.0, "lens_dirs": now.1 }),
        );
        self.folders.asked = Some(now);
    }

    /// 答えを受け取る（今の並びの分だけ）
    pub(super) fn take_folders(&mut self, params: &Value, v: &Value) {
        let list = |x: &Value| -> Vec<String> {
            x.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let asked = (list(&params["glass_dirs"]), list(&params["lens_dirs"]));
        if self.folders.asked.as_ref() != Some(&asked) {
            return;
        }
        self.folders.agf = list(&v["agf"]);
        self.folders.missing = list(&v["missing"]);
    }
}
