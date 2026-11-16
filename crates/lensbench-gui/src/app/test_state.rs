//! 試験用の口: 画面の今の様子（待っている要求・開いている系・解析パネルの結果の有無）を JSON で返す。
//! Web 版の起動確認（scripts/web_smoke.mjs）が固定の時間で待たずに、要求が片付くまで待って結果を確かめる（GAR-25）。
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
            "busy": c.worker.busy() || c.preview.busy(),
            "pending": c.worker.pending() + c.preview.pending(),
            "file": name(&c.loc.file),
            "doc": name(&c.loc.doc_path),
            "system": c.sys.summary.is_some(),
            "sys_gen": c.sys_gen,
            "status": c.status.text,
            "status_error": c.status.error,
            "analyses": analyses,
        })
    }
}
