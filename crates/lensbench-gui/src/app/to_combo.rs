//! 開いた系を組合せにする・開いた系にカタログの要素を差し込む（app.rs から分けた）

use super::*;

impl Core {
    /// 開いた系（組合せでない系）にカタログの要素を差し込む: 開いた系を組合せ（系の取り込み）にし、面 `after` の後の
    /// 空気の間隔（None は系の後ろ）に置く。今の組合せに要素があれば置き換えてよいか聞く。差し込むことにしたかを
    /// done に渡す（聞いたときは答えの後）
    pub(super) fn insert_into_open(
        &mut self,
        after: Option<usize>,
        pick: combo::Pick,
        n_old: usize,
        done: impl FnOnce(&mut Core, bool) + 'static,
    ) {
        let label = pick.label();
        let go = move |c: &mut Core, yes: bool| {
            if yes {
                c.open_insert = Some((after, pick));
                c.to_combo = Some(false);
            }
            done(c, yes);
        };
        if n_old == 0 {
            go(self, true);
            return;
        }
        let text = trf(
            "開いた系を組合せにして {} を足します。今の組合せ（{} 個の要素）は置き換わります。よいですか？",
            &[&label, &n_old],
        );
        self.ask(tr("開いた系に足す"), &text, go);
    }

    /// 開いた系を組合せにする: 系の取り込み（zmx ブロック、.zmx も lensbench の .json も）1 つと、系の設定。keep は今の
    /// 組合せの要素をその後ろに残す。系の設定がまだ無ければ読みに行き、届いた後の画面でもう一度呼ぶ（#135）
    pub(super) fn open_to_combo(&mut self, keep: bool) {
        self.open_to_combo_with(keep, false);
    }

    /// [`Self::open_to_combo`]。asked は保存していない変更を捨ててよいか聞いて「はい」だった
    pub(super) fn open_to_combo_with(&mut self, keep: bool, asked: bool) {
        let Some(f) = self.file.clone().filter(|_| !self.is_combo()) else {
            self.to_combo = None;
            self.open_insert = None;
            return;
        };
        let Some(d) = self.oss.data.clone() else {
            if self.oss.want() {
                self.worker.send(Tag::SysData, "system_data", json!({}));
            }
            return;
        };
        self.to_combo = None;
        if self.ld_dirty && !asked {
            self.ask(
                tr("組合せにする"),
                tr("開いた系の保存していない変更は組合せに入りません（ファイルから取り込みます）。続けますか？"),
                move |c, yes| {
                    if yes {
                        c.open_to_combo_with(keep, true);
                        return;
                    }
                    if keep {
                        // 足した要素は残し、開いた系はそのまま
                        c.built_combo = Some(c.doc.combo.clone());
                    }
                    c.open_insert = None;
                },
            );
            return;
        }
        let obj_t = self.ld_rows.first().map_or(f64::INFINITY, |r| r.t);
        let mut c = Combo {
            name: f
                .file_stem()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ..Default::default()
        };
        if let Err(e) = c.take_system_data(&Value::Object(d), obj_t) {
            self.fail(e);
            if keep {
                self.built_combo = Some(self.doc.combo.clone());
            }
            self.open_insert = None;
            return;
        }
        let mut z = combo::Pick::new(combo::ElemKind::ZmxRange);
        // 組合せは保存先から相対に読むので、開いた系は絶対パスで持つ（Web 版の置き場は初めから絶対）
        z.path = std::path::absolute(&f).unwrap_or(f).display().to_string();
        // 取り込みは像面の手前の面の厚さまで含むので、後の空気は 0
        z.air = 0.0;
        if let Some(cfg) = self
            .summary
            .as_ref()
            .and_then(|s| s["cfg"].as_u64())
            .filter(|&n| n > 1)
        {
            z.config = cfg as usize;
        }
        // 開いた系の玉・群（最上位のブロック）を取り込みの中の群として残す（取り込み全体が 1 つのブロックになり、
        // その中に元の玉が並ぶ）
        z.zgroups = open_groups(&self.ld_blocks);
        match self.open_insert.take() {
            Some((after, p)) => c.picks.extend(split_open(z, &self.ld_rows, after, p)),
            None => c.picks.push(z),
        }
        if keep {
            let old = std::mem::take(&mut self.doc.combo);
            c.picks.extend(old.picks);
            c.mechanics = old.mechanics;
        }
        self.doc.combo = c;
        self.combo_view = combo::View::default();
        self.combo_len_prev = self.doc.combo.picks.len();
        self.doc_path = None;
        self.info(tr("開いた系を組合せの要素にした（系の取り込み）"));
        self.build_combo(self.doc.combo.project());
    }
}
