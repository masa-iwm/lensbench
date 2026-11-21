//! 数の変え方の決まり（どこでも同じ）。表のセル・レンズデータの表・レイアウトの寸法・パネルの数の欄が使う
//!
//! - キー: Alt+↑↓ で [`key_step`] ずつ（素で 0.1、Shift で 1、Ctrl（⌘）で 0.01。整数は 1、Shift で 10）、
//!   PageUp/PageDown はその 10 倍（[`PAGE`]）
//! - ドラッグ: [`drag_value`]。1 点で今の値の桁の 1/100（0 は 0.01）、整数は 10 点で 1。Shift で 1/10
//! - パネルの数の欄は [`Num`]（egui の DragValue を包む。ドラッグの速さを上と同じにし、書き換え中の Alt+↑↓・
//!   PageUp/PageDown を足す）

use std::ops::RangeInclusive;

use egui::{Key, Modifiers, Response, Ui, Widget};

/// キーで数を増減する刻み（素で 0.1、Shift で 1、Ctrl（⌘）で 0.01。整数は 1、Shift で 10）
pub(crate) fn key_step(m: Modifiers, int: bool) -> f64 {
    match (int, m.shift, m.ctrl || m.command) {
        (true, true, _) => 10.0,
        (true, false, _) => 1.0,
        (false, true, _) => 1.0,
        (false, false, true) => 0.01,
        (false, false, false) => 0.1,
    }
}

/// PageUp/PageDown は矢印の何倍か
pub(crate) const PAGE: f64 = 10.0;

/// 押された Alt+↑↓（1）・PageUp/PageDown（[`PAGE`]）を足した数（↑ が +）と、そのときの修飾キー。
/// 押されていなければ None。キーは取る
pub(crate) fn take_alt_arrows(ui: &Ui) -> Option<(f64, Modifiers)> {
    ui.input_mut(|i| {
        let mut n = 0.0;
        let mut mods = None;
        i.events.retain(|e| match e {
            egui::Event::Key {
                key: k @ (Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown),
                pressed: true,
                modifiers,
                ..
            } if modifiers.alt || matches!(k, Key::PageUp | Key::PageDown) => {
                let up = if matches!(k, Key::ArrowUp | Key::PageUp) {
                    1.0
                } else {
                    -1.0
                };
                n += up
                    * if matches!(k, Key::PageUp | Key::PageDown) {
                        PAGE
                    } else {
                        1.0
                    };
                mods = Some(*modifiers);
                false
            }
            _ => true,
        });
        mods.map(|m| (n, m))
    })
}

/// ドラッグした距離 dx [点] での値。1 点で今の値の桁の 1/100（0 は 0.01）、整数は 10 点で 1
pub(crate) fn drag_value(start: f64, dx: f32, int: bool) -> f64 {
    if int {
        return (start + (dx as f64) * 0.1).round().max(0.0);
    }
    let step = drag_speed(start);
    let v = start + dx as f64 * step;
    // 刻みの桁で丸める（浮動小数の端数を残さない）
    let d = (-step.log10().round()).max(0.0) as usize;
    format!("{v:.d$}").parse().unwrap_or(v)
}

/// ドラッグの 1 点あたりの量（今の値の桁の 1/100、0 は 0.01）
pub(crate) fn drag_speed(x: f64) -> f64 {
    let a = x.abs();
    if a > 0.0 && a.is_finite() {
        10f64.powf(a.log10().floor() - 2.0)
    } else {
        0.01
    }
}

/// キーの増減の値（d を足し、浮動小数の端数を丸める。整数は 0 以上）
pub(crate) fn nudge(x: f64, d: f64, int: bool) -> f64 {
    if int {
        return (x + d).round().max(0.0);
    }
    ((x + d) * 1e9).round() / 1e9
}

type Fmt<'a> = Box<dyn 'a + Fn(f64, RangeInclusive<usize>) -> String>;
type Parse<'a> = Box<dyn 'a + Fn(&str) -> Option<f64>>;

/// パネルの数の欄。egui の DragValue と同じ書き方で、ドラッグの速さ・Alt+↑↓ をどこでも同じにする
pub(crate) struct Num<'a> {
    get_set: Box<dyn 'a + FnMut(Option<f64>) -> f64>,
    int: bool,
    range: RangeInclusive<f64>,
    prefix: String,
    suffix: String,
    max_decimals: Option<usize>,
    fmt: Option<Fmt<'a>>,
    parse: Option<Parse<'a>>,
}

impl<'a> Num<'a> {
    pub(crate) fn new<N: egui::emath::Numeric>(v: &'a mut N) -> Self {
        Self {
            get_set: Box::new(move |x| {
                if let Some(x) = x {
                    *v = N::from_f64(x);
                }
                v.to_f64()
            }),
            int: N::INTEGRAL,
            range: N::MIN.to_f64()..=N::MAX.to_f64(),
            prefix: String::new(),
            suffix: String::new(),
            max_decimals: None,
            fmt: None,
            parse: None,
        }
    }

    pub(crate) fn range<N: egui::emath::Numeric>(mut self, r: RangeInclusive<N>) -> Self {
        self.range = r.start().to_f64()..=r.end().to_f64();
        self
    }

    pub(crate) fn prefix(mut self, s: impl ToString) -> Self {
        self.prefix = s.to_string();
        self
    }

    pub(crate) fn suffix(mut self, s: impl ToString) -> Self {
        self.suffix = s.to_string();
        self
    }

    pub(crate) fn max_decimals(mut self, n: usize) -> Self {
        self.max_decimals = Some(n);
        self
    }

    pub(crate) fn custom_formatter(
        mut self,
        f: impl 'a + Fn(f64, RangeInclusive<usize>) -> String,
    ) -> Self {
        self.fmt = Some(Box::new(f));
        self
    }

    pub(crate) fn custom_parser(mut self, f: impl 'a + Fn(&str) -> Option<f64>) -> Self {
        self.parse = Some(Box::new(f));
        self
    }
}

impl Widget for Num<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Num {
            mut get_set,
            int,
            range,
            prefix,
            suffix,
            max_decimals,
            fmt,
            parse,
        } = self;
        // DragValue の id（次に置く部品の id）。キーは書き換え中（フォーカスがある）だけ取る。ポインタが上にあるだけで
        // 取ると、動いている表・図と両方に効くか、見ていない欄だけが変わる（キーの行き先は 1 つ）
        let id = ui.next_auto_id();
        let active = ui.memory(|m| m.has_focus(id));
        let mut keyed = false;
        if active && ui.is_enabled() {
            if let Some((n, m)) = take_alt_arrows(ui) {
                let x = get_set(None);
                let d = key_step(m, int) * n;
                let v = if int {
                    (x + d).round()
                } else {
                    nudge(x, d, false)
                };
                get_set(Some(v.clamp(*range.start(), *range.end())));
                // 打ち込み中の文字は捨てて、新しい値を出す
                ui.data_mut(|d| d.remove::<String>(id));
                keyed = true;
            }
        }
        let x = get_set(None);
        let speed = if int { 0.1 } else { drag_speed(x) };
        let mut dv = egui::DragValue::from_get_set(get_set)
            .speed(speed)
            .range(range)
            .prefix(prefix)
            .suffix(suffix);
        if let Some(n) = max_decimals.or(int.then_some(0)) {
            dv = dv.max_decimals(n);
        }
        if let Some(f) = fmt {
            dv = dv.custom_formatter(f);
        }
        if let Some(f) = parse {
            dv = dv.custom_parser(f);
        }
        let mut r = dv.ui(ui);
        if keyed {
            r.mark_changed();
        }
        r
    }
}
