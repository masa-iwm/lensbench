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
    /// 同じブロックの復路の面の範囲（load の `blocks` の back。復路の行の値を往路の面へ書く）
    pub(super) elem_back: Vec<(String, Vec<[usize; 2]>)>,
    /// 選んだ機構部品の札に出す mech-check の結果: 頼んだときの系の世代と、結果（None は待っている）
    pub(super) mech_check: Option<(u64, Option<std::sync::Arc<Value>>)>,
}

impl SysCopy {
    /// mech-check の返事（誤りなら {"error": …}）を受ける
    pub(super) fn took_mech_check(&mut self, v: Value) {
        if let Some((_, r)) = &mut self.mech_check {
            *r = Some(std::sync::Arc::new(v));
        }
    }
}

impl Core {
    /// 機構部品を選んでいる間、今の系の mech-check を（無ければ）頼み、結果を図の札へ渡す（[`crate::combo::mech::check`]）
    pub(super) fn mech_check_tick(&mut self, ctx: &egui::Context) {
        let want = crate::combo::mech::selected(ctx).is_some()
            && !self.doc.combo.mechanics.is_empty()
            && self.sys.summary.is_some();
        if !want {
            return;
        }
        if self
            .sys
            .mech_check
            .as_ref()
            .is_none_or(|(g, _)| *g != self.sys_gen)
        {
            self.sys.mech_check = Some((self.sys_gen, None));
            self.calc(Tag::MechCheck, serde_json::json!({ "cmd": "mech-check" }));
        }
        let r = self.sys.mech_check.as_ref().and_then(|(_, r)| r.clone());
        crate::combo::mech::put_check(ctx, r);
    }
}
