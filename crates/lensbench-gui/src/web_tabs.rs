//! Web 版: パネルをブラウザの別の窓（ポップアップ）に出す（ネイティブの「別の窓」の代わり）。
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
    /// バイト列の知らせ（別の窓のパネルの画面、[`crate::remote_frame`]）
    bins: Rc<RefCell<Vec<Vec<u8>>>>,
    _on: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _hide: Option<Closure<dyn FnMut()>>,
}

impl Chan {
    /// `bye` はページを閉じるときに送るもの
    fn new(ctx: &egui::Context, bye: Value) -> Option<Chan> {
        let ch = web_sys::BroadcastChannel::new(CHANNEL).ok()?;
        let inbox: Rc<RefCell<Vec<Value>>> = Rc::default();
        let bins: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
        let (i, b, c) = (inbox.clone(), bins.clone(), ctx.clone());
        let on = Closure::<dyn FnMut(_)>::new(move |e: web_sys::MessageEvent| {
            let d = e.data();
            if let Some(v) = d.as_string().and_then(|s| serde_json::from_str(&s).ok()) {
                i.borrow_mut().push(v);
                c.request_repaint();
            } else if d.is_instance_of::<js_sys::Uint8Array>()
                || d.is_instance_of::<js_sys::ArrayBuffer>()
            {
                b.borrow_mut().push(js_sys::Uint8Array::new(&d).to_vec());
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
            bins,
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

    fn post_bytes(&self, b: &[u8]) {
        let a = js_sys::Uint8Array::from(b);
        let _ = self.ch.post_message(&a);
    }

    fn take_bins(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.bins.borrow_mut())
    }
}

/// 別の窓のパネルの画面のバイト列の頭（`LBR1`、sid の長さ u8・sid、宛先のパネル id u64）
fn frame_head(sid: &str, to: u64) -> Vec<u8> {
    let mut h = b"LBR1".to_vec();
    h.push(sid.len().min(255) as u8);
    h.extend_from_slice(&sid.as_bytes()[..sid.len().min(255)]);
    h.extend_from_slice(&to.to_le_bytes());
    h
}

/// 頭を読む: (sid, 宛先, 残り)
fn split_head(b: &[u8]) -> Option<(&str, u64, &[u8])> {
    if b.get(..4)? != b"LBR1" {
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
pub enum Ev {
    /// 子のタブで設定を変えた（run は計算を頼んだ）
    Cfg(u64, AnaCfg, bool),
    /// 子のタブを閉じた（ドックに戻す）
    Gone(u64),
    /// 別の窓のパネル（解析以外）が始まった（画面を初めから送り直す）
    Hello(u64),
    /// 別の窓のパネルの入力
    Input(u64, Value),
}

struct Open {
    win: web_sys::Window,
    /// 解析以外のパネル（元のタブで描いた画面を送る、[`Host`]）
    remote: bool,
    /// 最後に知らせが届いた時刻 [ms]（開いた時刻から）
    seen: f64,
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

    /// パネル id を別のタブで開く。remote は解析以外のパネル（元のタブで描いた画面を受けて描くだけ）で、
    /// 別の窓（ポップアップ）にする（同じ窓のタブだと元のタブが裏に回り、ブラウザが元のタブの画面を止めるため）。
    /// ブラウザに止められたら false
    pub fn open(&mut self, id: u64, remote: bool, size: [f32; 2]) -> bool {
        if self.chan.is_none() || self.blocked.contains(&id) {
            return false;
        }
        let Some(w) = web_sys::window() else {
            return false;
        };
        let path = w.location().pathname().unwrap_or_default();
        let target = format!("lensbench-{}-{id}", self.sid);
        // 解析も解析以外も同じ小さな別の窓に（解析は結果を受けて自分で描き、ほかは元のタブで描いた画面を受ける）
        let url = format!(
            "{path}?panel={id}&main={}{}",
            self.sid,
            if remote { "&remote=1" } else { "" }
        );
        let [w0, h0] = size;
        let features = format!("popup=yes,width={},height={}", w0.round(), h0.round());
        let r = w.open_with_url_and_target_and_features(&url, &target, &features);
        match r {
            Ok(Some(win)) => {
                self.open.insert(
                    id,
                    Open {
                        win,
                        remote,
                        seen: now_ms(),
                        sent: None,
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
                let remote = self.open.get(&id).is_some_and(|o| o.remote);
                if let Some(o) = self.open.get_mut(&id) {
                    o.seen = now_ms();
                }
                if m["hello"].as_bool() == Some(true) {
                    // 始まった: 設定と結果を送り直す
                    if let Some(o) = self.open.get_mut(&id) {
                        o.sent = None;
                    }
                    if remote {
                        out.push(Ev::Hello(id));
                        if m["input"].is_object() {
                            out.push(Ev::Input(id, m["input"].clone()));
                        }
                    }
                } else if m["bye"].as_bool() == Some(true) {
                    out.push(Ev::Gone(id));
                } else if remote && m["input"].is_object() {
                    out.push(Ev::Input(id, m["input"].clone()));
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

    /// 別の窓のパネルに画面（[`crate::remote_frame`] のバイト列）を送る
    pub fn send_frame(&self, id: u64, frame: &[u8]) {
        if let Some(c) = &self.chan {
            let mut b = frame_head(&self.sid, id);
            b.extend_from_slice(frame);
            c.post_bytes(&b);
        }
    }

    /// 別の窓のパネルに画面の外の出力（カーソル・写した文字・IME の位置）を送る
    pub fn send_out(&self, id: u64, out: Value) {
        if let Some(c) = &self.chan {
            c.post(&json!({ "sid": self.sid, "to": id, "out": out }));
        }
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
        let t = now_ms();
        if has_pane && !self.closed && t - self.hello >= 2000.0 {
            // 閉じていないと元のタブに知らせる
            self.hello = t;
            c.post(&json!({ "sid": self.sid, "from": self.id, "beat": true }));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(2000));
        if !has_pane && snap.is_none() && !self.closed {
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

/// この窓を閉じる（元のタブに戻す）
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

// ---- 解析以外のパネル: 元のタブで描いて画面を送る（#137）

/// 元のタブ: 別の窓に出した解析以外のパネルを描く別の画面（egui の Context）。入力は別の窓から届く
pub struct Host {
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
}

impl Host {
    pub fn new(main: &egui::Context) -> Host {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let m = main.clone();
        ctx.set_request_repaint_callback(move |info| m.request_repaint_after(info.delay));
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
        }
    }

    /// 別の窓の入力（{events, rect: [x0, y0, x1, y1], ppp, mods}）
    pub fn input(&mut self, v: &Value) {
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
    pub fn frame(
        &mut self,
        main: &egui::Context,
        mut ui: impl FnMut(&mut egui::Ui),
    ) -> (Option<Vec<u8>>, Option<Value>) {
        if !self.sized {
            return (None, None);
        }
        self.ctx.set_style(main.style());
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
pub struct Viewer {
    chan: Option<Chan>,
    pub id: u64,
    sid: String,
    pub closed: bool,
    /// 送った側のテクスチャ id → このタブの id
    tex: HashMap<egui::TextureId, egui::TextureId>,
    /// 次の画面の前に消すテクスチャ（描いた後に消す）
    free: Vec<egui::TextureId>,
    meshes: Vec<(egui::Rect, egui::epaint::Mesh)>,
    cursor: egui::CursorIcon,
    ime: Option<egui::Rect>,
    last_rect: Option<egui::Rect>,
    /// 最後に hello を送った時刻 [ms]（画面が届くまで 1 秒ごと）
    hello: f64,
    got: bool,
    title: String,
}

impl Viewer {
    pub fn new(ctx: &egui::Context, id: u64, sid: String) -> Viewer {
        Viewer {
            chan: Chan::new(ctx, json!({ "sid": sid, "from": id, "bye": true })),
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
        }
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.chan else {
            return;
        };
        let tm = ctx.tex_manager();
        for id in self.free.drain(..) {
            tm.write().free(id);
        }
        for b in c.take_bins() {
            let Some((sid, to, rest)) = split_head(&b) else {
                continue;
            };
            if sid != self.sid || to != self.id {
                continue;
            }
            let Ok(f) = crate::remote_frame::decode(rest) else {
                continue;
            };
            self.got = true;
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
            self.meshes = f.meshes;
        }
        for m in c.take() {
            if m["sid"].as_str() != Some(&self.sid) {
                continue;
            }
            if m["closed"].as_bool() == Some(true) {
                self.closed = true;
            } else if m["to"].as_u64() == Some(self.id) {
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
        // 上の帯: パネルの名前と「元のタブに戻す」（解析のパネルの別の窓と同じ）
        egui::TopBottomPanel::top("remote-bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&self.title).strong());
                if ui.small_button(crate::i18n::tr("元のタブに戻す")).clicked() {
                    close_this();
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
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        } else if !events.is_empty() || self.last_rect != Some(rect) || t - self.hello >= 2000.0 {
            // 入力・大きさが変わったとき、ほかは 2 秒ごとに（元のタブが閉じていないとみるため）
            self.hello = t;
            c.post(&json!({ "sid": self.sid, "from": self.id, "input": input }));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(2000));
        self.last_rect = Some(rect);
        // 元のタブで描いた画面
        let painter = ctx.layer_painter(egui::LayerId::background());
        for (clip, m) in &self.meshes {
            let mut m = m.clone();
            match self.tex.get(&m.texture_id) {
                Some(l) => m.texture_id = *l,
                None => continue,
            }
            painter
                .with_clip_rect(clip.intersect(rect))
                .add(egui::Shape::mesh(m));
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
                        crate::i18n::tr("元のタブを閉じたので、このパネルは使えません。この窓を閉じてください。")
                    } else {
                        crate::i18n::tr("元のタブを待っています…")
                    })
                });
            });
        }
    }
}

/// ページの URL に `&remote=1`（解析以外のパネルの別の窓）
pub fn remote_of_location() -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .and_then(|q| web_sys::UrlSearchParams::new_with_str(&q).ok())
        .and_then(|q| q.get("remote"))
        .is_some_and(|r| r == "1")
}
