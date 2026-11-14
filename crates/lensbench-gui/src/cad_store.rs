//! 品の STEP の共有の置き場の設定（アプリ版。`lensbench_cad::Store`）と、カタログのフォルダの窓の欄
#[cfg(not(target_arch = "wasm32"))]
use crate::i18n::tr;
#[cfg(not(target_arch = "wasm32"))]
use egui::{RichText, Ui};
use serde::{Deserialize, Serialize};

/// 品の STEP の共有の置き場（`lensbench_cad::Store`）。取るときは配布元より先に見て、配布元から取った STEP を上げる
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CadStore {
    /// 口の URL（例 `https://<プロジェクト>.pages.dev/cad`）
    pub url: String,
    /// Cloudflare Access のサービストークン（Client ID・Client Secret）。空なら送らない
    pub id: String,
    /// Client Secret は設定ファイルに書かない（ポータブルの settings.ron をフォルダごと渡すと付いていくため）。
    /// 起動のたびに入れるか、環境変数 `LENSBENCH_CAD_STORE_SECRET` に置く（欄が空ならそれを使う）
    #[serde(skip_serializing)]
    pub secret: String,
}

impl CadStore {
    /// 取るときに使う置き場に決める（アプリ版。URL が空なら環境変数か組み込みの置き場）
    pub fn apply(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        lensbench_cad::set_store(Some(lensbench_cad::Store {
            url: self.url.trim().to_string(),
            id: self.id.trim().to_string(),
            secret: match self.secret.trim() {
                "" => std::env::var("LENSBENCH_CAD_STORE_SECRET")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                s => s.to_string(),
            },
            read_only: false,
        }));
    }
}

/// 品の STEP の共有の置き場の欄（カタログのフォルダの窓）。変えたらすぐ使う
#[cfg(not(target_arch = "wasm32"))]
pub fn cad_store_ui(ui: &mut Ui, st: &mut CadStore) {
    ui.label(tr("品の STEP の置き場（共有）"));
    let before = st.clone();
    egui::Grid::new("cad_store").num_columns(2).show(ui, |ui| {
        ui.label(tr("URL"));
        ui.add(
            egui::TextEdit::singleline(&mut st.url)
                .hint_text("https://….pages.dev/cad")
                .desired_width(360.0),
        );
        ui.end_row();
        ui.label(tr("Client ID"));
        ui.add(egui::TextEdit::singleline(&mut st.id).desired_width(360.0));
        ui.end_row();
        ui.label(tr("Client Secret"));
        ui.add(
            egui::TextEdit::singleline(&mut st.secret)
                .password(true)
                .desired_width(360.0),
        );
        ui.end_row();
    });
    ui.label(
        RichText::new(tr(
            "品の STEP を取るとき、配布元より先にここを見ます。配布元から取った STEP はここにも上げます。Client ID・Secret は Cloudflare Access のサービストークン（守っていない口なら空）。空の URL は環境変数 LENSBENCH_CAD_STORE を使います。Secret は設定ファイルに保存しません（起動のたびに入れるか、環境変数 LENSBENCH_CAD_STORE_SECRET に置く）。",
        ))
        .small()
        .weak(),
    );
    if lensbench_cad::has_default_store() && st.url.trim().is_empty() {
        ui.label(
            RichText::new(tr("空のときは、この版に組み込んだ置き場を使います。"))
                .small()
                .weak(),
        );
    }
    if *st != before {
        st.apply();
    }
}

#[cfg(test)]
mod tests {
    use super::CadStore;

    /// 置き場の Client Secret は設定ファイルに書かない（前の版の設定に書いてあれば読める）
    #[test]
    fn secret_is_not_saved() {
        let st = CadStore {
            url: "https://x/cad".into(),
            id: "cid".into(),
            secret: "s3cret".into(),
        };
        let t = ron::to_string(&st).unwrap();
        assert!(t.contains("cid") && !t.contains("s3cret"), "{t}");
        let back: CadStore = ron::from_str(&t).unwrap();
        assert!(back.secret.is_empty());
        let old: CadStore = ron::from_str(r#"(url: "u", id: "i", secret: "old")"#).unwrap();
        assert_eq!(old.secret, "old");
    }
}
