//! 系を作り直さずにパラメータを変える。
//!
//! 面は番号か名前（コメント）で選ぶ。変えたパラメータが影響しないキャッシュ（面の座標系・近軸量）は残す。
//! 変えた後の系は、同じ値で .zmx から読み直した系と同じデータになり、追跡はビット一致する
//! （屈折率のずれ dn・disp は .zmx に書けないので除く）。
//!
//! パラメータの意味:
//! - `c`（曲率）・`r`（曲率半径、0 は平面）・`k`（円錐係数）・`a2`〜`a16`（偶数次非球面の r^2〜r^16 の係数）
//! - `t`: 次の面までの厚さ（面 0 は物体距離）
//! - `dx` `dy` `tx` `ty` `tz`: 座標ブレークならその値。ほかの面では面の前の傾き偏心（OpticStudio の
//!   Tilt/Decenter の「前」）。前の傾き偏心がない面に与えると、面の後は「前を戻す」になる（その面だけが動く）。
//!   既にある面では面の後の扱いを変えない
//! - `dn`: 面の後の媒質の屈折率に全波長で足すずれ
//! - `disp`: 同じ媒質の分散の相対変化。n = n0(λ) + dn + disp·(n0(λ) − n0(主波長))
//!
//! 媒質は鏡・座標ブレークを越えて続くので、dn・disp はその後に続く鏡・座標ブレークの面にも反映する。
//! 物点の移動（`objx` `objy`）は系のデータではなく追跡の入力なので、ここでは変えない（微分 [`crate::fermat`] で使う）。
//!
//! ピックアップ（[`crate::pickup::Pickup`]、.zmx の曲率・厚さ・円錐係数・パラメータのピックアップ）は OpticStudio と同じく追従する:
//! 元の面のパラメータを変えると、ピックアップ先も「倍率 × 元の値 + オフセット」に解決し直す。
//! ピックアップ先のパラメータそのものは変えられない（エラー。元の面を変える）。
//! 玉の並びを組み直した系（[`crate::elements::assemble`]）はピックアップを引き継がない。
//!
//! 玉・群の単位のパラメータ（[`crate::units`]）も同じ形で指す: `lens2.tx`（玉モードの 2 番目の玉の傾き）、
//! `G1.shift`（群 G1 の移動）。[`Param::surf`] は玉・群の番号（[`System::units`]）、種類は [`Kind::Unit`]。
//! 名前が面の名前（コメント）と玉・群の名前の両方にあるときは、`unit:名前.種類` `surf:名前.種類` で区別する。

use crate::error::Result;
use crate::pickup::pick_kind;
use crate::solves::solve_kind_info;
use crate::surface::{After, Tilt};
use crate::system::{Keep, System};
use crate::units::UnitKind;
use crate::{invalid, notimpl};

/// パラメータの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Kind {
    /// 曲率 c
    Curv,
    /// 曲率半径 R = 1/c（0 は平面）
    Radius,
    /// 円錐係数
    Conic,
    /// 偶数次非球面の係数（0 が r^2、7 が r^16）
    Asph(u8),
    /// Zernike Sag の面の項 j の係数（1 から）
    Zern(u16),
    /// 次の面までの厚さ
    Thick,
    Dx,
    Dy,
    /// 傾き [deg]
    Tx,
    Ty,
    Tz,
    /// 媒質の屈折率のずれ
    Dn,
    /// 媒質の分散の相対変化
    Disp,
    /// 物点の x・y（有限物体。面の番号は使わない）
    ObjX,
    ObjY,
    /// 系の波長 i（0 始まり、名前は wave1 から）[µm]。面の番号は使わない。変えると全媒質の屈折率がその波長の値になる
    /// （光源の波長のずれの公差用。最適化の変数・多重構成の行には使えない）
    Wave(u8),
    /// 玉・群のパラメータ（[`Param::surf`] は玉・群の番号）
    Unit(UnitKind),
}

impl Kind {
    pub fn parse(s: &str) -> Result<Kind> {
        let l = s.trim().to_ascii_lowercase();
        Ok(match l.as_str() {
            "c" | "curv" => Kind::Curv,
            "r" | "radius" => Kind::Radius,
            "k" | "conic" => Kind::Conic,
            "t" | "thick" | "thickness" => Kind::Thick,
            "dx" => Kind::Dx,
            "dy" => Kind::Dy,
            "tx" => Kind::Tx,
            "ty" => Kind::Ty,
            "tz" => Kind::Tz,
            "dn" => Kind::Dn,
            "disp" => Kind::Disp,
            "objx" => Kind::ObjX,
            "objy" => Kind::ObjY,
            _ => {
                if let Some(n) = l.strip_prefix("wave").and_then(|v| v.parse::<u16>().ok()) {
                    if (1..=256).contains(&n) {
                        return Ok(Kind::Wave((n - 1) as u8));
                    }
                }
                if let Some(n) = l.strip_prefix('z').and_then(|v| v.parse::<u16>().ok()) {
                    if (1..=231).contains(&n) {
                        return Ok(Kind::Zern(n));
                    }
                }
                if let Some(n) = l.strip_prefix('a').and_then(|v| v.parse::<u8>().ok()) {
                    if (2..=16).contains(&n) && n % 2 == 0 {
                        return Ok(Kind::Asph(n / 2 - 1));
                    }
                }
                invalid!("parameter type {s} (c r k a2..a16 z1..z231 t dx dy tx ty tz dn disp objx objy wave1..)")
            }
        })
    }

    pub fn name(&self) -> String {
        match self {
            Kind::Curv => "c".into(),
            Kind::Radius => "r".into(),
            Kind::Conic => "k".into(),
            Kind::Asph(i) => format!("a{}", 2 * (*i as usize + 1)),
            Kind::Zern(j) => format!("z{j}"),
            Kind::Thick => "t".into(),
            Kind::Dx => "dx".into(),
            Kind::Dy => "dy".into(),
            Kind::Tx => "tx".into(),
            Kind::Ty => "ty".into(),
            Kind::Tz => "tz".into(),
            Kind::Dn => "dn".into(),
            Kind::Disp => "disp".into(),
            Kind::ObjX => "objx".into(),
            Kind::ObjY => "objy".into(),
            Kind::Wave(i) => format!("wave{}", *i as usize + 1),
            Kind::Unit(u) => u.name().into(),
        }
    }

    /// 傾き偏心の値の位置（dx dy tx ty tz）
    pub fn tilt_index(&self) -> Option<usize> {
        match self {
            Kind::Dx => Some(0),
            Kind::Dy => Some(1),
            Kind::Tx => Some(2),
            Kind::Ty => Some(3),
            Kind::Tz => Some(4),
            _ => None,
        }
    }

    /// 物点の移動（面のパラメータではない）
    pub fn is_object(&self) -> bool {
        matches!(self, Kind::ObjX | Kind::ObjY)
    }

    /// 系の波長
    pub fn is_wave(&self) -> bool {
        matches!(self, Kind::Wave(_))
    }

    /// 面・玉・群に付かない系全体のパラメータ（物点の移動・波長）。[`Param::surf`] は使わない
    pub fn is_global(&self) -> bool {
        self.is_object() || self.is_wave()
    }

    /// 玉・群のパラメータ
    pub fn is_unit(&self) -> bool {
        matches!(self, Kind::Unit(_))
    }

    /// 玉・群の剛体の動き（座標系を動かす。面のパラメータの組合せにならない）
    pub fn is_unit_motion(&self) -> bool {
        matches!(self, Kind::Unit(u) if u.motion_index().is_some())
    }
}

/// パラメータが付く先。面・玉/群の番号の意味は種類で変わらない
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Target {
    /// 面の番号
    Surf(usize),
    /// 玉・群の番号（[`System::units`]）
    Unit(usize),
    /// 面・玉・群に付かない系全体（物点の移動・波長）
    System,
}

/// パラメータ: 付く先と種類。付く先は種類から決まる（[`Kind::is_global`] は系全体、[`Kind::Unit`] は玉・群、
/// ほかは面）ので、[`Param::new`]・[`Param::unit`]・[`Param::system`] で作る
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Param {
    pub target: Target,
    pub kind: Kind,
}

impl Param {
    /// 面 surf のパラメータ kind（kind が系全体・玉/群の種類ならそちら。玉・群は surf が玉・群の番号）
    pub fn new(surf: usize, kind: Kind) -> Param {
        let target = if kind.is_global() {
            Target::System
        } else if kind.is_unit() {
            Target::Unit(surf)
        } else {
            Target::Surf(surf)
        };
        Param { target, kind }
    }
    /// 系全体のパラメータ（物点の移動・波長）
    pub fn system(kind: Kind) -> Param {
        debug_assert!(kind.is_global());
        Param {
            target: Target::System,
            kind,
        }
    }
    pub fn obj_x() -> Param {
        Param::system(Kind::ObjX)
    }
    pub fn obj_y() -> Param {
        Param::system(Kind::ObjY)
    }
    /// 玉・群 u のパラメータ
    pub fn unit(u: usize, kind: UnitKind) -> Param {
        Param {
            target: Target::Unit(u),
            kind: Kind::Unit(kind),
        }
    }
    /// 面の番号（面のパラメータでなければ None）
    pub fn surface(&self) -> Option<usize> {
        match self.target {
            Target::Surf(j) => Some(j),
            _ => None,
        }
    }
    /// 玉・群の番号（玉・群のパラメータでなければ None）
    pub fn unit_index(&self) -> Option<usize> {
        match self.target {
            Target::Unit(u) => Some(u),
            _ => None,
        }
    }
    /// 面・玉/群の番号（系全体は 0）。番号の意味は [`Self::target`] で決まる
    pub fn index(&self) -> usize {
        match self.target {
            Target::Surf(j) | Target::Unit(j) => j,
            Target::System => 0,
        }
    }
    pub fn name(&self) -> String {
        match self.target {
            Target::System => self.kind.name(),
            Target::Unit(u) => format!("unit{u}.{}", self.kind.name()),
            Target::Surf(j) => format!("{j}.{}", self.kind.name()),
        }
    }
}

impl System {
    /// パラメータを名前で: "3.t"、"L1 front.c"（面の名前.種類）、"objx"、"lens2.tx"（玉・群の名前.種類）。
    /// `unit:` `surf:` を前に付けると玉・群か面かを決める
    pub fn param(&self, spec: &str) -> Result<Param> {
        let s = spec.trim();
        if let Ok(k) = Kind::parse(s) {
            if k.is_global() {
                let p = Param::new(0, k);
                self.check_param(p)?;
                return Ok(p);
            }
        }
        let Some((t, k)) = s.rsplit_once('.') else {
            invalid!("parameter {s} (write surface.type, e.g. 3.t)");
        };
        let (t, force) = if let Some(x) = t.strip_prefix("unit:") {
            (x, Some(true))
        } else if let Some(x) = t.strip_prefix("surf:") {
            (x, Some(false))
        } else {
            (t, None)
        };
        let unit = if force == Some(false) || t.trim().parse::<usize>().is_ok() {
            None
        } else {
            self.find_unit(t)
        };
        if force == Some(true) && unit.is_none() {
            invalid!("no lens or group named {}", t.trim());
        }
        if let Some(u) = unit {
            if force.is_none() && self.find_surface(t).is_ok() {
                let t = t.trim();
                invalid!("{t} is the name of a surface and of a lens or group (write unit:{t}.{k} or surf:{t}.{k})");
            }
            let p = Param::unit(u, UnitKind::parse(k)?);
            self.check_param(p)?;
            return Ok(p);
        }
        let kind = Kind::parse(k)?;
        let surf = self.find_surface(t)?;
        let p = Param::new(surf, kind);
        self.check_param(p)?;
        Ok(p)
    }

    /// パラメータの名前（玉・群は名前で）
    pub fn param_name(&self, p: Param) -> String {
        match p.kind {
            Kind::Unit(k) => match self.units.get(p.index()) {
                Some(u) => format!("{}.{}", u.name, k.name()),
                None => p.name(),
            },
            _ => p.name(),
        }
    }

    /// パラメータがこの系で使えるか
    pub fn check_param(&self, p: Param) -> Result<()> {
        // 付く先は種類から決まる（読んだ JSON の食い違いを断る）
        if Param::new(p.index(), p.kind) != p {
            invalid!("parameter {}: the target doesn't match the kind", p.name());
        }
        if p.kind.is_object() {
            return Ok(());
        }
        if let Kind::Wave(i) = p.kind {
            if i as usize >= self.wavelengths.len() {
                let (kind, nw) = (p.kind.name(), self.wavelengths.len());
                invalid!("{kind} (there are {nw} wavelengths)");
            }
            return Ok(());
        }
        if let Kind::Unit(k) = p.kind {
            let Some(u) = self.units.get(p.index()) else {
                invalid!("lens or group {} ({} of them)", p.index(), self.units.len());
            };
            if matches!(k, UnitKind::Dn | UnitKind::Disp) && self.unit_media(p.index()).is_empty() {
                let (unit, medium) = (&u.name, k.name());
                invalid!("lens or group {unit} has no medium (glass) inside ({medium})");
            }
            return Ok(());
        }
        let Target::Surf(j) = p.target else {
            invalid!("parameter {} is not on a surface", p.name());
        };
        if j >= self.surfaces.len() {
            let last = self.surfaces.len() - 1;
            invalid!("surface {j} (surfaces are 0 to {last})");
        }
        let d = &self.surfaces[j];
        let curved = d.shape_params();
        match p.kind {
            Kind::Curv | Kind::Radius | Kind::Conic if j == 0 || !curved => {
                invalid!("surface {j} ({:?}) {}", d.typ, p.kind.name());
            }
            Kind::Asph(_) if !crate::pickup::takes_asph(d) => {
                invalid!("surface {j} is not an even asphere ({})", p.kind.name());
            }
            Kind::Asph(i) if i >= 8 => invalid!("asphere coefficient {}", p.kind.name()),
            Kind::Zern(n) => match &d.zern {
                Some(z) if (n as usize) <= crate::freeform::ZernSag::max_terms(z.fringe) => {}
                Some(_) => invalid!("surface {j} Zernike term {n} is over the limit"),
                None => invalid!(
                    "surface {j} is not a Zernike Sag surface ({})",
                    p.kind.name()
                ),
            },
            _ => {}
        }
        // 形に効かない値（Tilted の c・k、Tilted の 3 番目からの asph、ほかの型の面に残った Zernike の項など）は
        // 変数・微分にしない
        if matches!(
            p.kind,
            Kind::Curv | Kind::Radius | Kind::Conic | Kind::Asph(_) | Kind::Zern(_)
        ) && !d.takes_shape_param(p.kind)
        {
            let (typ, kind) = (d.typ, p.kind.name());
            invalid!("surface {j} ({typ:?}): {kind} doesn't change the shape of this surface type");
        }
        match p.kind {
            Kind::Dx | Kind::Dy | Kind::Tx | Kind::Ty | Kind::Tz if j == 0 => {
                notimpl!("tilt or decenter of the object surface");
            }
            Kind::Dn | Kind::Disp if d.is_mirror() || d.is_coord_break() => {
                let kind = p.kind.name();
                invalid!("surface {j} is a mirror or a coordinate break (the medium is surface {kind} before it)");
            }
            Kind::Thick if j == self.image() => invalid!("thickness of the image surface"),
            _ => {}
        }
        Ok(())
    }

    /// パラメータの今の値
    pub fn get_param(&self, p: Param) -> Result<f64> {
        self.check_param(p)?;
        if p.kind.is_object() {
            let kind = p.kind.name();
            invalid!("field point {kind} is not system data (it is a trace input)");
        }
        if let Kind::Wave(i) = p.kind {
            return Ok(self.wavelengths[i as usize]);
        }
        if let Kind::Unit(k) = p.kind {
            let u = &self.units[p.index()];
            if let Some(i) = k.motion_index() {
                return Ok(u.motion[i]);
            }
            return Ok(match k {
                UnitKind::Shift => u.shift,
                _ => {
                    let v: Vec<f64> = self
                        .unit_media(p.index())
                        .iter()
                        .map(|&j| {
                            let d = &self.surfaces[j];
                            if k == UnitKind::Dn {
                                d.dn
                            } else {
                                d.disp
                            }
                        })
                        .collect();
                    if v.iter().any(|&x| x != v[0]) {
                        let (unit, medium) = (&u.name, k.name());
                        invalid!("lens or group {unit}: medium {medium} differs between surfaces ({v:?})");
                    }
                    v[0]
                }
            });
        }
        let d = &self.surfaces[p.index()];
        Ok(match p.kind {
            Kind::Curv => d.c,
            Kind::Radius => {
                if d.c == 0.0 {
                    0.0
                } else {
                    1.0 / d.c
                }
            }
            Kind::Conic => d.k,
            Kind::Asph(i) => d.asph.get(i as usize).copied().unwrap_or(0.0),
            Kind::Zern(n) => d
                .zern
                .as_ref()
                .and_then(|z| z.coef.get(n as usize - 1).copied())
                .unwrap_or(0.0),
            Kind::Thick => d.t,
            Kind::Dn => d.dn,
            Kind::Disp => d.disp,
            k => {
                let i = k.tilt_index().expect("tilt kind");
                if d.is_coord_break() {
                    d.cb[i]
                } else {
                    d.before.map(|b| b.d[i]).unwrap_or(0.0)
                }
            }
        })
    }

    /// パラメータを変える。影響しないキャッシュ（座標系・近軸量）は残す
    pub fn set_param(&mut self, p: Param, v: f64) -> Result<()> {
        self.set_params(&[(p, v)])
    }

    /// まとめて変える（キャッシュの捨て方も 1 回で決める）。エラーのときは系を変えない
    pub fn set_params(&mut self, changes: &[(Param, f64)]) -> Result<()> {
        *self = self.with_params(changes)?;
        Ok(())
    }

    /// 複製してパラメータを変える（元の系のキャッシュのうち影響しないものを引き継ぐ）
    /// 自動の半直径も求め直す
    pub fn with_params(&self, changes: &[(Param, f64)]) -> Result<System> {
        let mut s = self.with_params_solve(changes, true)?;
        s.update_semi();
        Ok(s)
    }

    /// [`System::with_params`] で、自動の半直径を求め直さない（最適化・公差の差分など、半直径を使わない繰り返し用。
    /// 半直径は光線の追跡に効かない）
    pub fn with_params_fast(&self, changes: &[(Param, f64)]) -> Result<System> {
        let mut s = self.with_params_solve(changes, true)?;
        if s.semi_affects_trace() {
            s.update_semi();
        }
        Ok(s)
    }

    /// [`System::with_params`]。solve が false ならソルブを解き直さない（ソルブの微分の差分用）
    pub(crate) fn with_params_solve(
        &self,
        changes: &[(Param, f64)],
        solve: bool,
    ) -> Result<System> {
        let (flat, motions, picked) = self.unit_changes(changes)?;
        let mut keep = keep_for(&flat);
        if !motions.is_empty() {
            keep.frames = false;
        }
        let mut s = self.clone_keep(keep);
        s.apply(&flat)?;
        for &(u, k, v) in &motions {
            let x = &mut s.rx_mut().units[u];
            match k.motion_index() {
                Some(i) => x.motion[i] = v,
                None => x.shift = v,
            }
        }
        // ピックアップで決まる先は、展開した値と合っているか確かめる（倍率・オフセットが玉・群の動きと合わない）
        for &(p, v) in &picked {
            let now = s.get_param(p)?;
            if (now - v).abs() > 1e-12 * (1.0 + v.abs()) {
                let name = p.name();
                invalid!("{name} becomes {now} by a pickup and doesn't match {v} of the lens or group change");
            }
        }
        // 傾き・偏心だけの変更で近軸量を残せて、ソルブが近軸量と面の形・厚さだけで決まり既に解けていれば、
        // 解き直さない（解き直すと座標系などのキャッシュを捨てる）
        let any = !s.solves.is_empty() || !s.macro_parms.is_empty();
        if solve && any && !(keep.paraxial && s.macro_parms.is_empty() && s.solves_still_hold()) {
            s.solve_thickness()?;
        }
        if flat.iter().any(|(p, _)| p.index() == 0) {
            s.check()?;
        }
        Ok(s)
    }

    /// 玉・群のパラメータの変更を、面のパラメータの変更と、玉・群の動き・移動の値の更新に分ける。
    /// 3 つ目は、ピックアップ先なので変更から除いた面のパラメータと、なるべき値
    #[allow(clippy::type_complexity)]
    fn unit_changes(
        &self,
        changes: &[(Param, f64)],
    ) -> Result<(
        Vec<(Param, f64)>,
        Vec<(usize, UnitKind, f64)>,
        Vec<(Param, f64)>,
    )> {
        if !changes.iter().any(|(p, _)| p.kind.is_unit()) {
            return Ok((changes.to_vec(), Vec::new(), Vec::new()));
        }
        // 3 つ目は玉・群から展開した変更か
        let mut flat: Vec<(Param, f64, bool)> = Vec::new();
        let mut motions = Vec::new();
        // 移動の厚さの変化は、同じ面に重なれば足す（隣り合う範囲の間の間隔など）
        let mut shifted: Vec<(usize, f64)> = Vec::new();
        let mut shift_at: Option<usize> = None;
        for &(p, v) in changes {
            let Kind::Unit(k) = p.kind else {
                flat.push((p, v, false));
                continue;
            };
            self.check_param(p)?;
            if !v.is_finite() {
                invalid!("{} = {v}", self.param_name(p));
            }
            match k {
                UnitKind::Shift => {
                    let d = v - self.units[p.index()].shift;
                    for (q, f) in self.unit_terms(p, false)? {
                        match shifted.iter_mut().find(|x| x.0 == q.index()) {
                            Some(x) => x.1 += f * d,
                            None => shifted.push((q.index(), f * d)),
                        }
                    }
                    shift_at.get_or_insert(flat.len());
                    motions.push((p.index(), k, v));
                }
                UnitKind::Dn | UnitKind::Disp => {
                    for (q, _) in self.unit_terms(p, false)? {
                        flat.push((q, v, true));
                    }
                }
                _ => motions.push((p.index(), k, v)),
            }
        }
        if let Some(at) = shift_at {
            let th: Vec<(Param, f64, bool)> = shifted
                .iter()
                .filter(|x| x.1 != 0.0)
                .map(|&(j, d)| (Param::new(j, Kind::Thick), self.surfaces[j].t + d, true))
                .collect();
            flat.splice(at..at, th);
        }
        // ピックアップ先で、元も変える面は除く（ピックアップで追従する）
        let mut picked = Vec::new();
        let srcs: Vec<Param> = flat.iter().map(|x| x.0).collect();
        flat.retain(|&(p, v, gen)| match self.pickup_of(p) {
            Some(pk)
                if gen
                    && srcs.iter().any(|q| {
                        q.index() == pk.src
                            && pick_kind(&self.surfaces[q.index()], q.kind) == Some(pk.what)
                    }) =>
            {
                picked.push((p, v));
                false
            }
            _ => true,
        });
        Ok((
            flat.into_iter().map(|(p, v, _)| (p, v)).collect(),
            motions,
            picked,
        ))
    }

    /// 玉・群の shift・dn・disp を、面のパラメータの組合せ（パラメータ, 係数）にする。
    /// shift: 範囲ごとに前の間隔 +1・後の間隔 −1（無限遠の物体距離・厚さのソルブの面・ピックアップ先で元も
    /// 組合せに入る面は除く）。dn・disp: 範囲の中の媒質ごとに +1
    pub(crate) fn unit_linear(&self, p: Param) -> Result<Vec<(Param, f64)>> {
        self.unit_terms(p, true)
    }

    /// unit_linear の中身。drop_picked が false ならピックアップ先も残す（変更では後で確かめる）
    fn unit_terms(&self, p: Param, drop_picked: bool) -> Result<Vec<(Param, f64)>> {
        let Kind::Unit(k) = p.kind else {
            return Ok(vec![(p, 1.0)]);
        };
        let u = &self.units[p.index()];
        let mut out: Vec<(Param, f64)> = Vec::new();
        match k {
            UnitKind::Shift => {
                let mut add = |j: usize, f: f64| match out.iter_mut().find(|x| x.0.index() == j) {
                    Some(x) => x.1 += f,
                    None => out.push((Param::new(j, Kind::Thick), f)),
                };
                for s in &u.spans {
                    add(s[0] - 1, 1.0);
                    add(s[1], -1.0);
                }
                out.retain(|(q, f)| {
                    *f != 0.0
                        && self.surfaces[q.index()].t.is_finite()
                        && !self.solves.iter().any(|x| x.surf == q.index())
                });
                if drop_picked {
                    let all: Vec<usize> = out.iter().map(|x| x.0.index()).collect();
                    out.retain(|(q, _)| match self.pickup_of(*q) {
                        Some(pk) => !all.contains(&pk.src),
                        None => true,
                    });
                }
            }
            UnitKind::Dn | UnitKind::Disp => {
                let kind = if k == UnitKind::Dn {
                    Kind::Dn
                } else {
                    Kind::Disp
                };
                for j in self.unit_media(p.index()) {
                    out.push((Param::new(j, kind), 1.0));
                }
            }
            _ => out.push((p, 1.0)),
        }
        Ok(out)
    }

    /// 値を書き換える（キャッシュはそのまま）。残してよいキャッシュを返す
    fn apply(&mut self, changes: &[(Param, f64)]) -> Result<Keep> {
        // 書き換える前にすべて確かめる（途中で止まって半端に変わった系を残さない）
        for &(p, v) in changes {
            self.check_param(p)?;
            if p.kind.is_object() {
                let kind = p.kind.name();
                invalid!("field point {kind} is not system data (it is a trace input)");
            }
            if !v.is_finite() && !(p.kind == Kind::Thick && p.index() == 0) {
                invalid!("{} = {v}", p.name());
            }
            if p.kind.is_wave() {
                if !(v > 0.0) {
                    invalid!("{} = {v} µm", p.name());
                }
                continue;
            }
            let solved = |k: Kind| {
                self.solves
                    .iter()
                    .find(|q| q.surf == p.index() && q.kind.param_kind() == k)
            };
            let q = match p.kind {
                Kind::Thick => solved(Kind::Thick),
                Kind::Curv | Kind::Radius => solved(Kind::Curv),
                _ => None,
            };
            if let Some(q) = q {
                invalid!(
                    "{} is set by a solve ({})",
                    p.name(),
                    solve_kind_info(q.kind).0
                );
            }
            if let Some(pk) = self.pickup_of(p) {
                let (name, src, scale, offset) = (p.name(), pk.src, pk.scale, pk.offset);
                invalid!("{name} is a pickup from surface {src} (scale {scale}, offset {offset}); change the source surface");
            }
        }
        let mut index = false;
        // 波長は先に変える（屈折率は元の系の値の作り方に合わせて作り直す）
        let waves: Vec<(usize, f64)> = changes
            .iter()
            .filter_map(|&(p, v)| match p.kind {
                Kind::Wave(i) => Some((i as usize, v)),
                _ => None,
            })
            .collect();
        if !waves.is_empty() {
            self.set_wavelengths(&waves)?;
            index = true;
        }
        for &(p, v) in changes {
            if p.kind.is_wave() {
                continue;
            }
            let d = &mut self.rx_mut().surfaces[p.index()];
            match p.kind {
                Kind::Curv => d.c = v,
                Kind::Radius => d.c = if v == 0.0 { 0.0 } else { 1.0 / v },
                Kind::Conic => d.k = v,
                Kind::Asph(i) => {
                    let i = i as usize;
                    if d.asph.len() <= i {
                        d.asph.resize(8, 0.0);
                    }
                    d.asph[i] = v;
                }
                Kind::Zern(n) => {
                    let z = std::sync::Arc::make_mut(d.zern.as_mut().expect("Zernike data"));
                    let i = n as usize - 1;
                    if z.coef.len() <= i {
                        z.coef.resize(i + 1, 0.0);
                    }
                    z.coef[i] = v;
                    z.rebuild();
                }
                Kind::Thick => d.t = v,
                Kind::Dn => {
                    d.dn = v;
                    index = true;
                }
                Kind::Disp => {
                    d.disp = v;
                    index = true;
                }
                k => {
                    let i = k.tilt_index().expect("tilt kind");
                    if d.is_coord_break() {
                        d.cb[i] = v;
                    } else {
                        // 前の傾き偏心がなければ作り、面の後は「前を戻す」にする（その面だけが動く）。
                        // 既にあれば面の後はそのまま（後ろの面も一緒に動く系もある）
                        if d.before.is_none() {
                            d.before = Some(Tilt {
                                d: [0.0; 5],
                                order: 0,
                            });
                            if d.after == After::None {
                                d.after = After::Reverse;
                            }
                        }
                        d.before.as_mut().expect("coordinate break created above").d[i] = v;
                    }
                }
            }
        }
        if !self.pickups.is_empty() {
            self.resolve_pickups()?;
        }
        if index {
            self.resolve_media()?;
        }
        Ok(keep_for(changes))
    }
}

/// 変えるパラメータから、残してよいキャッシュを決める
pub(crate) fn keep_for(changes: &[(Param, f64)]) -> Keep {
    // 玉・群の剛体の動きは傾き偏心と同じ（座標系だけ）。移動は厚さ、dn・disp は屈折率
    let tilt = |k: Kind| k.tilt_index().is_some() || k.is_unit_motion();
    let geom = changes.iter().any(|(p, _)| {
        p.kind == Kind::Thick || p.kind == Kind::Unit(UnitKind::Shift) || tilt(p.kind)
    });
    let only_tilt = changes.iter().all(|(p, _)| tilt(p.kind));
    Keep {
        frames: !geom,
        paraxial: only_tilt,
    }
}
