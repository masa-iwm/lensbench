//! 系の編集: 面の挿入・削除・種類の変更、材料・膜・開口、ピックアップとソルブの付け外し、多重構成の表。
//!
//! どの操作も複製した系に施し、うまくいったときだけ差し替える（途中で失敗しても系は元のまま）。
//! 編集の後は、ピックアップを解決し直し、厚さのソルブを解き直し、屈折率（鏡・座標ブレークの媒質の引き継ぎ、
//! dn・disp）を作り直す。
//!
//! 面の挿入・削除では、面番号を持つもの（ピックアップの元・先、ソルブの面、玉・群の範囲、大域座標の基準面、
//! 条件で定める視野の面、多重構成の表の面）を付け替える。消した面を参照するピックアップ・ソルブ・表の行は外し、
//! 説明を返す。
//!
//! 媒質は OpticStudio の約束に合わせる: 面の後ろの媒質はその面の材料（空なら空気）、鏡と座標ブレークは前の
//! 媒質を引き継ぐ。面を入れると、入れた面の後ろの区間はその面の材料になる。

use crate::error::Result;
use crate::glass::{self, Glass};
use crate::param::{Kind, Param, Target};
use crate::pickup::Pickup;
use crate::solves::{solve_kind_of, Solve};
use crate::surface::{After, Aperture, Material, SurfType, Surface};
use crate::system::Prescription;
use crate::system::{Batch, FieldCond, FieldType, System};
use crate::{invalid, notimpl};
use std::collections::HashMap;
use std::path::PathBuf;

/// 面タイプの名前（.zmx の名前と、短い別名）
pub fn surf_type_parse(s: &str) -> Result<SurfType> {
    Ok(match s.trim().to_ascii_uppercase().as_str() {
        "STANDARD" | "STD" => SurfType::Standard,
        "EVENASPH" | "EVEN_ASPHERE" | "ASPH" => SurfType::EvenAsph,
        "COORDBRK" | "CB" | "COORDINATE_BREAK" => SurfType::CoordBrk,
        "JONESMAT" | "JONES" => SurfType::Jones,
        "PARAXIAL" => SurfType::Paraxial,
        "TOROIDAL" => SurfType::Toroidal,
        "SZERNSAG" => SurfType::ZernikeStd,
        "FZERNSAG" => SurfType::ZernikeFringe,
        "GRID_SAG" => SurfType::GridSag,
        "FRESNELS" => SurfType::Fresnel,
        "CFRESNEL" => SurfType::CylFresnel,
        "BINARY_2" => SurfType::Binary2,
        "GRINSUR8" => SurfType::Gradium,
        "GRINSUR9" => SurfType::Gradient9,
        "ODDASPHE" | "ODD_ASPHERE" => SurfType::OddAsph,
        "TILTSURF" | "TILTED" => SurfType::TiltSurf,
        "IRREGULA" | "IRREGULAR" => SurfType::Irregular,
        "USER" => SurfType::User,
        "USERSHAPE" => SurfType::UserShape,
        _ => invalid!("surface type {s}"),
    })
}

/// 面タイプの .zmx の名前
pub(crate) fn surf_type_name(t: SurfType) -> &'static str {
    match t {
        SurfType::Standard => "STANDARD",
        SurfType::EvenAsph => "EVENASPH",
        SurfType::CoordBrk => "COORDBRK",
        SurfType::Jones => "JONESMAT",
        SurfType::Paraxial => "PARAXIAL",
        SurfType::Toroidal => "TOROIDAL",
        SurfType::ZernikeStd => "SZERNSAG",
        SurfType::ZernikeFringe => "FZERNSAG",
        SurfType::GridSag => "GRID_SAG",
        SurfType::Fresnel => "FRESNELS",
        SurfType::CylFresnel => "CFRESNEL",
        SurfType::Binary2 => "BINARY_2",
        SurfType::Gradium => "GRINSUR8",
        SurfType::Gradient9 => "GRINSUR9",
        SurfType::OddAsph => "ODDASPHE",
        SurfType::TiltSurf => "TILTSURF",
        SurfType::Irregular => "IRREGULA",
        SurfType::User => "USER",
        SurfType::UserShape => "USERSHAPE",
    }
}

/// 面の種類の形のデータが揃っているか
fn type_data_ok(d: &Surface) -> Result<()> {
    let ok = match d.typ() {
        SurfType::ZernikeStd | SurfType::ZernikeFringe => d.zern().is_some(),
        SurfType::GridSag => d.grid().is_some(),
        SurfType::CylFresnel => d.cylf().is_some(),
        SurfType::Binary2 => d.bin2().is_some(),
        SurfType::Gradium | SurfType::Gradient9 => d.grin().is_some(),
        SurfType::User => d.user().is_some(),
        SurfType::UserShape => d.user_shape().is_some(),
        _ => true,
    };
    if !ok {
        notimpl!(
            "changing to surface type {} (it needs shape data; read it from a .zmx)",
            surf_type_name(d.typ())
        );
    }
    Ok(())
}

/// 面の値（[`System::set_surface_fields`] に名前と組で渡す）
#[derive(Debug, Clone, PartialEq)]
pub enum SurfValue {
    Num(f64),
    Nums(Vec<f64>),
    Text(String),
    Bool(bool),
    Aperture(Aperture),
}

/// 材料なし（座標ブレークの材料。読込・展開では [`Material::Inherit`]）
fn no_material(m: &Material) -> bool {
    matches!(m, Material::Air | Material::Inherit)
}

/// 面の値の名前（[`System::set_surface_fields`]・[`System::surface_fields`]）
pub const SURF_FIELDS: &[&str] = &[
    "type",
    "comment",
    "t",
    "c",
    "r",
    "k",
    "asph",
    "cx",
    "material",
    "coating",
    "semi",
    "semi_auto",
    "mech",
    "mech_auto",
    "chip",
    "ignore",
    "aperture",
    "cb",
    "cb_order",
    "cb_return",
    "focal",
    "jones",
    "dn",
    "disp",
];

/// 面の値の名前が表す 1 つの量（c r t k dn disp semi coating）。"r" は曲率（ピックアップ・ソルブの先は曲率）
fn field_qty(name: &str) -> Option<Kind> {
    match name {
        "r" => Some(Kind::Curv),
        "semi" => Some(Kind::Semi),
        "coating" => Some(Kind::Coating),
        _ => Kind::parse(name).ok().filter(|k| k.is_pick()),
    }
}

/// 面の値の名前 name を変えると、ピックアップの量 w の先も変わるか（面の値の名前とピックアップの量の対応はここだけ）
fn field_takes(name: &str, w: Kind) -> bool {
    match name {
        // 材料は媒質のずれのピックアップ（材料のピックアップ）と一緒に動く
        "material" => w == Kind::Dn,
        // 面タイプのパラメータに入る値
        "asph" | "cb" | "cx" | "focal" | "jones" => matches!(w, Kind::Parm(_)),
        _ => field_qty(name) == Some(w),
    }
}

impl SurfValue {
    fn num(&self, name: &str) -> Result<f64> {
        match self {
            SurfValue::Num(v) => Ok(*v),
            _ => invalid!("surface value {name} must be a number"),
        }
    }
    fn nums(&self, name: &str, n: Option<usize>) -> Result<Vec<f64>> {
        let v = match self {
            SurfValue::Nums(v) => v.clone(),
            SurfValue::Num(x) if n.is_none() => vec![*x],
            _ => invalid!("surface value {name} must be a list of numbers"),
        };
        if let Some(n) = n {
            if v.len() != n {
                invalid!("surface value {name} has {n} items ({} items)", v.len());
            }
        }
        if v.iter().any(|x| !x.is_finite()) {
            invalid!("surface value {name} = {v:?}");
        }
        Ok(v)
    }
    fn text(&self, name: &str) -> Result<String> {
        match self {
            SurfValue::Text(v) => Ok(v.clone()),
            _ => invalid!("surface value {name} must be a string"),
        }
    }
    fn flag(&self, name: &str) -> Result<bool> {
        match self {
            SurfValue::Bool(v) => Ok(*v),
            _ => invalid!("surface value {name} must be true or false"),
        }
    }
}

impl System {
    /// 文書を f で直して作り直す: 面の値の確かめ、材料からの屈折率、ピックアップの解決、厚さのソルブ、
    /// 系の確かめ、自動の半直径。失敗なら元のまま
    pub fn edit(&mut self, f: impl FnOnce(&mut Prescription) -> Result<()>) -> Result<()> {
        self.edit_sys(|s| f(s.rx_mut()))
    }

    /// 文書を f で直し、値はそのまま置く（ピックアップ・ソルブ・屈折率・自動の半直径は求め直さない）。
    /// 座標系・近軸量などは作り直し、系を確かめる。失敗なら元のまま（[`System::batch`] の中では系ごと捨てる）
    pub fn edit_raw(&mut self, f: impl FnOnce(&mut Prescription) -> Result<()>) -> Result<()> {
        if self.batch != Batch::Off {
            f(self.rx_mut())?;
            return self.changed();
        }
        let mut s = self.clone();
        f(s.rx_mut())?;
        s.changed()?;
        *self = s;
        Ok(())
    }

    /// ピックアップを解決し、ソルブを解き直す（[`System::edit_raw`] で足したピックアップ・ソルブを効かせる）。
    /// 座標系・近軸量などは作り直す
    pub fn resolve_links(&mut self) -> Result<()> {
        self.invalidate_frames();
        self.resolve_pickups()?;
        if !self.solves.is_empty() || !self.macro_parms.is_empty() {
            self.solve_thickness()?;
        }
        self.changed()
    }

    /// 系を複製して f で変え、編集の後始末（`refresh`）をしてから差し替える。失敗なら元のまま
    /// （[`System::batch`] の中では複製せずに直し、失敗した系は batch が捨てる）
    pub(crate) fn edit_sys(&mut self, f: impl FnOnce(&mut System) -> Result<()>) -> Result<()> {
        // まとめた編集の中では複製しない（失敗すれば batch が系ごと捨てる）
        if self.batch != Batch::Off {
            f(self)?;
            return self.refresh();
        }
        let mut s = self.clone();
        f(&mut s)?;
        s.refresh()?;
        *self = s;
        Ok(())
    }

    /// 編集の後始末: 面の値の確かめ、材料からの屈折率、ピックアップの解決、厚さのソルブ、系の確かめ
    pub(crate) fn refresh(&mut self) -> Result<()> {
        self.invalidate_frames();
        let ns = self.surfaces.len();
        if ns < 3 {
            invalid!("too few surfaces ({ns})");
        }
        if self.surfaces.iter().filter(|d| d.stop).count() != 1 {
            invalid!("there must be exactly one stop surface");
        }
        for (j, d) in self.surfaces.iter().enumerate() {
            type_data_ok(d)?;
            match d.mat {
                Material::Index(n) if !(n.is_finite() && n > 0.0) => {
                    invalid!("surface {j} index {n}")
                }
                Material::Model { nd, vd, dpgf }
                    if !(nd.is_finite() && nd > 0.0 && vd.is_finite() && dpgf.is_finite()) =>
                {
                    invalid!("surface {j}: model glass nd {nd}, vd {vd}, dpgf {dpgf}")
                }
                _ => {}
            }
            if d.typ() == SurfType::CoordBrk && !no_material(&d.mat) {
                invalid!("surface {j}: material {} on a coordinate break", d.mat);
            }
            if j > 0 && !d.t.is_finite() {
                invalid!("surface {j} thickness {}", d.t);
            }
            for (name, v) in [("c", d.c), ("k", d.k), ("semi", d.semi)] {
                if !v.is_finite() {
                    invalid!("surface {j} {name} = {v}");
                }
            }
        }
        if self.glrs >= ns {
            invalid!(
                "global coordinate reference surface {} ({ns} surfaces)",
                self.glrs
            );
        }
        self.pick_names()?;
        self.refresh_media()?;
        self.resolve_pickups()?;
        self.resolve_media()?;
        if !self.solves.is_empty() || !self.macro_parms.is_empty() {
            self.solve_thickness()?;
        }
        self.check_configs()?;
        self.changed()?;
        // 自動の半直径を光線から求め直す
        self.update_semi();
        Ok(())
    }

    /// ガラスを足す（同じ名前は置き換える）。屈折率は材料を設定したときに作る
    pub fn add_glasses(&mut self, g: HashMap<String, Glass>) {
        std::sync::Arc::make_mut(&mut self.rx_mut().glasses).extend(g);
        self.invalidate_frames();
    }

    /// カタログ（AGF の名前、例 "SCHOTT"）を探す場所 dirs から読んで足す。見つからないカタログの名前を返す
    pub fn add_glass_catalogs(&mut self, names: &[String], dirs: &[PathBuf]) -> Vec<String> {
        let (g, missing) = glass::load_catalogs(names, &glass::catalog_dirs(dirs));
        self.add_glasses(g);
        missing
    }

    // ------------------------------------------------------------------------------ 面の挿入・削除
    /// 面 at の前に面 d を入れる（at は 1〜像面）。d.n が空なら材料から屈折率を作る
    pub fn insert_surface(&mut self, at: usize, d: Surface) -> Result<()> {
        if at == 0 || at > self.image() {
            invalid!("position {at} to insert a surface (1 to {})", self.image());
        }
        if d.stop {
            invalid!("make the inserted surface the stop with set_stop");
        }
        self.edit_sys(|s| {
            let mut d = d;
            if d.is_mirror() {
                d.mat = "MIRROR".into();
            }
            s.rx_mut().surfaces.insert(at, d);
            renumber(s, |j| Some(if j >= at { j + 1 } else { j }), at, true);
            Ok(())
        })
    }

    /// 面 j の値をまとめて変える（名前は [`SURF_FIELDS`]。"r" は曲率半径で 0 なら平面、"type" は最初に当てる）。
    /// 厚さのソルブ・ピックアップで決まる値は変えられない。material は [`System::set_material`] と同じ
    pub fn set_surface_fields(&mut self, j: usize, fields: &[(String, SurfValue)]) -> Result<()> {
        if j > self.image() {
            invalid!("surface {j} (surfaces are 0 to {})", self.image());
        }
        for (name, _) in fields {
            if !SURF_FIELDS.contains(&name.as_str()) {
                invalid!("surface value name {name} ({})", SURF_FIELDS.join(" "));
            }
            if let Some(pk) = self
                .pickups
                .iter()
                .find(|q| q.dst == j && field_takes(name, q.what))
            {
                invalid!("surface {j} {name} is a pickup from surface {}", pk.src);
            }
            // 曲率・厚さ・円錐係数のソルブ（"r" は曲率のソルブ）
            let kind =
                field_qty(name).filter(|k| matches!(k, Kind::Curv | Kind::Thick | Kind::Conic));
            if let Some(k) = kind {
                if self
                    .solves
                    .iter()
                    .any(|q| q.surf == j && q.kind.param_kind() == k)
                {
                    invalid!("surface {j} {name} is set by a solve");
                }
            }
        }
        // 種類・材料・値を 1 つの複製に当て、後始末は最後に 1 回
        let typ = match fields.iter().find(|(n, _)| n == "type") {
            Some((_, v)) => {
                let t = surf_type_parse(&v.text("type")?)?;
                self.check_surface_type(j, t)?;
                Some(t)
            }
            None => None,
        };
        let mat = match fields.iter().find(|(n, _)| n == "material") {
            Some((_, v)) => {
                let txt = v.text("material")?;
                Some((self.check_material(j, &txt)?, txt))
            }
            None => None,
        };
        self.edit_sys(|s| {
            let last = s.surfaces.len() - 1;
            let d = &mut s.rx_mut().surfaces[j];
            if let Some(t) = typ {
                set_type(d, t);
            }
            if let Some((m, txt)) = mat {
                set_material(d, j, m, &txt)?;
            }
            for (name, v) in fields {
                match name.as_str() {
                    "type" | "material" => {}
                    "comment" => d.comment = v.text(name)?,
                    "t" => {
                        let t = v.num(name)?;
                        if j == 0 && t.is_infinite() && t > 0.0 {
                            d.t = t;
                        } else if !t.is_finite() {
                            invalid!("surface {j} thickness {t}");
                        } else {
                            d.t = t;
                        }
                    }
                    "c" => d.c = v.num(name)?,
                    "r" => {
                        let r = v.num(name)?;
                        d.c = if r == 0.0 || r.is_infinite() {
                            0.0
                        } else {
                            1.0 / r
                        };
                    }
                    "k" => d.k = v.num(name)?,
                    "asph" => {
                        let a = v.nums(name, None)?;
                        if !d.set_asph(a) {
                            invalid!(
                                "surface {j} ({:?}) can't hold these asphere coefficients (asph)",
                                d.typ()
                            );
                        }
                    }
                    "cx" => {
                        let x = v.num(name)?;
                        match d.cx_mut() {
                            Some(c) => *c = x,
                            // ほかの種類の 0 は持たない値と同じ
                            None if x == 0.0 => {}
                            None => invalid!("surface {j} is not a toroidal surface (cx)"),
                        }
                    }
                    "coating" => d.coat = v.text(name)?.trim().to_string(),
                    // 数なら固定、"auto" なら自動
                    "semi" => match v {
                        SurfValue::Text(t) if t.trim().eq_ignore_ascii_case("auto") => {
                            d.semi_auto = true
                        }
                        _ => {
                            d.semi = v.num(name)?;
                            d.semi_auto = false;
                        }
                    },
                    "semi_auto" => d.semi_auto = v.flag(name)?,
                    // 機械半径: 数なら固定、"auto" なら自動（読む値は玉の半径 [`System::lens_radius`]）
                    "mech" => match v {
                        SurfValue::Text(t) if t.trim().eq_ignore_ascii_case("auto") => {
                            d.mech_semi = None
                        }
                        _ => d.mech_semi = Some(v.num(name)?),
                    },
                    "mech_auto" => {
                        if v.flag(name)? {
                            d.mech_semi = None
                        }
                    }
                    "chip" => d.chip_zone = v.num(name)?,
                    "ignore" => {
                        if j == 0 || j == last {
                            invalid!("the object and image surfaces can't be ignored");
                        }
                        d.ignore = v.flag(name)?
                    }
                    "aperture" => match v {
                        SurfValue::Aperture(a) => d.ap = *a,
                        _ => invalid!("surface value aperture"),
                    },
                    "cb" => {
                        if d.typ() != SurfType::CoordBrk {
                            invalid!("surface {j} is not a coordinate break (cb)");
                        }
                        let v = v.nums(name, Some(5))?;
                        if let Some((cb, _)) = d.cb_mut() {
                            cb.copy_from_slice(&v);
                        }
                    }
                    "cb_order" => {
                        let o = v.num(name)?;
                        if o != 0.0 && o != 1.0 {
                            invalid!("cb_order must be 0 or 1 ({o})");
                        }
                        match d.cb_mut() {
                            Some((_, order)) => *order = o as u8,
                            None if o == 0.0 => {}
                            None => invalid!("surface {j} is not a coordinate break (cb_order)"),
                        }
                    }
                    // 座標の戻り [戻し方 0〜3, 戻り先の面]（0 は戻さない）
                    "cb_return" => {
                        use crate::coord_return::{CoordReturn, ReturnMode};
                        let v = v.nums(name, Some(2))?;
                        let Some(mode) = (v[0].fract() == 0.0)
                            .then(|| ReturnMode::from_os_code(v[0] as i64))
                            .flatten()
                        else {
                            invalid!("cb_return mode {} (0 to 3)", v[0]);
                        };
                        if mode == ReturnMode::Off {
                            d.coord_return = None;
                        } else {
                            if d.typ() != SurfType::CoordBrk {
                                invalid!("surface {j} is not a coordinate break (cb_return)");
                            }
                            if v[1].fract() != 0.0 || !(0.0..j as f64).contains(&v[1]) {
                                invalid!(
                                    "cb_return surface {} (0 to {})",
                                    v[1],
                                    j.saturating_sub(1)
                                );
                            }
                            d.coord_return = Some(CoordReturn {
                                mode,
                                surf: v[1] as usize,
                            });
                        }
                    }
                    "focal" => {
                        let f = v.num(name)?;
                        match d.paraxial_mut() {
                            Some((focal, _)) => *focal = f,
                            None if f == 0.0 => {}
                            None => invalid!("surface {j} is not a paraxial lens (focal)"),
                        }
                    }
                    "jones" => {
                        let v = v.nums(name, Some(8))?;
                        match d.jones_mut() {
                            Some(m) => m.copy_from_slice(&v),
                            None if v == crate::surface::JONES_IDENTITY => {}
                            None => invalid!("surface {j} is not a Jones surface (jones)"),
                        }
                        d.retarder = None;
                    }
                    "dn" | "disp" => {
                        if d.is_mirror() || d.typ() == SurfType::CoordBrk {
                            invalid!("surface {j} is a mirror or a coordinate break ({name})");
                        }
                        let x = v.num(name)?;
                        if name == "dn" {
                            d.dn = x
                        } else {
                            d.disp = x
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        })
    }

    /// 面 j の値（名前は [`SURF_FIELDS`] と "mirror"・"stop"・"n"）
    pub fn surface_fields(&self, j: usize) -> Result<Vec<(&'static str, SurfValue)>> {
        let Some(d) = self.surfaces.get(j) else {
            invalid!("surface {j} (surfaces are 0 to {})", self.image());
        };
        use SurfValue::*;
        Ok(vec![
            ("type", Text(surf_type_name(d.typ()).into())),
            ("comment", Text(d.comment.clone())),
            ("t", Num(d.t)),
            ("c", Num(d.c)),
            ("r", Num(if d.c == 0.0 { f64::INFINITY } else { 1.0 / d.c })),
            ("k", Num(d.k)),
            ("asph", Nums(d.asph().to_vec())),
            ("cx", Num(d.cx())),
            ("material", Text(d.mat.name())),
            ("mirror", Bool(d.is_mirror())),
            ("coating", Text(d.coat.clone())),
            ("semi", Num(d.semi)),
            ("semi_auto", Bool(d.semi_auto)),
            ("mech", Num(self.lens_radius(j))),
            ("mech_auto", Bool(d.mech_semi.is_none())),
            ("chip", Num(d.chip_zone)),
            ("stop", Bool(d.stop)),
            ("ignore", Bool(d.ignore)),
            ("aperture", Aperture(d.ap)),
            ("cb", Nums(d.cb().to_vec())),
            ("cb_order", Num(d.cb_order() as f64)),
            (
                "cb_return",
                Nums(d.coord_return.map_or(vec![0.0, 0.0], |c| {
                    vec![c.mode.os_code() as f64, c.surf as f64]
                })),
            ),
            ("focal", Num(d.focal())),
            ("jones", Nums(d.jones().to_vec())),
            ("dn", Num(d.dn)),
            ("disp", Num(d.disp)),
            ("n", Nums(self.n(j).to_vec())),
        ])
    }

    /// 面 j（1〜像面の 1 つ前）を消す。消した面を参照するピックアップ・ソルブ・多重構成の行は外し、その説明を返す
    pub fn delete_surface(&mut self, j: usize) -> Result<Vec<String>> {
        if j == 0 || j >= self.image() {
            invalid!("surface {j} to delete (1 to {})", self.image() - 1);
        }
        if self.surfaces[j].stop {
            invalid!("surface {j} is the stop (make another surface the stop with set_stop first)");
        }
        if let FieldType::Solve(c) = self.field_type {
            let used = match c {
                FieldCond::Position { surf } => surf == j,
                FieldCond::DirSum { s1, s2 } | FieldCond::DirDiff { s1, s2 } => s1 == j || s2 == j,
            };
            if used {
                invalid!("surface {j} is used by the field setting (change the field type first)");
            }
        }
        let mut notes = Vec::new();
        self.edit_sys(|s| {
            notes = drop_refs(s, j);
            s.rx_mut().surfaces.remove(j);
            renumber(s, |k| Some(if k > j { k - 1 } else { k }), j, false);
            Ok(())
        })?;
        Ok(notes)
    }

    /// 面 j を絞りにする
    pub fn set_stop(&mut self, j: usize) -> Result<()> {
        if j == 0 || j >= self.image() {
            invalid!("stop surface {j} (1 to {})", self.image() - 1);
        }
        self.edit_sys(|s| {
            for (k, d) in s.rx_mut().surfaces.iter_mut().enumerate() {
                d.stop = k == j;
            }
            Ok(())
        })
    }

    /// 面 j の種類を変える。種類で意味の変わる値（曲率・円錐係数・非球面、座標ブレークの値、傾き偏心）は、
    /// 新しい種類に無いものを 0 に戻す。形のデータの要る種類（Zernike・Grid Sag など）は、その面がすでに持つときだけ
    pub fn set_surface_type(&mut self, j: usize, typ: SurfType) -> Result<()> {
        self.check_surface_type(j, typ)?;
        self.edit_sys(|s| {
            set_type(&mut s.rx_mut().surfaces[j], typ);
            Ok(())
        })
    }

    /// [`Self::set_surface_type`] の前の確かめ
    fn check_surface_type(&self, j: usize, typ: SurfType) -> Result<()> {
        if j == 0 || j > self.image() {
            invalid!("surface {j}");
        }
        if typ == SurfType::CoordBrk && (j == self.image() || self.surfaces[j].stop) {
            invalid!("surface {j} (image or stop) can't be a coordinate break");
        }
        Ok(())
    }

    /// 面 j の材料（ガラス名・"nd,vd[,dpgf]"・空か AIR・MIRROR）を変える。dn・disp は残す
    pub fn set_material(&mut self, j: usize, mat: &str) -> Result<()> {
        let m = self.check_material(j, mat)?;
        self.edit_sys(|s| set_material(&mut s.rx_mut().surfaces[j], j, m, mat))
    }

    /// [`Self::set_material`] の前の確かめ。読んだ材料を返す
    fn check_material(&self, j: usize, mat: &str) -> Result<Material> {
        if j > self.image() {
            invalid!("surface {j}");
        }
        let m = Material::parse(mat);
        if j == 0 && m.is_mirror() {
            invalid!("the object surface can't be a mirror");
        }
        Ok(m)
    }

    /// 面 j の膜（膜ファイルの名前、空で無し）
    pub fn set_coating(&mut self, j: usize, coat: &str) -> Result<()> {
        if j > self.image() {
            invalid!("surface {j}");
        }
        self.edit_sys(|s| {
            s.rx_mut().surfaces[j].coat = coat.trim().to_string();
            Ok(())
        })
    }

    /// 面 j の開口（[`Aperture::None`] で無し）
    pub fn set_aperture(&mut self, j: usize, ap: Aperture) -> Result<()> {
        if j > self.image() {
            invalid!("surface {j}");
        }
        self.edit_sys(|s| {
            s.rx_mut().surfaces[j].ap = ap;
            Ok(())
        })
    }

    // ------------------------------------------------------------------------------ ピックアップ・ソルブ
    /// ピックアップを足す（同じ先・同じ量のものは置き換える）。先の値はすぐに元から決まる
    pub fn add_pickup(&mut self, p: Pickup) -> Result<()> {
        let ns = self.surfaces.len();
        if p.src >= ns || p.dst >= ns || p.src == p.dst {
            invalid!("pickup surfaces (from {}, to {})", p.src, p.dst);
        }
        if !p.scale.is_finite() || !p.offset.is_finite() {
            invalid!("pickup scale {} and offset {}", p.scale, p.offset);
        }
        if let Some(k) = solve_kind_of(p.what) {
            if self
                .solves
                .iter()
                .any(|q| q.surf == p.dst && q.kind.param_kind() == k)
            {
                invalid!(
                    "surface {} {:?} is set by a solve (remove_solve first)",
                    p.dst,
                    p.what
                );
            }
        }
        if let Some(v) = p.value_in(self) {
            if !v.is_finite() {
                invalid!(
                    "the pickup value is not finite (from surface {} {v})",
                    p.src
                );
            }
        }
        self.edit_sys(|s| {
            s.rx_mut()
                .pickups
                .retain(|q| !(q.dst == p.dst && q.what == p.what));
            s.rx_mut().pickups.push(p);
            Ok(())
        })
    }

    /// 面 dst の量 what のピックアップを外す（値は今の値のまま残る）。無ければエラー
    pub fn remove_pickup(&mut self, dst: usize, what: crate::param::Kind) -> Result<()> {
        if !self.pickups.iter().any(|q| q.dst == dst && q.what == what) {
            invalid!("no pickup of {what:?} on surface {dst}");
        }
        self.edit_sys(|s| {
            s.rx_mut()
                .pickups
                .retain(|q| !(q.dst == dst && q.what == what));
            Ok(())
        })
    }

    /// ソルブ（厚さか曲率、[`crate::solves::SolveKind`]）を足す（同じ面・同じ量のものは置き換える）。値はすぐに解き直す
    pub fn add_solve(&mut self, q: Solve) -> Result<()> {
        let ns = self.surfaces.len();
        if q.surf == 0 || q.surf + 1 >= ns || q.kind.other_surface().is_some_and(|f| f >= ns) {
            invalid!("solve surface {} (1 to {})", q.surf, ns - 2);
        }
        let pk = q.kind.param_kind();
        self.check_param(Param::new(q.surf, pk))?;
        if !q.kind.value().is_finite() {
            invalid!("solve value {}", q.kind.value());
        }
        if self
            .pickups
            .iter()
            .any(|p| p.dst == q.surf && solve_kind_of(p.what) == Some(pk))
        {
            invalid!(
                "surface {} {} is set by a pickup (remove_pickup first)",
                q.surf,
                pk.name()
            );
        }
        self.edit_sys(|s| {
            s.rx_mut()
                .solves
                .retain(|x| !(x.surf == q.surf && x.kind.param_kind() == pk));
            s.rx_mut().solves.push(q);
            Ok(())
        })
    }

    /// 円錐係数・半直径・機械半径・面タイプのパラメータの ZPL マクロのソルブ（[`crate::zpl_solve::MacroParm`]）を足す（同じ面・
    /// 同じ量のものは置き換える）。マクロは [`System::macros`] の番号。値はすぐに解き直す
    pub fn add_macro_parm(&mut self, q: crate::zpl_solve::MacroParm) -> Result<()> {
        let ns = self.surfaces.len();
        if q.surf == 0 || q.surf >= ns {
            invalid!("macro solve surface {} (1 to {})", q.surf, ns - 1);
        }
        if q.id as usize >= self.macros.len() {
            invalid!("no macro {} (the system has {})", q.id, self.macros.len());
        }
        if !matches!(
            q.what,
            Kind::Conic | Kind::Semi | Kind::MechSemi | Kind::Parm(_)
        ) {
            invalid!(
                "a macro solve sets k, semi, mech or p1.. (not {})",
                q.what.name()
            );
        }
        if !q.what.is_pick_only() {
            self.check_param(Param::new(q.surf, q.what))?;
        }
        self.edit_sys(|s| {
            s.rx_mut()
                .macro_parms
                .retain(|x| !(x.surf == q.surf && x.what == q.what));
            s.rx_mut().macro_parms.push(q);
            Ok(())
        })
    }

    /// 面 surf の量 kind（[`Kind::Thick`] か [`Kind::Curv`]）のソルブを外す（値は今のまま残る）。無ければエラー
    pub fn remove_solve(&mut self, surf: usize, kind: Kind) -> Result<()> {
        let hit = |x: &Solve| x.surf == surf && x.kind.param_kind() == kind;
        if !self.solves.iter().any(hit) {
            invalid!("no solve of {} on surface {surf}", kind.name());
        }
        self.edit_sys(|s| {
            s.rx_mut().solves.retain(|x| !hit(x));
            Ok(())
        })
    }
}

/// 面 j を消す前に、j を参照するピックアップ・ソルブ・表の行・玉・群の範囲を外す。説明を返す
fn drop_refs(s: &mut System, j: usize) -> Vec<String> {
    let mut notes = Vec::new();
    for (k, d) in s.rx_mut().surfaces.iter_mut().enumerate() {
        if d.coord_return.is_some_and(|c| c.surf == j) {
            d.coord_return = None;
            notes.push(format!(
                "removed the coordinate return of surface {k} (to surface {j})"
            ));
        }
    }
    s.rx_mut().pickups.retain(|p| {
        let used = p.src == j || p.dst == j;
        if used {
            notes.push(format!(
                "removed the pickup of surface {} {:?} (from surface {} {:?})",
                p.dst,
                p.what,
                p.src,
                p.from.unwrap_or(p.what)
            ));
        }
        !used
    });
    s.rx_mut().solves.retain(|q| {
        let used = q.surf == j || q.kind.other_surface() == Some(j);
        if used {
            notes.push(if q.kind.is_curv() {
                format!(
                    "removed the curvature solve of surface {} ({:?})",
                    q.surf, q.kind
                )
            } else {
                format!(
                    "removed the thickness solve of surface {} ({:?})",
                    q.surf, q.kind
                )
            });
        }
        !used
    });
    // 玉・群: 範囲を縮め、空になった範囲・玉・群は外す
    let mut unit_map: Vec<Option<usize>> = Vec::with_capacity(s.units.len());
    let mut kept = Vec::new();
    for u in std::mem::take(&mut s.rx_mut().units) {
        let mut u = u;
        u.spans.retain_mut(|sp| {
            let [a, b] = *sp;
            let a2 = if a > j { a - 1 } else { a };
            let b2 = if b >= j && b > 0 { b - 1 } else { b };
            if a == j && b == j {
                return false;
            }
            // 付け替えは renumber でなくここで済ませる（範囲の端の扱いが違う）
            *sp = [a2, b2];
            true
        });
        if u.spans.is_empty() {
            notes.push(format!(
                "removed lens or group {} (its surfaces are gone)",
                u.name
            ));
            unit_map.push(None);
        } else {
            unit_map.push(Some(kept.len()));
            kept.push(u);
        }
    }
    s.rx_mut().units = kept;
    if let Some(c) = &mut s.rx_mut().configs {
        c.rows.retain(|r| {
            let used = match r.param.target {
                Target::Unit(u) => unit_map.get(u).copied().flatten().is_none(),
                Target::System => false,
                Target::Surf(k) => k == j,
            };
            if used {
                notes.push(format!(
                    "removed multi-configuration row {}",
                    r.param.name()
                ));
            }
            !used
        });
        // 消した行を元にする別の行のピックアップは、値を残して外す
        let gone = |q: &Param| match q.target {
            Target::Unit(u) => unit_map.get(u).copied().flatten().is_none(),
            Target::System => false,
            Target::Surf(k) => k == j,
        };
        for r in c.rows.iter_mut() {
            r.picks.retain(|q| !q.row.as_ref().is_some_and(gone));
            for q in r.picks.iter_mut() {
                if let Some(Target::Unit(u)) = q.row.as_mut().map(|x| &mut x.target) {
                    if let Some(Some(k)) = unit_map.get(*u) {
                        *u = *k;
                    }
                }
            }
        }
        for r in c.rows.iter_mut() {
            if let Target::Unit(u) = &mut r.param.target {
                if let Some(Some(k)) = unit_map.get(*u) {
                    *u = *k;
                }
            }
        }
    }
    // 機構部品: 置く面を消したら外す。保持する面からは外す（番号の付け替えは renumber）
    s.rx_mut().mechanics.retain_mut(|m| {
        m.holds.retain(|&h| h != j);
        if m.surface == j {
            notes.push(format!(
                "removed mechanical part {} (its surface {j} was deleted)",
                m.name
            ));
            return false;
        }
        true
    });
    if s.glrs == j {
        notes.push(format!(
            "the global coordinate reference surface {j} was deleted, so it is now surface 1"
        ));
        s.rx_mut().glrs = 1;
    }
    notes
}

/// 面番号の付け替え（挿入・削除）。玉・群の範囲は、挿入なら at に入れた面を、範囲の最初が at なら範囲に含めない
/// （範囲の前に入る）。削除の範囲は [`drop_refs`] で済ませている
fn renumber(s: &mut System, m: impl Fn(usize) -> Option<usize>, at: usize, insert: bool) {
    let mm = |j: usize| m(j).unwrap_or(j);
    for d in s.rx_mut().surfaces.iter_mut() {
        if let Some(c) = &mut d.coord_return {
            c.surf = mm(c.surf);
        }
    }
    for p in s.rx_mut().pickups.iter_mut() {
        p.src = mm(p.src);
        p.dst = mm(p.dst);
    }
    for q in s.rx_mut().solves.iter_mut() {
        q.surf = mm(q.surf);
        if let Some(o) = q.kind.other_surface() {
            q.kind = q.kind.with_other_surface(mm(o));
        }
    }
    for m in s.rx_mut().mechanics.iter_mut() {
        m.surface = mm(m.surface);
        for h in m.holds.iter_mut() {
            *h = mm(*h);
        }
    }
    if insert {
        for u in s.rx_mut().units.iter_mut() {
            for sp in u.spans.iter_mut() {
                // 範囲の中（最初の面より後ろ、最後の面まで）に入れた面は範囲に含める
                let [a, b] = *sp;
                *sp = [mm(a), if b >= at { b + 1 } else { b }];
            }
        }
        if s.glrs >= at {
            s.rx_mut().glrs += 1;
        }
    } else if s.glrs > at {
        s.rx_mut().glrs -= 1;
    }
    s.rx_mut().field_type = match s.field_type {
        FieldType::Solve(FieldCond::Position { surf }) => {
            FieldType::Solve(FieldCond::Position { surf: mm(surf) })
        }
        FieldType::Solve(FieldCond::DirSum { s1, s2 }) => FieldType::Solve(FieldCond::DirSum {
            s1: mm(s1),
            s2: mm(s2),
        }),
        FieldType::Solve(FieldCond::DirDiff { s1, s2 }) => FieldType::Solve(FieldCond::DirDiff {
            s1: mm(s1),
            s2: mm(s2),
        }),
        t => t,
    };
    if let Some(c) = &mut s.rx_mut().configs {
        for r in c.rows.iter_mut() {
            if let Target::Surf(k) = &mut r.param.target {
                *k = mm(*k);
            }
            for q in r.picks.iter_mut() {
                if let Some(Target::Surf(k)) = q.row.as_mut().map(|x| &mut x.target) {
                    *k = mm(*k);
                }
            }
        }
    }
}

/// 面の種類を変える（[`System::set_surface_type`] の中身）
fn set_type(d: &mut Surface, typ: SurfType) {
    let old = d.typ();
    // 種類ごとの値は新しい種類の既定に（持ち越せる Zernike の項・屈折率分布は持ち越す）
    d.set_type(typ);
    if typ == SurfType::CoordBrk {
        d.c = 0.0;
        d.k = 0.0;
        d.mat = Material::Air;
        d.before = None;
        d.after = After::None;
        d.dn = 0.0;
        d.disp = 0.0;
        d.dnd = 0.0;
        d.dvd = 0.0;
    } else if old == SurfType::CoordBrk && d.mat == Material::Inherit {
        d.mat = Material::Air;
    }
    if typ == SurfType::Jones {
        d.c = 0.0;
        d.k = 0.0;
    }
}

/// 面 j の材料を変える（[`System::set_material`] の中身）。mat は誤りの文に書く元の綴り
fn set_material(d: &mut Surface, j: usize, m: Material, mat: &str) -> Result<()> {
    if d.typ() == SurfType::CoordBrk && !no_material(&m) {
        invalid!("surface {j}: material {mat} on a coordinate break");
    }
    let mirror = m.is_mirror();
    // 硝材の nd・vd のずれはガラスとモデルガラスだけ
    let glassy = matches!(m, Material::Glass(_) | Material::Model { .. });
    d.mat = m;
    if mirror {
        d.dn = 0.0;
        d.disp = 0.0;
    }
    if !glassy {
        d.dnd = 0.0;
        d.dvd = 0.0;
    }
    Ok(())
}
