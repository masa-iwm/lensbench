//! 機構部品の詳しい形（品の公式の STEP を三角形にしたもの、#121）。
//!
//! 部品に STEP のファイル（[`lensbench_core::mech::MechPart::step`]）があり、3D の書き出し・表示で詳しい形を選ぶと
//! （[`crate::mesh::Options3D::mech_detail`]）、簡易外形の代わりにこの三角形で描く。STEP を読んで三角形にするのは
//! 重いので、この crate では行わず、呼ぶ側（CLI・GUI）が [`set_loader`] で読み手を渡す。読み手が無いか読めなければ
//! 簡易外形のまま描く。
use lensbench_core::math::V3;
use std::sync::{Arc, OnceLock};

/// 部品の座標 [mm] の三角形（簡易外形と同じ座標: z は部品の端から光軸の向き）
#[derive(Debug, Clone, Default)]
pub struct Detail {
    pub tris: Vec<[V3; 3]>,
    /// 元の STEP（STEP の書き出しで立体を写す。[`crate::step_embed`]）
    pub step: Option<crate::step_embed::StepSource>,
}

/// STEP のファイル → 詳しい形（読めなければ None）
pub type Loader = fn(&str) -> Option<Arc<Detail>>;

static LOADER: OnceLock<Loader> = OnceLock::new();

/// 読み手を決める（起動時に 1 回）
pub fn set_loader(f: Loader) {
    let _ = LOADER.set(f);
}

/// STEP のファイルの詳しい形（読み手が無いか読めなければ None）
pub fn load(path: &str) -> Option<Arc<Detail>> {
    if path.is_empty() {
        return None;
    }
    LOADER.get().and_then(|f| f(path))
}
