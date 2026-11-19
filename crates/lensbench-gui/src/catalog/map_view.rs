//! ガラスマップの見えている範囲（拡大・移動・囲んで拡大）

/// ガラスマップの見えている範囲（νd は vd0 が左。左ほど大きい）
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MapView {
    pub vd0: f64,
    pub vd1: f64,
    pub nd0: f64,
    pub nd1: f64,
}

impl MapView {
    /// 全体表示
    pub(crate) const FULL: MapView = MapView {
        vd0: 95.0,
        vd1: 15.0,
        nd0: 1.4,
        nd1: 2.1,
    };
    /// 拡大の限り（幅の最小）
    const MIN_VD: f64 = 0.5;
    const MIN_ND: f64 = 0.005;

    pub(crate) fn is_full(&self) -> bool {
        *self == Self::FULL
    }

    /// (vd, nd) を中心に factor 倍に拡大する（1 より小さければ縮小）。全体より広くはしない
    pub(crate) fn zoom(&mut self, vd: f64, nd: f64, factor: f64) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let f = Self::FULL;
        let (fw, fh) = (f.vd0 - f.vd1, f.nd1 - f.nd0);
        let w = ((self.vd0 - self.vd1) / factor).clamp(Self::MIN_VD, fw);
        let h = ((self.nd1 - self.nd0) / factor).clamp(Self::MIN_ND, fh);
        // 指した点が画面上で動かないように
        let a = (self.vd0 - vd) / (self.vd0 - self.vd1);
        let b = (nd - self.nd0) / (self.nd1 - self.nd0);
        self.vd0 = vd + a * w;
        self.vd1 = self.vd0 - w;
        self.nd0 = nd - b * h;
        self.nd1 = self.nd0 + h;
        self.keep_inside();
    }

    /// 範囲を (dvd, dnd) だけずらす（全体の外へは出さない）
    pub(crate) fn pan(&mut self, dvd: f64, dnd: f64) {
        self.vd0 += dvd;
        self.vd1 += dvd;
        self.nd0 += dnd;
        self.nd1 += dnd;
        self.keep_inside();
    }

    fn keep_inside(&mut self) {
        let f = Self::FULL;
        if self.vd0 > f.vd0 {
            let d = self.vd0 - f.vd0;
            self.vd0 -= d;
            self.vd1 -= d;
        }
        if self.vd1 < f.vd1 {
            let d = f.vd1 - self.vd1;
            self.vd0 += d;
            self.vd1 += d;
        }
        if self.nd0 < f.nd0 {
            let d = f.nd0 - self.nd0;
            self.nd0 += d;
            self.nd1 += d;
        }
        if self.nd1 > f.nd1 {
            let d = self.nd1 - f.nd1;
            self.nd0 -= d;
            self.nd1 -= d;
        }
        // 全体と同じ幅なら丸めの誤差を残さない
        if (self.vd0 - self.vd1 - (f.vd0 - f.vd1)).abs() < 1e-9 {
            self.vd0 = f.vd0;
            self.vd1 = f.vd1;
        }
        if (self.nd1 - self.nd0 - (f.nd1 - f.nd0)).abs() < 1e-9 {
            self.nd0 = f.nd0;
            self.nd1 = f.nd1;
        }
    }
}

impl MapView {
    /// 図の上の操作 pz（[`crate::widgets::pan_zoom`]）を、点を描く所 rect（画面）の範囲に当てる。囲んで拡大は、
    /// 囲んだ枠（rect の外は切る）をそのまま見える範囲にする（縦横は別に決まる。移動を先にすると全体表示では
    /// 全体の外へ出せず効かない）
    pub(crate) fn apply(&mut self, pz: &crate::widgets::PanZoom, rect: egui::Rect) {
        if pz.reset {
            *self = Self::FULL;
            return;
        }
        if let Some(b) = pz.area {
            let b = b.intersect(rect);
            if b.is_positive() {
                let (v0, n1) = self.at(rect, b.left_top());
                let (v1, n0) = self.at(rect, b.right_bottom());
                self.set_range(v0, v1, n0, n1);
            }
            return;
        }
        if pz.pan != egui::Vec2::ZERO {
            let d = pz.pan;
            self.pan(
                -(d.x / rect.width()) as f64 * (self.vd1 - self.vd0),
                (d.y / rect.height()) as f64 * (self.nd1 - self.nd0),
            );
        }
        if let Some(p) = pz.at {
            let (v, n) = self.at(rect, p);
            self.zoom(v, n, pz.zoom as f64);
        }
    }

    /// 画面の点 p（点を描く所 rect の中）の (νd, nd)
    fn at(&self, rect: egui::Rect, p: egui::Pos2) -> (f64, f64) {
        let tx = ((p.x - rect.left()) / rect.width()) as f64;
        let ty = ((rect.bottom() - p.y) / rect.height()) as f64;
        (
            self.vd0 + tx * (self.vd1 - self.vd0),
            self.nd0 + ty * (self.nd1 - self.nd0),
        )
    }

    /// 見える範囲を νd vd0..vd1（vd0 が左）・nd nd0..nd1 にする（拡大の限りより狭ければ中心で広げ、全体の内側へ）
    fn set_range(&mut self, vd0: f64, vd1: f64, nd0: f64, nd1: f64) {
        let f = Self::FULL;
        let w = (vd0 - vd1).clamp(Self::MIN_VD, f.vd0 - f.vd1);
        let h = (nd1 - nd0).clamp(Self::MIN_ND, f.nd1 - f.nd0);
        let (cv, cn) = ((vd0 + vd1) / 2.0, (nd0 + nd1) / 2.0);
        self.vd0 = cv + w / 2.0;
        self.vd1 = cv - w / 2.0;
        self.nd0 = cn - h / 2.0;
        self.nd1 = cn + h / 2.0;
        self.keep_inside();
    }
}

impl Default for MapView {
    fn default() -> Self {
        Self::FULL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::PanZoom;
    use egui::{pos2, Rect, Vec2};

    /// アッベ図の拡大は指した点を動かさず、縮小は全体まで、移動は全体の外へ出ない
    #[test]
    fn map_view_zoom_pan() {
        let mut v = MapView::default();
        assert!(v.is_full());
        // (60, 1.6) を中心に 4 倍
        let a = (v.vd0 - 60.0) / (v.vd0 - v.vd1);
        let b = (1.6 - v.nd0) / (v.nd1 - v.nd0);
        v.zoom(60.0, 1.6, 4.0);
        assert!((v.vd0 - v.vd1 - 20.0).abs() < 1e-9);
        assert!((v.nd1 - v.nd0 - 0.175).abs() < 1e-9);
        assert!(((v.vd0 - 60.0) / (v.vd0 - v.vd1) - a).abs() < 1e-9);
        assert!(((1.6 - v.nd0) / (v.nd1 - v.nd0) - b).abs() < 1e-9);
        // 右下へ大きく動かしても全体の内側
        v.pan(-1000.0, -10.0);
        assert!((v.vd1 - 15.0).abs() < 1e-9 && (v.nd0 - 1.4).abs() < 1e-9);
        assert!((v.vd0 - v.vd1 - 20.0).abs() < 1e-9);
        // 縮小しすぎても全体まで
        v.zoom(30.0, 1.5, 1e-3);
        assert!(v.is_full());
        // 拡大の限り
        v.zoom(50.0, 1.7, 1e9);
        assert!((v.vd0 - v.vd1 - MapView::MIN_VD).abs() < 1e-9);
        assert!((v.nd1 - v.nd0 - MapView::MIN_ND).abs() < 1e-9);
    }

    /// 囲んで拡大: 全体表示から囲んだ枠がそのまま見える範囲になる（移動が全体の外へ出せなくても効く）
    #[test]
    fn map_view_zoom_to_box() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 700.0));
        let mut v = MapView::FULL;
        // 横 νd 95→15 を 800 px、縦 nd 2.1→1.4 を 700 px: x 100..300 は νd 85..65、y 100..200 は nd 2.0..1.9
        let pz = PanZoom {
            zoom: 3.0,
            at: Some(rect.center()),
            pan: rect.center() - pos2(200.0, 150.0),
            reset: false,
            area: Some(Rect::from_min_max(pos2(100.0, 100.0), pos2(300.0, 200.0))),
        };
        v.apply(&pz, rect);
        for (got, want) in [(v.vd0, 85.0), (v.vd1, 65.0), (v.nd0, 1.9), (v.nd1, 2.0)] {
            assert!((got - want).abs() < 1e-6, "{v:?}");
        }
        // 端の近くでも枠のとおり（全体の外の分は内側へ寄せる）
        let mut v = MapView::FULL;
        let pz = PanZoom {
            area: Some(Rect::from_min_max(pos2(-50.0, 600.0), pos2(80.0, 690.0))),
            ..pz
        };
        v.apply(&pz, rect);
        assert!(
            (v.vd0 - 95.0).abs() < 1e-6 && (v.vd1 - 87.0).abs() < 1e-6,
            "{v:?}"
        );
        assert!(
            (v.nd0 - 1.41).abs() < 1e-6 && (v.nd1 - 1.5).abs() < 1e-6,
            "{v:?}"
        );
        // 囲まない操作は前のとおり（移動 → 指した所で拡大）
        let mut v = MapView::FULL;
        v.apply(
            &PanZoom {
                zoom: 2.0,
                at: Some(rect.center()),
                pan: Vec2::ZERO,
                reset: false,
                area: None,
            },
            rect,
        );
        assert!((v.vd0 - v.vd1 - 40.0).abs() < 1e-6 && (v.vd0 - 75.0).abs() < 1e-6);
    }
}
