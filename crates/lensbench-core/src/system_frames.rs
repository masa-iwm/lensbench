//! 面ごとの座標系（大域・局所の変換）。[`super`] の `System::frames` が使う

use super::{Frame, Frames, Rel, System};
use crate::math::*;
use crate::surface::{After, SurfType, Surface, Tilt};

/// 座標系を 1 面ぶん進める（[`frames`] と同じ）。面の座標系と、次の面へ渡す座標系を返す
pub fn step_frame(o: V3, r: M3, d: &Surface) -> ((V3, M3), (V3, M3)) {
    if d.ignore {
        return ((o, r), (o, r));
    }
    let (mut o, mut r) = (o, r);
    let frame;
    if d.typ == SurfType::CoordBrk {
        (o, r) = apply_cb(
            o,
            r,
            &Tilt {
                d: d.cb,
                order: d.cb_order,
            },
        );
        frame = (o, r);
    } else {
        let (o_pre, r_pre) = (o, r);
        if let Some(b) = &d.before {
            (o, r) = apply_cb(o, r, b);
        }
        frame = (o, r);
        match &d.after {
            After::Reverse => {
                o = o_pre;
                r = r_pre;
            }
            After::Tilt(a) => (o, r) = apply_cb(o, r, a),
            After::None => {}
        }
    }
    o = add(o, scale(col(&r, 2), d.t));
    (frame, (o, r))
}

/// 座標ブレーク（dx dy tx ty tz）を座標系に適用する
pub fn apply_cb(o: V3, r: M3, t: &Tilt) -> (V3, M3) {
    let [dx, dy, tx, ty, tz] = t.d;
    let (rx, ry, rz) = (rot(0, tx), rot(1, ty), rot(2, tz));
    if t.order == 0 {
        let o = add(o, mv(&r, [dx, dy, 0.0]));
        (o, mm(&mm(&mm(&r, &rx), &ry), &rz))
    } else {
        let r = mm(&mm(&mm(&r, &rz), &ry), &rx);
        (add(o, mv(&r, [dx, dy, 0.0])), r)
    }
}

/// 有効な面 q から次の有効な面 j への局所の変換が z のずらしだけなら、その量（q の厚さの符号を変えたもの）。[`step_frame`] の
/// 変換のうち、q の後の傾き（「前」の変換を戻すときは前の傾き）と j の前の傾き・座標ブレークがすべて 0 のとき
fn local_shift(s: &System, q: usize, j: usize) -> Option<f64> {
    let zero = |t: &Tilt| t.d.iter().all(|&v| v == 0.0);
    let (a, b) = (&s.surfaces[q], &s.surfaces[j]);
    if !s.surfaces[q + 1..j].iter().all(|d| d.ignore) {
        return None;
    }
    let after = a.typ == SurfType::CoordBrk
        || match &a.after {
            After::None => true,
            After::Reverse => a.before.as_ref().is_none_or(zero),
            After::Tilt(t) => zero(t),
        };
    let before = if b.typ == SurfType::CoordBrk {
        b.cb.iter().all(|&v| v == 0.0)
    } else {
        b.before.as_ref().is_none_or(zero)
    };
    // 前の面の局所座標 p から q = p − (0, 0, t)
    (after && before && a.t.is_finite()).then_some(-a.t)
}

/// 面ごとの座標系（大域座標 = 面 glrs の座標系）。玉・群の剛体の動き（[`crate::units`]）も入れる
pub fn frames(s: &System) -> Frames {
    let glrs = s.glrs;
    let raw = crate::units::raw_frames(s);
    let mut out = raw.surf;
    if s.units.iter().any(|u| u.moved()) {
        crate::units::apply_frames(&s.units, &raw.entry, &mut out);
    }
    let g = if glrs < out.len() && out[glrs].is_some() {
        out[glrs].unwrap()
    } else {
        // 面 1 が無視（IGNR）なら、その次の有効な面を基準にする
        out.iter()
            .skip(1)
            .flatten()
            .next()
            .copied()
            .unwrap_or(Frame { o: [0.0; 3], r: I3 })
    };
    let gt = transpose(&g.r);
    let res = out
        .iter()
        .map(|f| {
            f.map(|f| Frame {
                o: mtv(&g.r, sub(f.o, g.o)),
                r: mm(&gt, &f.r),
            })
        })
        .collect::<Vec<Option<Frame>>>();
    let mut rel = vec![None; res.len()];
    let mut prev: Option<(usize, Frame)> = None;
    let moved = s.units.iter().any(|u| u.moved());
    let id = I3;
    for (j, f) in res.iter().enumerate() {
        if let Some(f) = f {
            if let Some((qi, q)) = prev {
                // 局所の変換が z のずらしだけなら、大域の座標系から逆算せず面の厚さそのものを使う（傾きの後でも
                // RᵀR の丸めで一般の変換に落ちない）。玉・群の動きがある系は大域から求める
                let rl = match local_shift(s, qi, j).filter(|_| !moved) {
                    Some(sh) => Rel {
                        r: id,
                        t: [0.0, 0.0, sh],
                        shift: Some(sh),
                    },
                    None => {
                        let ft = transpose(&f.r);
                        let r = mm(&ft, &q.r);
                        let t = mv(&ft, sub(q.o, f.o));
                        let shift = (r == id && t[0] == 0.0 && t[1] == 0.0).then_some(t[2]);
                        Rel { r, t, shift }
                    }
                };
                rel[j] = Some(rl);
            }
            prev = Some((j, *f));
        }
    }
    Frames {
        surf: std::sync::Arc::new(res),
        rel: std::sync::Arc::new(rel),
        obj: Frame {
            o: mv(&gt, scale(g.o, -1.0)),
            r: gt,
        },
        fres_ref: Vec::new(),
    }
}
