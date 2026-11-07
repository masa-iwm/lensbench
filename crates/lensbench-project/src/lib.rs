//! ブロックで組む系（プロジェクトファイル、JSON）と、その順次の面の列への展開。
//!
//! ユーザーが編集するのはブロック（玉・空気・絞り・鏡・面の並び・.zmx から取り込んだ系・群）で、計算の直前に
//! [`expand`] で OpticStudio と同じ順次の面の列（[`System`]）にする。
//!
//! - 玉の向きと面のデータは「光の進む向き」で書く。鏡のあと（光が −z へ進む区間）では、展開で曲率・非球面・厚さの
//!   符号を変える（OpticStudio の約束: 鏡のあとは厚さが負）。
//! - 偏心・傾きは座標ブレークの組で入れる。ブロックの前で傾け、ブロックの後で元の軸へ戻す（戻る点は元の軸上で
//!   ブロックの長さだけ進んだ点）。鏡の傾き（折り曲げ）は戻さない。
//! - ON/OFF: OFF のブロックは「詰める」（長さごと除く）か「空気で残す」（同じ長さの空気にする）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lensbench_core::elements::{self, Element, Kind};
use lensbench_core::glass::{self, Glass};
use lensbench_core::math::{mv, rot, M3};
use lensbench_core::surface::{SurfType, Surface};
use lensbench_core::system::{Aiming, ApertureKind, Field, FieldNorm, FieldType, System};
use lensbench_core::zmx;
use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}
fn one() -> usize {
    1
}
fn default_catalogs() -> Vec<String> {
    vec!["SCHOTT".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wave {
    pub um: f64,
    #[serde(default = "wave_weight")]
    pub weight: f64,
}
fn wave_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApertureType {
    Epd,
    Fnum,
    Na,
    Float,
    Pwfn,
    Cone,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApertureSpec {
    #[serde(rename = "type")]
    pub kind: ApertureType,
    #[serde(default)]
    pub value: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    #[default]
    Angle,
    ObjectHeight,
    ParaxialImageHeight,
    RealImageHeight,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldsSpec {
    #[serde(rename = "type", default)]
    pub kind: FieldKind,
    /// 視野の (x, y)
    pub points: Vec<[f64; 2]>,
    #[serde(default)]
    pub weights: Vec<f64>,
}

impl Default for FieldsSpec {
    fn default() -> Self {
        FieldsSpec {
            kind: FieldKind::Angle,
            points: vec![[0.0, 0.0]],
            weights: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AimingSpec {
    #[default]
    Off,
    Paraxial,
    Real,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OffMode {
    /// 長さごと除く
    #[default]
    Collapse,
    /// 同じ長さの空気で残す
    Air,
}

/// ブロックの置き方（ON/OFF、偏心・傾き）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Place {
    #[serde(default = "yes")]
    pub on: bool,
    #[serde(default)]
    pub off: OffMode,
    /// 偏心 [mm]
    #[serde(default)]
    pub decenter: [f64; 2],
    /// 傾き x・y・z [deg]（前面の頂点のまわり、x → y → z の順）
    #[serde(default)]
    pub tilt: [f64; 3],
}

impl Default for Place {
    fn default() -> Self {
        Place {
            on: true,
            off: OffMode::Collapse,
            decenter: [0.0; 2],
            tilt: [0.0; 3],
        }
    }
}

impl Place {
    fn moved(&self) -> bool {
        self.decenter
            .iter()
            .chain(self.tilt.iter())
            .any(|v| *v != 0.0)
    }
}

/// 面 1 つ（光の進む向きで書く）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SurfSpec {
    /// 曲率半径 [mm]（省略・0 なら平面）
    #[serde(default)]
    pub radius: Option<f64>,
    #[serde(default)]
    pub conic: f64,
    /// 偶数次非球面の係数 r², r⁴, …
    #[serde(default)]
    pub asph: Vec<f64>,
    /// 次の面までの厚さ（玉の最後の面では使わない）
    #[serde(default)]
    pub t: f64,
    /// 面の後のガラス（省略なら空気）。数字なら分散なしのモデルガラスの屈折率
    #[serde(default)]
    pub glass: Option<String>,
    #[serde(default)]
    pub semi: f64,
    #[serde(default)]
    pub coat: String,
    #[serde(default)]
    pub comment: String,
}

/// 玉の出どころ
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LensSource {
    /// .zmx の k 番目の玉（玉だけを数える、1 始まり）
    Zmx {
        zmx: String,
        #[serde(default = "one")]
        lens: usize,
    },
    /// レンズカタログ（メーカーと型番）
    Catalog { vendor: String, part: String },
    /// 面を直接
    Surfaces { surfaces: Vec<SurfSpec> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Lens {
        #[serde(default)]
        id: String,
        source: LensSource,
        #[serde(default)]
        reversed: bool,
        /// 玉の前面を絞りにする
        #[serde(default)]
        stop: bool,
        #[serde(flatten)]
        place: Place,
    },
    Air {
        #[serde(default)]
        id: String,
        t: f64,
    },
    /// 絞り（空気中のダミー面）
    Stop {
        #[serde(default)]
        id: String,
        #[serde(default)]
        semi: f64,
    },
    Mirror {
        #[serde(default)]
        id: String,
        /// 曲率半径（省略・0 なら平面）
        #[serde(default)]
        radius: Option<f64>,
        #[serde(default)]
        conic: f64,
        /// 折り曲げの傾き x・y [deg]（鏡の前後で同じ角だけ傾ける）
        #[serde(default)]
        tilt: [f64; 2],
        #[serde(default)]
        semi: f64,
        #[serde(default)]
        coat: String,
        #[serde(default = "yes")]
        on: bool,
        /// 往路を戻る: 同じ並び（群）で鏡より前のブロックを、逆の順・逆の向きにたどる
        #[serde(default)]
        back: bool,
    },
    /// 空気中の面の並び（玉に分けない）
    Surfaces {
        #[serde(default)]
        id: String,
        surfaces: Vec<SurfSpec>,
        #[serde(flatten)]
        place: Place,
    },
    /// .zmx から取り込んだ系（面 from〜to、既定は面 1 から像面の手前まで。空気間隔も含む）
    Zmx {
        #[serde(default)]
        id: String,
        path: String,
        #[serde(default)]
        from: Option<usize>,
        #[serde(default)]
        to: Option<usize>,
        #[serde(flatten)]
        place: Place,
    },
    Group {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        /// zoom・focus・fixed など
        #[serde(default)]
        role: String,
        blocks: Vec<Block>,
        #[serde(flatten)]
        place: Place,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_catalogs")]
    pub glass_catalogs: Vec<String>,
    pub wavelengths: Vec<Wave>,
    /// 主波長の番号（1 始まり）
    #[serde(default = "one")]
    pub primary: usize,
    pub aperture: ApertureSpec,
    #[serde(default)]
    pub fields: FieldsSpec,
    /// 物体面から最初のブロックまで [mm]（省略で無限遠）
    #[serde(default)]
    pub object_distance: Option<f64>,
    #[serde(default)]
    pub aiming: AimingSpec,
    pub blocks: Vec<Block>,
}

impl Project {
    pub fn from_json(txt: &str) -> Result<Project, String> {
        serde_json::from_str(txt).map_err(|e| format!("プロジェクトファイル: {e}"))
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap()
    }
}

/// レンズカタログを引く関数: (メーカー, 型番) → .zmx の本文
pub type CatalogLookup<'a> = &'a dyn Fn(&str, &str) -> Result<String, String>;

/// 展開の環境
pub struct Env<'a> {
    /// 相対パスの基準（プロジェクトファイルのフォルダ）
    pub base: PathBuf,
    pub glass_dirs: Vec<PathBuf>,
    pub catalog: Option<CatalogLookup<'a>>,
}

struct Ctx<'a, 'b> {
    env: &'a Env<'b>,
    sf: Vec<Surface>,
    /// 物体面の後の、最初の面までの距離（最初のブロックの前の空気）
    lead: f64,
    /// 鏡の数の偶奇（−1 なら光は −z へ進む）
    parity: f64,
    glasses: HashMap<String, Glass>,
    nstop: usize,
}

fn is_air(m: &str) -> bool {
    m.is_empty() || m.eq_ignore_ascii_case("AIR")
}

/// (dx, dy, tx, ty, tz) の座標ブレーク
fn cb(d: [f64; 5], order: u8, t: f64) -> Surface {
    Surface {
        typ: SurfType::CoordBrk,
        mat: "-".into(),
        cb: d,
        cb_order: order,
        t,
        ..Default::default()
    }
}

impl<'a, 'b> Ctx<'a, 'b> {
    fn gap(&mut self, t: f64) {
        match self.sf.last_mut() {
            Some(s) => s.t += t * self.parity,
            None => self.lead += t,
        }
    }

    /// 光の進む向きで書いた面を、今の向きに直して足す（厚さ・曲率・非球面）
    fn push(&mut self, mut s: Surface) {
        if self.parity < 0.0 {
            s.c = -s.c;
            s.asph.iter_mut().for_each(|a| *a = -*a);
            s.t = -s.t;
        }
        if s.stop {
            self.nstop += 1;
        }
        self.sf.push(s);
    }

    /// 符号つきの長さ（光の進む向き）: 今の位置から n 面前までの厚さの和
    fn length_since(&self, start: usize) -> f64 {
        self.sf[start..].iter().map(|s| s.t).sum::<f64>() * self.parity
    }

    /// 偏心・傾きをつけて body を入れる。戻りの座標ブレークで元の軸の、長さ L だけ進んだ点へ戻す
    fn placed(
        &mut self,
        place: &Place,
        body: &mut dyn FnMut(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        if !place.on {
            if place.off == OffMode::Air {
                // 長さを測るため一度入れて、空気に置き換える
                let start = self.sf.len();
                let (p0, n0) = (self.parity, self.nstop);
                body(self)?;
                if self.parity != p0 {
                    return Err("鏡を含むブロックは「空気で残す」にできない".into());
                }
                let l = self.length_since(start);
                self.sf.truncate(start);
                self.nstop = n0;
                self.gap(l);
            }
            return Ok(());
        }
        if !place.moved() {
            return body(self);
        }
        let [dx, dy] = place.decenter;
        let [tx, ty, tz] = place.tilt;
        self.push(cb([dx, dy, tx, ty, tz], 0, 0.0));
        let start = self.sf.len();
        let p0 = self.parity;
        body(self)?;
        if self.parity != p0 {
            return Err("偏心・傾きをつけたブロックに鏡は入れられない（鏡の傾きは鏡で）".into());
        }
        // 中身の最後の面の厚さを戻りの後へ回す
        let tail = self.sf.last().map(|s| s.t).unwrap_or(0.0);
        if let Some(s) = self.sf.last_mut() {
            s.t = 0.0;
        }
        let l: f64 = self.sf[start..].iter().map(|s| s.t).sum(); // 今の座標の z で
        let (dx2, dy2, t2) = cb_return(place, l);
        // 戻りの座標ブレークの x・y の偏心は傾きを戻したあとの（元の）座標で
        self.sf.push(cb([dx2, dy2, -tx, -ty, -tz], 1, t2 + tail));
        Ok(())
    }

    /// OpticStudio の座標のままの要素（.zmx から取り込んだもの、鏡のあとは負の厚さ）を足す。
    /// sign は取り込み始めの向き（逆なら全部の符号を返す）
    fn element(&mut self, e: &Element, sign: f64) {
        if e.kind == Kind::Air {
            match self.sf.last_mut() {
                Some(s) => s.t += e.t * sign,
                None => self.lead += e.t * sign,
            }
            return;
        }
        for s in &e.surfaces {
            let mut s = s.clone();
            if sign < 0.0 {
                s.c = -s.c;
                s.asph.iter_mut().for_each(|a| *a = -*a);
                s.t = -s.t;
            }
            if s.mirror {
                self.parity = -self.parity;
            }
            if s.stop {
                self.nstop += 1;
            }
            self.sf.push(s);
        }
    }

    /// 往路のブロックを復路でたどる（鏡のあと）
    fn back_block(&mut self, b: &Block) -> Result<(), String> {
        match b {
            Block::Air { t, .. } => self.gap(*t),
            Block::Stop { semi, .. } => {
                // 復路で絞りの面を通る位置（絞りにはしない）
                self.push(Surface {
                    semi: *semi,
                    comment: "STOP (return)".into(),
                    ..Default::default()
                });
            }
            Block::Lens {
                source,
                reversed,
                place,
                ..
            } => {
                let mut e = self.lens_element(source)?;
                if *reversed {
                    e = elements::reverse(&e).map_err(|x| x.to_string())?;
                }
                let len = e.length();
                if !place.on {
                    if place.off == OffMode::Air {
                        self.gap(len);
                    }
                    return Ok(());
                }
                let mut r = elements::reverse(&e).map_err(|x| x.to_string())?;
                r.surfaces.iter_mut().for_each(|s| s.stop = false);
                self.back_placed(place, len, &r.surfaces)?;
            }
            Block::Surfaces {
                surfaces, place, ..
            } => {
                let v: Vec<Surface> = surfaces.iter().map(surf).collect::<Result<_, _>>()?;
                if !place.on {
                    if place.off == OffMode::Air {
                        let l: f64 = v.iter().map(|s| s.t).sum();
                        self.gap(l);
                    }
                    return Ok(());
                }
                // 往路の最後の面の厚さ（後の空気）は、復路では最初の面の前の空気
                let tail = v.last().map(|s| s.t).unwrap_or(0.0);
                let len: f64 = v.iter().map(|s| s.t).sum::<f64>() - tail;
                self.gap(tail);
                self.back_placed(place, len, &reverse_surfaces(&v))?;
            }
            Block::Group { blocks, place, .. } => {
                if place.moved() {
                    return Err(
                        "往路を戻る範囲の、偏心・傾きをつけた群は未対応（群の中の玉に付ける）"
                            .into(),
                    );
                }
                if !place.on {
                    return Err("往路を戻る範囲の、OFF の群は未対応".into());
                }
                for x in blocks.iter().rev() {
                    self.back_block(x)?;
                }
            }
            Block::Mirror { .. } => return Err("往路を戻る範囲に鏡がある".into()),
            Block::Zmx { .. } => {
                return Err("往路を戻る範囲の .zmx の取り込みは未対応（玉のブロックにする）".into())
            }
        }
        Ok(())
    }

    /// 復路で、往路に偏心・傾き place・長さ len（光の進む向き）で置いたものを、逆向きの面 rev でたどる
    fn back_placed(&mut self, place: &Place, len: f64, rev: &[Surface]) -> Result<(), String> {
        if !place.moved() {
            for s in rev {
                self.push(s.clone());
            }
            return Ok(());
        }
        // 往路: CB1（順序 0、d・t）→ 中身（今の座標で l）→ CB2（順序 1、−t・d2、厚さ t2）。復路はその逆:
        // 元の軸で t2 戻る → CB2 の逆（順序 0、−d2・t）→ 逆向きの中身 → CB1 の逆（順序 1、−d・−t）
        let pf = -self.parity;
        let l = len * pf;
        let (dx2, dy2, t2) = cb_return(place, l);
        let [dx, dy] = place.decenter;
        let [tx, ty, tz] = place.tilt;
        // 今の座標で −t2 進む（gap は光の進む向きの長さを取る）
        self.gap(-t2 * self.parity);
        self.sf.push(cb([-dx2, -dy2, tx, ty, tz], 0, 0.0));
        for s in rev {
            self.push(s.clone());
        }
        if let Some(s) = self.sf.last_mut() {
            s.t = 0.0;
        }
        self.sf.push(cb([-dx, -dy, -tx, -ty, -tz], 1, 0.0));
        Ok(())
    }

    fn load_zmx(&mut self, path: &str) -> Result<System, String> {
        let p = self.env.base.join(path);
        let opt = zmx::LoadOptions {
            cfg: 1,
            catalog_dirs: self.env.glass_dirs.clone(),
            ..Default::default()
        };
        let s = zmx::load_with(&p, &opt).map_err(|e| format!("{}: {e}", p.display()))?;
        self.merge_glasses(&s);
        Ok(s)
    }

    fn merge_glasses(&mut self, s: &System) {
        for (k, g) in &s.glasses {
            self.glasses.entry(k.clone()).or_insert_with(|| g.clone());
        }
    }

    fn lens_element(&mut self, src: &LensSource) -> Result<Element, String> {
        match src {
            LensSource::Zmx { zmx, lens } => {
                let s = self.load_zmx(zmx)?;
                pick_lens(&s, *lens).map_err(|e| format!("{zmx}: {e}"))
            }
            LensSource::Catalog { vendor, part } => {
                let f = self
                    .env
                    .catalog
                    .ok_or_else(|| format!("レンズカタログ {vendor} {part}: カタログを読めない"))?;
                let txt = f(vendor, part)?;
                let raw = zmx::parse_text(&txt).map_err(|e| e.to_string())?;
                let s = zmx::build(
                    &raw,
                    &zmx::LoadOptions {
                        cfg: 1,
                        ..Default::default()
                    },
                    &glass::catalog_dirs(&self.env.glass_dirs),
                    None,
                )
                .map_err(|e| format!("{vendor} {part}: {e}"))?;
                self.merge_glasses(&s);
                pick_lens(&s, 1).map_err(|e| format!("{vendor} {part}: {e}"))
            }
            LensSource::Surfaces { surfaces } => {
                if surfaces.len() < 2 {
                    return Err("玉には面が 2 つ以上要る".into());
                }
                let n = surfaces.len();
                let mut v = Vec::new();
                for (i, q) in surfaces.iter().enumerate() {
                    let mut s = surf(q)?;
                    if i + 1 == n {
                        s.t = 0.0;
                        s.mat.clear();
                    } else if is_air(&s.mat) {
                        return Err(format!("玉の面 {}: 後がガラスでない", i + 1));
                    }
                    v.push(s);
                }
                Ok(Element {
                    kind: Kind::Lens,
                    from: 0,
                    surfaces: v,
                    t: 0.0,
                    reversed: false,
                })
            }
        }
    }

    fn block(&mut self, b: &Block, siblings: &[Block], idx: usize) -> Result<(), String> {
        match b {
            Block::Air { t, .. } => self.gap(*t),
            Block::Stop { semi, .. } => {
                let s = Surface {
                    stop: true,
                    semi: *semi,
                    comment: "STOP".into(),
                    ..Default::default()
                };
                self.push(s);
            }
            Block::Lens {
                source,
                reversed,
                stop,
                place,
                ..
            } => {
                let mut e = self.lens_element(source)?;
                if *reversed {
                    e = elements::reverse(&e).map_err(|x| x.to_string())?;
                }
                e.surfaces.iter_mut().for_each(|s| s.stop = false);
                if *stop {
                    e.surfaces[0].stop = true;
                }
                self.placed(place, &mut |c: &mut Self| {
                    for s in &e.surfaces {
                        c.push(s.clone());
                    }
                    Ok(())
                })?;
            }
            Block::Surfaces {
                surfaces, place, ..
            } => {
                let v: Vec<Surface> = surfaces.iter().map(surf).collect::<Result<_, _>>()?;
                if v.iter().any(|s| !is_air(&s.mat)) {
                    return Err("面の並びのブロックは空気中の面だけ（ガラスは玉で）".into());
                }
                self.placed(place, &mut |c: &mut Self| {
                    for s in &v {
                        c.push(s.clone());
                    }
                    Ok(())
                })?;
            }
            Block::Mirror {
                radius,
                conic,
                tilt,
                semi,
                coat,
                on,
                back,
                ..
            } => {
                if !on {
                    return Ok(());
                }
                let back = *back;
                let fold = tilt[0] != 0.0 || tilt[1] != 0.0;
                if fold {
                    self.sf.push(cb([0.0, 0.0, tilt[0], tilt[1], 0.0], 0, 0.0));
                }
                let mut m = Surface {
                    mirror: true,
                    mat: "MIRROR".into(),
                    c: curv(*radius),
                    k: *conic,
                    semi: *semi,
                    coat: coat.clone(),
                    ..Default::default()
                };
                if self.parity < 0.0 {
                    m.c = -m.c;
                }
                self.sf.push(m);
                self.parity = -self.parity;
                if fold {
                    self.sf.push(cb([0.0, 0.0, tilt[0], tilt[1], 0.0], 0, 0.0));
                }
                if back {
                    if fold {
                        return Err("往路を戻る鏡は傾けられない".into());
                    }
                    for x in siblings[..idx].iter().rev() {
                        self.back_block(x)?;
                    }
                }
            }
            Block::Zmx {
                path,
                from,
                to,
                place,
                ..
            } => {
                let s = self.load_zmx(path)?;
                let els = elements::split(&s).map_err(|e| format!("{path}: {e}"))?;
                let (a, z) = (from.unwrap_or(1), to.unwrap_or(s.image() - 1));
                let pick: Vec<Element> = els
                    .into_iter()
                    .filter(|e| e.kind == Kind::Air || (e.from >= a && e.from <= z))
                    .collect();
                // 範囲の外の空気（前後の要素が範囲外）は除く
                let mut keep = Vec::new();
                let mut last_in = false;
                for e in pick {
                    if e.kind == Kind::Air {
                        if last_in {
                            keep.push(e);
                        }
                    } else {
                        last_in = true;
                        keep.push(e);
                    }
                }
                self.placed(place, &mut |c: &mut Self| {
                    let sign = c.parity;
                    for e in &keep {
                        c.element(e, sign);
                    }
                    Ok(())
                })?;
            }
            Block::Group { blocks, place, .. } => {
                let bl = blocks.clone();
                self.placed(place, &mut |c: &mut Self| {
                    for (i, x) in bl.iter().enumerate() {
                        c.block(x, &bl, i)?;
                    }
                    Ok(())
                })?;
            }
        }
        Ok(())
    }
}

/// 前の座標ブレーク（順序 0 で偏心 d・傾き t）で傾けた座標の z に l 進んだ点から、元の座標の z に l 進んだ点へ戻る
/// 座標ブレーク（順序 0 で −t、偏心 (dx2, dy2)、厚さ t2）の値: (dx2, dy2, t2) = l e_z − d − R e_z l（R = Rx Ry Rz）
fn cb_return(place: &Place, l: f64) -> (f64, f64, f64) {
    let [dx, dy] = place.decenter;
    let [tx, ty, tz] = place.tilt;
    let r: M3 = lensbench_core::math::mm(
        &lensbench_core::math::mm(&rot(0, tx), &rot(1, ty)),
        &rot(2, tz),
    );
    let rz = mv(&r, [0.0, 0.0, l]);
    (-dx - rz[0], -dy - rz[1], l - rz[2])
}

/// 空気中の面の並びを逆向きに（厚さは 1 つ前の面のもの、曲率の符号を変える）。最後の面の厚さは 0
fn reverse_surfaces(v: &[Surface]) -> Vec<Surface> {
    let n = v.len();
    (0..n)
        .map(|i| {
            let mut s = v[n - 1 - i].clone();
            s.c = -s.c;
            s.asph.iter_mut().for_each(|a| *a = -*a);
            s.t = if i + 1 < n { v[n - 2 - i].t } else { 0.0 };
            s.stop = false;
            s
        })
        .collect()
}

fn curv(r: Option<f64>) -> f64 {
    match r {
        Some(r) if r != 0.0 && r.is_finite() => 1.0 / r,
        _ => 0.0,
    }
}

fn surf(q: &SurfSpec) -> Result<Surface, String> {
    let mut s = Surface {
        typ: if q.asph.iter().any(|a| *a != 0.0) {
            SurfType::EvenAsph
        } else {
            SurfType::Standard
        },
        c: curv(q.radius),
        k: q.conic,
        asph: q.asph.clone(),
        t: q.t,
        semi: q.semi,
        coat: q.coat.clone(),
        comment: q.comment.clone(),
        ..Default::default()
    };
    if let Some(g) = &q.glass {
        if let Ok(nd) = g.trim().parse::<f64>() {
            // モデルガラス（分散なし）: 名前は .zmx 読込と同じ形、屈折率は n に入れておく
            s.mat = format!("{nd:.2},{:.1}", 0.0);
            s.n = vec![nd];
        } else {
            s.mat = g.trim().to_string();
        }
    }
    Ok(s)
}

fn pick_lens(s: &System, k: usize) -> lensbench_core::Result<Element> {
    let els = elements::split(s)?;
    let lenses: Vec<&Element> = els.iter().filter(|e| e.kind == Kind::Lens).collect();
    match lenses.get(k.wrapping_sub(1)) {
        Some(e) => Ok((*e).clone()),
        None => Err(lensbench_core::Error::Invalid(format!(
            "玉 {k} はない（{} 個）",
            lenses.len()
        ))),
    }
}

/// プロジェクトを順次の面の列の系に展開する
pub fn expand(p: &Project, env: &Env) -> Result<System, String> {
    if p.wavelengths.is_empty() {
        return Err("波長がない".into());
    }
    if p.primary == 0 || p.primary > p.wavelengths.len() {
        return Err(format!(
            "主波長 {}（波長は {} 個）",
            p.primary,
            p.wavelengths.len()
        ));
    }
    let dirs = glass::catalog_dirs(&env.glass_dirs);
    let (glasses, missing) = glass::load_catalogs(&p.glass_catalogs, &dirs);
    if !missing.is_empty() {
        return Err(format!(
            "ガラスカタログが見つからない: {}",
            missing.join(", ")
        ));
    }
    let mut c = Ctx {
        env,
        sf: Vec::new(),
        lead: 0.0,
        parity: 1.0,
        glasses,
        nstop: 0,
    };
    for (i, b) in p.blocks.iter().enumerate() {
        c.block(b, &p.blocks, i)?;
    }
    if c.nstop != 1 {
        return Err(format!("絞りが {} 個（ちょうど 1 つ要る）", c.nstop));
    }
    let wl: Vec<f64> = p.wavelengths.iter().map(|w| w.um).collect();
    let wt: Vec<f64> = p.wavelengths.iter().map(|w| w.weight).collect();
    let obj = Surface {
        t: p.object_distance.unwrap_or(f64::INFINITY),
        ..Default::default()
    };
    let mut sf = vec![obj];
    if p.object_distance.is_some() {
        sf[0].t += c.lead;
    }
    sf.append(&mut c.sf);
    sf.push(Surface::default());
    // 面の後の屈折率（波長ごと）
    let mut prev = vec![1.0; wl.len()];
    for (j, d) in sf.iter_mut().enumerate() {
        let n = if j > 0 && (d.mirror || d.typ == SurfType::CoordBrk) {
            prev.clone()
        } else if is_air(&d.mat) {
            vec![1.0; wl.len()]
        } else if let Some(g) = c.glasses.get(&d.mat) {
            wl.iter().map(|&l| g.index(l, 20.0, 1.0, false)).collect()
        } else if let Some(&n0) = d.n.first() {
            if d.n.iter().any(|&v| v != n0) {
                return Err(format!("面 {j}: ガラス {} がカタログにない", d.mat));
            }
            vec![n0; wl.len()]
        } else {
            return Err(format!("面 {j}: ガラス {} がカタログにない", d.mat));
        };
        d.n = n.clone();
        prev = n;
    }
    let ap = match p.aperture.kind {
        ApertureType::Epd => ApertureKind::Epd,
        ApertureType::Fnum => ApertureKind::FNum,
        ApertureType::Na => ApertureKind::Na,
        ApertureType::Float => ApertureKind::Float,
        ApertureType::Pwfn => ApertureKind::Pwfn,
        ApertureType::Cone => ApertureKind::Cone,
    };
    let ft = match p.fields.kind {
        FieldKind::Angle => FieldType::Angle,
        FieldKind::ObjectHeight => FieldType::ObjH,
        FieldKind::ParaxialImageHeight => FieldType::ParH,
        FieldKind::RealImageHeight => FieldType::RealH,
    };
    let fields: Vec<Field> = p
        .fields
        .points
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let mut f = Field::xy(q[0], q[1]);
            f.w = p.fields.weights.get(i).copied().unwrap_or(1.0);
            f
        })
        .collect();
    let aim = match p.aiming {
        AimingSpec::Off => Aiming::Off,
        AimingSpec::Paraxial => Aiming::Paraxial,
        AimingSpec::Real => Aiming::Real,
    };
    let mut s = System::new(
        sf,
        wl,
        wt,
        p.primary - 1,
        fields,
        ft,
        FieldNorm::Radial,
        (ap, p.aperture.value),
        false,
        aim,
        c.glasses,
    )
    .map_err(|e| e.to_string())?;
    s.name = p.name.clone();
    s.validate().map_err(|e| e.to_string())?;
    Ok(s)
}

/// プロジェクトファイルを読んで展開する
pub fn load(
    path: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
) -> Result<System, String> {
    let txt = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let p = Project::from_json(&txt)?;
    let env = Env {
        base: path.parent().map(|p| p.to_path_buf()).unwrap_or_default(),
        glass_dirs: glass_dirs.to_vec(),
        catalog,
    };
    expand(&p, &env)
}
