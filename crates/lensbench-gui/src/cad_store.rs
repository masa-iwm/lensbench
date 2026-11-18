//! 品の STEP の共有の置き場の設定（アプリ版。`lensbench_cad::Store`）と、カタログのフォルダの窓の欄
#[cfg(not(target_arch = "wasm32"))]
use crate::i18n::tr;
#[cfg(not(target_arch = "wasm32"))]
use egui::{RichText, Ui};
use serde::{Deserialize, Serialize};

/// 品の STEP の共有の置き場（`lensbench_cad::Store`）。取るときは配布元より先に見て、配布元から取った STEP を上げる
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CadStore {
    /// 口の URL（例 `https://<プロジェクト>.pages.dev/cad`）
    pub url: String,
    /// Cloudflare Access のサービストークン（Client ID・Client Secret）。空なら送らない
    pub id: String,
    /// Client Secret は設定ファイルに書かない（ポータブルの settings.ron をフォルダごと渡すと付いていくため）。
    /// 起動のたびに入れるか、環境変数 `LENSBENCH_CAD_STORE_SECRET` に置く（欄が空ならそれを使う）
    #[serde(skip)]
    pub secret: String,
}

impl CadStore {
    /// 取るときに使う置き場に決める（アプリ版。URL が空なら環境変数か組み込みの置き場）
    pub(crate) fn apply(&self) {
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
pub(crate) fn cad_store_ui(ui: &mut Ui, st: &mut CadStore) {
    ui.label(tr("Shared store for item STEP files"));
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
            "When fetching an item's STEP, this is checked before the vendor. STEP files fetched from the vendor are also uploaded here. Client ID and Secret are a Cloudflare Access service token (leave empty for an unprotected endpoint). An empty URL uses the LENSBENCH_CAD_STORE environment variable. The Secret is not saved in the settings file (enter it each time, or set the LENSBENCH_CAD_STORE_SECRET environment variable).",
        ))
        .small()
        .weak(),
    );
    if lensbench_cad::has_default_store() && st.url.trim().is_empty() {
        ui.label(
            RichText::new(tr("When empty, the store built into this edition is used."))
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

    /// 置き場の Client Secret は設定ファイルに書かず、読まない（前の版の設定に書いてあっても捨てる）
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
        assert!(old.secret.is_empty() && old.id == "i");
    }
}
