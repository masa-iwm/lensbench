//! カタログの品の詳しい形（公式の STEP）を 1 件だけ取る（公開 #120）。
//!
//! 品の一覧で選んだ品に `cad.step` があれば「詳しい形（STEP）を取る」ボタンを出し、押したときにその 1 ファイルだけを
//! 裏のスレッドで取る（serve の `cad_step` と同じ [`lensbench_cli::cad`]）。キャッシュは設定のフォルダの `cad`。
//! 取れたら簡易外形との食い違いを示し、取れなければ品ページのリンクを出す（表示は簡易外形のまま）。
//! Web 版は取らず、リンクだけを出す。

// Web 版は取らない（状態・結果の表示は使わない）
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use egui::{RichText, Ui};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::catalog::LensRow;
use crate::i18n::{tr, trf};

/// キャッシュのフォルダ（設定のフォルダの `cad`）
#[cfg(not(target_arch = "wasm32"))]
static CAD_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// キャッシュのフォルダを決める（起動時に 1 回。無ければ CLI と同じ既定の場所）
#[cfg(not(target_arch = "wasm32"))]
pub fn set_cad_dir(settings_dir: Option<PathBuf>) {
    if let Some(d) = settings_dir {
        let _ = CAD_DIR.set(d.join("cad"));
        // 系を開くときも同じキャッシュを見る（詳しい形の描画）
        lensbench_project::catalog_fill::set_cad_cache_dir(d.join("cad"));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn cad_dir() -> PathBuf {
    CAD_DIR
        .get()
        .cloned()
        .unwrap_or_else(lensbench_cli::cad::default_cache_dir)
}

/// 品の取得の状態
#[derive(Debug, Clone)]
pub enum CadState {
    Pending,
    /// serve の `cad_step` と同じ結果
    Done(Value),
    Failed(String),
}

/// 品ごとの取得の状態（鍵はカタログのファイルと品の名前）
#[derive(Default)]
pub struct CadFetcher {
    pub state: HashMap<(String, String), CadState>,
    #[cfg(not(target_arch = "wasm32"))]
    rx: Option<std::sync::mpsc::Receiver<((String, String), CadState)>>,
    #[cfg(not(target_arch = "wasm32"))]
    tx: Option<std::sync::mpsc::Sender<((String, String), CadState)>>,
}

/// 行の品のカタログのファイルと名前（lensbench 形式のカタログの品だけ）
fn key_of(r: &LensRow) -> Option<(String, String)> {
    Some((
        r.raw["source"]["catalog"].as_str()?.to_string(),
        r.raw["source"]["item"].as_str()?.to_string(),
    ))
}

impl CadFetcher {
    /// 終わった取得の結果を受け取る
    pub fn poll(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(rx) = &self.rx {
            while let Ok((k, s)) = rx.try_recv() {
                self.state.insert(k, s);
            }
        }
    }

    /// 選んだ品の 1 件だけを裏で取る
    #[cfg(not(target_arch = "wasm32"))]
    fn start(&mut self, ctx: &egui::Context, key: (String, String)) {
        if matches!(self.state.get(&key), Some(CadState::Pending)) {
            return;
        }
        if self.tx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.tx = Some(tx);
            self.rx = Some(rx);
        }
        let tx = self.tx.clone().unwrap();
        self.state.insert(key.clone(), CadState::Pending);
        let ctx = ctx.clone();
        let dir = cad_dir();
        std::thread::spawn(move || {
            let r = lensbench_project::UserCatalog::load(std::path::Path::new(&key.0))
                .map_err(|e| e.to_string())
                .and_then(|cat| {
                    let it = cat
                        .items
                        .get(&key.1)
                        .ok_or_else(|| format!("{} に品 {} がない", key.0, key.1))?;
                    lensbench_cli::cad::fetch_item(&dir, &key.1, it).map(|r| r.to_json())
                });
            let s = match r {
                Ok(v) => CadState::Done(v),
                Err(e) => CadState::Failed(e),
            };
            let _ = tx.send((key, s));
            ctx.request_repaint();
        });
    }

    /// 品の CAD の欄: STEP を取るボタン・結果と、図面（PDF）のリンク。CAD の URL の無い品は何も出さない
    pub fn ui(&mut self, ui: &mut Ui, r: &LensRow) {
        let step = r.raw["cad"]["step"].as_str().unwrap_or("").to_string();
        let pdf = r.raw["cad"]["pdf"].as_str().unwrap_or("").to_string();
        if step.is_empty() && pdf.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if !pdf.is_empty() {
                ui.hyperlink_to(tr("📄 図面（PDF）"), &pdf);
            }
            if step.is_empty() {
                return;
            }
            #[cfg(target_arch = "wasm32")]
            ui.hyperlink_to(tr("STEP（公式）"), &step);
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(key) = key_of(r) {
                let st = self.state.get(&key).cloned();
                match st {
                    Some(CadState::Pending) => {
                        ui.spinner();
                        ui.label(tr("STEP を取っています…"));
                    }
                    _ => {
                        if ui
                            .button(tr("📥 詳しい形（STEP）を取る"))
                            .on_hover_text(format!(
                                "{}\n{step}",
                                tr("この品の STEP を 1 件だけ取り、設定のフォルダにキャッシュします（2 回目からはキャッシュ）")
                            ))
                            .clicked()
                        {
                            self.start(ui.ctx(), key.clone());
                        }
                    }
                }
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(st) = key_of(r).and_then(|k| self.state.get(&k)) {
            match st {
                CadState::Pending => {}
                CadState::Done(v) => {
                    let path = v["path"].as_str().unwrap_or("");
                    let l = ui
                        .label(trf(
                            "STEP: {}{}",
                            &[
                                &path,
                                &if v["cached"] == true {
                                    tr("（キャッシュ）")
                                } else {
                                    ""
                                },
                            ],
                        ))
                        .on_hover_text(tr("右クリックでパスを写す"));
                    l.context_menu(|ui| {
                        if ui.button(tr("パスを写す")).clicked() {
                            ui.ctx().copy_text(path.to_string());
                            ui.close_menu();
                        }
                    });
                    let notes: Vec<&str> = v["notes"]
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    if notes.is_empty() {
                        ui.label(tr("簡易外形と合います（長さ・外径・外形の形）"));
                    }
                    for n in notes {
                        ui.label(
                            RichText::new(trf("簡易外形と違う: {}", &[&n]))
                                .color(ui.visuals().warn_fg_color),
                        );
                    }
                }
                CadState::Failed(e) => {
                    ui.label(
                        RichText::new(trf(
                            "STEP を取れませんでした（簡易外形で描きます）: {}",
                            &[e],
                        ))
                        .color(ui.visuals().warn_fg_color),
                    );
                    if !r.url.is_empty() {
                        ui.hyperlink_to(tr("品ページを開く"), &r.url);
                    }
                }
            }
        }
    }
}
