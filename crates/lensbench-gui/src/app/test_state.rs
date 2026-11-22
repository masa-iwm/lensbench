//! 試験用の口: 画面の今の様子（待っている要求・開いている系・解析パネルの結果の有無）を JSON で返す。
//! Web 版の起動確認（scripts/web_smoke.mjs）が固定の時間で待たずに、要求が片付くまで待って結果を確かめる。
//! 画面は canvas なので DOM からは見えない

use super::*;

impl App {
    /// 今の様子。busy は計算と下書きの要求のどちらかを待っている間 true
    pub(crate) fn test_state(&self) -> Value {
        let c = &self.c;
        let name = |p: &Option<PathBuf>| {
            p.as_ref()
                .and_then(|p| p.file_name())
                .map(|x| x.to_string_lossy().into_owned())
        };
        let mut analyses: Vec<Value> = c
            .results
            .iter()
            .map(|(id, r)| {
                json!({
                    "panel": id,
                    "cmd": r.cmd,
                    "ok": r.value.is_some() && r.error.is_none(),
                    "error": r.error,
                    "inflight": r.inflight,
                })
            })
            .collect();
        analyses.sort_by_key(|a| a["panel"].as_u64());
        json!({
            "busy": c.busy() || c.preview_busy(),
            "pending": c.pending() + c.preview.pending(),
            "file": name(&c.loc.file),
            "doc": name(&c.loc.doc_path),
            "system": c.sys.summary.is_some(),
            "sys_gen": c.sys_gen,
            "status": c.status.text,
            "status_error": c.status.error,
            "analyses": analyses,
            "calc_window": self.is_calc_window(),
            // 組合せ（写しを持つ別の窓は元のタブの写し）: 要素の数・前の絞りの間隔・版
            "combo": { "picks": c.doc.combo.picks.len(), "stop_gap": c.doc.combo.stop_gap },
            "copy_ready": c.popup_doc.copies.recv.ready(),
            "sel": c.p.combo_view.sel(),
            "lenses": c.browser.lenses.len(),
            "hist": c.undo_list(true).len(),
        })
    }

    /// このタブは解析のパネルを出した別の窓か（Web 版）
    fn is_calc_window(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        return self.calc.is_some();
        #[cfg(not(target_arch = "wasm32"))]
        false
    }

    /// 試験用: 種類 kind（[`Kind`] の JSON。`"LensData"` など）のパネルを別の窓に出す（無ければ足して出す。出したら true）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn test_detach_panel(&mut self, kind: &str) -> bool {
        let Ok(k) = serde_json::from_str::<Kind>(kind) else {
            return false;
        };
        let same = |x: &Kind| std::mem::discriminant(x) == std::mem::discriminant(&k);
        let id = match self.pane_of(same) {
            Some(id) => id,
            None => self.add_pane(k),
        };
        self.detach(id);
        true
    }

    /// 試験用: 操作（写しを持つ別の窓から元のタブへ送る形の JSON）をパネルから出したように列に入れる（入れたら true）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn test_send(&mut self, op: &str) -> bool {
        match serde_json::from_str::<copy_panel::Fwd>(op) {
            Ok(f) => {
                self.c.take_fwd(0, f);
                true
            }
            Err(_) => false,
        }
    }

    /// 試験用: ドックの最初の解析のパネルを別の窓に出す（出したら true）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn test_detach_analysis(&mut self) -> bool {
        let id = self
            .dock
            .iter_all_tabs()
            .find(|(_, p)| matches!(p.kind, Kind::Analysis(_)))
            .map(|(_, p)| p.id);
        if let Some(id) = id {
            self.detach(id);
        }
        id.is_some()
    }
}
