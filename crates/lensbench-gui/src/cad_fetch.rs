//! カタログの品の詳しい形（公式の STEP）を 1 件だけ取る（公開 #120）。
//!
//! 品の一覧で選んだ品に `cad.step` があれば「詳しい形（STEP）を取る」ボタンを出し、押したときにその 1 ファイルだけを
//! 裏のスレッドで取る（serve の `cad_step` と同じ [`lensbench_cli::cad`]）。キャッシュは設定のフォルダの `cad`。
//! 取れたら簡易外形との食い違いを示し、取れなければ品ページのリンクを出す（表示は簡易外形のまま）。
//! Web 版は配布元からは取れない（CORS）。共有の置き場（[`set_web_store`]、同じサイトの口）があればそこから取り、
//! 置き場（vfs）の `/cad` に入れる（計算の Web Worker にも写り、詳しい形の描画に使う。#158）。

use egui::{RichText, Ui};
use serde_json::Value;
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
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

/// Web 版の共有の置き場の URL（例 `/cad`。別の入口が [`crate::set_web_cad_store`] で決める）
#[cfg(target_arch = "wasm32")]
static WEB_STORE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[cfg(target_arch = "wasm32")]
pub fn set_web_store(url: Option<String>) {
    *WEB_STORE.lock().unwrap_or_else(|e| e.into_inner()) = url.filter(|u| !u.trim().is_empty());
}

#[cfg(target_arch = "wasm32")]
fn web_store() -> Option<String> {
    WEB_STORE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Web 版: 置き場から品の STEP を取って vfs に入れ、serve の `cad_step` と同じ形の結果にする
#[cfg(target_arch = "wasm32")]
async fn web_fetch(store: String, catalog: String, name: String) -> Result<Value, String> {
    use js_sys::{Function, Promise, Reflect};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    let cat = lensbench_project::UserCatalog::load(std::path::Path::new(&catalog))?;
    let it = cat
        .items
        .get(&name)
        .ok_or_else(|| format!("{catalog} に品 {name} がない"))?;
    use lensbench_project::catalog_fill;
    let found = catalog_fill::cad_cache_find(&catalog_fill::cad_cache_dir(), &name, &it.cad.step);
    let cached = found.is_some();
    let path = found.unwrap_or_else(|| {
        catalog_fill::cad_cache_path(&catalog_fill::cad_cache_dir(), &name, &it.cad.step)
    });
    let text = if cached {
        lensbench_cli::cad_detail::gunzip(
            lensbench_core::vfs::read(&path).map_err(|e| e.to_string())?,
            512 << 20,
        )?
    } else {
        // 置き場の鍵はキャッシュのファイル名から .gz を除いたもの
        let key = path
            .file_name()
            .map(|f| f.to_string_lossy().trim_end_matches(".gz").to_string())
            .unwrap_or_default();
        let url = format!("{}/{}", store.trim_end_matches('/'), key);
        let js = |e: wasm_bindgen::JsValue| format!("置き場を読めない（{url}）: {e:?}");
        let g = js_sys::global();
        let f: Function = Reflect::get(&g, &"fetch".into())
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
        let p: Promise = f
            .call1(&g, &url.as_str().into())
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
        let r = JsFuture::from(p).await.map_err(js)?;
        let st = Reflect::get(&r, &"status".into())
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as u32;
        if st == 404 {
            return Err(tr(
                "置き場にこの品の STEP がありません（アプリ版で取ると置き場に上がります）",
            )
            .to_string());
        }
        if st != 200 {
            return Err(format!("置き場を読めない（{st}）: {url}"));
        }
        let ab: Function = Reflect::get(&r, &"arrayBuffer".into())
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
        let p: Promise = ab.call0(&r).map_err(js)?.dyn_into().map_err(js)?;
        let buf = JsFuture::from(p).await.map_err(js)?;
        // 置き場の STEP は gzip（展開してから STEP か確かめる）
        let b =
            lensbench_cli::cad_detail::gunzip(js_sys::Uint8Array::new(&buf).to_vec(), 512 << 20)
                .map_err(|e| format!("{e}: {url}"))?;
        let head = String::from_utf8_lossy(&b[..b.len().min(256)]).to_ascii_uppercase();
        if !head
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with("ISO-10303-21")
        {
            return Err(format!("置き場の中身が STEP でない: {url}"));
        }
        lensbench_core::vfs::write(&path, lensbench_cli::cad_detail::gzip(&b))
            .map_err(|e| e.to_string())?;
        b
    };
    let (measured, notes) = lensbench_cli::cad_detail::measure(it, &text);
    Ok(serde_json::json!({
        "item": name,
        "url": it.cad.step,
        "path": path.display().to_string(),
        "cached": cached,
        "source": if cached { "cache" } else { "store" },
        "store_notes": [],
        "measured": measured.as_ref().map(|m| serde_json::to_value(m).unwrap_or_default()),
        "notes": notes,
    }))
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
    rx: Option<std::sync::mpsc::Receiver<((String, String), CadState)>>,
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
        if let Some(rx) = &self.rx {
            while let Ok((k, s)) = rx.try_recv() {
                self.state.insert(k, s);
            }
        }
    }

    /// 選んだ品の 1 件だけを裏で取る（Web 版は置き場から非同期に）
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
        let done = move |key, r: Result<Value, String>| {
            let s = match r {
                Ok(v) => CadState::Done(v),
                Err(e) => CadState::Failed(e),
            };
            let _ = tx.send((key, s));
            ctx.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
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
                done(key, r);
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let Some(store) = web_store() else {
                done(key, Err(tr("置き場がありません").to_string()));
                return;
            };
            wasm_bindgen_futures::spawn_local(async move {
                let r = web_fetch(store, key.0.clone(), key.1.clone()).await;
                done(key, r);
            });
        }
    }

    /// 品の CAD の欄: STEP を取るボタン・結果と、図面（PDF）のリンク。CAD の URL の無い品は何も出さない
    pub fn ui(&mut self, ui: &mut Ui, r: &LensRow) {
        let step = r.raw["cad"]["step"].as_str().unwrap_or("").to_string();
        let pdf = r.raw["cad"]["pdf"].as_str().unwrap_or("").to_string();
        // STEP の URL の無い機構部品も、共有の置き場があれば置き場から取れる
        #[cfg(not(target_arch = "wasm32"))]
        let from_store =
            step.is_empty() && r.raw.get("mech").is_some() && lensbench_cli::cad::store().is_some();
        #[cfg(target_arch = "wasm32")]
        let from_store = web_store().is_some() && r.raw.get("mech").is_some();
        if step.is_empty() && pdf.is_empty() && !from_store {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if !pdf.is_empty() {
                ui.hyperlink_to(tr("📄 図面（PDF）"), &pdf);
            }
            if step.is_empty() && !from_store {
                return;
            }
            #[cfg(target_arch = "wasm32")]
            if !step.is_empty() {
                ui.hyperlink_to(tr("STEP（公式）"), &step);
            }
            if !from_store && cfg!(target_arch = "wasm32") {
                return;
            }
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
                                tr("この品の STEP を 1 件だけ取り、設定のフォルダにキャッシュします（2 回目からはキャッシュ）。置き場があれば先に置き場を見て、配布元から取ったら置き場にも上げます")
                            ))
                            .clicked()
                        {
                            self.start(ui.ctx(), key.clone());
                        }
                    }
                }
            }
        });
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
                                &match v["source"].as_str() {
                                    Some("cache") => tr("（キャッシュ）"),
                                    Some("store") => tr("（置き場から）"),
                                    Some("vendor_uploaded") => tr("（置き場にも上げました）"),
                                    _ => "",
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
                    for n in v["store_notes"].as_array().into_iter().flatten() {
                        ui.label(
                            RichText::new(trf("置き場: {}", &[&n.as_str().unwrap_or("")]))
                                .color(ui.visuals().warn_fg_color),
                        );
                    }
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
