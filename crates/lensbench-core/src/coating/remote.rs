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
