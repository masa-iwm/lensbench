//! ピックアップ（.zmx の曲率・厚さ・円錐係数・パラメータなどのピックアップ）: 量の種類、面の値の出し入れ、解決。

use crate::error::Result;
// ピックアップする量はパラメータと同じ量の型（ピックアップに使えるのは Kind::is_pick）
use crate::param::{Kind, Param};
use crate::surface::{After, SurfType, Surface, Tilt};
use crate::system::System;
use crate::{invalid, notimpl};

impl Kind {
    /// .zmx のピックアップの列のコード（0 = 同じ列、1 = 半径、2 = 厚さ、3 = 円錐、4 = 半直径、5 + k = パラメータ k。
    /// OpticStudio 2026 R1.01 で実測）から、元の量。先と同じ列なら None
    pub fn from_column(code: i64, dst: Kind) -> Option<Option<Kind>> {
        let k = match code {
            0 => return Some(None),
            1 => Kind::Radius,
            2 => Kind::Thick,
            3 => Kind::Conic,
            4 => Kind::Semi,
            c if (5..=5 + 255).contains(&c) => Kind::Parm((c - 5) as u8),
            _ => return None,
        };
        let same = k == dst || (k == Kind::Radius && dst == Kind::Curv);
        Some(if same { None } else { Some(k) })
    }

    /// .zmx のピックアップの列のコード（[`Kind::from_column`] の逆）。列にならない量は None
    pub fn column(self) -> Option<i64> {
        Some(match self {
            Kind::Curv | Kind::Radius => 1,
            Kind::Thick => 2,
            Kind::Conic => 3,
            Kind::Semi => 4,
            Kind::Parm(k) => 5 + k as i64,
            _ => return None,
        })
    }
}

/// ピックアップ: 面 dst の量 what = scale × 面 src の量 from（None なら what）+ offset。
///
/// 曲率は OpticStudio と同じく特別で（2026 R1.01 で実測）、同じ列（曲率・半径）からなら c = scale × c元、
/// 別の列の値 v からなら元の値を半径とみなして c = scale / v（オフセットなし）。ほかの量が半径の列から取るときは
/// 半径の値 R を使う（v = scale × R + offset）
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Pickup {
    pub dst: usize,
    pub src: usize,
    pub what: Kind,
    pub scale: f64,
    pub offset: f64,
    /// 元の量（別の列からのピックアップ）。None は what と同じ
    #[cfg_attr(feature = "serde", serde(default))]
    pub from: Option<Kind>,
}

impl Pickup {
    /// 同じ量どうしのピックアップ
    pub fn new(dst: usize, src: usize, what: Kind, scale: f64, offset: f64) -> Pickup {
        Pickup {
            dst,
            src,
            what,
            scale,
            offset,
            from: None,
        }
    }

    /// 元の量
    pub fn source(&self) -> Kind {
        self.from.unwrap_or(self.what)
    }

    /// 元の面 a から求めた、先の量の値（元の面にその量がなければ None）
    pub fn value(&self, a: &Surface) -> Option<f64> {
        let from = self.source();
        let v = pick_get(a, from)?;
        if self.what == Kind::Curv && !matches!(from, Kind::Curv | Kind::Radius) {
            return Some(if v == 0.0 || !v.is_finite() {
                0.0
            } else {
                self.scale / v
            });
        }
        if self.what == Kind::Curv && from == Kind::Radius {
            return Some(self.scale * a.c);
        }
        Some(self.scale * v + self.offset)
    }

    /// 元の量（[`pick_kind`] の言い方: 半径は曲率）が x のとき、d(先の量)/dx
    fn slope(&self, a: &Surface) -> f64 {
        let from = self.source();
        if self.what == Kind::Curv && !matches!(from, Kind::Curv | Kind::Radius) {
            let v = pick_get(a, from).unwrap_or(0.0);
            return if v == 0.0 { 0.0 } else { -self.scale / (v * v) };
        }
        if from == Kind::Radius && self.what != Kind::Curv {
            // R = 1/c: dR/dc = −1/c²
            return if a.c == 0.0 {
                0.0
            } else {
                -self.scale / (a.c * a.c)
            };
        }
        self.scale
    }

    /// 系 s でこのピックアップの関係がいまの値で成り立っているか（相対 1e-12。面がないか量がなければ false）
    pub fn holds(&self, s: &System) -> bool {
        let (Some(a), Some(b)) = (s.surfaces.get(self.src), s.surfaces.get(self.dst)) else {
            return false;
        };
        match (self.value(a), pick_get(b, self.what)) {
            (Some(v), Some(y)) => v == y || (v - y).abs() <= 1e-12 * (1.0 + v.abs().max(y.abs())),
            _ => false,
        }
    }
}

/// 面タイプの .zmx の PARM の番号のうち、係数の並び `Surface::asph` の最初に当たる番号（asph[k] は PARM base + k、
/// PARM 8 まで）。偶数次非球面・フレネル・バイナリ 2・自由曲面（奇数次非球面・傾いた面・不規則面・Zernike・Grid Sag）は 1、
/// トロイダルは 2（PARM 1 は回転半径）。係数を持たない面タイプは None（面の量の対応はここ 1 か所）
pub(crate) fn asph_parm_base(t: SurfType) -> Option<usize> {
    match t {
        SurfType::EvenAsph | SurfType::Fresnel | SurfType::CylFresnel | SurfType::Binary2 => {
            Some(1)
        }
        SurfType::Toroidal => Some(2),
        t if crate::freeform::is_freeform(t) => Some(1),
        _ => None,
    }
}

/// パラメータ Kind::Asph（a2・a4…）を持てる面か（偶数次非球面と自由曲面）
pub(crate) fn takes_asph(d: &Surface) -> bool {
    d.typ == SurfType::EvenAsph || d.shape().is_freeform()
}

/// 面タイプのパラメータ i（.zmx の PARM の番号）の値。この面タイプで値を持たない番号は None
pub fn surf_parm(d: &Surface, i: u8) -> Option<f64> {
    let i = i as usize;
    let at =
        |v: &[f64], j: usize| -> Option<f64> { (j < 8).then(|| v.get(j).copied().unwrap_or(0.0)) };
    match d.typ {
        SurfType::Toroidal if i == 1 => Some(if d.cx == 0.0 { 0.0 } else { 1.0 / d.cx }),
        t if asph_parm_base(t).is_some_and(|b| (b..=8).contains(&i)) => {
            at(&d.asph, i - asph_parm_base(t).unwrap_or(1))
        }
        SurfType::CoordBrk if (1..=5).contains(&i) => Some(d.cb[i - 1]),
        SurfType::CoordBrk if i == 6 => Some(d.cb_order as f64),
        SurfType::Paraxial if i == 1 => Some(d.focal),
        SurfType::Paraxial if i == 2 => Some(d.opd_mode as f64),
        SurfType::Jones if (1..=8).contains(&i) => Some(d.jones[i - 1]),
        SurfType::Gradium | SurfType::Gradient9 => {
            use crate::grin::GrinKind;
            let g = d.grin.as_deref()?;
            match (&g.kind, i) {
                (_, 1) => Some(g.dt),
                (GrinKind::Gradium { prof, .. }, 0) => Some(prof.zmax),
                (GrinKind::Gradium { dz, .. }, 2) => Some(*dz),
                (GrinKind::Gradium { .. }, 3) => g.ref_index(),
                (GrinKind::Gradium { dec, .. }, 4..=7) => Some(dec[i - 4]),
                (GrinKind::Gradium { cap, .. }, 8) => Some(*cap as f64),
                (GrinKind::Grad9 { tan, .. }, 2 | 3) => Some(tan[i - 2]),
                _ => None,
            }
        }
        _ => None,
    }
}

/// 面タイプのパラメータ i を書く。値を持たない番号なら false
pub(crate) fn set_surf_parm(d: &mut Surface, i: u8, v: f64) -> bool {
    let i = i as usize;
    let put = |a: &mut Vec<f64>, j: usize, n: usize| {
        if a.len() < n {
            a.resize(n, 0.0);
        }
        a[j] = v;
    };
    match d.typ {
        SurfType::Toroidal if i == 1 => d.cx = if v == 0.0 { 0.0 } else { 1.0 / v },
        t if asph_parm_base(t).is_some_and(|b| (b..=8).contains(&i)) => {
            let b = asph_parm_base(t).unwrap_or(1);
            put(&mut d.asph, i - b, 9 - b)
        }
        SurfType::CoordBrk if (1..=5).contains(&i) => d.cb[i - 1] = v,
        SurfType::CoordBrk if i == 6 => d.cb_order = v as u8,
        SurfType::Paraxial if i == 1 => d.focal = v,
        SurfType::Paraxial if i == 2 => d.opd_mode = v as i32,
        SurfType::Jones if (1..=8).contains(&i) => {
            d.jones[i - 1] = v;
            d.retarder = None;
        }
        // GRIN: Δt と Gradient 9 の形の傾きだけ（Δz などは頂点の屈折率 n も変わるので未対応）
        SurfType::Gradium | SurfType::Gradient9 => {
            use crate::grin::GrinKind;
            let Some(g) = d.grin.as_mut() else {
                return false;
            };
            let g = std::sync::Arc::make_mut(g);
            match (&mut g.kind, i) {
                (_, 1) => g.dt = v,
                (GrinKind::Grad9 { tan, .. }, 2 | 3) => tan[i - 2] = v,
                _ => return false,
            }
        }
        _ => return false,
    }
    true
}

pub(crate) fn pick_get(d: &Surface, what: Kind) -> Option<f64> {
    match what {
        Kind::Curv => Some(d.c),
        Kind::Thick => Some(d.t),
        Kind::Conic => Some(d.k),
        Kind::Parm(i) => surf_parm(d, i),
        Kind::Dn => Some(d.dn),
        Kind::Disp => Some(d.disp),
        // 平面の半径は 1e10（OpticStudio が平面を R = 1e10 として扱うのに合わせる）。曲率への写しは半径を通らず
        // 曲率から（[`Pickup::value`]）なので平面は平面のまま。これが効くのは半径を別の列（厚さなど）へ写す
        // ときだけで、∞ にすると先の値が無限大になる。面の値の表示（[`System::surface_fields`] の r）は ∞、
        // パラメータ r（[`System::get_param`]）は書くときと同じく 0 が平面
        Kind::Radius => Some(if d.c == 0.0 { 1e10 } else { 1.0 / d.c }),
        Kind::Semi => Some(d.semi),
        // 膜は名前を写すだけ（[`System::pick_names`]）。数の値は無いので一定の 0
        Kind::Coating => Some(0.0),
        Kind::Before(i) => Some(d.before.map(|t| t.d[i as usize]).unwrap_or(0.0)),
        Kind::After(i) => match d.after {
            After::Tilt(t) => Some(t.d[i as usize]),
            // 面の後の変換がない = 値が 0
            After::None => Some(0.0),
            After::Reverse => None,
        },
        // ピックアップの量でない種類
        _ => None,
    }
}

pub(crate) fn pick_set(d: &mut Surface, what: Kind, v: f64) -> bool {
    match what {
        Kind::Curv => d.c = v,
        Kind::Thick => d.t = v,
        Kind::Conic => d.k = v,
        Kind::Parm(i) => return set_surf_parm(d, i, v),
        Kind::Dn => d.dn = v,
        Kind::Disp => d.disp = v,
        Kind::Semi => {
            d.semi = v;
            if let crate::surface::Aperture::Floating { semi } = &mut d.ap {
                *semi = v;
            }
        }
        Kind::After(i) => match &mut d.after {
            After::Tilt(t) => t.d[i as usize] = v,
            // 元の面に前の傾き偏心がない（値 0）。後で元に値が入ったら作る（反転なら順序も逆。前の傾き偏心を
            // パラメータで作ると順序 0 なので、反転は 1）
            After::None if v == 0.0 => {}
            After::None => {
                let mut t = Tilt {
                    d: [0.0; 5],
                    order: 0,
                };
                t.d[i as usize] = v;
                d.after = After::Tilt(t);
            }
            After::Reverse => return false,
        },
        Kind::Before(i) => {
            if d.is_coord_break() {
                return false;
            }
            match &mut d.before {
                Some(t) => t.d[i as usize] = v,
                // 元の値が 0 のうちは作らない（後で元に値が入ったら作る）
                None if v == 0.0 => {}
                // パラメータで値を入れるとき（System::set_params）と同じく、前の傾き偏心を作り、
                // 面の後がなければ「前を戻す」にする（その面だけが動く）
                None => {
                    let mut t = Tilt {
                        d: [0.0; 5],
                        order: 0,
                    };
                    t.d[i as usize] = v;
                    d.before = Some(t);
                    if d.after == After::None {
                        d.after = After::Reverse;
                    }
                }
            }
        }
        Kind::Radius => return false,
        Kind::Coating => {}
        _ => return false,
    }
    true
}

/// パラメータ p が動かす、ピックアップの量（なければ None）
pub(crate) fn pick_kind(d: &Surface, kind: Kind) -> Option<Kind> {
    match kind {
        Kind::Curv | Kind::Radius => Some(Kind::Curv),
        Kind::Thick => Some(Kind::Thick),
        Kind::Conic => Some(Kind::Conic),
        Kind::Dn => Some(Kind::Dn),
        Kind::Disp => Some(Kind::Disp),
        Kind::Asph(i) if takes_asph(d) => asph_parm_base(d.typ).map(|b| Kind::Parm(i + b as u8)),
        k if d.is_coord_break() => k.tilt_index().map(|i| Kind::Parm(i as u8 + 1)),
        k => k.tilt_index().map(|i| Kind::Before(i as u8)),
    }
}

/// ピックアップの量を、面 d のパラメータの種類に戻す（Param で表せなければ None）
pub(crate) fn kind_of_pick(d: &Surface, what: Kind) -> Option<Kind> {
    match what {
        // 半径はピックアップの元にだけなる（パラメータ r は曲率のピックアップを動かす）
        Kind::Radius => None,
        w @ (Kind::Curv | Kind::Thick | Kind::Conic | Kind::Dn | Kind::Disp) => Some(w),
        Kind::Before(i) => [Kind::Dx, Kind::Dy, Kind::Tx, Kind::Ty, Kind::Tz]
            .get(i as usize)
            .copied(),
        Kind::Parm(i) => match d.typ {
            t if takes_asph(d)
                && asph_parm_base(t).is_some_and(|b| (b..=8).contains(&(i as usize))) =>
            {
                Some(Kind::Asph(i - asph_parm_base(t).unwrap_or(1) as u8))
            }
            SurfType::CoordBrk => [Kind::Dx, Kind::Dy, Kind::Tx, Kind::Ty, Kind::Tz]
                .get((i as usize).wrapping_sub(1))
                .copied(),
            _ => None,
        },
        // ピックアップの量でない種類
        _ => None,
    }
}

impl System {
    /// パラメータ p がピックアップ先ならそのピックアップ
    pub fn pickup_of(&self, p: Param) -> Option<Pickup> {
        let j = p.surface()?;
        let what = pick_kind(self.surfaces.get(j)?, p.kind)?;
        self.pickups
            .iter()
            .find(|q| q.dst == j && q.what == what)
            .copied()
    }

    /// 名前のピックアップを写す: 材料（[`Kind::Dn`] のピックアップは元の面の材料・鏡も一緒に追従する。
    /// OpticStudio の材料のピックアップ）と膜（[`Kind::Coating`]）。連なっていてもよい。屈折率は後で材料から作る
    pub(crate) fn pick_names(&mut self) -> Result<()> {
        let ns = self.surfaces.len();
        for _ in 0..=ns {
            let mut changed = false;
            for i in 0..self.pickups.len() {
                let q = self.pickups[i];
                if q.src >= ns || q.dst >= ns {
                    invalid!("no pickup surface (from {}, to {})", q.src, q.dst);
                }
                match q.what {
                    Kind::Dn => {
                        let a = &self.surfaces[q.src];
                        let mat = a.mat.clone();
                        let d = &mut self.rx_mut().surfaces[q.dst];
                        if d.is_coord_break() || (mat.is_mirror() && q.dst == 0) || d.mat == mat {
                            continue;
                        }
                        d.mat = mat;
                        changed = true;
                    }
                    Kind::Coating => {
                        let coat = self.surfaces[q.src].coat.clone();
                        let d = &mut self.rx_mut().surfaces[q.dst];
                        if d.coat != coat {
                            d.coat = coat;
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
            if !changed {
                return Ok(());
            }
        }
        invalid!("material or coating pickups don't converge (a loop?)")
    }

    /// ピックアップを解決する（元の面が後ろにあっても、連なっていてもよい）。循環して収まらなければエラー
    pub(crate) fn resolve_pickups(&mut self) -> Result<()> {
        let ns = self.surfaces.len();
        for q in &self.pickups {
            if q.src >= ns || q.dst >= ns {
                invalid!("no pickup surface (from {}, to {})", q.src, q.dst);
            }
        }
        for _ in 0..=ns {
            let mut changed = false;
            for i in 0..self.pickups.len() {
                let q = self.pickups[i];
                let Some(v) = q.value(&self.surfaces[q.src]) else {
                    invalid!("surface {} {:?} can't be picked up", q.src, q.source());
                };
                let d = &mut self.rx_mut().surfaces[q.dst];
                let before = pick_get(d, q.what);
                if before != Some(v) {
                    let created = matches!(q.what, Kind::After(_)) && d.after == After::None;
                    let made_before = matches!(q.what, Kind::Before(_)) && d.before.is_none();
                    let after_was = d.after;
                    let ok = pick_set(d, q.what, v);
                    // 前の傾き偏心を作ったとき、同じ面の後もピックアップの先なら「前を戻す」にしない（後はそのピックアップで決まる）
                    if made_before
                        && after_was == After::None
                        && self
                            .pickups
                            .iter()
                            .any(|k| k.dst == q.dst && matches!(k.what, Kind::After(_)))
                    {
                        self.rx_mut().surfaces[q.dst].after = After::None;
                    }
                    if created {
                        // 作ったばかりの面の後の変換の順序は元の面の前のもの（反転なら逆）
                        let o = self.surfaces[q.src].before.map(|b| b.order).unwrap_or(0);
                        if let After::Tilt(t) = &mut self.rx_mut().surfaces[q.dst].after {
                            t.order = if q.scale < 0.0 { 1 - o.min(1) } else { o };
                        }
                    }
                    let d = &mut self.rx_mut().surfaces[q.dst];
                    if !ok {
                        invalid!("surface {} {:?} can't take a pickup", q.dst, q.what);
                    }
                    // 書くと丸まる量（トロイダルの回転半径、整数の値）もあるので、書いた後の値で比べる
                    changed |= pick_get(d, q.what) != before;
                }
            }
            if !changed {
                return Ok(());
            }
        }
        invalid!("pickups don't converge (a loop?)")
    }

    /// パラメータ p を動かしたときに一緒に動くパラメータと、その倍率 d(先)/d(p)（p 自身を含む）。
    /// ピックアップ先の量が Param で表せない（トロイダルのパラメータなど）なら未対応
    pub(crate) fn pickup_chain(&self, p: Param) -> Result<Vec<(Param, f64)>> {
        if p.kind.is_unit() && !p.kind.is_unit_motion() {
            // 面のパラメータの組合せ（それぞれのピックアップの先も）
            let mut out = Vec::new();
            for (q, f) in self.unit_linear(p)? {
                for (r, g) in self.pickup_chain(q)? {
                    out.push((r, f * g));
                }
            }
            return Ok(out);
        }
        let mut out = vec![(p, 1.0)];
        if p.kind.is_global() || p.kind.is_unit() || self.pickups.is_empty() {
            return Ok(out);
        }
        // 半径を変えたときの曲率の変化 dc/dR = −c²
        let first = if p.kind == Kind::Radius {
            let c = self.surfaces[p.index()].c;
            (Param::new(p.index(), Kind::Curv), -c * c)
        } else {
            (p, 1.0)
        };
        let mut stack = vec![first];
        let mut depth = 0;
        while let Some((q, f)) = stack.pop() {
            depth += 1;
            if depth > 64 * self.surfaces.len() {
                invalid!("pickups go round in a loop");
            }
            let Some(what) = pick_kind(&self.surfaces[q.index()], q.kind) else {
                continue;
            };
            for pk in self.pickups.iter().filter(|k| {
                let from = match k.source() {
                    Kind::Radius => Kind::Curv,
                    w => w,
                };
                k.src == q.index() && from == what
            }) {
                let d = &self.surfaces[pk.dst];
                let Some(kind) = kind_of_pick(d, pk.what) else {
                    let (dst, what) = (pk.dst, pk.what);
                    notimpl!("derivative of surface {dst} {what:?} (a pickup target)");
                };
                let r = (
                    Param::new(pk.dst, kind),
                    f * pk.slope(&self.surfaces[pk.src]),
                );
                out.push(r);
                stack.push(r);
            }
        }
        Ok(out)
    }
}

/// ピックアップの量の名前: c（曲率）・r（半径）・t（厚さ）・k（円錐係数）・p1〜（面タイプのパラメータ）・
/// dn・disp（材料も一緒に追従）・semi・coating（膜の名前）・before0〜4・after0〜4（傾き偏心の値 dx dy tx ty tz）
pub fn pick_kind_parse(s: &str) -> Result<Kind> {
    match Kind::parse(s) {
        Ok(k) if k.is_pick() => Ok(k),
        _ => {
            let s = s.trim();
            invalid!("pickup quantity {s} (c r t k p1.. dn disp semi coating before0..4 after0..4)")
        }
    }
}

/// ピックアップの量の名前（[`pick_kind_parse`] の逆、[`Kind::name`]）
pub fn pick_kind_name(k: Kind) -> String {
    k.name()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ピックアップの量の名前はパラメータの名前と同じ表（別名も同じ）で、名前の行き来が戻る（対応表を 1 か所にした）
    #[test]
    fn pickup_names_share_the_param_names() {
        for (n, w) in [
            ("c", Kind::Curv),
            ("curv", Kind::Curv),
            ("radius", Kind::Radius),
            ("thickness", Kind::Thick),
            ("thick", Kind::Thick),
            ("CONIC", Kind::Conic),
            ("dn", Kind::Dn),
            ("disp", Kind::Disp),
            ("p3", Kind::Parm(3)),
            ("before2", Kind::Before(2)),
            ("coat", Kind::Coating),
        ] {
            assert_eq!(pick_kind_parse(n).unwrap(), w, "{n}");
            assert_eq!(pick_kind_parse(&pick_kind_name(w)).unwrap(), w, "{n}");
        }
        assert!(pick_kind_parse("dx").is_err());
    }

    /// 面の係数のパラメータ（a2・a4…）と .zmx の PARM の番号の対応は面タイプによらず 1 か所:
    /// 自由曲面でもピックアップの量になり、PARM の値が読み書きできる（前は偶数次非球面だけで、奇数次非球面などの
    /// PPAR のピックアップは読込で外れた）
    #[test]
    fn asphere_parms_follow_one_table() {
        for (typ, base) in [
            (SurfType::EvenAsph, 1u8),
            (SurfType::OddAsph, 1),
            (SurfType::Irregular, 1),
            (SurfType::TiltSurf, 1),
        ] {
            let mut d = Surface {
                typ,
                asph: vec![0.1, 0.2, 0.3],
                ..Default::default()
            };
            assert!(takes_asph(&d), "{typ:?}");
            let w = pick_kind(&d, Kind::Asph(1)).unwrap();
            assert_eq!(w, Kind::Parm(base + 1), "{typ:?}");
            assert_eq!(kind_of_pick(&d, w), Some(Kind::Asph(1)), "{typ:?}");
            assert_eq!(pick_get(&d, w), Some(0.2), "{typ:?}");
            assert!(pick_set(&mut d, w, 0.5));
            assert_eq!(d.asph[1], 0.5);
        }
        // トロイダル: PARM 1 は回転半径、係数は PARM 2 から。a2 などのパラメータは持たない
        let d = Surface {
            typ: SurfType::Toroidal,
            cx: 0.02,
            asph: vec![0.1, 0.2],
            ..Default::default()
        };
        assert_eq!(surf_parm(&d, 1), Some(50.0));
        assert_eq!(surf_parm(&d, 3), Some(0.2));
        assert_eq!(surf_parm(&d, 9), None);
        assert!(!takes_asph(&d));
        assert_eq!(pick_kind(&d, Kind::Asph(0)), None);
        // フレネルは PARM 1〜8 が係数（パラメータ a2 は持たない）
        let d = Surface {
            typ: SurfType::Fresnel,
            asph: vec![0.4],
            ..Default::default()
        };
        assert_eq!(surf_parm(&d, 1), Some(0.4));
        assert_eq!(kind_of_pick(&d, Kind::Parm(1)), None);
    }
}
