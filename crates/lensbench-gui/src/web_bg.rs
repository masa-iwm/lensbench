//! Web 版: 元のタブが裏に回っている間も、別の窓に出したパネルを動かす。
//!
//! ブラウザは見えていないタブの requestAnimationFrame を止めるので、eframe の画面（と、そこで描いていた
//! 別の窓の分）も止まる。一方、メッセージ（別の窓の入力・計算の Web Worker の結果）は裏のタブにも届く。
//! それを合図に [`kick`] し、MessageChannel の次のタスク（タイマーと違ってブラウザが間引かない）で
//! [`App::background`]（描かない処理と、別の窓の分の描画）を回す。タブが前に戻れば、いつもの画面に任せる

use std::cell::{Cell, RefCell};
use std::rc::Weak;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::app::App;

struct Bg {
    /// ここに送ると、次のタスクで回る
    port: web_sys::MessagePort,
    _keep: (web_sys::MessagePort, Closure<dyn FnMut()>),
}

thread_local! {
    static BG: RefCell<Option<Bg>> = const { RefCell::new(None) };
    /// 送って、まだ回していない
    static QUEUED: Cell<bool> = const { Cell::new(false) };
}

/// このタブが見えていないか
pub(crate) fn hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
}

/// 起動のときに 1 回（元のタブだけ）。`app` は eframe に渡したのと同じ画面
pub(crate) fn install(app: Weak<RefCell<App>>, ctx: egui::Context) {
    let Ok(ch) = web_sys::MessageChannel::new() else {
        return;
    };
    let on = Closure::<dyn FnMut()>::new(move || {
        QUEUED.set(false);
        if !hidden() {
            return;
        }
        let Some(a) = app.upgrade() else {
            return;
        };
        // eframe が描いている最中（前のタブ）には来ないが、念のため借りられなければ飛ばす
        if let Ok(mut a) = a.try_borrow_mut() {
            a.background(&ctx);
        };
    });
    let rx = ch.port1();
    rx.set_onmessage(Some(on.as_ref().unchecked_ref()));
    BG.with(|b| {
        *b.borrow_mut() = Some(Bg {
            port: ch.port2(),
            _keep: (rx, on),
        })
    });
}

/// 裏に回っていれば、次のタスクで [`App::background`] を回す（何度呼んでも 1 回にまとめる）。
/// 別の窓の入力・計算の結果が届いたときに呼ぶ
pub(crate) fn kick() {
    if QUEUED.get() || !hidden() {
        return;
    }
    BG.with(|b| {
        if let Some(b) = b.borrow().as_ref() {
            if b.port.post_message(&JsValue::NULL).is_ok() {
                QUEUED.set(true);
            }
        }
    });
}
