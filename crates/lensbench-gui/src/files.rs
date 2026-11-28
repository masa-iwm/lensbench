//! ファイルを選ぶ・確かめる窓。ネイティブは rfd（Linux は zenity を呼ぶ [`zenity`]）の窓を別のスレッドで出し、
//! 選んだパスを後のフレームで返す（画面を描いている途中で OS の窓のメッセージループを回さない）。
//! 呼ぶ側はネイティブもブラウザも同じ: 毎フレーム同じ `id` で [`open`]・[`open_folder`]・[`save`] を呼び、
//! 選び始めるフレームだけ `start` を true にする。
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
#[derive(Default, Clone, Debug, PartialEq)]
pub(crate) struct FileDialog {
    filters: Vec<(String, Vec<String>)>,
    file_name: Option<String>,
    /// 始めのフォルダ（開いているファイルのフォルダなど。無ければ OS が決める）
    directory: Option<PathBuf>,
}

impl FileDialog {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn add_filter(mut self, name: impl Into<String>, ext: &[impl ToString]) -> Self {
        self.filters
            .push((name.into(), ext.iter().map(|e| e.to_string()).collect()));
        self
    }
    pub(crate) fn set_file_name(mut self, n: impl Into<String>) -> Self {
        self.file_name = Some(n.into());
        self
    }
    /// 始めのフォルダ（もう決めてあれば変えない）
    pub(crate) fn or_directory(mut self, dir: Option<PathBuf>) -> Self {
        if self.directory.is_none() {
            self.directory = dir.filter(|d| d.is_dir());
        }
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
        if let Some(dir) = &self.directory {
            d = d.set_directory(dir);
        }
        // 持ち主を本体の窓にする（本体をクリックしても窓が後ろに回らない）
        #[cfg(windows)]
        if let Some(o) = native::owner() {
            d = d.set_parent(&o);
        }
        d
    }

    #[cfg(target_os = "linux")]
    fn rfd(&self) -> zenity::FileDialog<'_> {
        zenity::FileDialog(self)
    }

    /// 保存先（wasm）: `/downloads/名前`。書かれたら [`flush_downloads`] がダウンロードにする
    #[cfg(target_arch = "wasm32")]
    fn save_file(self) -> Option<PathBuf> {
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
pub(crate) const DOWNLOADS: &str = "/downloads";
/// wasm: 読み込んだファイルを置く場所
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) const UPLOADS: &str = "/uploads";

/// ファイルを開く。`start` のフレームで選び始め、選んだファイルを返す（ネイティブはその場で返す。
/// wasm は選び終わった後のフレームで、同じ `id` の呼び出しが返す）
pub(crate) fn open(id: &str, start: bool, dialog: impl FnOnce() -> FileDialog) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if start {
            native::spawn(id, dialog(), |d| d.rfd().pick_file());
        }
        native::take(id).flatten()
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
pub(crate) fn open_folder(
    id: &str,
    start: bool,
    dialog: impl FnOnce() -> FileDialog,
) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if start {
            native::spawn(id, dialog(), |d| d.rfd().pick_folder());
        }
        native::take(id).flatten()
    }
    #[cfg(target_arch = "wasm32")]
    {
        if start {
            web::pick(id, dialog(), true);
        }
        web::take(id)
    }
}

/// 保存先を選ぶ。`start` のフレームで選び始め、選んだパスを返す（ネイティブは選び終わった後のフレームで、同じ `id` の
/// 呼び出しが返す。wasm はその場で `/downloads/名前` を返し、書かれたら [`flush_downloads`] がダウンロードにする）
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn save(id: &str, start: bool, dialog: impl FnOnce() -> FileDialog) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if start {
            native::spawn(id, dialog(), |d| d.rfd().save_file());
        }
        native::take(id).flatten()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = id;
        start.then(|| dialog().save_file()).flatten()
    }
}

/// ファイルの窓の種類
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogKind {
    /// ファイルを開く（答えは同じ id の [`open`] が返す）
    Open,
    /// 保存先を選ぶ（答えは [`finished`] で受ける）
    Save,
}

/// ファイルの窓を出し始める（答えは [`DialogKind`] の所で受ける）
pub(crate) fn start(id: &str, kind: DialogKind, d: FileDialog) {
    match kind {
        DialogKind::Save => start_save(id, d),
        DialogKind::Open => {
            #[cfg(not(target_arch = "wasm32"))]
            native::spawn(id, d, |d| d.rfd().pick_file());
            #[cfg(target_arch = "wasm32")]
            web::pick(id, d, false);
        }
    }
}

/// 保存先を選び始める（答えは [`finished`] で受ける。毎フレーム同じ所で呼べない所から使う）
pub(crate) fn start_save(id: &str, d: FileDialog) {
    #[cfg(not(target_arch = "wasm32"))]
    native::spawn(id, d, |d| d.rfd().save_file());
    #[cfg(target_arch = "wasm32")]
    SAVED.with(|m| m.borrow_mut().insert(id.to_string(), d.save_file()));
}

/// [`start_save`] の答え: 選び終わっていなければ None、選んだら Some(パス)、やめたら Some(None)
pub(crate) fn finished(id: &str) -> Option<Option<PathBuf>> {
    #[cfg(not(target_arch = "wasm32"))]
    return native::take(id);
    #[cfg(target_arch = "wasm32")]
    SAVED.with(|m| m.borrow_mut().remove(id))
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// wasm: [`start_save`] の答え（その場で決まる）
    static SAVED: std::cell::RefCell<std::collections::HashMap<String, Option<PathBuf>>> =
        Default::default();
}

/// ネイティブ: 窓を別のスレッドで出し、選んだパスを id ごとに置く
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::FileDialog;
    use lensbench_core::LockExt;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// 選び終わった窓の答え（id → 選んだパス、やめたら None）と、窓を出している id
    static DONE: Mutex<Option<HashMap<String, Option<PathBuf>>>> = Mutex::new(None);
    static OPEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    static CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

    pub(super) fn set_ctx(ctx: &egui::Context) {
        *CTX.lock_ok() = Some(ctx.clone());
    }

    /// Windows: 本体の窓（HWND）。ファイルの窓の持ち主にする
    #[cfg(windows)]
    static OWNER: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

    /// 本体の窓を覚える（Windows だけ。ほかは何もしない）
    pub(crate) fn set_owner(cc: &eframe::CreationContext<'_>) {
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = cc.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    OWNER.store(w.hwnd.get(), std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        #[cfg(not(windows))]
        let _ = cc;
    }

    #[cfg(windows)]
    pub(crate) fn owner() -> Option<Owner> {
        std::num::NonZeroIsize::new(OWNER.load(std::sync::atomic::Ordering::Relaxed)).map(Owner)
    }

    /// rfd の set_parent に渡す本体の窓
    #[cfg(windows)]
    pub(crate) struct Owner(std::num::NonZeroIsize);

    #[cfg(windows)]
    impl raw_window_handle::HasWindowHandle for Owner {
        fn window_handle(
            &self,
        ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
            let h = raw_window_handle::Win32WindowHandle::new(self.0);
            // 本体の窓はアプリが終わるまで生きている
            Ok(unsafe {
                raw_window_handle::WindowHandle::borrow_raw(
                    raw_window_handle::RawWindowHandle::Win32(h),
                )
            })
        }
    }

    #[cfg(windows)]
    impl raw_window_handle::HasDisplayHandle for Owner {
        fn display_handle(
            &self,
        ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
            Ok(raw_window_handle::DisplayHandle::windows())
        }
    }

    /// 窓を出す（同じ id の窓が出ている間は出さない）
    pub(super) fn spawn(
        id: &str,
        d: FileDialog,
        f: impl FnOnce(&FileDialog) -> Option<PathBuf> + Send + 'static,
    ) {
        if !OPEN
            .lock_ok()
            .get_or_insert_with(HashSet::new)
            .insert(id.to_string())
        {
            return;
        }
        let id = id.to_string();
        // 試験: 答えが置いてあれば窓を出さずにそれを返す（[`super::answer_next`]）
        #[cfg(test)]
        if let Some(p) = super::ANSWER.lock_ok().take() {
            DONE.lock_ok()
                .get_or_insert_with(HashMap::new)
                .insert(id.clone(), p);
            if let Some(o) = OPEN.lock_ok().as_mut() {
                o.remove(&id);
            }
            return;
        }
        std::thread::spawn(move || {
            let p = f(&d);
            DONE.lock_ok()
                .get_or_insert_with(HashMap::new)
                .insert(id.clone(), p);
            if let Some(o) = OPEN.lock_ok().as_mut() {
                o.remove(&id);
            }
            if let Some(c) = &*CTX.lock_ok() {
                c.request_repaint();
            }
        });
    }

    /// 選び終わっていれば Some（やめたら Some(None)）
    pub(super) fn take(id: &str) -> Option<Option<PathBuf>> {
        DONE.lock_ok().as_mut().and_then(|d| d.remove(id))
    }
}

/// 試験: 次に出すファイルの窓の答え（選んだパス、None はやめた）
#[cfg(all(test, not(target_arch = "wasm32")))]
static ANSWER: std::sync::Mutex<Option<Option<PathBuf>>> = std::sync::Mutex::new(None);

/// 試験: 次に出すファイルの窓（開く・保存・フォルダ）を出さずに、答え p を返させる
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn answer_next(p: Option<PathBuf>) {
    use lensbench_core::LockExt;
    *ANSWER.lock_ok() = Some(p);
}

/// ファイルを選ぶ窓が出せなかった（やめたのとは別）。[`take_failed`] で読んで戻す
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
static FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 前に呼んでから、ファイルを選ぶ窓が出せなかったことがあったか（読むと戻る）
pub(crate) fn take_failed() -> bool {
    FAILED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// 窓に落としたファイルのパス。wasm はブラウザが中身を後から渡すので、ここでは読み始めるだけで None を返し、
/// `/uploads/` に書けたら [`take_dropped`] で返す（egui 0.37 から落としたファイルの中身は非同期に読む）
pub(crate) fn dropped(f: &egui::DroppedFileHandle) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(f.path().to_path_buf())
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::read_dropped(f.clone());
        None
    }
}

/// wasm: 窓に落としたファイルを `/uploads/` に書き終えたら、そのパス（一度だけ）。ネイティブは常に None
pub(crate) fn take_dropped() -> Option<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        web::take(web::DROPPED)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

/// wasm: `/downloads/` に書かれたファイルをブラウザのダウンロードにする（毎フレーム呼ぶ）。ネイティブは何もしない
pub(crate) fn flush_downloads() {
    #[cfg(target_arch = "wasm32")]
    web::flush_downloads();
}

/// 結果が出たときに画面を描き直させる（選び終わったときに使う）
pub(crate) fn set_ctx(ctx: &egui::Context) {
    #[cfg(target_arch = "wasm32")]
    web::set_ctx(ctx);
    #[cfg(not(target_arch = "wasm32"))]
    native::set_ctx(ctx);
}

/// 本体の窓を覚える（Windows はファイルの窓の持ち主にする）
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::set_owner;

/// 人の答えを待たない運転（画面の通し確認 `--tour`）。確かめる窓は出さずに「いいえ」（CI の Linux で
/// zenity の無いときと同じ答え。巡回の段はこの答えで組んである）、エラーの窓は出さない
static UNATTENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 人の答えを待たない運転にする（確かめ（`Core::ask`）は「いいえ」、[`error_box`] は出さない）
pub(crate) fn set_unattended(on: bool) {
    UNATTENDED.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn unattended() -> bool {
    UNATTENDED.load(std::sync::atomic::Ordering::Relaxed)
}

/// エラーを窓で知らせる（Linux は zenity、無ければ何もしない）
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn error_box(title: &str, text: &str) {
    if unattended() {
        return;
    }
    #[cfg(target_os = "linux")]
    zenity::error(title, text);
    #[cfg(not(target_os = "linux"))]
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(text)
        .show();
}

/// Linux: rfd の代わりに zenity（GTK の窓）を呼ぶ。zenity が無ければ何も選ばない（「いいえ」）。
/// rfd の xdg-portal（zbus など）をビルドに乗せないため（Linux の GUI は CI 用）
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
            // 1 はやめた（キャンセル）。それ以外は窓が出せなかった（画面が無いなど）
            Ok(o) => {
                if o.status.code() != Some(1) {
                    super::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                None
            }
            Err(e) => {
                eprintln!("lensbench-gui: cannot start zenity ({e}); drop files on the window to open them");
                super::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
                None
            }
        }
    }

    pub(super) struct FileDialog<'a>(pub &'a super::FileDialog);

    impl FileDialog<'_> {
        fn args(&self, extra: &[&str]) -> Vec<String> {
            let mut a: Vec<String> = vec!["--file-selection".into()];
            a.extend(extra.iter().map(|s| s.to_string()));
            for (n, e) in &self.0.filters {
                let pats: Vec<String> = e.iter().map(|x| format!("*.{x}")).collect();
                a.push(format!("--file-filter={n} | {}", pats.join(" ")));
            }
            let dir = self
                .0
                .directory
                .as_ref()
                .map(|d| format!("{}/", d.display()));
            if let Some(f) = &self.0.file_name {
                a.push(format!("--filename={}{f}", dir.unwrap_or_default()));
            } else if let Some(d) = dir {
                a.push(format!("--filename={d}"));
            }
            a
        }
        pub(super) fn pick_file(self) -> Option<PathBuf> {
            run(&self.args(&[])).map(PathBuf::from)
        }
        pub(super) fn pick_folder(self) -> Option<PathBuf> {
            run(&self.args(&["--directory"])).map(PathBuf::from)
        }
        pub(super) fn save_file(self) -> Option<PathBuf> {
            run(&self.args(&["--save", "--confirm-overwrite"])).map(PathBuf::from)
        }
    }

    pub(super) fn error(title: &str, text: &str) {
        let _ = Command::new("zenity")
            .args([
                "--error",
                "--no-markup",
                &format!("--title={title}"),
                &format!("--text={text}"),
            ])
            .status();
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

    pub(super) fn set_ctx(ctx: &egui::Context) {
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    }

    pub(super) fn take(id: &str) -> Option<PathBuf> {
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

    pub(super) fn pick(id: &str, d: FileDialog, folder: bool) {
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
                if first.as_ref().is_none_or(|(r, _)| rank < *r) {
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

    /// 落としたファイルを選び終わったファイルと同じ表に入れるときの id
    pub(super) const DROPPED: &str = "lensbench-dropped";

    pub(super) fn read_dropped(f: egui::DroppedFileHandle) {
        wasm_bindgen_futures::spawn_local(async move {
            let Ok(bytes) = f.bytes_async().await else {
                return;
            };
            let name = file_name_only(&f.path().to_string_lossy());
            let p = next_dir().join(name);
            if lensbench_core::vfs::write(&p, bytes).is_ok() {
                DONE.with(|d| d.borrow_mut().insert(DROPPED.to_string(), p));
            }
            CTX.with(|c| {
                if let Some(c) = &*c.borrow() {
                    c.request_repaint();
                }
            });
        });
    }

    pub(super) fn flush_downloads() {
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
