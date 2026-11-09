//! 拡大率（DPI）の違うモニターの間で窓を動かしたときに、窓の大きさ（論理座標）を保つ。
//!
//! Windows では、拡大率の違うモニターへ移ると OS（winit）が窓の大きさを変える。大きくなった窓が元のモニターに
//! はみ出すと拡大率の変更が往復し、窓が縮んだり大きくなったりしたまま残ることがある。拡大率が変わる前の大きさを
//! 覚えておき、拡大率が落ち着いたら（しばらく変わらなければ）その大きさに戻す。

use egui::{Context, Vec2, ViewportCommand};

/// 拡大率が変わってから、大きさを戻すまで待つ秒数
const SETTLE_S: f64 = 0.5;

#[derive(Default, Clone)]
pub struct KeepSize {
    ppp: f32,
    /// 拡大率が落ち着いていたときの大きさ（論理座標）
    size: Option<Vec2>,
    /// 戻す大きさと、拡大率が最後に変わった時刻
    want: Option<(Vec2, f64)>,
}

impl KeepSize {
    /// 1 フレーム分。戻す大きさを返す
    fn step(
        &mut self,
        ppp: f32,
        size: Vec2,
        now: f64,
        maximized: bool,
        monitor: Option<Vec2>,
    ) -> Option<Vec2> {
        if maximized {
            self.want = None;
            self.size = None;
            self.ppp = ppp;
            return None;
        }
        if self.ppp != 0.0 && (ppp - self.ppp).abs() > 1e-3 {
            // 拡大率が変わった。往復しても、最初に変わる前の大きさを戻す
            let keep = self.want.map(|w| w.0).or(self.size);
            self.ppp = ppp;
            if let Some(k) = keep {
                self.want = Some((k, now));
            }
            return None;
        }
        self.ppp = ppp;
        if let Some((w, t)) = self.want {
            if now - t < SETTLE_S {
                return None;
            }
            self.want = None;
            // モニターより大きくはしない
            let w = match monitor {
                Some(m) if m.x > 0.0 && m.y > 0.0 => w.min(m * 0.95),
                _ => w,
            };
            self.size = Some(w);
            if (w - size).length() > 2.0 {
                return Some(w);
            }
            return None;
        }
        self.size = Some(size);
        None
    }
}

/// 毎フレーム呼ぶ（その窓の ctx で）
pub fn keep_size(ctx: &Context, k: &mut KeepSize) {
    let (ppp, size, now, maximized, monitor) = ctx.input(|i| {
        let v = i.viewport();
        (
            v.native_pixels_per_point,
            v.inner_rect.map(|r| r.size()),
            i.time,
            v.maximized.unwrap_or(false) || v.fullscreen.unwrap_or(false),
            v.monitor_size,
        )
    });
    let (Some(ppp), Some(size)) = (ppp, size) else {
        return;
    };
    if let Some(w) = k.step(ppp, size, now, maximized, monitor) {
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(w));
    }
    if k.want.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(SETTLE_S));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_size_after_dpi_round_trip() {
        let mut k = KeepSize::default();
        let v = |x, y| Vec2::new(x, y);
        // 175% で 1000×600、100% へ移る（OS が大きさを変える）、さらに往復して縮んだまま
        assert_eq!(k.step(1.75, v(1000.0, 600.0), 0.0, false, None), None);
        assert_eq!(k.step(1.0, v(990.0, 590.0), 1.0, false, None), None);
        assert_eq!(k.step(1.75, v(560.0, 340.0), 1.1, false, None), None);
        // 落ち着く前は戻さない
        assert_eq!(k.step(1.75, v(560.0, 340.0), 1.3, false, None), None);
        // 落ち着いたら、最初の大きさに戻す
        assert_eq!(
            k.step(1.75, v(560.0, 340.0), 1.7, false, None),
            Some(v(1000.0, 600.0))
        );
        // 戻った後は、利用者が変えた大きさを覚える
        assert_eq!(k.step(1.75, v(800.0, 500.0), 2.0, false, None), None);
        assert_eq!(k.step(1.0, v(400.0, 250.0), 3.0, false, None), None);
        // モニターより大きくはしない
        assert_eq!(
            k.step(1.0, v(400.0, 250.0), 4.0, false, Some(v(700.0, 400.0))),
            Some(v(665.0, 380.0))
        );
    }

    #[test]
    fn maximized_is_left_alone() {
        let mut k = KeepSize::default();
        let v = |x, y| Vec2::new(x, y);
        k.step(1.75, v(1000.0, 600.0), 0.0, false, None);
        assert_eq!(k.step(1.0, v(1920.0, 1000.0), 1.0, true, None), None);
        assert_eq!(k.step(1.0, v(1920.0, 1000.0), 2.0, true, None), None);
    }
}
