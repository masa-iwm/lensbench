//! 利用者が選んだ系のファイルを開く。プロジェクトの .json は、組合せを同梱しているか（`gui.combo`）を下書きの
//! Worker に見てもらってから開く（画面のスレッドでファイルを丸ごと読まない、GAR-30）

use super::*;

impl Core {
    /// 利用者が選んだ系のファイルを開く（最近のファイルに足す）。組合せを同梱したプロジェクト（`gui.combo`）は
    /// 組合せに戻して系を作る（#129）。同梱していないブロックのプロジェクトも、組合せで書ければ組合せにする（GUX-11）
    pub(super) fn open_doc(&mut self, p: &Path) {
        self.open_doc_with(p, false);
    }

    /// [`Self::open_doc`]。asked は今の組合せを消してよいか聞いて「はい」だった
    pub(super) fn open_doc_with(&mut self, p: &Path, asked: bool) {
        self.combo_census = None;
        self.push_recent(&p.display().to_string());
        let is_json = p
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
        // 続きは答え（{gui, blocks}。.zmx は組合せにできたら combo）を受けて [`Self::open_doc_peeked`]
        let mut q = json!({ "file": p.display().to_string() });
        if !is_json {
            q["glass_dirs"] = glass_dirs(&self.s.glass_dirs);
            q["lens_catalogs"] = dirs(&self.s.lens_dirs);
        }
        self.preview.send(Tag::OpenDoc(asked), "doc_peek", q);
    }

    /// [`Self::open_doc_with`] の続き。v は下書きの Worker の `doc_peek` の答え（{gui, blocks}。.zmx・読めない
    /// ファイルは None）
    pub(super) fn open_doc_peeked(&mut self, p: &Path, asked: bool, v: Option<&Value>) {
        // 同梱の組合せか、ブロックから作った組合せ（doc_peek の combo、文書モデル 1 段目）
        let made = |v: &Value| serde_json::from_value(v.get("combo")?.clone()).ok();
        if let Some(c) = v.and_then(|v| combo_of_project(v).or_else(|| made(v))) {
            if !self.doc.combo.picks.is_empty()
                && self.doc.combo != c
                && self.loc.built_combo.as_ref() != Some(&self.doc.combo)
                && !asked
            {
                let p = p.to_path_buf();
                self.ask(
                    tr("Expand"),
                    tr("Discard the current combination and open the one in the file?"),
                    move |c, yes| {
                        if yes {
                            c.open_doc_with(&p, true);
                        }
                    },
                );
                return;
            }
            // 前の保存（面の上書きに印が無い）は、開いたときに今の並びの印を付けてから「保存した形」にする。
            // 付ける前を持つと、開いただけで未保存になる
            let mut c = c;
            c.drop_stale_edits();
            self.loc.saved_combo = Some(c.clone());
            if let Some(g) = v.and_then(|v| v.get("gui")) {
                self.set_panels(g);
            }
            self.doc.combo = c;
            // 別の組合せ: 元に戻す段は空から
            self.combo_hist = Default::default();
            self.p.combo_view = combo::View::default();
            // .zmx から作った組合せは .zmx に上書きしない（保存でプロジェクトの書き先を選ばせる）
            let from_zmx = v.is_some_and(|v| v["zmx"] == json!(true));
            self.loc.doc_path = (!from_zmx).then(|| p.to_path_buf());
            self.loc.doc_from = from_zmx.then(|| p.to_path_buf());
            self.build_combo(self.doc.combo.project());
            return;
        }
        // 系ファイル（面の並び）だけはその場に上書きする。ブロックで書いたプロジェクトは、保存すると面の並びに
        // なるので書き先を選ばせる
        self.loc.doc_path = v
            .is_some_and(|v| v["blocks"] != json!(true) && v["zmx"] != json!(true))
            .then(|| p.to_path_buf());
        self.loc.doc_from = None;
        self.open(p);
    }
}

impl Core {
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
            "glass_dirs": glass_dirs(&self.s.glass_dirs),
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
