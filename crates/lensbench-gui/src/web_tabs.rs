//! Web 版: パネルをブラウザの別の窓（ポップアップ）に出す（ネイティブの「別の窓」の代わり）。
//!
//! - 元のタブ（[`Tabs`]）が系と計算を持ったまま。パネルは `?panel=<id>&main=<sid>` のページを別の窓で開く。
//!   - 解析以外のパネル: 元のタブがそのパネルを別の egui の画面（[`Host`]）で描いて網を送り、別の窓（[`Viewer`]）は
//!     網を描いて入力を返すだけ
//!   - 解析のパネル（`&calc=1`）: 別の窓（[`Calc`]）が自分の計算の Web Worker を持ち、元のタブから系の写しと
//!     パネルの設定を受けて自分で計算して描く。元のタブが裏に回ってブラウザに止められても、別の窓の解析は動く
//! - 受け渡しは BroadcastChannel（[`CHANNEL`]）。`sid` は元のタブごとの番号で、同じページを 2 つ開いても混ざらない。
//!   画面のバイト列だけは、元のタブが開いた窓（`window.open` の戻り）へ `postMessage` で直に送り、バッファを
//!   写さずに渡す（transfer。BroadcastChannel は写すうえ、共有の口だと別の窓すべてがほかの窓の画面まで受ける）。
//!   別の窓は自分の窓の `message` で、同じ origin から来たものだけを受ける（[`Frames`]）
//!   - 窓 → 元: `{sid, from, hello, input}`（始めたとき）、`{sid, from, input}`（入力・大きさ。ほかは 2 秒ごと）、
//!     `{sid, from, bye}`（閉じた。pagehide で送る）
//!   - 元 → 窓: 網のバイト列（[`crate::remote_frame`]）、`{sid, to, out}`（カーソル・写した文字など）、
//!     `{sid, to, ack}`（入力を受けたが画面は変わらない）、`{sid, closed}`（元のタブを閉じた）
//!   - 解析のパネル: 窓 → 元 `{sid, from, hello}`（パネルの設定が届くまで 1 秒ごと）・`{sid, from}`（2 秒ごと）・
//!     `{sid, from, pane}`（パネルの設定を変えた）・`{sid, from, flat}`（写した文書を読めなかった）・`{sid, from, bye}`、
//!     元 → 窓 `{sid, to, pane}`（hello への返事）と系の写し（`LBD1` の頭、`postMessage` で直に。系を変えるたび。
//!     元のタブが読んだ文書のファイルと読み方・その後の変更。写せなければ平たい系ファイル）
//! - 元のタブが裏に回るとブラウザはその画面（requestAnimationFrame）を止めるが、メッセージは届くので、
//!   別の窓の入力・計算の結果を合図に元のタブが描いて送る（[`crate::web_bg`]）。それでも返事が
//!   3 秒より長く無ければ（ブラウザがタブごと止めたときなど）、窓にその旨と「元の窓を前へ」を出す
//! - 窓を閉じると、パネルは元のタブのドックに戻る。元のタブを閉じると、窓にその知らせを出す

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// BroadcastChannel の名前
pub(crate) const CHANNEL: &str = "lensbench-panels";

/// ページの URL の `?panel=<id>&main=<sid>`（別のタブのパネルのとき）
pub(crate) fn panel_of_location() -> Option<(u64, String)> {
    let q = web_sys::window()?.location().search().ok()?;
    let q = web_sys::UrlSearchParams::new_with_str(&q).ok()?;
    let id = q.get("panel")?.parse().ok()?;
    let sid = q.get("main")?;
    Some((id, sid))
}

/// このページが解析のパネルを出した別の窓か（`?panel=` に `&calc=1`。自分で計算する）
pub(crate) fn calc_panel() -> bool {
    let q = web_sys::window().and_then(|w| w.location().search().ok());
    let q = q.and_then(|q| web_sys::UrlSearchParams::new_with_str(&q).ok());
    panel_of_location().is_some() && q.and_then(|q| q.get("calc")).as_deref() == Some("1")
}

struct Chan {
    ch: web_sys::BroadcastChannel,
    inbox: Rc<RefCell<Vec<Value>>>,
    _on: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _hide: Option<Closure<dyn FnMut()>>,
}

/// 窓が閉じて口を外すときは、受け手を外して閉じる（閉じないとページが生きている間ブラウザに残る）
impl Drop for Chan {
    fn drop(&mut self) {
        self.ch.set_onmessage(None);
        if let (Some(w), Some(h)) = (web_sys::window(), self._hide.as_ref()) {
            let _ = w.remove_event_listener_with_callback("pagehide", h.as_ref().unchecked_ref());
        }
        self.ch.close();
    }
}

impl Chan {
    /// 口 `name` を開く。`bye` はページを閉じるときに送るもの
    fn new(ctx: &egui::Context, name: &str, bye: Option<Value>) -> Option<Chan> {
        let ch = web_sys::BroadcastChannel::new(name).ok()?;
        let inbox: Rc<RefCell<Vec<Value>>> = Rc::default();
        let (i, c) = (inbox.clone(), ctx.clone());
        let on = Closure::<dyn FnMut(_)>::new(move |e: web_sys::MessageEvent| {
            let d = e.data();
            if let Some(v) = d.as_string().and_then(|s| serde_json::from_str(&s).ok()) {
                i.borrow_mut().push(v);
                c.request_repaint();
                // 元のタブが裏に回っていても、別の窓の入力で描く（別の窓のタブでは何もしない）
                crate::web_bg::kick();
            }
        });
        ch.set_onmessage(Some(on.as_ref().unchecked_ref()));
        let hide = bye.and_then(|bye| {
            let ch2 = ch.clone();
            let bye = bye.to_string();
            let hide = Closure::<dyn FnMut()>::new(move || {
                let _ = ch2.post_message(&JsValue::from_str(&bye));
            });
            web_sys::window()
                .and_then(|w| {
                    w.add_event_listener_with_callback("pagehide", hide.as_ref().unchecked_ref())
                        .ok()
                })
                .map(|_| hide)
        });
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

/// この窓の origin（画面を送る相手・受ける相手を同じ origin に限る）
fn origin() -> Option<String> {
    web_sys::window()?.location().origin().ok()
}

/// 別の窓: 元のタブから `postMessage` で届くバイト列（画面・系ファイル）を受ける口（窓の `message`）
struct Frames {
    bins: Rc<RefCell<Vec<Vec<u8>>>>,
    on: Closure<dyn FnMut(web_sys::MessageEvent)>,
}

impl Frames {
    fn new(ctx: &egui::Context) -> Option<Frames> {
        let w = web_sys::window()?;
        let me = origin()?;
        let bins: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
        let (b, c) = (bins.clone(), ctx.clone());
        let on = Closure::<dyn FnMut(_)>::new(move |e: web_sys::MessageEvent| {
            if e.origin() != me {
                return;
            }
            let d = e.data();
            if d.is_instance_of::<js_sys::ArrayBuffer>() || d.is_instance_of::<js_sys::Uint8Array>()
            {
                b.borrow_mut().push(js_sys::Uint8Array::new(&d).to_vec());
                c.request_repaint();
            }
        });
        w.add_event_listener_with_callback("message", on.as_ref().unchecked_ref())
            .ok()?;
        Some(Frames { bins, on })
    }

    fn take(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.bins.borrow_mut())
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        if let Some(w) = web_sys::window() {
            let _ =
                w.remove_event_listener_with_callback("message", self.on.as_ref().unchecked_ref());
        }
    }
}

/// 画面のバイト列の印
const FRAME: &[u8; 4] = b"LBR1";
/// 系の写しのバイト列の印（頭の後に、JSON の長さ u32・JSON（読み方と変更）・ファイルの中身）
const DOC: &[u8; 4] = b"LBD1";

/// 別の窓に送るバイト列の頭（印 4 バイト、sid の長さ u8・sid、宛先のパネル id u64）
fn frame_head(magic: &[u8; 4], sid: &str, to: u64) -> Vec<u8> {
    let mut h = magic.to_vec();
    h.push(sid.len().min(255) as u8);
    h.extend_from_slice(&sid.as_bytes()[..sid.len().min(255)]);
    h.extend_from_slice(&to.to_le_bytes());
    h
}

/// 印が magic の頭を読む: (sid, 宛先, 残り)
fn split_head<'a>(magic: &[u8; 4], b: &'a [u8]) -> Option<(&'a str, u64, &'a [u8])> {
    if b.get(..4)? != magic {
        return None;
    }
    let n = *b.get(4)? as usize;
    let sid = std::str::from_utf8(b.get(5..5 + n)?).ok()?;
    let to = u64::from_le_bytes(b.get(5 + n..13 + n)?.try_into().ok()?);
    Some((sid, to, &b[13 + n..]))
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

// ---- 元のタブ

/// 元のタブで起きたこと
pub(crate) enum Ev {
    /// 子のタブを閉じた（ドックに戻す）
    Gone(u64),
    /// 別の窓のパネル（解析以外）が始まった（画面を初めから送り直す）
    Hello(u64),
    /// 別の窓のパネルの入力
    Input(u64, Value),
    /// 別の窓の解析のパネルが設定を変えた（[`crate::layout::Pane`] の JSON）
    Pane(u64, Value),
    /// 別の窓の解析のパネルが、写した文書を読めなかった（平たい系ファイルで送り直す）
    Flat(u64),
    /// 別の窓の解析のパネルに届いた写しが前のものと続かなかった（全部を送り直す）
    Resend(u64),
}

struct Open {
    win: web_sys::Window,
    /// 最後に知らせが届いた時刻 [ms]（開いた時刻から）
    seen: f64,
    /// 入力を受けて、まだ何も返していない（[`Tabs::flush_acks`] で返す）
    ack: bool,
}

/// 元のタブ: 別のタブに出したパネル
pub(crate) struct Tabs {
    chan: Option<Chan>,
    sid: String,
    open: HashMap<u64, Open>,
    /// 画面を送る相手の origin（自分と同じ）
    origin: String,
    /// 開けなかった（ポップアップを止められた）パネル。本体の中の窓にする
    pub blocked: std::collections::HashSet<u64>,
    /// 自分で計算する別の窓に出した解析のパネル
    calc: std::collections::HashSet<u64>,
}

impl Tabs {
    pub(crate) fn new(ctx: &egui::Context) -> Tabs {
        let sid = format!("{:x}", (js_sys::Math::random() * 1e15) as u64);
        Tabs {
            chan: Chan::new(ctx, CHANNEL, Some(json!({ "sid": sid, "closed": true }))),
            sid,
            open: HashMap::new(),
            origin: origin().unwrap_or_default(),
            blocked: Default::default(),
            calc: Default::default(),
        }
    }

    /// パネル id は自分で計算する別の窓に出した解析のパネルか
    pub(crate) fn is_calc(&self, id: u64) -> bool {
        self.calc.contains(&id) && self.open.contains_key(&id)
    }

    pub(crate) fn is_open(&self, id: u64) -> bool {
        self.open.contains_key(&id)
    }

    pub(crate) fn any_open(&self) -> bool {
        !self.open.is_empty()
    }

    /// パネル id を別の窓（ポップアップ）で開く（同じ窓のタブだと元のタブが裏に回り、ブラウザが元のタブの画面を
    /// 止めるため）。calc なら別の窓が自分で計算する（解析のパネル）。ブラウザに止められたら false
    pub(crate) fn open(&mut self, id: u64, size: [f32; 2], calc: bool) -> bool {
        if self.chan.is_none() || self.blocked.contains(&id) {
            return false;
        }
        let Some(w) = web_sys::window() else {
            return false;
        };
        let path = w.location().pathname().unwrap_or_default();
        let target = format!("lensbench-{}-{id}", self.sid);
        let url = format!(
            "{path}?panel={id}&main={}{}",
            self.sid,
            if calc { "&calc=1" } else { "" }
        );
        let [w0, h0] = size;
        let features = format!("popup=yes,width={},height={}", w0.round(), h0.round());
        let r = w.open_with_url_and_target_and_features(&url, &target, &features);
        match r {
            Ok(Some(win)) => {
                if calc {
                    self.calc.insert(id);
                } else {
                    self.calc.remove(&id);
                }
                self.open.insert(
                    id,
                    Open {
                        win,
                        seen: now_ms(),
                        ack: false,
                    },
                );
                true
            }
            _ => {
                self.blocked.insert(id);
                false
            }
        }
    }

    /// 子のタブから届いたこと（閉じたタブも）
    pub(crate) fn events(&mut self) -> Vec<Ev> {
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
                if let Some(o) = self.open.get_mut(&id) {
                    o.seen = now_ms();
                    o.ack = true;
                }
                if m["bye"].as_bool() == Some(true) {
                    out.push(Ev::Gone(id));
                    continue;
                }
                if m["hello"].as_bool() == Some(true) {
                    // 始まった: 画面を初めから送り直す
                    out.push(Ev::Hello(id));
                }
                if m["input"].is_object() {
                    out.push(Ev::Input(id, m["input"].clone()));
                }
                if m["pane"].is_object() {
                    out.push(Ev::Pane(id, m["pane"].clone()));
                }
                if m["flat"].as_bool() == Some(true) {
                    out.push(Ev::Flat(id));
                }
                if m["resend"].as_bool() == Some(true) {
                    out.push(Ev::Resend(id));
                }
            }
        }
        for (id, o) in &self.open {
            // 別の窓（ポップアップ）は、開いた直後に closed が true を返すブラウザがあるので、少し前まで知らせが
            // 届いていれば閉じていないとみる（別の窓は 2 秒ごとに知らせる。閉じるときは bye も送る）
            let quiet = now_ms() - o.seen > 6000.0;
            if quiet
                && o.win.closed().unwrap_or(true)
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

    /// 頭（印 magic）と中身を 1 つの JS のバッファに置き、別の窓 id に写さずに渡す（送った後このタブからは使えなくなる）
    fn post_bytes(&mut self, id: u64, magic: &[u8; 4], body: &[&[u8]]) {
        let Some(o) = self.open.get_mut(&id) else {
            return;
        };
        o.ack = false;
        let head = frame_head(magic, &self.sid, id);
        let n = head.len() + body.iter().map(|b| b.len()).sum::<usize>();
        let a = js_sys::Uint8Array::new_with_length(n as u32);
        let mut at = 0;
        for b in std::iter::once(&head[..]).chain(body.iter().copied()) {
            a.subarray(at as u32, (at + b.len()) as u32).copy_from(b);
            at += b.len();
        }
        let buf = a.buffer();
        let _ = o
            .win
            .post_message_with_transfer(&buf, &self.origin, &js_sys::Array::of1(&buf));
    }

    /// 解析のパネルの別の窓に、系の写しを送る（how は読み方と変更の JSON、bytes は読むファイルの中身）
    pub(crate) fn send_doc(&mut self, id: u64, how: &Value, bytes: &[u8]) {
        let j = how.to_string();
        let n = (j.len() as u32).to_le_bytes();
        self.post_bytes(id, DOC, &[&n, j.as_bytes(), bytes]);
    }

    /// 解析のパネルの別の窓に、パネル（[`crate::layout::Pane`] の JSON）を送る
    pub(crate) fn send_pane(&mut self, id: u64, pane: &Value) {
        if let Some(c) = &self.chan {
            c.post(&json!({ "sid": self.sid, "to": id, "pane": pane }));
        }
    }

    /// 別の窓のパネルに画面（[`crate::remote_frame`] のバイト列）を送る
    pub(crate) fn send_frame(&mut self, id: u64, frame: &[u8]) {
        self.post_bytes(id, FRAME, &[frame]);
    }

    /// 別の窓のパネルに画面の外の出力（カーソル・写した文字・IME の位置）を送る
    pub(crate) fn send_out(&mut self, id: u64, out: Value) {
        if let Some(o) = self.open.get_mut(&id) {
            o.ack = false;
        }
        if let Some(c) = &self.chan {
            c.post(&json!({ "sid": self.sid, "to": id, "out": out }));
        }
    }

    /// 元のタブが裏に回っている間: 別の窓に、ms のあとに入力（空でも）を送って合図するよう頼む。
    /// 動き続けるもの（計算中の印など）で回り続けないよう、100 ms より短くはしない
    pub(crate) fn send_again(&mut self, id: u64, ms: f64) {
        if let Some(c) = &self.chan {
            c.post(&json!({ "sid": self.sid, "to": id, "again": ms.max(100.0) }));
        }
    }

    /// 入力を受けたのに画面も出力も送らなかった窓に、受けたことだけ返す（窓が「止まった」と出さないように）
    pub(crate) fn flush_acks(&mut self) {
        let Some(c) = &self.chan else {
            return;
        };
        let calc = &self.calc;
        for (id, o) in self
            .open
            .iter_mut()
            .filter(|(id, o)| o.ack && !calc.contains(id))
        {
            o.ack = false;
            c.post(&json!({ "sid": self.sid, "to": id, "ack": true }));
        }
    }

    /// keep に無いパネルのタブを閉じる（並びを呼び出して窓のパネルが無くなったときなど）
    pub(crate) fn retain(&mut self, keep: &[u64]) {
        self.open.retain(|id, o| {
            let k = keep.contains(id);
            if !k {
                let _ = o.win.close();
            }
            k
        });
    }
}

/// この窓を閉じる（元のタブに戻す）
pub(crate) fn close_this() {
    if let Some(w) = web_sys::window() {
        let _ = w.close();
    }
}

/// この窓を開いた元のタブの窓を前に出す（ブラウザによっては効かない）
fn focus_opener() {
    let opener = web_sys::window().and_then(|w| w.opener().ok());
    if let Some(o) = opener.and_then(|o| o.dyn_into::<web_sys::Window>().ok()) {
        let _ = o.focus();
    }
}

/// ブラウザのタブの題
pub(crate) fn set_title(t: &str) {
    if let Some(d) = web_sys::window().and_then(|w| w.document()) {
        d.set_title(t);
    }
}

// ---- 解析以外のパネル: 元のタブで描いて画面を送る

/// 元のタブ: 別の窓に出した解析以外のパネルを描く別の画面（egui の Context）。入力は別の窓から届く
pub(crate) struct Host {
    ctx: egui::Context,
    events: Vec<egui::Event>,
    rect: egui::Rect,
    ppp: f32,
    mods: egui::Modifiers,
    /// 最後に送った網の指紋
    hash: u64,
    /// 別の窓の大きさを受けた（受けるまで描かない）
    sized: bool,
    /// 最後に送った画面の外の出力（変わったときだけ送る）
    last_out: Value,
    /// パネルの名前（別の窓の上の帯と窓の名前に出す）
    pub title: String,
    /// このパネルの egui が頼んだ描き直しまでの時間 [ms]（一番短いもの。[`Host::take_again`]）
    again: std::sync::Arc<std::sync::Mutex<Option<f64>>>,
    /// フォントを並べた言語（日本語か。変わったら並べ直す）
    ja: Option<bool>,
}

impl Host {
    pub(crate) fn new(main: &egui::Context) -> Host {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let m = main.clone();
        let again: std::sync::Arc<std::sync::Mutex<Option<f64>>> = Default::default();
        let a = again.clone();
        ctx.set_request_repaint_callback(move |info| {
            m.request_repaint_after(info.delay);
            let d = info.delay.as_secs_f64() * 1e3;
            if let Ok(mut a) = a.lock() {
                *a = Some(a.map_or(d, |x| x.min(d)));
            }
        });
        Host {
            ctx,
            events: Vec::new(),
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0)),
            ppp: 1.0,
            mods: Default::default(),
            hash: 0,
            sized: false,
            last_out: Value::Null,
            title: String::new(),
            again,
            ja: None,
        }
    }

    /// このパネルの egui が描き直しを頼んでいれば、その時間 [ms]（取ると消える）。元のタブが裏に回っている間は
    /// 元のタブの画面（rAF）が回らないので、別の窓にこの時間のあとの合図を頼む（[`Tabs::send_again`]）
    pub(crate) fn take_again(&self) -> Option<f64> {
        self.again.lock().ok().and_then(|mut a| a.take())
    }

    /// 別の窓の入力（{events, rect: [x0, y0, x1, y1], ppp, mods}）
    pub(crate) fn input(&mut self, v: &Value) {
        if let Ok(ev) = serde_json::from_value::<Vec<egui::Event>>(v["events"].clone()) {
            self.events.extend(ev);
        }
        if let Some(r) = v["rect"].as_array().filter(|r| r.len() == 4) {
            let f = |i: usize| r[i].as_f64().unwrap_or(0.0) as f32;
            let rect = egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3)));
            if rect.is_positive() {
                self.rect = rect;
                self.sized = true;
            }
        }
        if let Some(p) = v["ppp"].as_f64().filter(|p| *p > 0.1) {
            self.ppp = p as f32;
        }
        if let Ok(m) = serde_json::from_value(v["mods"].clone()) {
            self.mods = m;
        }
    }

    /// パネルを描き、変わっていれば送る画面（バイト列）と画面の外の出力を返す
    pub(crate) fn frame(
        &mut self,
        main: &egui::Context,
        mut ui: impl FnMut(&mut egui::Ui),
    ) -> (Option<Vec<u8>>, Option<Value>) {
        if !self.sized {
            return (None, None);
        }
        self.ctx.set_style(main.style());
        // 文章のフォントの順を元のタブと同じ言語に（言語を変えたら並べ直す）
        let ja = crate::i18n::is_ja();
        if self.ja != Some(ja) {
            crate::fonts::follow_lang(&self.ctx);
            self.ja = Some(ja);
        }
        let mut raw = egui::RawInput {
            screen_rect: Some(self.rect),
            time: Some(now_ms() / 1000.0),
            modifiers: self.mods,
            events: std::mem::take(&mut self.events),
            focused: true,
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(self.ppp);
        let out = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |u| ui(u));
        });
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        let h = crate::remote_frame::mesh_hash(&prims);
        let frame = (h != self.hash || !out.textures_delta.is_empty()).then(|| {
            self.hash = h;
            crate::remote_frame::encode(&out.textures_delta, &prims)
        });
        let po = out.platform_output;
        let copy: Vec<&String> = po
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t),
                _ => None,
            })
            .collect();
        let url = po.commands.iter().find_map(|c| match c {
            egui::OutputCommand::OpenUrl(u) => Some(u.url.clone()),
            _ => None,
        });
        let ime = po.ime.map(|i| {
            [
                i.cursor_rect.min.x,
                i.cursor_rect.min.y,
                i.cursor_rect.max.x,
                i.cursor_rect.max.y,
            ]
        });
        let extra = json!({
            "cursor": po.cursor_icon,
            "copy": copy.last(),
            "url": url,
            "ime": ime,
            "title": self.title,
        });
        // 写した文字はいつも、ほかは変わったときだけ
        let send = !extra["copy"].is_null() || !extra["url"].is_null() || extra != self.last_out;
        self.last_out = extra.clone();
        (frame, send.then_some(extra))
    }
}

/// 別の窓: 元のタブから届く画面を描き、入力を返す（解析以外のパネル）
pub(crate) struct Viewer {
    chan: Option<Chan>,
    /// このパネルの画面を受ける口（[`Frames`]）
    frames: Option<Frames>,
    pub id: u64,
    sid: String,
    pub closed: bool,
    /// 送った側のテクスチャ id → このタブの id
    tex: HashMap<egui::TextureId, egui::TextureId>,
    /// 次の画面の前に消すテクスチャ（描いた後に消す）
    free: Vec<egui::TextureId>,
    /// 届いた網（テクスチャ id はこのタブのものに替えて Arc で持ち、描くたびに複製しない）
    meshes: Vec<(egui::Rect, std::sync::Arc<egui::epaint::Mesh>)>,
    cursor: egui::CursorIcon,
    ime: Option<egui::Rect>,
    last_rect: Option<egui::Rect>,
    /// 最後に hello を送った時刻 [ms]（画面が届くまで 1 秒ごと）
    hello: f64,
    got: bool,
    title: String,
    /// 返事の無い入力を初めに送った時刻 [ms]（何か届いたら None）
    waiting: Option<f64>,
    /// この時刻 [ms] を過ぎたら入力を送る（裏に回った元のタブが描き直しの合図を頼んだ、[`Tabs::send_again`]）
    poke: Option<f64>,
}

impl Viewer {
    pub(crate) fn new(ctx: &egui::Context, id: u64, sid: String) -> Viewer {
        Viewer {
            chan: Chan::new(
                ctx,
                CHANNEL,
                Some(json!({ "sid": sid, "from": id, "bye": true })),
            ),
            frames: Frames::new(ctx),
            id,
            sid,
            closed: false,
            tex: HashMap::new(),
            free: Vec::new(),
            meshes: Vec::new(),
            cursor: egui::CursorIcon::Default,
            ime: None,
            last_rect: None,
            hello: f64::NEG_INFINITY,
            got: false,
            title: String::new(),
            waiting: None,
            poke: None,
        }
    }

    pub(crate) fn update(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.chan else {
            return;
        };
        let tm = ctx.tex_manager();
        for id in self.free.drain(..) {
            tm.write().free(id);
        }
        let bins = self.frames.as_ref().map(Frames::take).unwrap_or_default();
        for b in bins {
            let Some((sid, to, rest)) = split_head(FRAME, &b) else {
                continue;
            };
            if sid != self.sid || to != self.id {
                continue;
            }
            let Ok(f) = crate::remote_frame::decode(rest) else {
                continue;
            };
            self.got = true;
            self.waiting = None;
            for (rid, pos, opt, img) in f.set {
                match self.tex.get(&rid) {
                    Some(l) => tm
                        .write()
                        .set(*l, crate::remote_frame::delta_of(pos, opt, img)),
                    None if pos.is_none() => {
                        let l = tm.write().alloc(
                            format!("remote-{rid:?}"),
                            egui::ImageData::Color(std::sync::Arc::new(img)),
                            opt,
                        );
                        self.tex.insert(rid, l);
                    }
                    None => {}
                }
            }
            for rid in f.free {
                if let Some(l) = self.tex.remove(&rid) {
                    self.free.push(l);
                }
            }
            let tex = &self.tex;
            self.meshes = f
                .meshes
                .into_iter()
                .filter_map(|(clip, mut m)| {
                    m.texture_id = *tex.get(&m.texture_id)?;
                    Some((clip, std::sync::Arc::new(m)))
                })
                .collect();
        }
        for m in c.take() {
            if m["sid"].as_str() != Some(&self.sid) {
                continue;
            }
            if m["closed"].as_bool() == Some(true) {
                self.closed = true;
            } else if m["to"].as_u64() == Some(self.id) {
                self.waiting = None;
                if let Some(ms) = m["again"].as_f64() {
                    let at = now_ms() + ms;
                    self.poke = Some(self.poke.map_or(at, |p| p.min(at)));
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(ms / 1e3));
                }
                let o = &m["out"];
                if let Ok(cur) = serde_json::from_value(o["cursor"].clone()) {
                    self.cursor = cur;
                }
                if let Some(t) = o["copy"].as_str() {
                    ctx.copy_text(t.to_string());
                }
                if let Some(u) = o["url"].as_str() {
                    ctx.open_url(egui::OpenUrl::new_tab(u));
                }
                self.ime = o["ime"].as_array().filter(|r| r.len() == 4).map(|r| {
                    let f = |i: usize| r[i].as_f64().unwrap_or(0.0) as f32;
                    egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3)))
                });
                if let Some(t) = o["title"].as_str().filter(|t| *t != self.title) {
                    self.title = t.to_string();
                    set_title(&format!("lensbench — {t}"));
                }
            }
        }
        // 元のタブの返事が 3 秒より長く無い: 止まっている（裏に回っても描くが、ブラウザがタブごと止めることがある）
        let t0 = now_ms();
        let stalled = !self.closed && self.got && self.waiting.is_some_and(|w| t0 - w > 3000.0);
        // 上の帯: パネルの名前と「元のタブに戻す」
        egui::TopBottomPanel::top("remote-bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(&self.title).strong());
                if ui.small_button(crate::i18n::tr("Back to the main tab")).clicked() {
                    close_this();
                }
                if stalled {
                    ui.label(
                        egui::RichText::new(crate::i18n::tr(
                            "No reply from the main tab (the browser may have paused it; bring the main window to the front)",
                        ))
                        .small()
                        .color(crate::theme::warn(ui.visuals())),
                    );
                    if ui.small_button(crate::i18n::tr("Bring main window to front")).clicked() {
                        focus_opener();
                    }
                }
            });
        });
        // 入力を元のタブへ（帯の下が元のタブで描く所）
        let rect = ctx.available_rect();
        let (events, ppp, mods) = ctx.input(|i| {
            let events: Vec<egui::Event> = i
                .raw
                .events
                .iter()
                .filter(|e| match e {
                    egui::Event::PointerButton {
                        pos, pressed: true, ..
                    } => rect.contains(*pos),
                    _ => true,
                })
                .cloned()
                .collect();
            (events, i.pixels_per_point, i.modifiers)
        });
        let input = json!({
            "events": events,
            "rect": [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
            "ppp": ppp,
            "mods": mods,
        });
        let t = now_ms();
        if !self.got && !self.closed {
            // 画面が届くまで 1 秒ごとに hello
            if t - self.hello >= 1000.0 {
                self.hello = t;
                c.post(&json!({ "sid": self.sid, "from": self.id, "hello": true, "input": input }));
                self.waiting.get_or_insert(t);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        } else if !events.is_empty()
            || self.last_rect != Some(rect)
            || t - self.hello >= 2000.0
            || self.poke.is_some_and(|p| t >= p)
        {
            self.poke = None;
            // 入力・大きさが変わったとき、ほかは 2 秒ごとに（元のタブが閉じていないとみるため）
            self.hello = t;
            c.post(&json!({ "sid": self.sid, "from": self.id, "input": input }));
            self.waiting.get_or_insert(t);
        }
        // 返事を待っている間は、止まったと出すために早めに見直す。合図を頼まれていればその時刻に
        ctx.request_repaint_after(std::time::Duration::from_millis(
            if self.waiting.is_some() { 500 } else { 2000 },
        ));
        if let Some(p) = self.poke {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((p - t).max(0.0) / 1e3));
        }
        self.last_rect = Some(rect);
        // 元のタブで描いた画面
        let painter = ctx.layer_painter(egui::LayerId::background());
        for (clip, m) in &self.meshes {
            painter
                .with_clip_rect(clip.intersect(rect))
                .add(egui::Shape::Mesh(m.clone()));
        }
        ctx.set_cursor_icon(self.cursor);
        if let Some(r) = self.ime {
            ctx.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect: r,
                    cursor_rect: r,
                })
            });
        }
        if self.meshes.is_empty() {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(if self.closed {
                        crate::i18n::tr("The main tab was closed, so this panel cannot be used. Please close this window.")
                    } else {
                        crate::i18n::tr("Waiting for the main tab…")
                    })
                });
            });
        }
    }
}

// ---- 解析のパネル: 別の窓が自分で計算する

/// 別の窓（解析のパネル）に元のタブから届いたもの
pub(crate) enum CalcEv {
    /// パネル（[`crate::layout::Pane`] の JSON）
    Pane(Value),
    /// 系の写し（読み方と変更の JSON、読むファイルの中身）
    Doc(Value, Vec<u8>),
}

/// 別の窓（解析のパネル）: 元のタブから系とパネルの設定を受ける口。計算と描くのは画面（`App`）が自分で
pub(crate) struct Calc {
    chan: Option<Chan>,
    frames: Option<Frames>,
    pub id: u64,
    sid: String,
    /// 元のタブを閉じた
    pub closed: bool,
    /// 最後に知らせを送った時刻 [ms]
    hello: f64,
    /// 最後に受けた・送ったパネル（変えたときだけ元のタブへ送る）
    pane: Option<Value>,
    /// ブラウザのタブの題に出したパネルの名前
    title: String,
}

impl Calc {
    pub(crate) fn new(ctx: &egui::Context, id: u64, sid: String) -> Calc {
        Calc {
            chan: Chan::new(
                ctx,
                CHANNEL,
                Some(json!({ "sid": sid, "from": id, "bye": true })),
            ),
            frames: Frames::new(ctx),
            id,
            sid,
            closed: false,
            hello: f64::NEG_INFINITY,
            pane: None,
            title: String::new(),
        }
    }

    /// ブラウザのタブの題をパネルの名前 t に（変わったときだけ）
    pub(crate) fn set_title(&mut self, t: &str) {
        if t != self.title {
            self.title = t.to_string();
            set_title(&format!("lensbench — {t}"));
        }
    }

    /// 元のタブから届いたもの。パネルが届くまでは 1 秒ごとに hello、届いた後は 2 秒ごとに知らせる
    /// （元のタブは知らせが途切れると窓を閉じたとみる）
    pub(crate) fn events(&mut self, ctx: &egui::Context) -> Vec<CalcEv> {
        let mut out = Vec::new();
        let Some(c) = &self.chan else {
            return out;
        };
        for b in self.frames.as_ref().map(Frames::take).unwrap_or_default() {
            let Some((sid, to, rest)) = split_head(DOC, &b) else {
                continue;
            };
            if sid != self.sid || to != self.id || rest.len() < 4 {
                continue;
            }
            let n = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            let Some(how) = rest
                .get(4..4 + n)
                .and_then(|j| serde_json::from_slice::<Value>(j).ok())
            else {
                continue;
            };
            out.push(CalcEv::Doc(how, rest[4 + n..].to_vec()));
        }
        for m in c.take() {
            if m["sid"].as_str() != Some(&self.sid) {
                continue;
            }
            if m["closed"].as_bool() == Some(true) {
                self.closed = true;
            } else if m["to"].as_u64() == Some(self.id) && m["pane"].is_object() {
                self.pane = Some(m["pane"].clone());
                out.push(CalcEv::Pane(m["pane"].clone()));
            }
        }
        let t = now_ms();
        let every = if self.pane.is_none() { 1000.0 } else { 2000.0 };
        if !self.closed && t - self.hello >= every {
            self.hello = t;
            let mut m = json!({ "sid": self.sid, "from": self.id });
            if self.pane.is_none() {
                m["hello"] = true.into();
            }
            c.post(&m);
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(every / 1e3));
        out
    }

    /// 写した文書を読めなかったと元のタブへ知らせる（平たい系ファイルで送り直してもらう）
    pub(crate) fn ask_flat(&self) {
        if let Some(c) = &self.chan {
            c.post(&json!({ "sid": self.sid, "from": self.id, "flat": true }));
        }
    }

    /// 届いた写しが前のものと続かなかったと元のタブへ知らせる（全部を送り直してもらう）
    pub(crate) fn ask_resend(&self) {
        if let Some(c) = &self.chan {
            c.post(&json!({ "sid": self.sid, "from": self.id, "resend": true }));
        }
    }

    /// パネル（[`crate::layout::Pane`] の JSON）を変えたら元のタブへ送る（元のタブに戻したときに同じ設定にする）
    pub(crate) fn send_pane(&mut self, pane: Value) {
        if self.pane.as_ref() == Some(&pane) || self.closed {
            return;
        }
        if let Some(c) = &self.chan {
            c.post(&json!({ "sid": self.sid, "from": self.id, "pane": pane }));
        }
        self.pane = Some(pane);
    }
}
