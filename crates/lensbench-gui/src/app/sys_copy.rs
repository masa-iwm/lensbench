//! serve の系の写し（文書モデルの 1 段目）。serve から読んだ結果で、GUI はこれを見て描く。
//! 書き換えるのは返事を受けたとき（[`Core::take`]）と、系を開き直す・新規にするときだけ

use super::*;

/// serve の系の写し
#[derive(Default)]
pub(super) struct SysCopy {
    /// load の結果（系の概要）
    pub(super) summary: Option<Value>,
    /// info の結果
    pub(super) info: Option<Value>,
    /// パラメータの今の値
    pub(super) param_vals: HashMap<String, f64>,
    /// 面の表の行・玉と群・ブロック
    pub(super) ld_rows: Vec<lens_data::Row>,
    pub(super) ld_units: Vec<lens_data::Unit>,
    pub(super) ld_blocks: Vec<lens_data::Block>,
    /// 組合せの要素のブロックの id と往路の面の範囲（load の `blocks`。要素の行の範囲）
    pub(super) elem_blocks: Vec<(String, Vec<[usize; 2]>)>,
}
