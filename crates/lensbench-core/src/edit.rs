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
use crate::param::{Kind, Param, PickKind, Pickup, ThickSolve};
use crate::surface::{After, Aperture, SurfType, Surface};
use crate::system::{FieldCond, FieldType, System};
use crate::{invalid, notimpl};
use std::collections::HashMap;
use std::path::PathBuf;

/// 多重構成の表（OpticStudio の Multi-Configuration Editor の数の行に当たる）。
/// 行ごとにパラメータ 1 つと、構成ごとの値を持つ。[`System::config`] で構成 k の値を入れた系を作る
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Configs {
    /// 構成の数（1 以上）
    pub n: usize,
    /// 今の系に入っている構成（1 始まり）
    pub active: usize,
    pub rows: Vec<ConfigRow>,
}

/// 多重構成の表の 1 行
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ConfigRow {
    pub param: Param,
    /// 構成ごとの値（長さは構成の数）
    pub values: Vec<f64>,
}

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
        _ => invalid!("面タイプ {s}"),
    })
}

/// 面タイプの .zmx の名前
pub fn surf_type_name(t: SurfType) -> &'static str {
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
    }
}

/// 面の種類の形のデータが揃っているか
fn type_data_ok(d: &Surface) -> Result<()> {
    let ok = match d.typ {
        SurfType::ZernikeStd | SurfType::ZernikeFringe => d.zern.is_some(),
        SurfType::GridSag => d.grid.is_some(),
        SurfType::CylFresnel => d.cylf.is_some(),
        SurfType::Binary2 => d.bin2.is_some(),
        SurfType::Gradium | SurfType::Gradient9 => d.grin.is_some(),
        SurfType::User => d.user.is_some(),
        _ => true,
    };
    if !ok {
        notimpl!(
            "面タイプ {} への変更（形のデータが要る。.zmx から読む）",
            surf_type_name(d.typ)
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

/// 材料なし（座標ブレークの材料。読込・展開では "-"）
fn no_material(m: &str) -> bool {
    matches!(m.trim(), "" | "-")
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
    "ignore",
    "aperture",
    "cb",
    "cb_order",
    "focal",
    "jones",
    "dn",
    "disp",
];

impl SurfValue {
    fn num(&self, name: &str) -> Result<f64> {
        match self {
            SurfValue::Num(v) => Ok(*v),
            _ => invalid!("面の値 {name} は数"),
        }
    }
    fn nums(&self, name: &str, n: Option<usize>) -> Result<Vec<f64>> {
        let v = match self {
            SurfValue::Nums(v) => v.clone(),
            SurfValue::Num(x) if n.is_none() => vec![*x],
            _ => invalid!("面の値 {name} は数の並び"),
        };
        if let Some(n) = n {
            if v.len() != n {
                invalid!("面の値 {name} は {n} 個（{} 個）", v.len());
            }
        }
        if v.iter().any(|x| !x.is_finite()) {
            invalid!("面の値 {name} = {v:?}");
        }
        Ok(v)
    }
    fn text(&self, name: &str) -> Result<String> {
        match self {
            SurfValue::Text(v) => Ok(v.clone()),
            _ => invalid!("面の値 {name} は文字列"),
        }
    }
    fn flag(&self, name: &str) -> Result<bool> {
        match self {
            SurfValue::Bool(v) => Ok(*v),
            _ => invalid!("面の値 {name} は真偽"),
        }
    }
}

impl System {
    /// 系を複製して f で変え、編集の後始末（[`System::refresh`]）をしてから差し替える。失敗なら元のまま
    pub fn edit(&mut self, f: impl FnOnce(&mut System) -> Result<()>) -> Result<()> {
        let mut s = self.clone();
        f(&mut s)?;
        s.refresh()?;
        *self = s;
        Ok(())
    }

    /// 編集の後始末: 面の値の確かめ、材料からの屈折率、ピックアップの解決、厚さのソルブ、系の確かめ。
    /// 面のデータを直接書き換えたときに呼ぶ
    pub fn refresh(&mut self) -> Result<()> {
        self.invalidate_frames();
        let ns = self.surfaces.len();
        if ns < 3 {
            invalid!("面が少なすぎる（{ns}）");
        }
        if self.surfaces.iter().filter(|d| d.stop).count() != 1 {
            invalid!("絞り面がちょうど 1 つでない");
        }
        let nw = self.wavelengths.len();
        for (j, d) in self.surfaces.iter().enumerate() {
            type_data_ok(d)?;
            if d.typ == SurfType::CoordBrk && (d.mirror || !no_material(&d.mat)) {
                invalid!("面 {j}: 座標ブレークに材料 {}", d.mat);
            }
            if d.typ == SurfType::Jones && d.c != 0.0 {
                notimpl!("曲面の Jones 行列面 {j}");
            }
            if j > 0 && !d.t.is_finite() {
                invalid!("面 {j} の厚さ {}", d.t);
            }
            for (name, v) in [("c", d.c), ("k", d.k), ("semi", d.semi)] {
                if !v.is_finite() {
                    invalid!("面 {j} の {name} = {v}");
                }
            }
            if !d.n.is_empty() && d.n.len() != nw {
                invalid!("面 {j}: 屈折率の数 {}（波長 {nw}）", d.n.len());
            }
        }
        if self.glrs >= ns {
            invalid!("大域座標の基準面 {}（面 {ns} 個）", self.glrs);
        }
        self.pick_names()?;
        self.refresh_media()?;
        self.resolve_pickups()?;
        self.update_indices();
        self.propagate_media();
        if !self.solves.is_empty() || !self.macro_parms.is_empty() {
            self.solve_thickness()?;
        }
        self.check_configs()?;
        self.changed()?;
        // 自動の半直径を光線から求め直す
        self.update_semi();
        Ok(())
    }

    /// 材料の屈折率（波長ごと、dn・disp をかける前）。空・AIR は 1、"nd,vd[,dpgf]" はモデルガラス、
    /// ほかは系のガラス（[`System::add_glasses`] で足せる）
    pub fn material_index(&self, mat: &str) -> Result<Vec<f64>> {
        let m = mat.trim();
        if m.is_empty() || m.eq_ignore_ascii_case("AIR") {
            return Ok(vec![1.0; self.wavelengths.len()]);
        }
        if m.eq_ignore_ascii_case("MIRROR") {
            invalid!("MIRROR は媒質でない");
        }
        if let Some(g) = self.glasses.get(m) {
            let (t, p, a) = (self.temperature, self.pressure, self.adjust_index);
            return Ok(self
                .wavelengths
                .iter()
                .map(|&l| g.index(l, t, p, a))
                .collect());
        }
        let v: Vec<f64> = m
            .split(',')
            .map(|x| x.trim().parse::<f64>())
            .collect::<std::result::Result<_, _>>()
            .unwrap_or_default();
        if (2..=3).contains(&v.len()) && v.iter().all(|x| x.is_finite()) && v[0] > 0.0 {
            let (nd, vd, dp) = (v[0], v[1], v.get(2).copied().unwrap_or(0.0));
            return Ok(self
                .wavelengths
                .iter()
                .map(|&l| {
                    if vd == 0.0 {
                        nd
                    } else {
                        glass::model_index(nd, vd, dp, l)
                    }
                })
                .collect());
        }
        invalid!("ガラス {m} がない（カタログを足す: add_glasses・add_glass_catalogs）")
    }

    /// ガラスを足す（同じ名前は置き換える）。屈折率は材料を設定したときに作る
    pub fn add_glasses(&mut self, g: HashMap<String, Glass>) {
        std::sync::Arc::make_mut(&mut self.glasses).extend(g);
        self.invalidate_frames();
    }

    /// カタログ（AGF の名前、例 "SCHOTT"）を探す場所 dirs から読んで足す。見つからないカタログの名前を返す
    pub fn add_glass_catalogs(&mut self, names: &[String], dirs: &[PathBuf]) -> Vec<String> {
        let (g, missing) = glass::load_catalogs(names, &glass::catalog_dirs(dirs));
        self.add_glasses(g);
        missing
    }

    /// 材料の印 `n` が空の面（挿入した面・材料を変えた面）の屈折率を材料から作る。
    /// GRIN の面は頂点の屈折率
    pub(crate) fn refresh_media(&mut self) -> Result<()> {
        for j in 0..self.surfaces.len() {
            let d = &self.surfaces[j];
            if !d.n.is_empty() || (j > 0 && (d.mirror || d.typ == SurfType::CoordBrk)) {
                continue;
            }
            let base = if let Some(g) = &d.grin {
                self.wavelengths
                    .iter()
                    .map(|&l| {
                        g.vertex_index(l).ok_or_else(|| {
                            crate::error::Error::Invalid(format!(
                                "面 {j}: GRIN の頂点が分布の範囲の外"
                            ))
                        })
                    })
                    .collect::<Result<Vec<f64>>>()?
            } else {
                self.material_index(&d.mat)
                    .map_err(|e| crate::error::Error::Invalid(format!("面 {j}: {e}")))?
            };
            let d = &mut self.surfaces[j];
            d.n_base.clear();
            d.n = base;
            // 複屈折材の板の異常光の屈折率も材料（名前-E）から作り直す
            if let Some(crate::surface::Bire::In { ne, .. }) = &mut d.bire {
                ne.clear();
            }
        }
        let nw = self.wavelengths.len();
        for j in 0..self.surfaces.len() {
            let d = &self.surfaces[j];
            let Some(crate::surface::Bire::In { ne, .. }) = &d.bire else {
                continue;
            };
            if ne.len() == nw {
                continue;
            }
            let en = crate::surface::Bire::e_name(&d.mat);
            let v = self.material_index(&en).map_err(|e| {
                crate::error::Error::Invalid(format!("面 {j}: 複屈折材の異常光の材料 {en}: {e}"))
            })?;
            if let Some(crate::surface::Bire::In { ne, .. }) = &mut self.surfaces[j].bire {
                *ne = v;
            }
        }
        Ok(())
    }

    /// 鏡・座標ブレークの媒質を前の媒質（無視した面は飛ばす）にそろえる
    fn propagate_media(&mut self) {
        let mut prev: Vec<f64> = self.surfaces[0].n.clone();
        for j in 1..self.surfaces.len() {
            let d = &mut self.surfaces[j];
            if d.mirror || d.typ == SurfType::CoordBrk {
                d.n = prev.clone();
                d.n_base.clear();
            }
            if !d.ignore {
                prev = d.n.clone();
            }
        }
    }

    // ------------------------------------------------------------------------------ 面の挿入・削除
    /// 面 at の前に面 d を入れる（at は 1〜像面）。d.n が空なら材料から屈折率を作る
    pub fn insert_surface(&mut self, at: usize, d: Surface) -> Result<()> {
        if at == 0 || at > self.image() {
            invalid!("面を入れる位置 {at}（1〜{}）", self.image());
        }
        if d.stop {
            invalid!("入れる面を絞りにするのは set_stop で");
        }
        self.edit(|s| {
            let mut d = d;
            if d.mirror {
                d.mat = "MIRROR".into();
            }
            s.surfaces.insert(at, d);
            renumber(s, |j| Some(if j >= at { j + 1 } else { j }), at, true);
            Ok(())
        })
    }

    /// 面 j の値をまとめて変える（名前は [`SURF_FIELDS`]。"r" は曲率半径で 0 なら平面、"type" は最初に当てる）。
    /// 厚さのソルブ・ピックアップで決まる値は変えられない。material は [`System::set_material`] と同じ
    pub fn set_surface_fields(&mut self, j: usize, fields: &[(String, SurfValue)]) -> Result<()> {
        if j > self.image() {
            invalid!("面 {j}（面は 0〜{}）", self.image());
        }
        for (name, _) in fields {
            if !SURF_FIELDS.contains(&name.as_str()) {
                invalid!("面の値の名前 {name}（{}）", SURF_FIELDS.join(" "));
            }
            let kind = match name.as_str() {
                "c" | "r" => Some(Kind::Curv),
                "t" => Some(Kind::Thick),
                "k" => Some(Kind::Conic),
                _ => None,
            };
            if let Some(k) = kind {
                if let Some(pk) = self.pickup_of(Param::new(j, k)) {
                    invalid!("面 {j} の {name} は面 {} からのピックアップ", pk.src);
                }
            }
            let direct = |w: PickKind| match name.as_str() {
                "dn" => w == PickKind::Dn,
                "disp" => w == PickKind::Disp,
                "semi" => w == PickKind::Semi,
                "material" => w == PickKind::Dn,
                "coating" => w == PickKind::Coating,
                "asph" | "cb" | "cx" | "focal" | "jones" => matches!(w, PickKind::Parm(_)),
                _ => false,
            };
            if let Some(pk) = self.pickups.iter().find(|q| q.dst == j && direct(q.what)) {
                invalid!("面 {j} の {name} は面 {} からのピックアップ", pk.src);
            }
            if let Some(k) = kind {
                if self
                    .solves
                    .iter()
                    .any(|q| q.surf == j && q.kind.param_kind() == k)
                {
                    invalid!("面 {j} の {name} はソルブで決まる");
                }
            }
        }
        let mut s = self.clone();
        if let Some((_, v)) = fields.iter().find(|(n, _)| n == "type") {
            s.set_surface_type(j, surf_type_parse(&v.text("type")?)?)?;
        }
        if let Some((_, v)) = fields.iter().find(|(n, _)| n == "material") {
            s.set_material(j, &v.text("material")?)?;
        }
        s.edit(|s| {
            let last = s.surfaces.len() - 1;
            let d = &mut s.surfaces[j];
            for (name, v) in fields {
                match name.as_str() {
                    "type" | "material" => {}
                    "comment" => d.comment = v.text(name)?,
                    "t" => {
                        let t = v.num(name)?;
                        if j == 0 && t.is_infinite() && t > 0.0 {
                            d.t = t;
                        } else if !t.is_finite() {
                            invalid!("面 {j} の厚さ {t}");
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
                    "asph" => d.asph = v.nums(name, None)?,
                    "cx" => d.cx = v.num(name)?,
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
                    "ignore" => {
                        if j == 0 || j == last {
                            invalid!("物体面・像面は無視できない");
                        }
                        d.ignore = v.flag(name)?
                    }
                    "aperture" => match v {
                        SurfValue::Aperture(a) => d.ap = *a,
                        _ => invalid!("面の値 aperture"),
                    },
                    "cb" => {
                        if d.typ != SurfType::CoordBrk {
                            invalid!("面 {j} は座標ブレークでない（cb）");
                        }
                        let v = v.nums(name, Some(5))?;
                        d.cb.copy_from_slice(&v);
                    }
                    "cb_order" => {
                        let o = v.num(name)?;
                        if o != 0.0 && o != 1.0 {
                            invalid!("cb_order は 0 か 1（{o}）");
                        }
                        d.cb_order = o as u8;
                    }
                    "focal" => d.focal = v.num(name)?,
                    "jones" => {
                        let v = v.nums(name, Some(8))?;
                        d.jones.copy_from_slice(&v);
                        d.retarder = None;
                    }
                    "dn" | "disp" => {
                        if d.mirror || d.typ == SurfType::CoordBrk {
                            invalid!("面 {j} は鏡か座標ブレーク（{name}）");
                        }
                        let x = v.num(name)?;
                        if name == "dn" {
                            d.dn = x
                        } else {
                            d.disp = x
                        }
                        if d.n_base.is_empty() {
                            d.n_base = d.n.clone();
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        })?;
        *self = s;
        Ok(())
    }

    /// 面 j の値（名前は [`SURF_FIELDS`] と "mirror"・"stop"・"n"）
    pub fn surface_fields(&self, j: usize) -> Result<Vec<(&'static str, SurfValue)>> {
        let Some(d) = self.surfaces.get(j) else {
            invalid!("面 {j}（面は 0〜{}）", self.image());
        };
        use SurfValue::*;
        Ok(vec![
            ("type", Text(surf_type_name(d.typ).into())),
            ("comment", Text(d.comment.clone())),
            ("t", Num(d.t)),
            ("c", Num(d.c)),
            ("r", Num(if d.c == 0.0 { f64::INFINITY } else { 1.0 / d.c })),
            ("k", Num(d.k)),
            ("asph", Nums(d.asph.clone())),
            ("cx", Num(d.cx)),
            ("material", Text(d.mat.clone())),
            ("mirror", Bool(d.mirror)),
            ("coating", Text(d.coat.clone())),
            ("semi", Num(d.semi)),
            ("semi_auto", Bool(d.semi_auto)),
            ("stop", Bool(d.stop)),
            ("ignore", Bool(d.ignore)),
            ("aperture", Aperture(d.ap)),
            ("cb", Nums(d.cb.to_vec())),
            ("cb_order", Num(d.cb_order as f64)),
            ("focal", Num(d.focal)),
            ("jones", Nums(d.jones.to_vec())),
            ("dn", Num(d.dn)),
            ("disp", Num(d.disp)),
            ("n", Nums(d.n.clone())),
        ])
    }

    /// 面 j（1〜像面の 1 つ前）を消す。消した面を参照するピックアップ・ソルブ・多重構成の行は外し、その説明を返す
    pub fn delete_surface(&mut self, j: usize) -> Result<Vec<String>> {
        if j == 0 || j >= self.image() {
            invalid!("消す面 {j}（1〜{}）", self.image() - 1);
        }
        if self.surfaces[j].stop {
            invalid!("面 {j} は絞り（先に set_stop で別の面を絞りにする）");
        }
        if let FieldType::Solve(c) = self.field_type {
            let used = match c {
                FieldCond::Position { surf } => surf == j,
                FieldCond::DirSum { s1, s2 } | FieldCond::DirDiff { s1, s2 } => s1 == j || s2 == j,
            };
            if used {
                invalid!("面 {j} は視野の条件に使っている（先に視野の種類を変える）");
            }
        }
        let mut notes = Vec::new();
        self.edit(|s| {
            notes = drop_refs(s, j);
            s.surfaces.remove(j);
            renumber(s, |k| Some(if k > j { k - 1 } else { k }), j, false);
            Ok(())
        })?;
        Ok(notes)
    }

    /// 面 j を絞りにする
    pub fn set_stop(&mut self, j: usize) -> Result<()> {
        if j == 0 || j >= self.image() {
            invalid!("絞り面 {j}（1〜{}）", self.image() - 1);
        }
        self.edit(|s| {
            for (k, d) in s.surfaces.iter_mut().enumerate() {
                d.stop = k == j;
            }
            Ok(())
        })
    }

    /// 面 j の種類を変える。種類で意味の変わる値（曲率・円錐係数・非球面、座標ブレークの値、傾き偏心）は、
    /// 新しい種類に無いものを 0 に戻す。形のデータの要る種類（Zernike・Grid Sag など）は、その面がすでに持つときだけ
    pub fn set_surface_type(&mut self, j: usize, typ: SurfType) -> Result<()> {
        if j == 0 || j > self.image() {
            invalid!("面 {j}");
        }
        if typ == SurfType::CoordBrk && (j == self.image() || self.surfaces[j].stop) {
            invalid!("面 {j}（像面・絞り）を座標ブレークにはできない");
        }
        self.edit(|s| {
            let d = &mut s.surfaces[j];
            let old = d.typ;
            d.typ = typ;
            if typ == SurfType::CoordBrk {
                d.c = 0.0;
                d.k = 0.0;
                d.asph.clear();
                d.cx = 0.0;
                d.mat.clear();
                d.mirror = false;
                d.before = None;
                d.after = After::None;
                d.dn = 0.0;
                d.disp = 0.0;
                d.n_base.clear();
            } else if old == SurfType::CoordBrk {
                d.cb = [0.0; 5];
                d.cb_order = 0;
                // 前の媒質を引き継いでいた屈折率を材料（空なら空気）から作り直す
                d.n.clear();
            }
            if typ == SurfType::Jones {
                d.c = 0.0;
                d.k = 0.0;
                d.asph.clear();
            }
            if matches!(
                typ,
                SurfType::Standard | SurfType::CoordBrk | SurfType::Jones
            ) {
                d.asph.clear();
            }
            if typ != SurfType::Toroidal {
                d.cx = 0.0;
            }
            Ok(())
        })
    }

    /// 面 j の材料（ガラス名・"nd,vd[,dpgf]"・空か AIR・MIRROR）を変える。dn・disp は残す
    pub fn set_material(&mut self, j: usize, mat: &str) -> Result<()> {
        if j > self.image() {
            invalid!("面 {j}");
        }
        let mirror = mat.trim().eq_ignore_ascii_case("MIRROR");
        if j == 0 && mirror {
            invalid!("物体面を鏡にはできない");
        }
        self.edit(|s| {
            let d = &mut s.surfaces[j];
            if d.typ == SurfType::CoordBrk && !no_material(mat) {
                invalid!("面 {j}: 座標ブレークに材料 {mat}");
            }
            d.mirror = mirror;
            d.mat = if mirror {
                "MIRROR".into()
            } else {
                mat.trim().to_string()
            };
            d.n.clear();
            d.n_base.clear();
            if mirror {
                d.dn = 0.0;
                d.disp = 0.0;
            }
            Ok(())
        })
    }

    /// 面 j の膜（膜ファイルの名前、空で無し）
    pub fn set_coating(&mut self, j: usize, coat: &str) -> Result<()> {
        if j > self.image() {
            invalid!("面 {j}");
        }
        self.edit(|s| {
            s.surfaces[j].coat = coat.trim().to_string();
            Ok(())
        })
    }

    /// 面 j の開口（[`Aperture::None`] で無し）
    pub fn set_aperture(&mut self, j: usize, ap: Aperture) -> Result<()> {
        if j > self.image() {
            invalid!("面 {j}");
        }
        self.edit(|s| {
            s.surfaces[j].ap = ap;
            Ok(())
        })
    }

    // ------------------------------------------------------------------------------ ピックアップ・ソルブ
    /// ピックアップを足す（同じ先・同じ量のものは置き換える）。先の値はすぐに元から決まる
    pub fn add_pickup(&mut self, p: Pickup) -> Result<()> {
        let ns = self.surfaces.len();
        if p.src >= ns || p.dst >= ns || p.src == p.dst {
            invalid!("ピックアップの面（元 {}、先 {}）", p.src, p.dst);
        }
        if !p.scale.is_finite() || !p.offset.is_finite() {
            invalid!("ピックアップの倍率 {}・オフセット {}", p.scale, p.offset);
        }
        if let Some(k) = solve_kind_of(p.what) {
            if self
                .solves
                .iter()
                .any(|q| q.surf == p.dst && q.kind.param_kind() == k)
            {
                invalid!(
                    "面 {} の {:?} はソルブで決まる（先に remove_solve）",
                    p.dst,
                    p.what
                );
            }
        }
        if let Some(v) = p.value(&self.surfaces[p.src]) {
            if !v.is_finite() {
                invalid!("ピックアップの値が有限でない（面 {} から {v}）", p.src);
            }
        }
        self.edit(|s| {
            s.pickups.retain(|q| !(q.dst == p.dst && q.what == p.what));
            s.pickups.push(p);
            Ok(())
        })
    }

    /// 面 dst の量 what のピックアップを外す（値は今の値のまま残る）。無ければエラー
    pub fn remove_pickup(&mut self, dst: usize, what: crate::param::PickKind) -> Result<()> {
        if !self.pickups.iter().any(|q| q.dst == dst && q.what == what) {
            invalid!("面 {dst} の {what:?} のピックアップは無い");
        }
        self.edit(|s| {
            s.pickups.retain(|q| !(q.dst == dst && q.what == what));
            Ok(())
        })
    }

    /// ソルブ（厚さか曲率、[`crate::param::SolveKind`]）を足す（同じ面・同じ量のものは置き換える）。値はすぐに解き直す
    pub fn add_solve(&mut self, q: ThickSolve) -> Result<()> {
        let ns = self.surfaces.len();
        if q.surf == 0 || q.surf + 1 >= ns || q.kind.other_surface().is_some_and(|f| f >= ns) {
            invalid!("ソルブの面 {}（1〜{}）", q.surf, ns - 2);
        }
        let pk = q.kind.param_kind();
        self.check_param(Param::new(q.surf, pk))?;
        if !q.height.is_finite() {
            invalid!("ソルブの値 {}", q.height);
        }
        if self
            .pickups
            .iter()
            .any(|p| p.dst == q.surf && solve_kind_of(p.what) == Some(pk))
        {
            invalid!(
                "面 {} の {} はピックアップで決まる（先に remove_pickup）",
                q.surf,
                pk.name()
            );
        }
        self.edit(|s| {
            s.solves
                .retain(|x| !(x.surf == q.surf && x.kind.param_kind() == pk));
            s.solves.push(q);
            Ok(())
        })
    }

    /// 面 surf の量 kind（[`Kind::Thick`] か [`Kind::Curv`]）のソルブを外す（値は今のまま残る）。無ければエラー
    pub fn remove_solve(&mut self, surf: usize, kind: Kind) -> Result<()> {
        let hit = |x: &ThickSolve| x.surf == surf && x.kind.param_kind() == kind;
        if !self.solves.iter().any(hit) {
            invalid!("面 {surf} の {} のソルブは無い", kind.name());
        }
        self.edit(|s| {
            s.solves.retain(|x| !hit(x));
            Ok(())
        })
    }

    // ------------------------------------------------------------------------------ 多重構成
    /// 多重構成の表（無ければ None）
    pub fn configs(&self) -> Option<&Configs> {
        self.configs.as_ref()
    }

    /// 構成の数を n にする（増やした構成の値は今の値。1 で表を消す）。今の構成が n を超えたら 1 にする
    pub fn set_config_count(&mut self, n: usize) -> Result<()> {
        if n == 0 {
            invalid!("構成の数 0");
        }
        if n == 1 {
            self.configs = None;
            return Ok(());
        }
        let mut c = self.configs.clone().unwrap_or(Configs {
            n: 1,
            active: 1,
            rows: Vec::new(),
        });
        for r in c.rows.iter_mut() {
            let now = self.get_param(r.param)?;
            r.values.resize(n, now);
        }
        c.n = n;
        let back = c.active > n;
        if back {
            c.active = 1;
        }
        if back {
            // 今の構成が無くなるので構成 1 の値を系に入れる
            let mut t = self.clone();
            t.configs = Some(c);
            *self = t.config(1)?;
        } else {
            self.configs = Some(c);
        }
        Ok(())
    }

    /// 表に行を足す（同じパラメータの行は置き換える）。values の長さは構成の数。今の構成の値を系に入れる
    pub fn set_config_row(&mut self, p: Param, values: Vec<f64>) -> Result<()> {
        let Some(c) = &self.configs else {
            invalid!("構成が 1 つ（先に set_config_count）");
        };
        if values.len() != c.n {
            invalid!("値の数 {}（構成 {}）", values.len(), c.n);
        }
        self.check_param(p)?;
        if p.kind.is_global() {
            invalid!("{} は表に入れられない（物点・波長）", p.name());
        }
        let mut c = c.clone();
        c.rows.retain(|r| r.param != p);
        c.rows.push(ConfigRow { param: p, values });
        let mut s = self.clone();
        s.configs = Some(c);
        let k = s.configs.as_ref().map(|c| c.active).unwrap_or(1);
        *self = s.config(k)?;
        Ok(())
    }

    /// 表の今の構成の列を、系の今の値に合わせる（面の表・set で表の行の値を変えたとき。
    /// OpticStudio で Lens Data の値を変えると Multi-Configuration Editor の今の構成の値も変わるのと同じ）
    pub fn sync_active_config(&mut self) {
        let Some(mut c) = self.configs.take() else {
            return;
        };
        let k = c.active.saturating_sub(1);
        for r in &mut c.rows {
            if let (Ok(v), Some(x)) = (self.get_param(r.param), r.values.get_mut(k)) {
                *x = v;
            }
        }
        self.configs = Some(c);
    }

    /// 表の行（パラメータ p）を消す。系の値は今のまま
    pub fn remove_config_row(&mut self, p: Param) -> Result<()> {
        let Some(c) = &mut self.configs else {
            invalid!("構成が 1 つ");
        };
        let n0 = c.rows.len();
        c.rows.retain(|r| r.param != p);
        if c.rows.len() == n0 {
            invalid!("表に {} の行は無い", p.name());
        }
        Ok(())
    }

    /// 構成 k（1 始まり）の値を入れた系（表は残し、今の構成を k にする）
    pub fn config(&self, k: usize) -> Result<System> {
        let Some(c) = &self.configs else {
            if k == 1 {
                return Ok(self.clone());
            }
            invalid!("構成 {k}（構成は 1 つ）");
        };
        if k == 0 || k > c.n {
            invalid!("構成 {k}（1〜{}）", c.n);
        }
        let ch: Vec<(Param, f64)> = c.rows.iter().map(|r| (r.param, r.values[k - 1])).collect();
        let mut s = self.with_params(&ch)?;
        if let Some(c) = &mut s.configs {
            c.active = k;
        }
        Ok(s)
    }

    fn check_configs(&self) -> Result<()> {
        let Some(c) = &self.configs else {
            return Ok(());
        };
        if c.n < 2 || c.active == 0 || c.active > c.n {
            invalid!("多重構成の表（構成 {}、今 {}）", c.n, c.active);
        }
        for r in &c.rows {
            if r.values.len() != c.n {
                invalid!(
                    "多重構成の行 {} の値の数 {}",
                    r.param.name(),
                    r.values.len()
                );
            }
            self.check_param(r.param)?;
        }
        Ok(())
    }
}

/// 面 j を消す前に、j を参照するピックアップ・ソルブ・表の行・玉・群の範囲を外す。説明を返す
fn drop_refs(s: &mut System, j: usize) -> Vec<String> {
    let mut notes = Vec::new();
    s.pickups.retain(|p| {
        let used = p.src == j || p.dst == j;
        if used {
            notes.push(format!(
                "面 {} の {:?} のピックアップ（元 面 {} の {:?}）を外した",
                p.dst,
                p.what,
                p.src,
                p.from.unwrap_or(p.what)
            ));
        }
        !used
    });
    s.solves.retain(|q| {
        let used = q.surf == j || q.kind.other_surface() == Some(j);
        if used {
            notes.push(format!(
                "面 {} の {} のソルブ（{:?}）を外した",
                q.surf,
                q.kind.param_kind().name(),
                q.kind
            ));
        }
        !used
    });
    // 玉・群: 範囲を縮め、空になった範囲・玉・群は外す
    let mut unit_map: Vec<Option<usize>> = Vec::with_capacity(s.units.len());
    let mut kept = Vec::new();
    for u in std::mem::take(&mut s.units) {
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
            notes.push(format!("玉・群 {} を外した（面が無くなった）", u.name));
            unit_map.push(None);
        } else {
            unit_map.push(Some(kept.len()));
            kept.push(u);
        }
    }
    s.units = kept;
    if let Some(c) = &mut s.configs {
        c.rows.retain(|r| {
            let used = match r.param.kind {
                Kind::Unit(_) => unit_map.get(r.param.surf).copied().flatten().is_none(),
                k if k.is_global() => false,
                _ => r.param.surf == j,
            };
            if used {
                notes.push(format!("多重構成の行 {} を外した", r.param.name()));
            }
            !used
        });
        for r in c.rows.iter_mut() {
            if let Kind::Unit(_) = r.param.kind {
                if let Some(Some(k)) = unit_map.get(r.param.surf) {
                    r.param.surf = *k;
                }
            }
        }
    }
    // 機構部品: 置く面を消したら外す。保持する面からは外す（番号の付け替えは renumber）
    s.mechanics.retain_mut(|m| {
        m.holds.retain(|&h| h != j);
        if m.surface == j {
            notes.push(format!(
                "機構部品 {} を外した（置く面 {j} を消した）",
                m.name
            ));
            return false;
        }
        true
    });
    if s.glrs == j {
        notes.push(format!("大域座標の基準面 {j} を消したので面 1 にした"));
        s.glrs = 1;
    }
    notes
}

/// 面番号の付け替え（挿入・削除）。玉・群の範囲は、挿入なら at に入れた面を、範囲の最初が at なら範囲に含めない
/// （範囲の前に入る）。削除の範囲は [`drop_refs`] で済ませている
fn renumber(s: &mut System, m: impl Fn(usize) -> Option<usize>, at: usize, insert: bool) {
    let mm = |j: usize| m(j).unwrap_or(j);
    for p in s.pickups.iter_mut() {
        p.src = mm(p.src);
        p.dst = mm(p.dst);
    }
    for q in s.solves.iter_mut() {
        q.surf = mm(q.surf);
        if let Some(o) = q.kind.other_surface() {
            q.kind = q.kind.with_other_surface(mm(o));
        }
    }
    for m in s.mechanics.iter_mut() {
        m.surface = mm(m.surface);
        for h in m.holds.iter_mut() {
            *h = mm(*h);
        }
    }
    if insert {
        for u in s.units.iter_mut() {
            for sp in u.spans.iter_mut() {
                // 範囲の中（最初の面より後ろ、最後の面まで）に入れた面は範囲に含める
                let [a, b] = *sp;
                *sp = [mm(a), if b >= at { b + 1 } else { b }];
            }
        }
        if s.glrs >= at {
            s.glrs += 1;
        }
    } else if s.glrs > at {
        s.glrs -= 1;
    }
    s.field_type = match s.field_type {
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
    if let Some(c) = &mut s.configs {
        for r in c.rows.iter_mut() {
            if !r.param.kind.is_global() && !r.param.kind.is_unit() {
                r.param.surf = mm(r.param.surf);
            }
        }
    }
}

/// ピックアップの量がソルブの決める量（厚さ・曲率）に当たるならその種類
fn solve_kind_of(w: crate::param::PickKind) -> Option<Kind> {
    use crate::param::PickKind;
    match w {
        PickKind::Thick => Some(Kind::Thick),
        PickKind::Curv => Some(Kind::Curv),
        _ => None,
    }
}

/// ソルブの種類の名前（[`solve_kind_parse`]・[`solve_kind_info`]）
pub const SOLVE_KINDS: &[&str] = &[
    "marginal_height",
    "marginal_height_zone",
    "chief_height",
    "pupil_position",
    "edge",
    "opd",
    "position",
    "compensator",
    "center_of_curvature",
    "marginal_angle",
    "chief_angle",
    "marginal_normal",
    "chief_normal",
    "aplanatic",
    "element_power",
    "concentric_surface",
    "concentric_radius",
    "f_number",
];

/// ソルブの種類を名前から。param は marginal_height_zone・opd の瞳の帯・edge の高さ、other は別の面を参照する種類の面
pub fn solve_kind_parse(
    name: &str,
    param: f64,
    other: Option<usize>,
) -> Result<crate::param::SolveKind> {
    use crate::param::SolveKind as K;
    let n = name.trim().to_ascii_lowercase();
    let need = || -> Result<usize> {
        match other {
            Some(s) => Ok(s),
            None => invalid!("ソルブ {n} には別の面（other）が要る"),
        }
    };
    Ok(match n.as_str() {
        "marginal_height" => K::MarginalHeight,
        "marginal_height_zone" => K::MarginalHeightZone(param),
        "chief_height" => K::ChiefHeight,
        "pupil_position" => K::PupilPosition,
        "edge" => K::Edge(param),
        "opd" => K::Opd(param),
        "position" => K::Position(need()?),
        "compensator" => K::Compensator(need()?),
        "center_of_curvature" => K::CenterOfCurvature(need()?),
        "marginal_angle" => K::MarginalAngle,
        "chief_angle" => K::ChiefAngle,
        "marginal_normal" => K::MarginalNormal,
        "chief_normal" => K::ChiefNormal,
        "aplanatic" => K::Aplanatic,
        "element_power" => K::ElementPower,
        "concentric_surface" => K::ConcentricSurface(need()?),
        "concentric_radius" => K::ConcentricRadius(need()?),
        "f_number" => K::FNumber,
        // ZPL のマクロ: param は系のマクロの番号（System::macros）。マクロは .zmx の読込で付くので、
        // 新しく選ぶ種類の一覧（SOLVE_KINDS）には入れない
        "macro_curv" | "macro_thick" => {
            if !(param >= 0.0 && param.fract() == 0.0 && param <= u16::MAX as f64) {
                invalid!("マクロの番号 {param}");
            }
            if n == "macro_curv" {
                K::MacroCurv(param as u16)
            } else {
                K::MacroThick(param as u16)
            }
        }
        _ => invalid!("ソルブの種類 {name}（{}）", SOLVE_KINDS.join(" ")),
    })
}

/// ソルブの種類の (名前, param, other)（[`solve_kind_parse`] の逆）
pub fn solve_kind_info(k: crate::param::SolveKind) -> (&'static str, f64, Option<usize>) {
    use crate::param::SolveKind as K;
    let o = k.other_surface();
    match k {
        K::MarginalHeight => ("marginal_height", 0.0, o),
        K::MarginalHeightZone(z) => ("marginal_height_zone", z, o),
        K::ChiefHeight => ("chief_height", 0.0, o),
        K::PupilPosition => ("pupil_position", 0.0, o),
        K::Edge(r) => ("edge", r, o),
        K::Opd(z) => ("opd", z, o),
        K::Position(_) => ("position", 0.0, o),
        K::Compensator(_) => ("compensator", 0.0, o),
        K::CenterOfCurvature(_) => ("center_of_curvature", 0.0, o),
        K::MarginalAngle => ("marginal_angle", 0.0, o),
        K::ChiefAngle => ("chief_angle", 0.0, o),
        K::MarginalNormal => ("marginal_normal", 0.0, o),
        K::ChiefNormal => ("chief_normal", 0.0, o),
        K::Aplanatic => ("aplanatic", 0.0, o),
        K::ElementPower => ("element_power", 0.0, o),
        K::ConcentricSurface(_) => ("concentric_surface", 0.0, o),
        K::ConcentricRadius(_) => ("concentric_radius", 0.0, o),
        K::FNumber => ("f_number", 0.0, o),
        K::MacroCurv(i) => ("macro_curv", i as f64, o),
        K::MacroThick(i) => ("macro_thick", i as f64, o),
    }
}

/// ピックアップの量の名前: c（曲率）・r（半径）・t（厚さ）・k（円錐係数）・p1〜（面タイプのパラメータ）・
/// dn・disp（材料も一緒に追従）・semi・coating（膜の名前）・before0〜4・after0〜4（傾き偏心の値 dx dy tx ty tz）
pub fn pick_kind_parse(s: &str) -> Result<crate::param::PickKind> {
    use crate::param::PickKind;
    let t = s.trim().to_ascii_lowercase();
    let num = |p: &str| t.strip_prefix(p).and_then(|n| n.parse::<u8>().ok());
    Ok(match t.as_str() {
        "c" | "curv" => PickKind::Curv,
        "r" | "radius" => PickKind::Radius,
        "t" | "thick" => PickKind::Thick,
        "k" | "conic" => PickKind::Conic,
        "dn" => PickKind::Dn,
        "disp" => PickKind::Disp,
        "semi" => PickKind::Semi,
        "coating" | "coat" => PickKind::Coating,
        _ => {
            if let Some(i) = num("before").filter(|&i| i < 5) {
                PickKind::Before(i)
            } else if let Some(i) = num("after").filter(|&i| i < 5) {
                PickKind::After(i)
            } else if let Some(i) = num("parm").or_else(|| num("p")) {
                PickKind::Parm(i)
            } else {
                invalid!("ピックアップの量 {s}（c r t k p1〜 dn disp semi coating before0〜4 after0〜4）")
            }
        }
    })
}

/// ピックアップの量の名前（[`pick_kind_parse`] の逆）
pub fn pick_kind_name(k: crate::param::PickKind) -> String {
    use crate::param::PickKind;
    match k {
        PickKind::Curv => "c".into(),
        PickKind::Thick => "t".into(),
        PickKind::Conic => "k".into(),
        PickKind::Parm(i) => format!("p{i}"),
        PickKind::Dn => "dn".into(),
        PickKind::Disp => "disp".into(),
        PickKind::Semi => "semi".into(),
        PickKind::Coating => "coating".into(),
        PickKind::Radius => "r".into(),
        PickKind::Before(i) => format!("before{i}"),
        PickKind::After(i) => format!("after{i}"),
    }
}
