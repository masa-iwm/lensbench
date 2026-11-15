//! ヘルプの窓（キーの一覧など。GAR-18 で app.rs から分けた）

use super::*;

impl Core {
    pub(super) fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.help_open;
        // 画面の中央に、高さは画面の 8 割まで（上の帯に重ねず、下が切れない。中はスクロール、GUX-22）
        let screen = ctx.screen_rect();
        egui::Window::new(tr("キーボード操作"))
            .open(&mut open)
            .default_width(460.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .max_height(screen.height() * 0.8)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("keys").striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("Ctrl+K", tr("コマンドパレット（操作・解析・パネル・並びを名前で探す）")),
                        ("Ctrl+N", tr("新規（組合せを空にする）")),
                        ("Ctrl+O", tr("系のファイルを開く")),
                        ("F5", tr("開いている解析をすべて計算し直す")),
                        ("Ctrl+R", tr("系を読み直す（変えた値を捨てる）")),
                        ("Ctrl+Z", tr("系の変更を元に戻す（100 段まで。入力欄の中では欄の文字）")),
                        ("Ctrl+Y / Ctrl+Shift+Z", tr("元に戻した変更をやり直す")),
                        ("Ctrl+Tab", tr("次のパネルへ（Shift で前へ）")),
                        ("Tab / Shift+Tab", tr("パネルの中の入力欄・ボタンを順に移る")),
                        ("Enter / Space", tr("ボタンを押す・選ぶ")),
                        ("↑ ↓", tr("数の欄で値を増減（パラメータ・解析の設定は解析をすぐ計算し直す）")),
                        ("Ctrl+A", tr("多重構成の次の構成（Shift で前へ）。レンズデータの表を使っているときは全部の行を選ぶ")),
                        ("Enter", tr("解析の設定の欄: 打った値で計算し直す")),
                        (tr("文字"), tr("解析の種類の一覧: 打って絞り込む（Enter で最初の候補）")),
                        ("F1", tr("この一覧")),
                        ("Esc", tr("パレット・窓を閉じる")),
                    ] {
                        ui.label(RichText::new(k).monospace().strong());
                        ui.label(v);
                        ui.end_row();
                    }
                });
                for (title, keys) in [
                    (
                        tr("レンズデータ"),
                        vec![
                            ("← → ↑ ↓ / Tab", tr("セルを移る（Shift+↑ ↓・Shift+クリックで行の範囲）")),
                            ("Enter / F2", tr("書き換え（文字を打っても始まる）。Enter で決め、Esc で取り消し")),
                            ("Insert", tr("今の面の前に面を挿入")),
                            ("Alt+Insert", tr("今の面の後に面を挿入")),
                            ("Delete", tr("選んだ面を消す")),
                            ("Ctrl+C", tr("選んだ行をタブ区切りでコピー")),
                            ("Ctrl+V", tr("今のセルから上書きで貼り付け")),
                            ("Ctrl+Shift+V", tr("写した行の数だけ面を挿入して貼り付け")),
                            ("Shift+F10", tr("右クリックの欄")),
                            ("Ctrl+L", tr("ブロックのロック")),
                            ("Enter / Backspace", tr("ブロックの中へ・外へ")),
                        ],
                    ),
                    (
                        tr("多重構成"),
                        vec![
                            ("← → ↑ ↓ / Tab", tr("セルを移る")),
                            ("Enter / F2", tr("書き換え（文字を打っても始まる）")),
                            ("Delete", tr("選んだ行を消す（列の見出しを選んでいれば構成を消す）")),
                            ("Ctrl+C / Ctrl+V", tr("行のコピー・上書きで貼り付け")),
                            ("Shift+F10", tr("右クリックの欄")),
                        ],
                    ),
                    (
                        tr("レイアウト図（組合せ）"),
                        vec![
                            ("[ ]", tr("前・次の要素を選ぶ")),
                            ("← →", tr("光軸に沿って動かす（寸法を選んでいればその間隔）。Ctrl で 10 倍、Alt で 1/10")),
                            ("↑ ↓", tr("偏心（Shift で傾き）")),
                            ("R", tr("反転")),
                            ("F", tr("BS の分ける面の向きを逆に")),
                            ("L", tr("ロック")),
                            ("Delete", tr("外す（Ctrl+Z で戻す）")),
                            ("Esc", tr("選ぶのをやめる")),
                        ],
                    ),
                ] {
                    ui.separator();
                    ui.strong(title);
                    egui::Grid::new(("keys", title)).striped(true).show(ui, |ui| {
                        for (k, v) in keys {
                            ui.label(RichText::new(k).monospace().strong());
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                }
                ui.separator();
                ui.label(
                    RichText::new(tr("パネルはタブを引きずって並べ替え・分割・フロートできます。タブを右クリックすると別の窓（別モニター）に出せます。")).small(),
                );
                });
            });
        self.help_open = open;
    }
}
