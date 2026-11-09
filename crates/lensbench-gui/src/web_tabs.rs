//! Web 版: 解析のパネルをブラウザの別のタブに出す（ネイティブの「別の窓」の代わり）。
//!
//! - 元のタブ（[`Tabs`]）が系と計算を持ったまま。パネルは `?panel=<id>&main=<sid>` のページを別のタブで開き、
//!   そのタブ（[`Child`]）は計算しないで、元のタブから届く系の概要・結果を描くだけ
//! - 受け渡しは BroadcastChannel（[`CHANNEL`]）の JSON の文字列。`sid` は元のタブごとの番号で、
//!   同じページを 2 つ開いても混ざらない
//!   - 子 → 元: `{sid, from, hello}`（始めたとき）、`{sid, from, cfg, run}`（設定を変えた・計算を頼む）、
//!     `{sid, from, bye}`（閉じた）
//!   - 元 → 子: `{sid, to, snap}`（設定・系の概要・結果）、`{sid, closed}`（元のタブを閉じた）
//! - 子のタブを閉じると、パネルは元のタブのドックに戻る。元のタブを閉じると、子のタブにその知らせを出す

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::layout::AnaCfg;

/// BroadcastChannel の名前
pub const CHANNEL: &str = "lensbench-panels";

/// ページの URL の `?panel=<id>&main=<sid>`（別のタブのパネルのとき）
pub fn panel_of_location() -> Option<(u64, String)> {
    let q = web_sys::window()?.location().search().ok()?;
    let q = web_sys::UrlSearchParams::new_with_str(&q).ok()?;
    let id = q.get("panel")?.parse().ok()?;
    let sid = q.get("main")?;
    Some((id, sid))
}

struct Chan {
    ch: web_sys::BroadcastChannel,
    inbox: Rc<RefCell<Vec<Value>>>,
    _on: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _hide: Option<Closure<dyn FnMut()>>,
}

impl Chan {
    /// `bye` はページを閉じるときに送るもの
    fn new(ctx: &egui::Context, bye: Value) -> Option<Chan> {
        let ch = web_sys::BroadcastChannel::new(CHANNEL).ok()?;
        let inbox: Rc<RefCell<Vec<Value>>> = Rc::default();
        let (i, c) = (inbox.clone(), ctx.clone());
        let on = Closure::<dyn FnMut(_)>::new(move |e: web_sys::MessageEvent| {
            if let Some(v) = e
                .data()
                .as_string()
                .and_then(|s| serde_json::from_str(&s).ok())
            {
                i.borrow_mut().push(v);
                c.request_repaint();
            }
        });
        ch.set_onmessage(Some(on.as_ref().unchecked_ref()));
        let ch2 = ch.clone();
        let bye = bye.to_string();
        let hide = Closure::<dyn FnMut()>::new(move || {
            let _ = ch2.post_message(&JsValue::from_str(&bye));
        });
        let hide = web_sys::window()
            .and_then(|w| {
                w.add_event_listener_with_callback("pagehide", hide.as_ref().unchecked_ref())
                    .ok()
            })
            .map(|_| hide);
        Some(Chan {
            ch,
            inbox,
            _on: on,
            _hide: hide,
        })
    }

    fn post(&self, v: &Value) {
        let _ = self.ch.post_message(&JsValue::from_str(&v.to_string()));
    }

    fn take(&self) -> Vec<Value> {
        std::mem::take(&mut *self.inbox.borrow_mut())
    }
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

// ---- 元のタブ

/// 元のタブで起きたこと
pub enum Ev {
    /// 子のタブで設定を変えた（run は計算を頼んだ）
    Cfg(u64, AnaCfg, bool),
    /// 子のタブを閉じた（ドックに戻す）
    Gone(u64),
}

struct Open {
    win: web_sys::Window,
    /// 最後に送った（結果の版、計算中か）
    sent: Option<(u64, bool)>,
}

/// 元のタブ: 別のタブに出したパネル
pub struct Tabs {
    chan: Option<Chan>,
    sid: String,
    open: HashMap<u64, Open>,
    /// 開けなかった（ポップアップを止められた）パネル。本体の中の窓にする
    pub blocked: std::collections::HashSet<u64>,
}

impl Tabs {
    pub fn new(ctx: &egui::Context) -> Tabs {
        let sid = format!("{:x}", (js_sys::Math::random() * 1e15) as u64);
        Tabs {
            chan: Chan::new(ctx, json!({ "sid": sid, "closed": true })),
            sid,
            open: HashMap::new(),
            blocked: Default::default(),
        }
    }

    pub fn is_open(&self, id: u64) -> bool {
        self.open.contains_key(&id)
    }

    pub fn any_open(&self) -> bool {
        !self.open.is_empty()
    }

    /// パネル id を別のタブで開く。ブラウザに止められたら false
    pub fn open(&mut self, id: u64) -> bool {
        if self.chan.is_none() || self.blocked.contains(&id) {
            return false;
        }
        let Some(w) = web_sys::window() else {
            return false;
        };
        let path = w.location().pathname().unwrap_or_default();
        let url = format!("{path}?panel={id}&main={}", self.sid);
        match w.open_with_url_and_target(&url, &format!("lensbench-{}-{id}", self.sid)) {
            Ok(Some(win)) => {
                self.open.insert(id, Open { win, sent: None });
                true
            }
            _ => {
                self.blocked.insert(id);
                false
            }
        }
    }

    /// 子のタブから届いたこと（閉じたタブも）
    pub fn events(&mut self) -> Vec<Ev> {
        let mut out = Vec::new();
        if let Some(c) = &self.chan {
            for m in c.take() {
                if m["sid"].as_str() != Some(&self.sid) {
                    continue;
                }
                let Some(id) = m["from"].as_u64() else {
                    continue;
                };
                if !self.open.contains_key(&id) {
                    continue;
                }
                if m["hello"].as_bool() == Some(true) {
                    // 始まった: 設定と結果を送り直す
                    if let Some(o) = self.open.get_mut(&id) {
                        o.sent = None;
                    }
                } else if m["bye"].as_bool() == Some(true) {
                    out.push(Ev::Gone(id));
                } else if let Ok(cfg) = serde_json::from_value(m["cfg"].clone()) {
                    // 返事に送り直す（子のタブは返事を待っている）
                    if let Some(o) = self.open.get_mut(&id) {
                        o.sent = None;
                    }
                    out.push(Ev::Cfg(id, cfg, m["run"].as_bool() == Some(true)));
                }
            }
        }
        for (id, o) in &self.open {
            if o.win.closed().unwrap_or(true)
                && !out.iter().any(|e| matches!(e, Ev::Gone(x) if x == id))
            {
                out.push(Ev::Gone(*id));
            }
        }
        for e in &out {
            if let Ev::Gone(id) = e {
                self.open.remove(id);
            }
        }
        out
    }

    /// 送り直すか（結果の版 rev・計算中か busy が前に送ったものと違う）
    pub fn stale(&self, id: u64, rev: u64, busy: bool) -> bool {
        self.open
            .get(&id)
            .is_some_and(|o| o.sent != Some((rev, busy)))
    }

    /// 子のタブに設定・結果を送る
    pub fn send(&mut self, id: u64, rev: u64, busy: bool, snap: Value) {
        let (Some(c), Some(o)) = (&self.chan, self.open.get_mut(&id)) else {
            return;
        };
        o.sent = Some((rev, busy));
        c.post(&json!({ "sid": self.sid, "to": id, "snap": snap.to_string() }));
    }

    /// keep に無いパネルのタブを閉じる（並びを呼び出して窓のパネルが無くなったときなど）
    pub fn retain(&mut self, keep: &[u64]) {
        self.open.retain(|id, o| {
            let k = keep.contains(id);
            if !k {
                let _ = o.win.close();
            }
            k
        });
    }
}

// ---- 別のタブ

/// 別のタブのパネル
pub struct Child {
    chan: Option<Chan>,
    pub id: u64,
    sid: String,
    /// 元のタブを閉じた
    pub closed: bool,
    /// 最後に送った設定
    sent: Option<AnaCfg>,
    /// 最後に hello を送った時刻 [ms]（設定が届くまで 1 秒ごと）
    hello: f64,
    /// 送ってから返事を待っている（送った時刻 [ms]）
    waiting: Option<f64>,
}

impl Child {
    pub fn new(ctx: &egui::Context, id: u64, sid: String) -> Child {
        Child {
            chan: Chan::new(ctx, json!({ "sid": sid, "from": id, "bye": true })),
            id,
            sid,
            closed: false,
            sent: None,
            hello: f64::NEG_INFINITY,
            waiting: None,
        }
    }

    /// 元のタブから届いた設定・結果（最後の 1 つ）。has_pane が false なら届くまで hello を送る
    pub fn poll(&mut self, ctx: &egui::Context, has_pane: bool) -> Option<Value> {
        let c = self.chan.as_ref()?;
        let mut snap = None;
        for m in c.take() {
            if m["sid"].as_str() != Some(&self.sid) {
                continue;
            }
            if m["closed"].as_bool() == Some(true) {
                self.closed = true;
            } else if m["to"].as_u64() == Some(self.id) {
                if let Some(v) = m["snap"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                {
                    snap = Some(v);
                    self.waiting = None;
                }
            }
        }
        if !has_pane && snap.is_none() && !self.closed {
            let t = now_ms();
            if t - self.hello >= 1000.0 {
                self.hello = t;
                c.post(&json!({ "sid": self.sid, "from": self.id, "hello": true }));
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        snap
    }

    /// 設定が変わったか計算を頼むなら元のタブに送る。送ったら true
    pub fn send(&mut self, cfg: &AnaCfg, run: bool) -> bool {
        if !run && self.sent.as_ref() == Some(cfg) {
            return false;
        }
        let Some(c) = &self.chan else {
            return false;
        };
        self.sent = Some(cfg.clone());
        c.post(&json!({ "sid": self.sid, "from": self.id, "cfg": cfg, "run": run }));
        self.waiting.get_or_insert_with(now_ms);
        true
    }

    /// 元のタブが返事をしない（1.5 秒より長く待っている）。ブラウザは見えていないタブの画面を
    /// 回さないので、元のタブが同じ窓の裏にあると、表に出すまで計算しない
    pub fn stalled(&self, ctx: &egui::Context) -> bool {
        match self.waiting {
            Some(t) if now_ms() - t > 1500.0 => true,
            Some(_) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
                false
            }
            None => false,
        }
    }

    /// 元のタブからの設定を受けた（送り返さない）
    pub fn got(&mut self, cfg: &AnaCfg) {
        self.sent = Some(cfg.clone());
    }
}

/// このタブを閉じる（元のタブに戻す）
pub fn close_this() {
    if let Some(w) = web_sys::window() {
        let _ = w.close();
    }
}

/// ブラウザのタブの題
pub fn set_title(t: &str) {
    if let Some(d) = web_sys::window().and_then(|w| w.document()) {
        d.set_title(t);
    }
}
