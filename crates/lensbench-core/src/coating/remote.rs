//! フォルダに無い膜のモデルを取る口（拡張の表の欄）

use std::path::{Path, PathBuf};

/// フォルダに無い膜のモデルを取る口（膜ファイルのフォルダ、膜の名前）→ 取ったファイル（`.lbcb` など）
pub type RemoteLoader = fn(&Path, &str) -> Option<PathBuf>;

/// 拡張の表（[`crate::ext::Ext`]）の欄: フォルダに無い膜のモデルを取る口
#[derive(Clone, Copy, Default)]
pub struct RemoteCoatings(pub Option<RemoteLoader>);

/// [`super::Coatings::load_dir_for`] がフォルダで膜のモデル・実測表を見つけられなかったときに呼ぶ口を、プロセスの既定の
/// 拡張の表に決める（CLI・GUI の起動時。共有の置き場から取る、lensbench_cad::coat_remote）
pub fn set_remote_loader(f: RemoteLoader) {
    crate::ext::update_default(|e| e.slot_mut::<RemoteCoatings>().0 = Some(f));
}

pub(in crate::coating) fn remote_loader() -> Option<RemoteLoader> {
    crate::ext::current()
        .slot::<RemoteCoatings>()
        .and_then(|r| r.0)
}

/// 膜ファイルのフォルダから共有の置き場で取れる膜の名前の一覧を返す口（膜の欄の候補。取らずに目録だけ読む）
pub type RemoteLister = fn(&Path) -> Vec<String>;

/// 拡張の表の欄: 共有の置き場で取れる膜の名前の一覧を返す口
#[derive(Clone, Copy, Default)]
pub struct RemoteCoatingNames(pub Option<RemoteLister>);

/// [`super::coat_choices`] が膜ファイルのフォルダの共有の置き場の膜を候補に出す口を、プロセスの既定の拡張の表に
/// 決める（[`set_remote_loader`] と一緒に。lensbench_cad::coat_remote）
pub fn set_remote_lister(f: RemoteLister) {
    crate::ext::update_default(|e| e.slot_mut::<RemoteCoatingNames>().0 = Some(f));
}

pub(in crate::coating) fn remote_lister() -> Option<RemoteLister> {
    crate::ext::current()
        .slot::<RemoteCoatingNames>()
        .and_then(|r| r.0)
}
