//! 利用者が選んだ系のファイルを開く。プロジェクトの .json は、組合せを同梱しているか（`gui.combo`）を下書きの
//! Worker に見てもらってから開く（画面のスレッドでファイルを丸ごと読まない）

use super::*;

impl Core {
    /// 利用者が選んだ系のファイルを開く（最近のファイルに足す）。組合せを同梱したプロジェクト（`gui.combo`）は
    /// 組合せに戻して系を作る。同梱していないブロックのプロジェクトも、組合せで書ければ組合せにする
    /// 開く前の文書は元に戻す段に積む（確かめずに替え、元に戻せる）
    pub(super) fn open_doc(&mut self, p: &Path) {
        self.push_recent(&p.display().to_string());
        self.open_doc_unlisted(p);
    }

    /// [`Self::open_doc`] で、最近のファイルに足さない（モデルガラスにした写しなど一時のファイル）
    pub(super) fn open_doc_unlisted(&mut self, p: &Path) {
        self.combo_census = None;
        // 続きは答え（{gui, blocks}。.zmx・系ファイルは組合せにできたら combo）を受けて [`Self::open_doc_peeked`]
        let q = json!({
            "file": p.display().to_string(),
            "glass_dirs": dirs(&self.s.glass_dirs),
            "lens_catalogs": dirs(&self.s.lens_dirs),
        });
        self.preview.send(Tag::OpenDoc, "doc_peek", q);
    }

    /// [`Self::open_doc`] の続き。v は下書きの Worker の `doc_peek` の答え（{gui, blocks}。.zmx・読めない
    /// ファイルは None）
    pub(super) fn open_doc_peeked(&mut self, p: &Path, v: Option<&Value>) {
        // 同梱の組合せか、ブロックから作った組合せ（doc_peek の combo、文書モデル 1 段目）
        let made = |v: &Value| serde_json::from_value(v.get("combo")?.clone()).ok();
        let name = p
            .file_name()
            .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into());
        self.flush_set();
        // モデルガラスの写しを開き直すときは、元のファイルを開いたときの段を使う
        if !std::mem::take(&mut self.reopen_same) || !self.docs.opening {
            self.docs.opening = self.push_doc(trf("Open {}", &[&name]));
        }
        if let Some(mut c) = v.and_then(made) {
            // ブロックから作った組合せに見る状態（ロック・今の状態）を戻し、面の上書きを中身へ入れてから
            // 「保存した形」にする。入れる前を持つと、開いただけで未保存になる
            doc_file::apply_view(&mut c, v.and_then(|v| v.get("gui")));
            c.drop_stale_edits();
            c.bake_edits();
            self.loc.saved_combo = crate::combo_doc::Snap::new(Some(c.clone()));
            if let Some(g) = v.and_then(|v| v.get("gui")) {
                self.set_panels(g);
            }
            self.doc.combo = c;
            // 別の組合せ: 元に戻す段は空から
            self.combo_hist = Default::default();
            self.p.combo_view = combo::View::default();
            // .zmx・系ファイルから作った組合せは元のファイルに上書きしない（保存でプロジェクトの書き先を選ばせる）
            let from_zmx = v.is_some_and(|v| v["zmx"] == json!(true));
            self.loc.doc_path = (!from_zmx).then(|| p.to_path_buf());
            self.loc.doc_from = from_zmx.then(|| p.to_path_buf());
            self.build_combo(self.doc.combo.project());
            return;
        }
        // 読めたが組合せで同じ系を書けないファイルは開かない（理由を出す）。読めないファイルは系として読んで、
        // 誤り（ガラスが無いときの案内など）を出す
        let why = v
            .filter(|v| v["load_error"] != json!(true))
            .and_then(|v| v["why"].as_str());
        if let Some(why) = why {
            self.revert_open();
            self.fail(trf(
                "{} can't be opened (it can't be written as a combination: {})",
                &[&name, &why],
            ));
            return;
        }
        self.loc.doc_path = None;
        self.loc.doc_from = None;
        self.open(p);
    }
}

impl Core {
    /// ブロックのプロジェクト p を、その場で組合せにして組み直す（下書きの Worker を待たない。ツアーの例）。
    /// 文書の段は呼ぶ側で積む
    pub(super) fn open_combo_now(&mut self, p: &Path) -> Result<(), String> {
        let b = lensbench_core::vfs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let v: Value = serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", p.display()))?;
        let base = p.parent().unwrap_or(Path::new(""));
        let mut c = combo::Combo::from_project(&v, base)?;
        c.drop_stale_edits();
        c.bake_edits();
        self.loc.saved_combo = crate::combo_doc::Snap::new(Some(c.clone()));
        self.doc.combo = c;
        self.combo_hist = Default::default();
        self.p.combo_view = combo::View::default();
        self.loc.doc_path = Some(p.to_path_buf());
        self.loc.doc_from = None;
        self.build_combo(self.doc.combo.project());
        Ok(())
    }

    /// 系のファイル（.zmx・プロジェクトの .json）を読む
    pub(super) fn open(&mut self, p: &Path) {
        let file = p.display().to_string();
        // 別のファイルを開いたら、待たせていた組合せの読み直しはやめる（後から組合せで上書きしない）
        self.load_stale &= self.loc.combo_path.as_deref() == Some(p);
        let mut params = self.load_params(p);
        // 開く前の系は serve の段に積む（開くを元に戻せる）。組合せの作り直しは組合せの段で戻すので積まない
        params["history"] = json!(
            if self.is_combo() && self.loc.combo_path.as_deref() == Some(p) {
                "keep"
            } else {
                "push"
            }
        );
        self.worker.send(Tag::Load, "load", params);
        self.info(trf("Reading {}…", &[&file]));
    }

    pub(super) fn load_params(&self, p: &Path) -> Value {
        let mut v = json!({
            "file": p.display().to_string(),
            "glass_dirs": dirs(&self.s.glass_dirs),
            "lens_catalogs": dirs(&self.s.lens_dirs),
        });
        // 組合せの今の状態
        if self.loc.combo_path.as_deref() == Some(p) {
            if let Some(st) = self.doc.combo.active_state() {
                v["state"] = json!(st);
            }
        }
        v
    }
}
