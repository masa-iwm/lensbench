//! ファイルを選ぶ・確かめる窓。ネイティブは rfd の同期の窓そのまま（Linux は zenity を呼ぶ [`zenity`]）。
//!
//! ブラウザ（wasm）には同期の窓もファイルシステムも無いので:
//! - **開く**（[`open`]・[`open_folder`]）: ブラウザのファイル選択で選んだファイルを
//!   [`lensbench_core::vfs`] の置き場（`/uploads/...`）に入れ、そのパスを次のフレーム以降に返す。
//!   呼ぶ側は毎フレーム同じ `id` で呼ぶ（選び始めるフレームだけ `start` を true に）。
//!   複数選べるので、プロジェクトと参照する .zmx・カタログを一緒に読み込める（同じフォルダに入る）
//! - **保存**（[`FileDialog::save_file`]）: `/downloads/名前` を返し、そこに書かれたファイルを
//!   [`flush_downloads`]（毎フレーム）がブラウザのダウンロードにして置き場から消す
//! - 窓に落としたファイル（[`dropped`]）も `/uploads/` に入れる

use std::path::PathBuf;

/// ファイルを選ぶ窓の設定（ネイティブは rfd の FileDialog に渡す）
#[derive(Default, Clone)]
pub struct FileDialog {
    filters: Vec<(String, Vec<String>)>,
    file_name: Option<String>,
}

impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_filter(mut self, name: impl Into<String>, ext: &[impl ToString]) -> Self {
        self.filters
            .push((name.into(), ext.iter().map(|e| e.to_string()).collect()));
        self
    }
    pub fn set_file_name(mut self, n: impl Into<String>) -> Self {
        self.file_name = Some(n.into());
        self
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "linux")))]
    fn rfd(&self) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        for (n, e) in &self.filters {
            d = d.add_filter(n, e);
        }
        if let Some(f) = &self.file_name {
            d = d.set_file_name(f);
        }
        d
    }

    #[cfg(target_os = "linux")]
    fn rfd(&self) -> zenity::FileDialog<'_> {
        zenity::FileDialog(self)
    }

    /// 保存先を選ぶ（ネイティブ）
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_file(self) -> Option<PathBuf> {
        self.rfd().save_file()
    }

    /// 保存先（wasm）: `/downloads/名前`。書かれたら [`flush_downloads`] がダウンロードにする
    #[cfg(target_arch = "wasm32")]
    pub fn save_file(self) -> Option<PathBuf> {
        let name = self.file_name.clone().unwrap_or_else(|| {
            let ext = self
                .filters
                .first()
                .and_then(|f| f.1.first().cloned())
                .unwrap_or_else(|| "txt".into());
            format!("lensbench.{ext}")
        });
        Some(PathBuf::from(DOWNLOADS).join(file_name_only(&name)))
    }
}

/// 名前のファイル名の部分（パスの区切りを取る）
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn file_name_only(n: &str) -> String {
    n.rsplit(['/', '\\']).next().unwrap_or(n).to_string()
}

/// wasm: ダウンロードにするファイルを書く場所
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const DOWNLOADS: &str = "/downloads";
/// wasm: 読み込んだファイルを置く場所
pub const UPLOADS: &str = "/uploads";

/// ファイルを開く。`start` のフレームで選び始め、選んだファイルを返す（ネイティブはその場で返す。
/// wasm は選び終わった後のフレームで、同じ `id` の呼び出しが返す）
pub fn open(id: &str, start: bool, dialog: impl FnOnce() -> FileDialog) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
        start.then(|| dialog().rfd().pick_file()).flatten()
    }
    #[cfg(target_arch = "wasm32")]
    {
        if start {
            web::pick(id, dialog(), false);
        }
        web::take(id)
    }
}

/// フォルダを選ぶ（wasm は複数のファイルを選んで 1 つのフォルダに入れ、そのフォルダを返す）
pub fn open_folder(id: &str, start: bool, dialog: impl FnOnce() -> FileDialog) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
        start.then(|| dialog().rfd().pick_folder()).flatten()
    }
    #[cfg(target_arch = "wasm32")]
    {
        if start {
            web::pick(id, dialog(), true);
        }
        web::take(id)
    }
}

/// 窓に落としたファイルのパス（wasm は中身を `/uploads/` に入れてそのパス）
pub fn dropped(f: &egui::DroppedFile) -> Option<PathBuf> {
    if let Some(p) = &f.path {
        return Some(p.clone());
    }
    let bytes = f.bytes.as_ref()?;
    let p = PathBuf::from(UPLOADS).join(file_name_only(&f.name));
    lensbench_core::vfs::write(&p, &bytes[..]).ok()?;
    Some(p)
}

/// wasm: `/downloads/` に書かれたファイルをブラウザのダウンロードにする（毎フレーム呼ぶ）。ネイティブは何もしない
pub fn flush_downloads() {
    #[cfg(target_arch = "wasm32")]
    web::flush_downloads();
}

/// 結果が出たときに画面を描き直させる（wasm で、選び終わったときに使う）
pub fn set_ctx(ctx: &egui::Context) {
    #[cfg(target_arch = "wasm32")]
    web::set_ctx(ctx);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = ctx;
}

/// 「はい・いいえ」で確かめる
#[cfg(target_os = "linux")]
pub fn confirm(title: &str, text: &str) -> bool {
    zenity::confirm(title, text)
}

/// 「はい・いいえ」で確かめる
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "linux")))]
pub fn confirm(title: &str, text: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

/// 「はい・いいえ」で確かめる（ブラウザの confirm）
#[cfg(target_arch = "wasm32")]
pub fn confirm(_title: &str, text: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.confirm_with_message(text).ok())
        .unwrap_or(false)
}

/// Linux: rfd の代わりに zenity（GTK の窓）を呼ぶ。zenity が無ければ何も選ばない（「いいえ」）。
/// rfd の xdg-portal（zbus など）をビルドに乗せないため（Linux の GUI は CI 用。公開 #216 BLD-23）
#[cfg(target_os = "linux")]
mod zenity {
    use std::path::PathBuf;
    use std::process::Command;

    fn run(args: &[String]) -> Option<String> {
        match Command::new("zenity").args(args).output() {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout)
                    .trim_end_matches('\n')
                    .to_string();
                (!s.is_empty()).then_some(s)
            }
            Ok(_) => None,
            Err(e) => {
                eprintln!("lensbench-gui: zenity を起こせない（{e}）。ファイルは窓に落として開く");
                None
            }
        }
    }

    pub struct FileDialog<'a>(pub &'a super::FileDialog);

    impl FileDialog<'_> {
        fn args(&self, extra: &[&str]) -> Vec<String> {
            let mut a: Vec<String> = vec!["--file-selection".into()];
            a.extend(extra.iter().map(|s| s.to_string()));
            for (n, e) in &self.0.filters {
                let pats: Vec<String> = e.iter().map(|x| format!("*.{x}")).collect();
                a.push(format!("--file-filter={n} | {}", pats.join(" ")));
            }
            if let Some(f) = &self.0.file_name {
                a.push(format!("--filename={f}"));
            }
            a
        }
        pub fn pick_file(self) -> Option<PathBuf> {
            run(&self.args(&[])).map(PathBuf::from)
        }
        pub fn pick_folder(self) -> Option<PathBuf> {
            run(&self.args(&["--directory"])).map(PathBuf::from)
        }
        pub fn save_file(self) -> Option<PathBuf> {
            run(&self.args(&["--save", "--confirm-overwrite"])).map(PathBuf::from)
        }
    }

    pub fn confirm(title: &str, text: &str) -> bool {
        Command::new("zenity")
            .args([
                "--question",
                &format!("--title={title}"),
                &format!("--text={text}"),
            ])
            .status()
            .is_ok_and(|s| s.success())
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use wasm_bindgen::JsCast;

    thread_local! {
        /// 選び終わったファイル（id → パス）
        static DONE: RefCell<HashMap<String, PathBuf>> = RefCell::new(HashMap::new());
        static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    }

    pub fn set_ctx(ctx: &egui::Context) {
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    }

    pub fn take(id: &str) -> Option<PathBuf> {
        DONE.with(|d| d.borrow_mut().remove(id))
    }

    /// 読み込むたびに別のフォルダにする（同じ名前のファイルを読み直しても前のものと混ざらない。
    /// ブラウザに残した前のフォルダとも重ねない）
    fn next_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(1);
        loop {
            let d = PathBuf::from(UPLOADS).join(N.fetch_add(1, Ordering::Relaxed).to_string());
            if !lensbench_core::vfs::exists(&d) {
                return d;
            }
        }
    }

    pub fn pick(id: &str, d: FileDialog, folder: bool) {
        let id = id.to_string();
        let mut a = rfd::AsyncFileDialog::new();
        // フォルダ（カタログなど）は形式を限らない
        if !folder {
            for (n, e) in &d.filters {
                a = a.add_filter(n, e);
            }
        }
        let exts: Vec<String> = d.filters.iter().flat_map(|f| f.1.clone()).collect();
        wasm_bindgen_futures::spawn_local(async move {
            let Some(files) = a.pick_files().await else {
                return;
            };
            let dir = next_dir();
            let mut first: Option<(usize, PathBuf)> = None;
            for f in files {
                let name = file_name_only(&f.file_name());
                let p = dir.join(&name);
                if lensbench_core::vfs::write(&p, f.read().await).is_err() {
                    continue;
                }
                // 開くのは、形式の並びで先に来る拡張子のファイル（無ければ最初のファイル）
                let rank = exts
                    .iter()
                    .position(|e| {
                        name.to_ascii_lowercase()
                            .ends_with(&format!(".{}", e.to_ascii_lowercase()))
                    })
                    .unwrap_or(exts.len());
                if first.as_ref().map_or(true, |(r, _)| rank < *r) {
                    first = Some((rank, p));
                }
            }
            let out = if folder {
                Some(dir)
            } else {
                first.map(|x| x.1)
            };
            if let Some(p) = out {
                DONE.with(|d| d.borrow_mut().insert(id, p));
            }
            CTX.with(|c| {
                if let Some(c) = &*c.borrow() {
                    c.request_repaint();
                }
            });
        });
    }

    pub fn flush_downloads() {
        let Ok(files) = lensbench_core::vfs::read_dir(DOWNLOADS) else {
            return;
        };
        for p in files {
            // 書きかけ（別の名前に書いてから置き換える）は待つ
            if p.extension().is_some_and(|e| e == "tmp") || !lensbench_core::vfs::is_file(&p) {
                continue;
            }
            let Ok(b) = lensbench_core::vfs::read(&p) else {
                continue;
            };
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "lensbench".into());
            download(&name, &b);
            let _ = lensbench_core::vfs::remove_file(&p);
        }
    }

    /// バイト列をブラウザのダウンロードにする（Blob の URL を a 要素で開く）
    fn download(name: &str, b: &[u8]) {
        let run = || -> Option<()> {
            let arr = js_sys::Uint8Array::from(b);
            let parts = js_sys::Array::of1(&arr);
            let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).ok()?;
            let url = web_sys::Url::create_object_url_with_blob(&blob).ok()?;
            let doc = web_sys::window()?.document()?;
            let a: web_sys::HtmlAnchorElement = doc.create_element("a").ok()?.dyn_into().ok()?;
            a.set_href(&url);
            a.set_download(name);
            a.click();
            // URL はすぐ消すとダウンロードが始まらないブラウザがあるので残す（ページを閉じれば消える）
            Some(())
        };
        let _ = run();
    }
}
