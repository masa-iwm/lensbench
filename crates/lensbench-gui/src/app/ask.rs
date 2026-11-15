//! 「はい・いいえ」の確かめ（GAR-24）。パネルを描いている途中で OS の窓を開かず、答えは後で続き（`then`）に渡す。
//! 窓は egui のモーダルで、すべてのパネルを描いた後に出す。人の答えを待たない運転（`--tour`）では聞かずに「いいえ」

use super::*;

/// 答え（true が「はい」）を受けて続ける処理
type Then = Box<dyn FnOnce(&mut Core, bool)>;

/// 答えを待っている確かめ
pub(super) struct Ask {
    title: String,
    text: String,
    /// 答え（true が「はい」）を受けて続ける
    then: Then,
    /// 作ったフレームか（そのフレームの Enter は答えに取らない）
    fresh: bool,
    /// Web 版: 確かめを出した別の窓のパネル（その窓に出す）
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    from: Option<u64>,
}

impl Core {
    /// 確かめる。答えは後で `then` に渡す（待たない運転では今すぐ「いいえ」）。前の確かめが残っていれば「いいえ」で閉じる
    pub(super) fn ask(
        &mut self,
        title: &str,
        text: &str,
        then: impl FnOnce(&mut Core, bool) + 'static,
    ) {
        if let Some(a) = self.ask.take() {
            (a.then)(self, false);
        }
        if crate::files::unattended() {
            then(self, false);
            return;
        }
        self.ask = Some(Ask {
            title: title.to_string(),
            text: text.to_string(),
            then: Box::new(then),
            fresh: true,
            from: self.drawing_in,
        });
    }

    /// 答えを待っている確かめを出した別の窓のパネル
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(super) fn ask_from(&self) -> Option<u64> {
        self.ask.as_ref().and_then(|a| a.from)
    }

    /// 確かめの窓（すべてのパネルを描いた後に呼ぶ）
    pub(super) fn ask_ui(&mut self, ctx: &egui::Context) {
        let Some(a) = &self.ask else {
            return;
        };
        let fresh = a.fresh;
        let mut answer = None;
        let r = egui::Modal::new(egui::Id::new("lensbench-ask")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(&a.title);
            ui.add_space(4.0);
            ui.label(&a.text);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let yes = ui.button(tr("yes"));
                if yes.clicked() || (!fresh && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                    answer = Some(true);
                }
                if ui.button(tr("no")).clicked() {
                    answer = Some(false);
                }
            });
        });
        // 外のクリック・Esc は「いいえ」
        if answer.is_none() && r.should_close() {
            answer = Some(false);
        }
        if let Some(a) = &mut self.ask {
            a.fresh = false;
        }
        if let Some(yes) = answer {
            if let Some(a) = self.ask.take() {
                (a.then)(self, yes);
            }
        }
    }
}
