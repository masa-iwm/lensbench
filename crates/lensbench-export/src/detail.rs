//! 機構部品の詳しい形（品の公式の STEP を三角形にしたもの）。
//!
//! 部品に STEP のファイル（[`lensbench_core::mech::MechPart::step`]）があり、3D の書き出し・表示で詳しい形を選ぶと
//! （[`crate::mesh::Options3D::mech_detail`]・[`crate::step::OptionsStep::mech_detail`] に読み手 [`Details`] を渡す）、
//! 簡易外形の代わりにこの三角形で描く。STEP を読んで三角形にするのは重いので、この crate では行わず、呼ぶ側
//! （CLI・GUI・serve）が lensbench-cad の読み手を渡す（以前はプロセス全体の読み手を 1 回だけ決める形で、
//! 決め忘れると黙って簡易外形になった）。読めない部品は簡易外形のまま描く。
use lensbench_core::math::V3;
use std::sync::Arc;

/// 部品の座標 [mm] の三角形（簡易外形と同じ座標: z は部品の端から光軸の向き）
#[derive(Debug, Clone, Default)]
pub struct Detail {
    pub tris: Vec<[V3; 3]>,
    /// 元の STEP（STEP の書き出しで立体を写す。[`crate::step_embed`]）
    pub step: Option<crate::step_embed::StepSource>,
}

/// 詳しい形の読み手: STEP のファイル → 詳しい形（読めなければ None）
pub trait DetailSource: Send + Sync {
    fn load(&self, path: &str) -> Option<Arc<Detail>>;
}

impl<F: Fn(&str) -> Option<Arc<Detail>> + Send + Sync> DetailSource for F {
    fn load(&self, path: &str) -> Option<Arc<Detail>> {
        self(path)
    }
}

/// 書き出し・表示の設定に持たせる読み手
#[derive(Clone)]
pub struct Details(pub Arc<dyn DetailSource>);

impl Details {
    pub fn new(src: impl DetailSource + 'static) -> Details {
        Details(Arc::new(src))
    }

    /// STEP のファイルの詳しい形（空のパス・読めなければ None）
    pub fn load(&self, path: &str) -> Option<Arc<Detail>> {
        if path.is_empty() {
            return None;
        }
        self.0.load(path)
    }
}

impl std::fmt::Debug for Details {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Details(..)")
    }
}
