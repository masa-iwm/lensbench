//! 組合せ: 要素（カタログの玉・自作の玉・平板・鏡・絞り・ビームスプリッター・.zmx の玉）の並びと、
//! それをレイアウト図で組む画面。
//!
//! 組合せは GUI が持つ「元」で、変えるたびにプロジェクトファイル（ブロック）にして読み直す（[`Combo::project`]）。
//! 図はserve の `layout`（断面の図形・面の頂点・玉の範囲）で描き、要素のブロックの id（= 玉・群の名前）で
//! 図の玉と組合せの要素を結ぶ。間隔は CAD 風の寸法線で示し、選んだときだけ入力欄とスライダーを出す。

use egui::{Color32, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::combo_tree::Loc;
pub use crate::combo_tree::{Branch, Gap};
use crate::i18n::{n_, tr, trf};
use crate::json_view::fmt_num;
use crate::sysset::{Aiming, ApKind, FieldKind, FieldPt, FieldSolve, WavePt};
use crate::theme;
use lensbench_project::edit_ops::{BlockRef, Ref, StopOp, SurfaceOp};

mod ap_shape;
mod bs;
mod detector;
mod element;
mod figure;
mod mech;
mod raw;
mod sys_extra;
mod write_face;
mod zgroup;
pub use ap_shape::{ApData, ApShape};
pub use bs::{Bs, BsForm, BsSplit};
pub use detector::DetectorPick;
use figure::figure;
#[cfg(test)]
use figure::{cube_of_prism, inside};
pub use mech::MechPick;
pub use sys_extra::SysExtra;
pub use zgroup::ZGroup;

/// 要素の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ElemKind {
    /// レンズカタログの玉（vendor・part）
    #[default]
    Catalog,
    /// 自作の玉（既定は 2 面の単玉。面を足すと貼り合わせ・3 枚玉など）
    Lens,
    /// 平行平板
    Plate,
    /// 鏡（傾けると光軸が折れる）
    Mirror,
    /// 絞り
    Stop,
    /// ビームスプリッター（キューブ）
    Splitter,
    /// .zmx の k 番目の玉
    Zmx,
    /// 開口・遮蔽（空気中の面に開口を付ける）
    Aperture,
    /// 空気中の面の並び（Jones 行列の面・座標ブレーク・平面など。surfaces ブロック）
    Surfaces,
    /// .zmx の面の範囲をまるごと取り込む（zmx ブロック）
    ZmxRange,
    /// 組合せの欄で書けないブロックを、そのまま持つ（開いたプロジェクトのブロック。中身は extra）
    Raw,
}

/// 面の並びの面の種類
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfKind {
    #[default]
    Standard,
    /// Jones 行列の面（偏光子・波長板など）
    Jones,
    /// 座標ブレーク
    Coordbrk,
    /// 直線偏光子（理想）
    Polarizer,
    /// 波長板（理想、波長ごとに遅延が変わる）
    Waveplate,
}

impl SurfKind {
    pub fn label(self) -> &'static str {
        match self {
            SurfKind::Standard => tr("Surface"),
            SurfKind::Jones => tr("Jones matrix"),
            SurfKind::Coordbrk => tr("Coordinate break"),
            SurfKind::Polarizer => tr("Polarizer"),
            SurfKind::Waveplate => tr("Waveplate"),
        }
    }
}

/// 面 1 つ（自作の玉・面の並びの表の行。project の SurfSpec）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfRow {
    pub kind: SurfKind,
    /// 曲率半径 [mm]（0 は平面）
    pub radius: f64,
    pub conic: f64,
    /// 次の面までの厚さ [mm]
    pub t: f64,
    /// 面の後のガラス（空なら空気）
    pub glass: String,
    pub semi: f64,
    /// 半径を光線から求める
    pub semi_auto: bool,
    /// Jones 行列 [Ar, Ai, Br, Bi, Cr, Ci, Dr, Di]
    pub jones: [f64; 8],
    /// 座標ブレーク [dx, dy, tx, ty, tz] と順序（0: 偏心してから傾き）
    pub cb: [f64; 5],
    pub order: u8,
    /// 偏光子・波長板の軸 [deg]（偏光子は透過軸、波長板は速い軸）
    pub axis_deg: f64,
    /// 偏光子の消光比（0 は無限）
    pub extinction_ratio: f64,
    /// 偏光子・波長板の強度透過率
    pub transmittance: f64,
    /// 波長板の遅延 [波長]・設計波長 [µm]・次数
    pub retardance: f64,
    pub wavelength_um: f64,
    pub wp_order: u32,
    /// 波長板の複屈折材と厚さ [mm]（空なら理想の遅延、厚さ 0 は遅延から決める）。2 つ目の材料があれば
    /// 速い軸を直交させた 2 材の組（アクロマティック）
    pub wp_material: String,
    pub wp_thickness: f64,
    pub wp_material2: String,
    pub wp_thickness2: f64,
    /// 欄の無い SurfSpec の値（非球面の係数・面の形・膜・コメント・開口・面の傾き・複屈折など。[`SURF_MORE`]）。
    /// そのまま project の面に書く
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub more: serde_json::Map<String, Value>,
}

/// [`SurfRow::more`] に持つ SurfSpec の鍵
pub const SURF_MORE: &[&str] = &[
    "asph",
    "shape",
    "radius_x",
    "coat",
    "comment",
    "aperture",
    "tilt",
    "birefringent",
];

impl Default for SurfRow {
    fn default() -> Self {
        SurfRow {
            kind: SurfKind::Standard,
            radius: 0.0,
            conic: 0.0,
            t: 0.0,
            glass: String::new(),
            semi: 12.7,
            semi_auto: false,
            jones: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            cb: [0.0; 5],
            order: 0,
            axis_deg: 0.0,
            extinction_ratio: 0.0,
            transmittance: 1.0,
            retardance: 0.25,
            wavelength_um: 0.6328,
            wp_order: 0,
            wp_material: String::new(),
            wp_thickness: 0.0,
            wp_material2: String::new(),
            wp_thickness2: 0.0,
            more: serde_json::Map::new(),
        }
    }
}

impl SurfRow {
    /// project の SurfSpec の JSON
    pub fn json(&self) -> Value {
        let mut v = self.json_fields();
        for (k, x) in &self.more {
            v[k] = x.clone();
        }
        v
    }

    /// 欄だけの SurfSpec の JSON
    fn json_fields(&self) -> Value {
        match self.kind {
            SurfKind::Standard => {
                let mut v = json!({ "radius": Pick::radius(self.radius), "t": self.t,
                    "semi": semi_json(self.semi, self.semi_auto) });
                if self.conic != 0.0 {
                    v["conic"] = json!(self.conic);
                }
                if !self.glass.trim().is_empty() {
                    v["glass"] = json!(self.glass.trim());
                }
                v
            }
            SurfKind::Jones => json!({ "jones": self.jones, "t": self.t, "semi": self.semi }),
            SurfKind::Polarizer => {
                let mut pol =
                    json!({ "axis_deg": self.axis_deg, "transmittance": self.transmittance });
                if self.extinction_ratio > 0.0 {
                    pol["extinction_ratio"] = json!(self.extinction_ratio);
                }
                json!({ "polarizer": pol, "t": self.t, "semi": self.semi })
            }
            SurfKind::Waveplate => {
                let mut wp = json!({
                    "retardance": self.retardance, "wavelength_um": self.wavelength_um,
                    "axis_deg": self.axis_deg, "order": self.wp_order,
                    "transmittance": self.transmittance,
                });
                let (m1, m2) = (self.wp_material.trim(), self.wp_material2.trim());
                if !m1.is_empty() && !m2.is_empty() {
                    wp["plates"] = json!([
                        { "material": m1, "thickness_mm": self.wp_thickness },
                        { "material": m2, "thickness_mm": self.wp_thickness2, "axis_deg": 90 },
                    ]);
                } else if !m1.is_empty() {
                    wp["material"] = json!(m1);
                    if self.wp_thickness > 0.0 {
                        wp["thickness_mm"] = json!(self.wp_thickness);
                    }
                }
                json!({ "waveplate": wp, "t": self.t, "semi": self.semi })
            }
            SurfKind::Coordbrk => {
                let mut v = json!({ "coordbrk": self.cb, "t": self.t });
                if self.order != 0 {
                    v["order"] = json!(self.order);
                }
                v
            }
        }
    }
}

impl ElemKind {
    pub fn label(self) -> &'static str {
        match self {
            ElemKind::Catalog => tr("Catalog element"),
            ElemKind::Lens => tr("Custom lens"),
            ElemKind::Plate => tr("Plate"),
            ElemKind::Mirror => tr("Mirror"),
            ElemKind::Stop => tr("Aperture stop"),
            ElemKind::Splitter => tr("Beam splitter"),
            ElemKind::Zmx => tr("Element from .zmx"),
            ElemKind::Aperture => tr("Aperture / obscuration"),
            ElemKind::Surfaces => tr("Surfaces"),
            ElemKind::ZmxRange => tr(".zmx import"),
            ElemKind::Raw => tr("Block (as written)"),
        }
    }
    /// ON/OFF を持つ（絞りは無い。そのまま持つブロックは中身の on）
    pub fn has_on(self) -> bool {
        !matches!(self, ElemKind::Stop | ElemKind::Raw)
    }
    /// OFF の残し方（詰める・空気）を選べる（鏡は長さが無い）
    pub fn has_off_mode(self) -> bool {
        !matches!(self, ElemKind::Stop | ElemKind::Mirror)
    }
    /// ブロックの id の頭
    pub(crate) fn prefix(self) -> &'static str {
        match self {
            ElemKind::Catalog => "",
            ElemKind::Lens => "L",
            ElemKind::Plate => "P",
            ElemKind::Mirror => "M",
            ElemKind::Stop => "STO",
            ElemKind::Splitter => "BS",
            ElemKind::Zmx => "Z",
            ElemKind::Aperture => "AP",
            ElemKind::Surfaces => "S",
            ElemKind::ZmxRange => "ZX",
            ElemKind::Raw => "B",
        }
    }
}

/// 組合せの 1 つの要素（項目は種類ごとに使うものだけ。無い項目は既定）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pick {
    pub kind: ElemKind,
    pub vendor: String,
    pub part: String,
    pub reversed: bool,
    /// この要素の後の空気 [mm]（最後の要素では像面まで）
    pub air: f64,
    /// 自作の玉・鏡の曲率半径 [mm]（0 は平面）
    pub r1: f64,
    pub r2: f64,
    /// 玉・平板の中心厚、BS の一辺 [mm]
    pub t: f64,
    pub glass: String,
    /// 半径 [mm]
    pub semi: f64,
    /// 自作の玉・平板の半径を光線から求める
    pub semi_auto: bool,
    /// 偏心 x・y [mm]
    pub decenter: [f64; 2],
    /// 傾き x・y・z [deg]（鏡は x・y）
    pub tilt: [f64; 3],
    /// BS: 反射側へ進む
    pub arm_r: bool,
    /// .zmx の玉: ファイルと、何番目の玉か
    pub path: String,
    pub lens: usize,
    /// ロック: 中身（種類ごとの欄・面・反転）を変えない。位置・後の空気は変えられる
    pub locked: bool,
    /// ロックしたとき、位置（図のドラッグ・キー・偏心・傾き）と後の空気も変えない
    pub lock_place: bool,
    /// 鏡のコーニック
    pub conic: f64,
    /// 鏡の膜・BS の分ける面の膜（膜ファイル・LBCM の名前。空なら理想の鏡・ratio の分岐）
    pub coat: String,
    /// カタログの BS の、組合せの欄に無い項目（thickness・semi など）。そのまま持つブロック（Raw）はブロックの JSON
    pub extra: Value,
    /// 開口・遮蔽の形と寸法（開口の要素だけが持つ。[`ApData`]）。円・遮蔽の外径は semi、偏心は decenter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ap: Option<Box<ApData>>,
    /// 面の表で変えた面の値（ブロックの surface_edits になる）
    pub edits: Vec<SurfEdit>,
    /// 玉の前面を絞りにする（カタログの玉・自作の玉・平板・.zmx の玉）
    pub stop_front: bool,
    /// .zmx の玉・取り込み: マルチコンフィグの番号（1 始まり）
    pub config: usize,
    /// 自作の玉（3 面以上・貼り合わせ）と面の並びの面。自作の玉は空なら r1・r2・t・glass の 2 面
    pub surfs: Vec<SurfRow>,
    /// .zmx の取り込み: 面 from〜to（0 は既定: 面 1〜像面の手前）
    pub range: [usize; 2],
    /// 面の絞り・ソルブ・ピックアップ（ブロックの surface_ops になる）。この要素の面（surface・dst）は要素の面の
    /// 番号（座標ブレークを除く、`BlockRef::Sys(Ref::Index(i))`）、ほかの要素の面（src・other）は {"block": id, "surface": i}
    pub ops: Vec<SurfaceOp>,
    /// ON/OFF（OFF の要素は系に入らない）。状態ごとの値は state_on
    pub on: bool,
    /// OFF のとき同じ長さの空気で残す（既定は詰める）
    pub off_air: bool,
    /// 状態ごとの ON/OFF（状態の名前 → 値。無ければ on）
    pub state_on: BTreeMap<String, bool>,
    /// 状態ごとの後の空気 [mm]（無ければ air）
    pub state_air: BTreeMap<String, f64>,
    /// 状態ごとの偏心・傾き（無ければ decenter・tilt。置き方を持つ要素と鏡）
    pub state_place: BTreeMap<String, StatePlace>,
    /// BS の形・分け方・膜など（BS だけが持つ。[`Bs`]）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bs: Option<Box<Bs>>,
    /// BS: 口 B・C・D の先の並びを持つ（並びの最後に置く）
    pub branched: bool,
    /// BS: 口 B（透過）・C（反射）・D（B から戻る光の反射）の先
    pub ports: [Branch; 3],
    /// 鏡: 往路を戻る（ダブルパス・干渉計の腕の端）
    pub back: bool,
    /// 群の名前（続く同じ名前の要素を 1 つの群のブロックにする。空なら群に入れない）
    pub group: String,
    /// .zmx の取り込み: 玉モードの群（面 from〜to、名前・役割）
    pub zgroups: Vec<ZGroup>,
    /// edits・ops を付けたときの面の並びの印（[`Pick::surf_sig`]）。面の数・順・元が変わったら上書きを外す
    #[serde(skip_serializing_if = "String::is_empty")]
    pub edits_for: String,
    /// ブロックの id・後の空気の id（開いたプロジェクトの id を残す。空なら種類から付ける、[`Combo::ids_all`]）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub air_id: String,
}

/// 要素の面の値の上書き（面は要素の面のうち座標ブレークを除いた番号）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfEdit {
    pub surface: usize,
    pub field: String,
    pub value: Value,
}

impl Default for Pick {
    fn default() -> Self {
        Pick {
            kind: ElemKind::Catalog,
            vendor: String::new(),
            part: String::new(),
            reversed: false,
            air: 5.0,
            r1: 50.0,
            r2: -50.0,
            t: 4.0,
            glass: "N-BK7".into(),
            semi: 12.7,
            semi_auto: false,
            decenter: [0.0; 2],
            tilt: [0.0; 3],
            arm_r: false,
            path: String::new(),
            lens: 1,
            locked: false,
            lock_place: false,
            conic: 0.0,
            coat: String::new(),
            extra: Value::Null,
            ap: None,
            edits: Vec::new(),
            bs: None,
            branched: false,
            ports: Default::default(),
            back: false,
            stop_front: false,
            config: 1,
            surfs: Vec::new(),
            range: [0, 0],
            ops: Vec::new(),
            on: true,
            off_air: false,
            state_on: BTreeMap::new(),
            state_air: BTreeMap::new(),
            state_place: BTreeMap::new(),
            group: String::new(),
            zgroups: Vec::new(),
            edits_for: String::new(),
            id: String::new(),
            air_id: String::new(),
        }
    }
}

impl Pick {
    /// 面の上書き（edits・ops）の面番号を決める並びの印。種類・元（型番・ファイル・何番目の玉・構成・範囲）・
    /// 反転・面の並びの種類が変わると変わる
    pub fn surf_sig(&self) -> String {
        let kinds: Vec<SurfKind> = self.surfs.iter().map(|r| r.kind).collect();
        format!(
            "{:?}|{}|{}|{}|{}|{}|{:?}|{}|{:?}|{:?}",
            self.kind,
            self.vendor,
            self.part,
            self.path,
            self.lens,
            self.config,
            self.range,
            self.reversed,
            self.bs().form,
            kinds
        )
    }

    /// 面の並びが edits・ops を付けたときから変わっていれば、上書きを外して外した数を返す。
    /// 上書きが無ければ印を消し、印が無い（前の保存）上書きは今の並びのものとみなす
    pub fn drop_stale_edits(&mut self) -> usize {
        if self.edits.is_empty() && self.ops.is_empty() {
            self.edits_for.clear();
            return 0;
        }
        let sig = self.surf_sig();
        if self.edits_for.is_empty() {
            self.edits_for = sig;
            return 0;
        }
        if self.edits_for == sig {
            return 0;
        }
        let n = self.edits.len() + self.ops.len();
        self.edits.clear();
        self.ops.clear();
        self.edits_for.clear();
        n
    }

    /// 位置もロックしている（図・キーで動かせず、偏心・傾き・後の空気も変えられない）
    pub fn fixed(&self) -> bool {
        self.locked && self.lock_place
    }

    /// 前面を絞りにできる要素（玉のブロックになるもの。ユーザーカタログの項目は群なので除く）
    pub fn can_stop_front(&self) -> bool {
        match self.kind {
            ElemKind::Catalog => self.path.is_empty(),
            ElemKind::Lens | ElemKind::Plate | ElemKind::Zmx | ElemKind::ZmxRange => true,
            _ => false,
        }
    }

    /// 絞りの要素か、前面を絞りにした玉か、.zmx の絞りを使う取り込み（範囲に絞りの面があるときだけ）
    pub fn is_stop(&self) -> bool {
        match self.kind {
            ElemKind::Stop => true,
            ElemKind::Raw => self.raw_is_stop(),
            ElemKind::ZmxRange => self.stop_front && self.range_has_stop(),
            _ => self.stop_front && self.can_stop_front(),
        }
    }

    /// .zmx の取り込みの範囲に .zmx の絞りの面があるか（ファイルが読めなければ、あるとみなす）
    pub fn range_has_stop(&self) -> bool {
        let Some((stop, image)) = zmx_stop_info(&self.path) else {
            return true;
        };
        let a = if self.range[0] == 0 { 1 } else { self.range[0] };
        let z = if self.range[1] == 0 {
            image.saturating_sub(1)
        } else {
            self.range[1]
        };
        a <= stop && stop <= z
    }

    pub fn new(kind: ElemKind) -> Pick {
        let mut p = Pick {
            kind,
            ..Pick::default()
        };
        match kind {
            ElemKind::Plate => {
                // 平板の面は平ら（ブロックにも半径を書かない。開き直しても同じ欄になるように）
                (p.r1, p.r2) = (0.0, 0.0);
                p.t = 3.0;
            }
            ElemKind::Mirror => {
                p.r1 = 0.0;
                p.tilt = [45.0, 0.0, 0.0];
                p.semi = 12.7;
                p.air = 20.0;
            }
            ElemKind::Surfaces => {
                p.surfs = vec![SurfRow::default()];
            }
            ElemKind::ZmxRange => {
                // .zmx の絞りをそのまま使う（範囲に絞りが無ければ外す）
                p.stop_front = true;
                p.air = 20.0;
            }
            ElemKind::Stop => {
                p.semi = 2.5;
                p.air = 2.0;
            }
            ElemKind::Splitter => {
                p.t = 12.7;
                p.air = 10.0;
                p.bs = Some(Box::default());
            }
            ElemKind::Aperture => {
                p.semi = 5.0;
                p.air = 10.0;
                p.ap = Some(Box::default());
            }
            _ => {}
        }
        p
    }

    /// 一覧・図に出す名前
    pub fn label(&self) -> String {
        match self.kind {
            ElemKind::Catalog => format!("{} {}", self.vendor, self.part),
            ElemKind::Zmx => {
                let f = std::path::Path::new(&self.path)
                    .file_name()
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_default();
                format!("{f} #{}", self.lens)
            }
            // カタログの鏡・BS は型番
            ElemKind::Mirror | ElemKind::Splitter if !self.part.is_empty() => {
                format!("{} {}", self.vendor, self.part)
            }
            ElemKind::Aperture => self.ap().shape.label().to_string(),
            ElemKind::Raw => self.raw_label(),
            ElemKind::ZmxRange => {
                let f = std::path::Path::new(&self.path)
                    .file_name()
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match self.range {
                    [0, 0] => f,
                    [a, b] => format!(
                        "{f} {}〜{}",
                        if a == 0 { "1".into() } else { a.to_string() },
                        if b == 0 { "".into() } else { b.to_string() }
                    ),
                }
            }
            k => k.label().to_string(),
        }
    }

    /// 開口・遮蔽の面の開口（project の SurfAperture）
    fn aperture(&self) -> Value {
        let [dx, dy] = self.decenter;
        match self.ap().shape {
            ApShape::Circle => json!({
                "type": "circular", "min": self.ap().min.max(0.0), "max": self.semi, "dx": dx, "dy": dy,
            }),
            ApShape::Rect => json!({
                "type": "rect", "hx": self.ap().half[0], "hy": self.ap().half[1], "dx": dx, "dy": dy,
            }),
            ApShape::Ellipse => json!({
                "type": "ellipse", "hx": self.ap().half[0], "hy": self.ap().half[1], "dx": dx, "dy": dy,
            }),
            ApShape::Obscuration => json!({
                "type": "obscuration", "min": self.ap().min.max(0.0), "max": self.semi, "dx": dx, "dy": dy,
            }),
            ApShape::Spider => json!({
                "type": "spider", "width": self.ap().spider_width.max(0.0), "arms": self.ap().spider_arms.max(1),
                "dx": dx, "dy": dy,
            }),
        }
    }

    /// 開口・遮蔽の、図の縦の広がり（光軸から）。外の縁
    pub fn ap_extent(&self) -> f64 {
        let h = match self.ap().shape {
            ApShape::Rect | ApShape::Ellipse => self.ap().half[0].hypot(self.ap().half[1]),
            // スパイダーの腕は無限に伸びるので、図では半径（semi）まで
            ApShape::Circle | ApShape::Obscuration | ApShape::Spider => self.semi,
        };
        h + self.decenter[0].hypot(self.decenter[1])
    }

    fn place(&self, b: &mut Value) {
        if self.decenter != [0.0; 2] {
            b["decenter"] = json!(self.decenter);
        }
        if self.tilt != [0.0; 3] {
            b["tilt"] = json!(self.tilt);
        }
    }

    pub fn radius(r: f64) -> Value {
        if r == 0.0 {
            Value::Null
        } else {
            json!(r)
        }
    }

    /// ブロック（id 付き）
    pub fn block(&self, id: &str) -> Value {
        let mut b = match self.kind {
            ElemKind::Raw => return self.raw_block(id),
            // lensbench 形式のカタログの玉（ユーザーカタログの項目）
            ElemKind::Catalog if !self.path.is_empty() => json!({
                "type": "user", "id": id, "catalog": self.path, "item": self.part,
            }),
            ElemKind::Catalog => json!({
                "type": "lens", "id": id,
                "source": { "vendor": self.vendor, "part": self.part },
            }),
            ElemKind::Zmx => {
                let mut b = json!({
                    "type": "lens", "id": id,
                    "source": { "zmx": self.path, "lens": self.lens.max(1) },
                });
                if self.config > 1 {
                    b["source"]["config"] = json!(self.config);
                }
                b
            }
            ElemKind::Lens if self.surfs.len() >= 2 => {
                let n = self.surfs.len();
                let rows: Vec<Value> = self
                    .surfs
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let mut v = SurfRow {
                            kind: SurfKind::Standard,
                            ..r.clone()
                        }
                        .json();
                        // 最後の面の後は空気（間隔は要素の後の空気）
                        if i + 1 == n {
                            v.as_object_mut()
                                .map(|o| (o.remove("glass"), o.remove("t")));
                        }
                        v
                    })
                    .collect();
                json!({ "type": "lens", "id": id, "source": { "surfaces": rows } })
            }
            ElemKind::Surfaces => json!({
                "type": "surfaces", "id": id,
                "surfaces": self.surfs.iter().map(SurfRow::json).collect::<Vec<_>>(),
            }),
            ElemKind::ZmxRange => {
                let mut b = json!({ "type": "zmx", "id": id, "path": self.path });
                if self.range[0] > 0 {
                    b["from"] = json!(self.range[0]);
                }
                if self.range[1] > 0 {
                    b["to"] = json!(self.range[1]);
                }
                if self.config > 1 {
                    b["config"] = json!(self.config);
                }
                if !self.is_stop() {
                    b["stop"] = json!(false);
                }
                let gs: Vec<Value> = self.zgroups.iter().filter_map(ZGroup::json).collect();
                if !gs.is_empty() {
                    b["groups"] = Value::Array(gs);
                }
                b
            }
            ElemKind::Lens | ElemKind::Plate => {
                let (r1, r2) = if self.kind == ElemKind::Plate {
                    (0.0, 0.0)
                } else {
                    (self.r1, self.r2)
                };
                json!({
                    "type": "lens", "id": id,
                    "source": { "surfaces": [
                        { "radius": Self::radius(r1), "t": self.t, "glass": self.glass,
                          "semi": semi_json(self.semi, self.semi_auto) },
                        { "radius": Self::radius(r2), "semi": semi_json(self.semi, self.semi_auto) },
                    ] },
                })
            }
            ElemKind::Mirror => json!({
                "type": "mirror", "id": id, "radius": Self::radius(self.r1), "conic": self.conic,
                "tilt": [self.tilt[0], self.tilt[1]], "semi": self.semi,
            }),
            ElemKind::Stop => json!({ "type": "stop", "id": id, "semi": self.semi }),
            ElemKind::Aperture => json!({
                "type": "surfaces", "id": id,
                "surfaces": [{ "t": 0.0, "semi": self.ap_extent(), "aperture": self.aperture() }],
            }),
            ElemKind::Splitter => self.splitter_block(id),
        };
        if self.kind == ElemKind::Mirror {
            if !self.coat.trim().is_empty() {
                b["coat"] = json!(self.coat.trim());
            }
            if self.back {
                b["back"] = json!(true);
            }
        }
        // カタログの BS の、欄の無い項目（組合せの欄のある項目は欄の値のまま）
        if self.kind == ElemKind::Splitter {
            if let (Some(m), Some(o)) = (self.extra.as_object(), b.as_object_mut()) {
                for (k, v) in m {
                    if !o.contains_key(k) {
                        o.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        // 玉の反転・前面の絞り（ユーザーカタログの項目は群として入るので無い）
        let lens = matches!(
            self.kind,
            ElemKind::Catalog | ElemKind::Zmx | ElemKind::Lens | ElemKind::Plate
        );
        if lens && !(self.kind == ElemKind::Catalog && !self.path.is_empty()) {
            if self.reversed {
                b["reversed"] = json!(true);
            }
            if self.stop_front {
                b["stop"] = json!(true);
            }
        }
        // 置き方（鏡は傾きを本体に、絞り・開口は置き方を持たない）
        if lens
            || matches!(
                self.kind,
                ElemKind::Splitter | ElemKind::Surfaces | ElemKind::ZmxRange
            )
        {
            self.place(&mut b);
        }
        // ON/OFF
        if self.kind.has_on() {
            if !self.on {
                b["on"] = json!(false);
            }
            if self.off_air && self.kind.has_off_mode() {
                b["off"] = json!("air");
            }
        }
        b
    }

    /// 自作の玉の面の表（2 面の単玉なら r1・r2・t・glass から作る）
    pub fn lens_rows(&self) -> Vec<SurfRow> {
        if self.surfs.len() >= 2 {
            return self.surfs.clone();
        }
        vec![
            SurfRow {
                radius: self.r1,
                t: self.t,
                glass: self.glass.clone(),
                semi: self.semi,
                semi_auto: self.semi_auto,
                ..Default::default()
            },
            SurfRow {
                radius: self.r2,
                semi: self.semi,
                semi_auto: self.semi_auto,
                ..Default::default()
            },
        ]
    }

    /// 状態 st での ON/OFF
    pub fn on_in(&self, st: Option<&str>) -> bool {
        st.and_then(|n| self.state_on.get(n).copied())
            .unwrap_or(self.on)
    }

    /// 状態 st での後の空気
    pub fn air_in(&self, st: Option<&str>) -> f64 {
        st.and_then(|n| self.state_air.get(n).copied())
            .unwrap_or(self.air)
    }

    /// 状態ごとに偏心・傾きを変えられる要素か（置き方を持つ要素と鏡）
    pub fn has_state_place(&self) -> bool {
        place_fields(self.kind) || self.kind == ElemKind::Mirror
    }

    /// 状態 st での偏心・傾き
    pub fn place_in(&self, st: Option<&str>) -> StatePlace {
        st.and_then(|n| self.state_place.get(n).copied())
            .filter(|_| self.has_state_place())
            .unwrap_or(StatePlace {
                decenter: self.decenter,
                tilt: self.tilt,
            })
    }

    /// 状態 st で変える偏心・傾き（状態なら状態の値を作る）
    pub fn place_mut(&mut self, st: Option<&str>) -> (&mut [f64; 2], &mut [f64; 3]) {
        match st {
            Some(n) if self.has_state_place() => {
                let base = StatePlace {
                    decenter: self.decenter,
                    tilt: self.tilt,
                };
                let e = self.state_place.entry(n.to_string()).or_insert(base);
                (&mut e.decenter, &mut e.tilt)
            }
            _ => (&mut self.decenter, &mut self.tilt),
        }
    }

    /// 状態 n で変えた値があるか
    pub fn has_state_values(&self, n: &str) -> bool {
        self.state_on.contains_key(n)
            || self.state_air.contains_key(n)
            || self.state_place.contains_key(n)
    }

    /// 状態 n で変えた値をやめる
    pub fn clear_state(&mut self, n: &str) {
        self.state_on.remove(n);
        self.state_air.remove(n);
        self.state_place.remove(n);
    }

    /// 状態 st で変える後の空気（状態なら状態の値を作る）
    pub fn air_mut(&mut self, st: Option<&str>) -> &mut f64 {
        match st {
            Some(n) => self.state_air.entry(n.to_string()).or_insert(self.air),
            None => &mut self.air,
        }
    }
}

/// 状態ごとの偏心 [mm]・傾き [deg]（[`Pick::state_place`]）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StatePlace {
    pub decenter: [f64; 2],
    pub tilt: [f64; 3],
}

/// 組合せ（要素の並びと、系の条件。系の設定は [`crate::sysset`]）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Combo {
    pub name: String,
    pub picks: Vec<Pick>,
    /// 置く機構部品
    pub mechanics: Vec<MechPick>,
    /// 開口の種類
    pub aperture: ApKind,
    /// 開口の値（F 値・NA など。EPD・Float では使わない）
    pub ap_value: f64,
    /// 絞りの径 [mm]。開口が EPD なら入射瞳径。絞りの要素が無ければ、絞りは最初の要素の前
    pub epd: f64,
    /// 絞りから最初の要素まで [mm]
    pub stop_gap: f64,
    /// 視野の種類
    pub field_kind: FieldKind,
    /// 条件で定める視野の条件
    pub field_solve: FieldSolve,
    /// 視野（x, y と重み）
    pub fields: Vec<FieldPt>,
    /// 波長 [µm] と重み
    pub waves: Vec<WavePt>,
    /// 主波長（1 始まり）
    pub primary: usize,
    /// 物体面から最初の要素（前の絞り）まで [mm]。None なら無限遠
    pub object_distance: Option<f64>,
    /// 物体側テレセントリック（物体が有限のときだけ）
    pub telecentric: bool,
    pub aiming: Aiming,
    /// アフォーカル像空間の角度の単位（ur・mr・rr・as・am・dg）。None ならアフォーカルにしない
    pub afocal: Option<String>,
    /// 浮動開口（半直径を固定した面）の外の光線をけるか（プロジェクトの floating_clip）。既定（ける）は書かない
    #[serde(skip_serializing_if = "Clone::clone")]
    pub floating_clip: bool,
    /// BS を通る順の透過 T・反射 R の並び（プロジェクトの path。足りない文字は BS の既定の口）
    pub path: String,
    /// 硝材カタログ（AGF の名前、前ほど優先。プロジェクトの glass_catalogs）
    pub glass_catalogs: Vec<String>,
    /// 状態の名前（プロジェクトの states。要素ごとの値は Pick の state_on・state_air）
    pub states: Vec<String>,
    /// 今の状態（None は基本）
    pub state: Option<String>,
    /// 状態ごとの系の値（開口・視野・波長・物体距離など。無ければ基本の値）
    pub state_sys: BTreeMap<String, crate::sysset::SysSnap>,
    /// 状態ごとの BS の経路（無ければ path）
    pub state_path: BTreeMap<String, String>,
    /// 膜（フィルタ）の定義（プロジェクトの coatings）
    pub coatings: BTreeMap<String, lensbench_project::filter::CoatSpec>,
    /// 群の役割・移動（名前は要素の group）
    pub groups: Vec<GroupDef>,
    /// 最後に置く検出器（拡張の検出器のブロック、[`crate::ext::Detector`]）
    pub detector: Option<DetectorPick>,
    /// 最初の要素の前の空気（前の絞りが無いとき。開いたプロジェクトの先頭の空気、往路を戻る鏡は復路でも通る）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_air: Option<f64>,
    /// 欄に出さない系の値（視野の正規化・GLRS・温度と気圧）
    #[serde(skip_serializing_if = "SysExtra::is_empty")]
    pub extra: SysExtra,
}

/// 組合せの群（プロジェクトの group ブロック）の役割と移動
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GroupDef {
    pub name: String,
    /// zoom・focus・fixed（空は無し）
    pub role: String,
    /// 群の移動 [mm]（光の進む向きが正。前の間隔に足し、後の間隔から引く）
    pub shift: f64,
    /// 状態ごとの移動
    pub state_shift: BTreeMap<String, f64>,
}

impl Default for Combo {
    fn default() -> Self {
        Combo {
            name: tr("Catalog lens combination").into(),
            picks: Vec::new(),
            mechanics: Vec::new(),
            aperture: ApKind::Epd,
            ap_value: 0.0,
            epd: 5.0,
            stop_gap: 2.0,
            field_kind: FieldKind::Angle,
            field_solve: FieldSolve::default(),
            fields: [0.0, 1.0, 2.0].map(FieldPt::y).to_vec(),
            waves: [0.4861327, 0.5875618, 0.6562725].map(WavePt::um).to_vec(),
            primary: 2,
            object_distance: None,
            telecentric: false,
            aiming: Aiming::Off,
            afocal: None,
            floating_clip: true,
            path: String::new(),
            glass_catalogs: vec!["SCHOTT".into()],
            states: Vec::new(),
            state: None,
            state_sys: BTreeMap::new(),
            state_path: BTreeMap::new(),
            coatings: BTreeMap::new(),
            groups: Vec::new(),
            detector: None,
            lead_air: None,
            extra: SysExtra::default(),
        }
    }
}

impl Combo {
    /// 要素の数（分岐の先も）・状態の数・面の上書きの数（消す操作の後に「Ctrl+Z で戻す」を出す、GPN-20）
    pub fn census(&self) -> [usize; 3] {
        fn walk(ps: &[Pick], n: &mut [usize; 3]) {
            for p in ps {
                n[0] += 1;
                n[2] += p.edits.len() + p.ops.len();
                for b in &p.ports {
                    walk(&b.picks, n);
                }
            }
        }
        let mut n = [0, self.states.len(), 0];
        walk(&self.picks, &mut n);
        n
    }

    /// 面の並びが変わった要素（分岐の先も）の面の上書きを外す（GPN-02）。外した数
    pub fn drop_stale_edits(&mut self) -> usize {
        fn walk(ps: &mut [Pick]) -> usize {
            let mut n = 0;
            for p in ps {
                n += p.drop_stale_edits();
                for b in p.ports.iter_mut() {
                    n += walk(&mut b.picks);
                }
            }
            n
        }
        walk(&mut self.picks)
    }

    /// 要素（分岐の先も）の .zmx のファイルのパスを f で変える（設定の相対パス）
    pub fn map_paths(&mut self, f: &impl Fn(&str) -> String) {
        fn walk(ps: &mut [Pick], f: &impl Fn(&str) -> String) {
            for p in ps {
                if !p.path.is_empty() {
                    p.path = f(&p.path);
                }
                for b in p.ports.iter_mut() {
                    walk(&mut b.picks, f);
                }
            }
        }
        walk(&mut self.picks, f);
        // 機構部品のカタログ
        for m in &mut self.mechanics {
            if !m.catalog.is_empty() {
                m.catalog = f(&m.catalog);
            }
        }
    }

    /// 前に絞りを置くか（絞りの要素が分岐の先にも無いとき）
    pub fn lead_stop(&self) -> bool {
        !self
            .all()
            .iter()
            .any(|l| self.get(l).is_some_and(Pick::is_stop))
            && self.stop_face().is_none()
    }

    /// 絞りにした要素の面（要素の場所、要素の面の番号）
    pub fn stop_face(&self) -> Option<(Loc, usize)> {
        self.all().into_iter().find_map(|l| {
            let i = self.get(&l)?.ops.iter().find_map(SurfaceOp::stop_index)?;
            Some((l.clone(), i))
        })
    }

    /// 絞りを場所 loc の要素の面 i に（None なら要素の面の絞りをやめる）
    pub fn set_stop_face(&mut self, at: Option<(Loc, usize)>) {
        for l in self.all() {
            if let Some(p) = self.get_mut(&l) {
                p.ops.retain(|o| !matches!(o, SurfaceOp::SetStop(_)));
                // 玉の前面の絞りも面の絞りに置き換える
                if at.is_some() {
                    p.stop_front = false;
                }
            }
        }
        if let Some((l, i)) = at {
            if let Some(p) = self.get_mut(&l) {
                p.ops.push(SurfaceOp::SetStop(StopOp {
                    surface: BlockRef::Sys(Ref::Index(i)),
                }));
            }
        }
    }

    /// 今の経路に沿った要素のブロックの id（= 玉・群の名前、[`Combo::ids_all`]）
    pub fn ids(&self) -> Vec<String> {
        let mut all: BTreeMap<Loc, String> = self.ids_all().into_iter().collect();
        self.chain()
            .iter()
            .map(|s| all.remove(&s.loc).unwrap_or_default())
            .collect()
    }

    /// プロジェクトファイル（JSON）にする
    pub fn project(&self) -> Value {
        let mut blocks = Vec::new();
        if self.lead_stop() {
            blocks.push(json!({ "type": "stop", "semi": self.epd / 2.0 }));
            blocks.push(json!({ "type": "air", "t": self.stop_gap }));
        } else if let Some(t) = self.lead_air {
            blocks.push(json!({ "type": "air", "t": t }));
        }
        let grouped = self.blocks(&mut blocks);
        // 状態
        let mut states: Vec<(String, serde_json::Map<String, Value>)> = self
            .states
            .iter()
            .map(|n| (n.clone(), serde_json::Map::new()))
            .collect();
        for (loc, id) in self.ids_all() {
            let Some(p) = self.get(&loc) else { continue };
            for (n, set) in states.iter_mut() {
                if let Some(&on) = p.state_on.get(n).filter(|_| p.kind.has_on()) {
                    set.insert(id.clone(), json!({ "on": on }));
                }
                if let (Some(&t), Some(a)) = (p.state_air.get(n), p.air_name(&id)) {
                    set.insert(a, json!({ "t": t }));
                }
                if let Some(pl) = p.state_place.get(n).filter(|_| p.has_state_place()) {
                    let e = set.entry(id.clone()).or_insert_with(|| json!({}));
                    if p.kind == ElemKind::Mirror {
                        e["tilt"] = json!([pl.tilt[0], pl.tilt[1]]);
                    } else {
                        e["decenter"] = json!(pl.decenter);
                        e["tilt"] = json!(pl.tilt);
                    }
                }
            }
        }
        for g in self
            .all()
            .iter()
            .filter_map(|l| self.get(l))
            .flat_map(|p| &p.zgroups)
        {
            for (n, set) in states.iter_mut() {
                if let Some(m) = g.state_set(n) {
                    set.insert(g.key().to_string(), Value::Object(m));
                }
            }
        }
        // 群の移動。群のブロックにした名前だけ（要素から外した群・要素の名前と同じ名前は書かない）
        for d in self.groups.iter().filter(|d| grouped.contains(&d.name)) {
            for (n, set) in states.iter_mut() {
                if let Some(&x) = d.state_shift.get(n) {
                    set.insert(d.name.clone(), json!({ "shift": x }));
                }
            }
        }
        // 検出器は最後のブロック（前の空気が検出器までの距離）
        if let Some(d) = &self.detector {
            if let Ok(b) = d.block() {
                blocks.push(b);
            }
        }
        let mut v = json!({ "name": self.name });
        self.put_settings(&mut v);
        v["blocks"] = Value::Array(blocks);
        let path: String = self
            .path
            .chars()
            .map(|c| c.to_ascii_uppercase())
            .filter(|c| matches!(c, 'T' | 'R'))
            .collect();
        if !path.is_empty() {
            v["path"] = json!(path);
        }
        if !self.mechanics.is_empty() {
            v["mechanics"] = Value::Array(self.mechanics.iter().map(MechPick::json).collect());
        }
        if !self.coatings.is_empty() {
            v["coatings"] = serde_json::to_value(&self.coatings).unwrap_or_default();
        }
        if !states.is_empty() {
            v["states"] = Value::Array(
                states
                    .into_iter()
                    .map(|(name, set)| {
                        let pr = self.state_project(&name);
                        if pr.is_empty() {
                            json!({ "name": name, "set": set })
                        } else {
                            json!({ "name": name, "set": set, "project": pr })
                        }
                    })
                    .collect(),
            );
        }
        v
    }

    /// 今の状態（名前が states にあるときだけ）
    pub fn active_state(&self) -> Option<&str> {
        self.state
            .as_deref()
            .filter(|n| self.states.iter().any(|s| s == n))
    }

    /// 要素と系の玉の対応に使う id（今の状態で OFF の要素は空。[`LayoutData::spans_for`]）
    pub fn span_ids(&self) -> Vec<String> {
        let st = self.active_state();
        self.ids()
            .into_iter()
            .zip(self.chain_view().picks)
            .map(|(id, p)| if p.on_in(st) { id } else { String::new() })
            .collect()
    }

    /// 経路に沿った要素 k を光軸に沿って d [mm] 動かす（前の間隔を d 増やし、後の間隔を d 減らす）
    pub fn shift(&mut self, k: usize, d: f64) {
        let Some(s) = self.chain().get(k).cloned() else {
            return;
        };
        if let Some(g) = &s.before {
            if let Some(v) = self.gap(g) {
                *v = if *g == Gap::Stop {
                    (*v + d).max(0.0)
                } else {
                    *v + d
                };
            }
        }
        if let Some(v) = s.after.as_ref().and_then(|g| self.gap(g)) {
            *v -= d;
        }
    }
}

/// serve の `layout` の結果
#[derive(Debug, Clone, Default)]
pub struct LayoutData {
    pub solids: Vec<Solid>,
    /// 像面・物体面などの線
    pub plates: Vec<(String, usize, Vec<[f64; 2]>)>,
    /// 機構部品の名前と、壁の断面の四角
    pub mechs: Vec<(String, Vec<Vec<[f64; 2]>>)>,
    /// 光線の色の番号（1 始まり。視野、構成を重ねたときは構成）と点
    pub rays: Vec<(usize, Vec<[f64; 2]>)>,
    /// 重ねた構成の名前（2 つ以上のとき光線は構成ごとの色）
    pub configs: Vec<String>,
    /// 面の頂点 [z, y]
    pub vertices: Vec<Option<[f64; 2]>>,
    pub stop: Option<usize>,
    /// 絞りの面の半径 [mm]（layout の surfaces の semi。無ければ組合せの epd/2）
    pub stop_semi: Option<f64>,
    /// id のあるブロック・群と、その往路の面の範囲（serve の `blocks`。要素の面の範囲は id でここから引く、GPN-01）
    pub blocks: Vec<(String, Vec<[usize; 2]>)>,
    /// xz 面の図（縦は x）。偏心・傾きの操作も x の向き
    pub xz: bool,
    /// 光路に沿って展開した図。光軸に沿う操作だけ（偏心・傾きは向きが裏返る区間があるので止める）
    pub unfold: bool,
    /// 3D 表示の場面（`plane: "3d"`）。このときの solids・rays は空
    pub d3: Option<std::sync::Arc<crate::view3d::Scene3>>,
}

#[derive(Debug, Clone, Default)]
pub struct Solid {
    pub mirror: bool,
    pub surfaces: [usize; 2],
    pub pts: Vec<[f64; 2]>,
}

/// serve の `blocks`（load・layout の返事）: id のあるブロック・群と、その往路の面の範囲
pub fn block_spans(v: &Value) -> Vec<(String, Vec<[usize; 2]>)> {
    let us = |x: &Value| x.as_u64().unwrap_or(0) as usize;
    v["blocks"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|b| {
                    (
                        b["id"].as_str().unwrap_or_default().to_string(),
                        b["forward"]
                            .as_array()
                            .map(|a| a.iter().map(|x| [us(&x[0]), us(&x[1])]).collect())
                            .unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn pts(v: &Value) -> Vec<[f64; 2]> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                .collect()
        })
        .unwrap_or_default()
}

impl LayoutData {
    pub fn parse(v: &Value) -> LayoutData {
        let arr = |k: &str| v[k].as_array().cloned().unwrap_or_default();
        let us = |x: &Value| x.as_u64().unwrap_or(0) as usize;
        let configs: Vec<String> = arr("configs")
            .iter()
            .map(|x| x.as_str().unwrap_or_default().to_string())
            .collect();
        let by_cfg = configs.len() > 1;
        let color = v["color"].as_str().unwrap_or_default();
        LayoutData {
            solids: arr("solids")
                .iter()
                .map(|s| Solid {
                    mirror: s["kind"] == "mirror",
                    surfaces: [us(&s["surfaces"][0]), us(&s["surfaces"][1])],
                    pts: pts(&s["pts"]),
                })
                .collect(),
            plates: arr("plates")
                .iter()
                .map(|p| {
                    (
                        p["kind"].as_str().unwrap_or_default().to_string(),
                        us(&p["surface"]),
                        pts(&p["pts"]),
                    )
                })
                // 3D: 像面などは網に（要素と面の対応に像面の番号が要る）
                .chain(
                    arr("meshes")
                        .iter()
                        .filter(|m| !matches!(m["kind"].as_str(), Some("glass" | "mirror")))
                        .map(|m| {
                            (
                                m["kind"].as_str().unwrap_or_default().to_string(),
                                us(&m["surfaces"][0]),
                                Vec::new(),
                            )
                        }),
                )
                .collect(),
            mechs: arr("mechs")
                .iter()
                .map(|m| {
                    (
                        m["name"].as_str().unwrap_or_default().to_string(),
                        m["quads"]
                            .as_array()
                            .map(|a| a.iter().map(pts).collect())
                            .unwrap_or_default(),
                    )
                })
                .collect(),
            rays: arr("rays")
                .iter()
                .map(|r| {
                    let c = crate::layout_rays::ray_class(r, color, by_cfg);
                    (c, pts(&r["pts"]))
                })
                .collect(),
            configs,
            vertices: arr("surfaces")
                .iter()
                .map(|s| {
                    let a = s["vertex"].as_array()?;
                    // 3D の頂点 [x, y, z] は [z, y] に
                    if a.len() == 3 {
                        Some([a[2].as_f64()?, a[1].as_f64()?])
                    } else {
                        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
                    }
                })
                .collect(),
            stop: arr("surfaces").iter().position(|s| s["stop"] == true),
            stop_semi: arr("surfaces")
                .iter()
                .find(|s| s["stop"] == true)
                .and_then(|s| s["semi"].as_f64())
                .filter(|r| r.is_finite() && *r > 0.0),
            blocks: block_spans(v),
            xz: v["plane"] == "xz",
            unfold: v["unfold"] == true,
            d3: (v["plane"] == "3d")
                .then(|| std::sync::Arc::new(crate::view3d::Scene3::parse(v, by_cfg))),
        }
    }

    /// 要素ごとの面の範囲（最初・最後）。要素のブロックの id で serve の `blocks` から引く（GPN-01: 前後の面からの推定は
    /// しない）。往路で同じブロックを 2 度通る系は、前の要素より後の通りを当てる
    pub fn spans_for(&self, ids: &[String], lead_stop: bool) -> Vec<Option<[usize; 2]>> {
        let mut v: Vec<Option<[usize; 2]>> = Vec::with_capacity(ids.len());
        let mut prev = if lead_stop { self.stop } else { None };
        for id in ids {
            // 空の id（OFF の要素）は系に無い
            let s = (!id.is_empty())
                .then(|| self.block_after(id, prev, ids))
                .flatten();
            if let Some(s) = s {
                prev = Some(s[1]);
            }
            v.push(s);
        }
        v
    }

    /// id のブロックの、面 after より後の往路の通りの範囲。続く通りは、ほかの要素（ids）の面が間に入るまでつなぐ
    fn block_after(&self, id: &str, after: Option<usize>, ids: &[String]) -> Option<[usize; 2]> {
        let (_, all) = self.blocks.iter().find(|(n, _)| n == id)?;
        let mut sp: Vec<[usize; 2]> = all.clone();
        sp.sort();
        let first = sp
            .iter()
            .copied()
            .find(|s| after.is_none_or(|a| s[0] > a))
            .or_else(|| sp.first().copied())?;
        let limit = self
            .blocks
            .iter()
            .filter(|(n, _)| n != id && ids.contains(n))
            .flat_map(|(_, s)| s.iter().map(|x| x[0]))
            .filter(|&x| x > first[0])
            .min()
            .unwrap_or(usize::MAX);
        let end = sp
            .iter()
            .filter(|s| s[0] >= first[0] && s[0] < limit)
            .map(|s| s[1])
            .max()
            .unwrap_or(first[1]);
        Some([first[0], end])
    }

    /// 物体が最初の面より後ろ（虚物体）のとき、入る光を最初の面の前から描く。
    /// 光線の最初の点（後ろの物点）を、最初の面に当たる点から前へ物点までの距離の半分だけ伸ばした点に替え、物体面の線は消す
    pub fn virtual_object_from_front(&mut self) {
        let (Some(Some(o)), Some(Some(s1))) = (self.vertices.first(), self.vertices.get(1)) else {
            return;
        };
        let d = o[0] - s1[0];
        if d <= 1e-9 {
            return;
        }
        let len = 0.5 * d;
        for (_, p) in &mut self.rays {
            if p.len() < 2 {
                continue;
            }
            let (a, b) = (p[0], p[1]);
            if (a[0] - b[0]).abs() < 1e-12 {
                continue;
            }
            let k = (a[1] - b[1]) / (a[0] - b[0]);
            p[0] = [b[0] - len, b[1] - k * len];
        }
        self.plates.retain(|x| x.0 != "object");
    }

    /// 像面の面の番号
    pub fn image_surface(&self) -> Option<usize> {
        self.plates.iter().find(|p| p.0 == "image").map(|p| p.1)
    }

    fn image_vertex(&self) -> Option<[f64; 2]> {
        let (_, j, _) = self.plates.iter().find(|p| p.0 == "image")?;
        self.vertices.get(*j).copied().flatten()
    }
}

/// 像面の前の面の座標での、軸上の細い光線（位置 y・z、方向 m・n）が軸と交わる所までの距離（光の進む向きに正）
pub fn paraxial_focus(y: f64, z: f64, m: f64, n: f64) -> Option<f64> {
    if m.abs() < 1e-15 || !(y.is_finite() && z.is_finite() && m.is_finite() && n.is_finite()) {
        return None;
    }
    let t = z - y * n / m;
    // 鏡の後ろ（n < 0）は面の座標の z が逆向き
    Some(if n < 0.0 { -t } else { t })
}

/// レイアウト図の画面の状態（選んだ要素・寸法）
#[derive(Default)]
pub struct View {
    pub sel: Option<usize>,
    /// 足す状態の名前の欄
    pub new_state: String,
    /// 選んだ寸法（要素 k の後の空気。usize::MAX は絞りから最初の要素）
    pub dim: Option<usize>,
    /// ドラッグの始まりの組合せ（ドラッグ中の動きを始まりからの差で決める）
    drag0: Option<(Pos2, Combo)>,
    /// 寸法の数字をその場で書き換え中（寸法、文字、入力欄にフォーカスを渡したか）
    edit: Option<(usize, String, bool)>,
    /// 要素ごとの画面の上の光軸の向き（単位ベクトル、画面の座標）
    axes: Vec<Vec2>,
    /// 要素ごとの画面の上の真ん中（カタログから落としたときの差し込み位置を決める）
    mids: Vec<Option<Pos2>>,
    /// 面の表で見る要素（図の要素をダブルクリックした）
    open: Option<usize>,
    /// ユーザーカタログに保存する要素
    save: Option<usize>,
    /// 図が xz 面（[`LayoutData::xz`]）
    xz: bool,
    /// 偏心・傾きの操作を止める（展開した図・3D）
    fixed: bool,
    /// 3D 表示（矢印キーは図を回す）
    d3: bool,
    /// 2D の図の拡大縮小・移動を使う（レイアウトのパネルの図。カタログのプレビューは使わない）
    pub nav: bool,
    /// 2D の図の拡大率の対数（0 で全体表示）と、画面の上の移動 [pt]
    zoom_log: f32,
    pan: Vec2,
    /// 面の表で選んだ面（図で目立たせる、#125）
    pub hl_surf: Option<usize>,
    /// ポインタの下の玉の面の範囲（開いた系の図のクリックで面の表の行を選ぶ）
    pub hover_surfs: Option<[usize; 2]>,
    /// 面の頂点の画面の位置（開いた系の図へカタログから落とすとき、差し込む間隔を決める）
    pub surf_pos: Vec<Option<Pos2>>,
}

/// カタログの行をレイアウト図へドラッグするときの荷物
pub struct DropPick(pub Pick);

impl View {
    /// 2D の図を画面の点 c を中心に f 倍する（c の下の図の点は動かない）。fit_center は全体表示の中心
    fn zoom_about(&mut self, fit_center: Pos2, c: Pos2, f: f32) {
        let old = self.zoom_log;
        self.zoom_log = (self.zoom_log + f.ln()).clamp(-3.0, 8.0);
        let k = (self.zoom_log - old).exp();
        let o = fit_center + self.pan;
        self.pan = (o - c) * k + c.to_vec2() - fit_center.to_vec2();
    }

    /// 図の縦の向きの偏心の番号（yz 面は y、xz 面は x）
    fn vert(&self) -> usize {
        if self.xz {
            0
        } else {
            1
        }
    }

    /// 図の縦の向きの偏心 d mm と傾き（縦へ光軸を向ける向き）a° を足す。xz 面では y 軸まわりの傾き
    /// （yz 面の x 軸まわりと、画面で同じ向きに回るよう符号を変える）
    /// 状態 st を選んでいれば、その状態の値を変える
    fn nudge(&self, p: &mut Pick, st: Option<&str>, d: f64, a: f64) {
        if self.fixed || (d == 0.0 && a == 0.0) {
            return;
        }
        // 開口・遮蔽の偏心は面の開口の値（状態ごとには変えない）
        let (dec, tilt) = if p.kind == ElemKind::Aperture {
            (&mut p.decenter, &mut p.tilt)
        } else {
            p.place_mut(st)
        };
        if a != 0.0 {
            if self.xz {
                tilt[1] -= a;
            } else {
                tilt[0] += a;
            }
        }
        if d != 0.0 {
            dec[self.vert()] += d;
        }
    }

    /// 画面の向き d（矢印キー・ドラッグ）を、要素 k の光軸に沿う量と、それに垂直な量（画面で光軸の左手、
    /// 折り曲げなしなら上）に分ける
    fn along(&self, k: usize, d: Vec2) -> (f32, f32) {
        let u = self.axes.get(k).copied().unwrap_or(Vec2::X);
        let n = Vec2::new(u.y, -u.x);
        (d.dot(u), d.dot(n))
    }
}

impl View {
    /// 矢印キー dv を、光軸に沿う量か垂直な量のどちらか一方に割り当てる。光軸の画面の向きが横に近ければ
    /// ← → が光軸に沿う（45° ちょうどでも ← →）、縦に近ければ ↑ ↓。符号は画面の向きに合わせる
    fn along_key(&self, k: usize, dv: Vec2) -> (f32, f32) {
        let u = self.axes.get(k).copied().unwrap_or(Vec2::X);
        let horiz = u.x.abs() + 1e-3 >= u.y.abs();
        let (a, n) = self.along(k, dv);
        let on_axis = if horiz { dv.x != 0.0 } else { dv.y != 0.0 };
        if on_axis {
            (a.signum(), 0.0)
        } else {
            (0.0, if n == 0.0 { 1.0 } else { n.signum() })
        }
    }

    /// 要素 k を選ぶ
    pub fn select(&mut self, k: usize) {
        self.sel = Some(k);
        self.dim = None;
        self.edit = None;
    }

    pub fn start_edit(&mut self, combo: &mut Combo, g: usize) {
        let v = gap_get(combo, g).unwrap_or(0.0);
        self.dim = Some(g);
        self.sel = None;
        self.edit = Some((g, fmt_num((v * 1e4).round() / 1e4), false));
    }
}

/// 画面から GUI に頼むこと
pub enum Act {
    /// カタログのパネルを出す
    Catalog,
    /// 像面を近軸の焦点に合わせる
    Focus,
    /// 今の組合せで系を作る
    Build,
    /// 要素 k の面を面の表で見る（図の要素のダブルクリック）
    Open(usize),
    /// 要素 k（から始まる範囲）をユーザーカタログに保存する
    SaveToCatalog(usize),
}

const STOP_GAP: usize = usize::MAX;

/// 要素を足すボタンの並び・図・選んだ要素の設定
#[allow(clippy::too_many_arguments)]
pub fn ui(
    ui: &mut Ui,
    combo: &mut Combo,
    data: Option<&LayoutData>,
    view: &mut View,
    auto: &mut bool,
    building: bool,
    accent: Color32,
    paths: &[String],
) -> Option<Act> {
    let mut act = None;
    let weak = theme::weak_text(ui.visuals());
    combo.normalize();
    // 要素のパレット: 足すものは 1 つのメニューに（図の高さを取らない、GUX-30）
    const ZMX: [ElemKind; 2] = [ElemKind::Zmx, ElemKind::ZmxRange];
    let mut zmx_start = [false; 2];
    ui.horizontal_wrapped(|ui| {
        ui.menu_button(trf("+ {}", &[&tr("Add")]), |ui| {
            if ui
                .button(trf("{}…", &[&tr("Catalog element")]))
                .on_hover_text(tr(
                    "Double-click a row in the catalog panel, or drag it onto the layout",
                ))
                .clicked()
            {
                act = Some(Act::Catalog);
                ui.close_menu();
            }
            for k in [
                ElemKind::Lens,
                ElemKind::Plate,
                ElemKind::Mirror,
                ElemKind::Stop,
                ElemKind::Aperture,
                ElemKind::Splitter,
                ElemKind::Surfaces,
            ] {
                if ui.button(k.label()).clicked() {
                    insert(combo, view, Pick::new(k));
                    ui.close_menu();
                }
            }
            ui.separator();
            let hints = [
                tr("The k-th lens of a .zmx"),
                tr("Insert a range of .zmx surfaces as is (default: surface 1 to the one before the image), e.g. a lens under test inside a measurement setup"),
            ];
            for (i, kind) in ZMX.into_iter().enumerate() {
                if ui.button(trf("{}…", &[&kind.label()])).on_hover_text(hints[i]).clicked() {
                    zmx_start[i] = true;
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        if ui
            .add_enabled(
                !combo.is_empty(),
                egui::Button::new(tr("Image at paraxial focus")),
            )
            .on_hover_text(tr("Set the last air gap to where a thin on-axis ray crosses the axis"))
            .clicked()
        {
            act = Some(Act::Focus);
        }
        // 経路を変えたら、選んでいた要素を新しい経路の中で探し直す（無ければ外す。GPN-28）
        let sel_id = view.sel.and_then(|k| combo.ids().get(k).cloned());
        if path_select(ui, combo, paths) {
            view.sel = sel_id.and_then(|id| combo.ids().iter().position(|x| *x == id));
            view.dim = None;
        }
        ui.checkbox(auto, tr("Rebuild on every change"));
        if (!*auto || (data.is_none() && !building)) && ui.button(tr("▶ Build system")).clicked()
        {
            act = Some(Act::Build);
        }
    });
    // .zmx を選ぶ窓（閉じた後も毎フレーム見る）
    for (i, kind) in ZMX.into_iter().enumerate() {
        let key = format!("combo-zmx-{:?}", kind);
        if let Some(p) = crate::files::open(&key, zmx_start[i], || {
            // lensbench の系（系ファイル・プロジェクトの .json）も取り込める（#132）
            crate::files::FileDialog::new()
                .add_filter(tr("System (.zmx / .json)"), &["zmx", "json"])
        }) {
            let mut e = Pick::new(kind);
            e.path = p.display().to_string();
            insert(combo, view, e);
        }
    }
    if !combo.picks.is_empty() {
        ui.horizontal_wrapped(|ui| states_ui(ui, combo, view, weak));
        ui.horizontal_wrapped(|ui| groups_ui(ui, combo, weak, None));
    }
    ui.separator();
    if combo.picks.is_empty() {
        let r = ui
            .weak(tr(
                "Add elements to see the layout here (light travels left to right).",
            ))
            .union(ui.allocate_response(ui.available_size(), Sense::hover()));
        if let Some(p) = r.dnd_release_payload::<DropPick>() {
            combo.picks.push(p.0.clone());
            view.sel = Some(0);
        } else if r.dnd_hover_payload::<DropPick>().is_some() {
            ui.painter().rect_stroke(
                r.rect,
                4.0,
                Stroke::new(1.5_f32, accent),
                egui::StrokeKind::Inside,
            );
        }
        return act;
    }
    // 図の下に、選んだ要素・寸法の設定
    let sel = view.sel.and_then(|k| combo.el(k));
    let bs_sel = sel.is_some_and(|p| p.kind == ElemKind::Splitter);
    // 面の表（面の並び。自作の玉の面はレンズデータの表で編集、GPN-19）を出すときはその分も
    let rows = sel
        .filter(|p| p.kind == ElemKind::Surfaces)
        .map_or(0, |p| p.surfs.len() + 1);
    let editor_h = if bs_sel { 200.0 } else { 96.0 } + 24.0 * rows as f32;
    // 図と設定に残りを分ける（パネルが低くても設定が押し出されないように。横に広く低いと左右に並べる）
    let avail = ui.available_height().max(1.0);
    let (mut top, mut bottom) = crate::split::two(
        ui,
        "combo_fig",
        // 図は少なくとも 200 px（GUX-30）
        (
            ((avail - editor_h).max(200.0) / avail).clamp(0.5, 0.85),
            0.6,
        ),
        80.0,
    );
    let ui = &mut top;
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let id = resp.id;
    if resp.clicked() || resp.drag_started() {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    if let Some(d) = data {
        view.nav = true;
        figure(ui, rect, &resp, Some(combo), d, view, accent);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            if building {
                tr("Building the system…")
            } else {
                tr("Press \"▶ Build system\" to see the layout")
            },
            egui::FontId::proportional(14.0),
            weak,
        );
    }
    if focused {
        // 矢印キーは図の中で使う（パネルの間の移動に取られない）
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
        keys(ui, combo, view);
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0_f32, accent.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    }
    // カタログの行を落とす: 一番近い要素の前か後ろへ差し込む
    // 別のウィジェット（カタログの行）をドラッグ中は hover_pos が無いので、ポインタの位置を直に見る
    if let Some(q) = ui.ctx().pointer_latest_pos().filter(|q| rect.contains(*q)) {
        if resp.dnd_hover_payload::<DropPick>().is_some() {
            let at = drop_index(view, q, combo.len());
            drop_marker(ui, rect, view, at, accent);
        }
        if let Some(p) = resp.dnd_release_payload::<DropPick>() {
            let at = drop_index(view, q, combo.len());
            insert_at(combo, view, at, p.0.clone());
        }
    }
    let opened = egui::ScrollArea::both()
        .id_salt("combo_editor")
        .auto_shrink([false, false])
        .show(&mut bottom, |ui| editor(ui, combo, view))
        .inner;
    if let Some(k) = opened.or(view.open.take()) {
        act = Some(Act::Open(k));
    }
    if let Some(k) = view.save.take() {
        act = Some(Act::SaveToCatalog(k));
    }
    act
}

/// BS の経路（T・R の並び）を選ぶ欄（BS が無ければ出さない）。変えたら true
fn path_select(ui: &mut Ui, combo: &mut Combo, paths: &[String]) -> bool {
    let any_bs = combo
        .all()
        .iter()
        .any(|l| combo.get(l).is_some_and(|p| p.kind == ElemKind::Splitter));
    if !any_bs {
        return false;
    }
    let st = combo.active_state().map(str::to_string);
    let base = combo.path.clone();
    let mut path = match &st {
        Some(n) => combo.state_path.get(n).cloned().unwrap_or(base.clone()),
        None => base.clone(),
    };
    let was = path.clone();
    ui.separator();
    ui.label(RichText::new(tr("Path")).color(theme::weak_text(ui.visuals())))
        .on_hover_text(tr("Transmit T / reflect R at each BS in the order passed (missing letters use each BS's default port). The figure and analyses use this path; other paths can be drawn with \"Overlay configurations\""));
    let shown = if path.is_empty() {
        tr("Default").to_string()
    } else {
        path.clone()
    };
    egui::ComboBox::from_id_salt("combo_path")
        .selected_text(shown)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut path, String::new(), tr("Default"));
            for p in paths {
                ui.selectable_value(&mut path, p.clone(), p.as_str());
            }
        });
    ui.add(
        egui::TextEdit::singleline(&mut path)
            .desired_width(48.0)
            .hint_text("TR…"),
    );
    path = path
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| matches!(c, 'T' | 'R'))
        .collect();
    if let Some(n) = &st {
        let own = combo.state_path.contains_key(n);
        if own {
            if ui
                .small_button(tr("Use the base path"))
                .on_hover_text(trf("Drop the path changed in state {}", &[n]))
                .clicked()
            {
                combo.state_path.remove(n);
                return true;
            }
        } else {
            ui.label(
                RichText::new(tr("(same as base)"))
                    .small()
                    .color(theme::weak_text(ui.visuals())),
            )
            .on_hover_text(trf("Changing it makes a path for state {} only", &[n]));
        }
    }
    if path == was {
        return false;
    }
    match st {
        Some(n) => {
            combo.state_path.insert(n, path);
        }
        None => combo.path = path,
    }
    true
}

/// 組合せでない系のレイアウト図（見るだけ。玉・鏡・像面・光線は組合せの図と同じ描き方）
pub fn view_only(
    ui: &mut Ui,
    d: &LayoutData,
    h: f32,
    accent: Color32,
    mut nav: Option<&mut crate::widgets::Nav>,
    hl_surf: Option<usize>,
) -> Option<OpenAct> {
    // 3D は回す・拡大の操作を受ける。2D も nav があれば拡大縮小・移動を受ける（#124）
    let sense = if d.d3.is_some() || nav.is_some() {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), sense);
    if resp.clicked() || resp.drag_started() {
        resp.request_focus();
    }
    let mut view = View {
        hl_surf,
        ..Default::default()
    };
    if let Some(n) = nav.as_deref() {
        view.nav = true;
        view.zoom_log = n.zoom_log;
        view.pan = n.pan;
    }
    figure(ui, rect, &resp, None, d, &mut view, accent);
    if let Some(n) = nav.as_deref_mut() {
        n.zoom_log = view.zoom_log;
        n.pan = view.pan;
    }
    // カタログの行を落とす（nav のある開いた系の図だけ。2D の図）: 差し込む間隔の印
    if nav.is_some() && d.d3.is_none() && !d.unfold {
        let q = ui.ctx().pointer_hover_pos().filter(|q| rect.contains(*q));
        let after = q.and_then(|q| open_drop_after(&view.surf_pos, q));
        if resp.dnd_hover_payload::<DropPick>().is_some() {
            if let Some(a) = after {
                let pa = view.surf_pos.get(a).copied().flatten();
                let pb = view.surf_pos.get(a + 1).copied().flatten();
                if let (Some(pa), Some(pb)) = (pa, pb) {
                    let x = (pa.x + pb.x) / 2.0;
                    let p = ui.painter_at(rect);
                    p.line_segment(
                        [
                            Pos2::new(x, rect.top() + 8.0),
                            Pos2::new(x, rect.bottom() - 8.0),
                        ],
                        Stroke::new(2.5_f32, accent),
                    );
                    p.text(
                        Pos2::new(x + 4.0, rect.top() + 10.0),
                        egui::Align2::LEFT_TOP,
                        trf("Insert after surface {}", &[&a]),
                        egui::FontId::proportional(12.0),
                        accent,
                    );
                }
            }
        }
        if let Some(p) = resp.dnd_release_payload::<DropPick>() {
            if let Some(a) = after {
                return Some(OpenAct::Drop {
                    after: a,
                    pick: Box::new(p.0.clone()),
                });
            }
        }
    }
    // クリックした玉の面の範囲（ドラッグで動かしたときは除く）
    resp.clicked()
        .then_some(view.hover_surfs)
        .flatten()
        .map(OpenAct::Clicked)
}

/// 開いた系（組合せでない系）の図の操作
pub enum OpenAct {
    /// クリックした玉の面の範囲
    Clicked([usize; 2]),
    /// カタログの行を落とした: 面 `after` の後の間隔に差し込む
    Drop { after: usize, pick: Box<Pick> },
}

/// 開いた系の図へ落とす位置 q から、差し込む間隔（その前の面の番号）: 一番近い面の頂点より右なら、その面の後、
/// 左なら前の面の後。物体面の後（面 0）と像面の後には差し込まない
fn open_drop_after(surf_pos: &[Option<Pos2>], q: Pos2) -> Option<usize> {
    let n = surf_pos.len();
    if n < 3 {
        return None;
    }
    let (j, v) = surf_pos
        .iter()
        .enumerate()
        .take(n - 1)
        .skip(1)
        .filter_map(|(j, v)| v.map(|v| (j, v)))
        .min_by(|a, b| (a.1 - q).length().total_cmp(&(b.1 - q).length()))?;
    let after = if q.x >= v.x { j } else { j - 1 };
    (1..n - 1).contains(&after).then_some(after)
}

/// 落とす位置 q から差し込む番号: 一番近い要素の真ん中より光軸の先なら後ろ、手前なら前
fn drop_index(view: &View, q: Pos2, n: usize) -> usize {
    let near = view
        .mids
        .iter()
        .enumerate()
        .filter_map(|(k, m)| m.map(|m| (k, m)))
        .min_by(|a, b| (a.1 - q).length().total_cmp(&(b.1 - q).length()));
    match near {
        Some((k, m)) => {
            let (u, _) = view.along(k, q - m);
            if u >= 0.0 {
                k + 1
            } else {
                k
            }
        }
        None => n,
    }
    .min(n)
}

/// 差し込む位置の印（前後の要素の真ん中の間の縦線）
fn drop_marker(ui: &Ui, rect: Rect, view: &View, at: usize, accent: Color32) {
    let m = |k: usize| view.mids.get(k).copied().flatten();
    let pos = match (at.checked_sub(1).and_then(m), m(at)) {
        (Some(a), Some(b)) => a + (b - a) / 2.0,
        (Some(a), None) => a + view.axes.get(at - 1).copied().unwrap_or(Vec2::X) * 24.0,
        (None, Some(b)) => b - view.axes.get(at).copied().unwrap_or(Vec2::X) * 24.0,
        (None, None) => rect.center(),
    };
    let k = at.min(view.axes.len().saturating_sub(1));
    let u = view.axes.get(k).copied().unwrap_or(Vec2::X);
    let n = Vec2::new(u.y, -u.x) * 40.0;
    let p = ui.painter_at(rect);
    p.line_segment([pos - n, pos + n], Stroke::new(2.5_f32, accent));
    p.circle_filled(pos, 4.0, accent);
}

/// 経路に沿った at 番目に差し込む。間に入れるときは前の要素の後ろの空気を半分ずつ分ける。差し込んだ番号を返す
pub fn insert_pick(combo: &mut Combo, at: usize, p: Pick) -> usize {
    combo.insert_chain(at, p)
}

fn insert_at(combo: &mut Combo, view: &mut View, at: usize, p: Pick) {
    let at = insert_pick(combo, at, p);
    view.select(at);
}

fn insert(combo: &mut Combo, view: &mut View, p: Pick) {
    let n = combo.len();
    let at = view.sel.map(|k| k + 1).unwrap_or(n).min(n);
    // 間に入れても空気は分けない（足した要素の後の空気は初めの値）
    let k = combo.insert_chain_with(at, p, false);
    view.sel = Some(k);
    view.dim = None;
}

/// キーボード: [ ] で要素を選ぶ、← → で光軸に沿って動かす（寸法を選んでいればその間隔）、↑ ↓ で偏心、
/// Shift+↑ ↓ で傾き、R で反転、Delete で外す、Ctrl+L でロック（表と同じ、GPN-15）、Esc でやめる。Ctrl で 10 倍、Alt で 1/10
fn keys(ui: &mut Ui, combo: &mut Combo, view: &mut View) {
    let n = combo.len();
    // 経路に沿った要素（ロックしていない）。ロックは中身だけを止め、動かすのはできる
    let free = |combo: &Combo, k: usize| combo.el(k).is_some_and(|p| !p.locked);
    let ev: Vec<(Key, Modifiers)> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect()
    });
    for (key, m) in ev {
        let step = if m.command {
            1.0
        } else if m.alt {
            0.01
        } else {
            0.1
        };
        match key {
            Key::OpenBracket if n > 0 => {
                view.sel = Some(view.sel.map(|k| (k + n - 1) % n).unwrap_or(n - 1));
                view.dim = None;
            }
            Key::CloseBracket if n > 0 => {
                view.sel = Some(view.sel.map(|k| (k + 1) % n).unwrap_or(0));
                view.dim = None;
            }
            Key::Escape => {
                view.sel = None;
                view.dim = None;
            }
            Key::Enter | Key::F2 => {
                if let Some(g) = view.dim {
                    view.start_edit(combo, g);
                }
            }
            // 3D 表示では矢印キーは図を回す（view3d）
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown if !view.d3 => {
                // 画面の向き
                let dv = match key {
                    Key::ArrowLeft => Vec2::new(-1.0, 0.0),
                    Key::ArrowRight => Vec2::new(1.0, 0.0),
                    Key::ArrowUp => Vec2::new(0.0, -1.0),
                    _ => Vec2::new(0.0, 1.0),
                };
                if let Some(g) = view.dim {
                    // 寸法: 右・上で広げる
                    let d = if dv.x > 0.0 || dv.y < 0.0 {
                        step
                    } else {
                        -step
                    };
                    if let Some(v) = gap_mut(combo, g) {
                        *v = (*v + d).max(0.0);
                    }
                } else if let Some(k) = view
                    .sel
                    .filter(|&k| combo.el(k).is_some_and(|p| !p.fixed()))
                {
                    // 要素: 光軸に近い向きの矢印（光軸が斜めでも ← → か ↑ ↓ のどちらか一方）で光軸に沿って
                    // 動かし、もう一方で偏心（Shift で傾き）
                    let (a, n) = view.along_key(k, dv);
                    if a != 0.0 {
                        if !m.shift {
                            combo.shift(k, step * a.signum() as f64);
                        }
                    } else {
                        let d = step * n.signum() as f64;
                        let st = combo.active_state().map(str::to_string);
                        let Some(p) = combo.el_mut(k) else { continue };
                        if m.shift {
                            view.nudge(p, st.as_deref(), 0.0, d * 10.0);
                        } else if p.kind != ElemKind::Mirror && p.kind != ElemKind::Stop {
                            view.nudge(p, st.as_deref(), d, 0.0);
                        }
                    }
                }
            }
            Key::L if m.command => {
                if let Some(p) = view.sel.and_then(|k| combo.el_mut(k)) {
                    p.locked = !p.locked;
                }
            }
            Key::R if m.is_none() => {
                if let Some(p) = view
                    .sel
                    .filter(|&k| free(combo, k))
                    .and_then(|k| combo.el_mut(k))
                {
                    p.reversed = !p.reversed;
                }
            }
            Key::F if m.is_none() => {
                if let Some(p) = view
                    .sel
                    .filter(|&k| free(combo, k))
                    .and_then(|k| combo.el_mut(k))
                    .filter(|p| p.kind == ElemKind::Splitter)
                {
                    p.bs_mut().bs_flip = !p.bs_mut().bs_flip;
                }
            }
            Key::Delete => {
                // 外すのは中身だけのロック（カタログの玉の既定）でもできる。「位置も」ロックした要素は外さない
                if let Some(k) = view
                    .sel
                    .filter(|&k| combo.el(k).is_some_and(|p| !p.fixed()))
                {
                    combo.remove_chain(k);
                    view.sel = None;
                    view.dim = None;
                }
            }
            _ => {}
        }
    }
}

/// 寸法 g の今の値（今の状態の値）
fn gap_get(combo: &Combo, g: usize) -> Option<f64> {
    if g == STOP_GAP {
        Some(combo.stop_gap)
    } else {
        combo.gap_after_val(g)
    }
}

/// 寸法 g（経路に沿った要素 g の後の空気、[`STOP_GAP`] は絞りから最初の要素）
fn gap_mut(combo: &mut Combo, g: usize) -> Option<&mut f64> {
    if g == STOP_GAP {
        Some(&mut combo.stop_gap)
    } else if combo.el(g).is_some_and(|p| p.fixed()) {
        // 位置もロックした要素の後の間隔は変えない
        None
    } else {
        combo.gap_after(g)
    }
}

/// 面の表で選んだ要素 k（ブロック）の設定。図の下の欄と同じで、置き方（偏心 x・y、傾き x・y・z）まで全部出す
pub fn block_settings(ui: &mut Ui, combo: &mut Combo, k: usize) {
    element::element_settings(ui, combo, k, element::Detail::Full, |_, _| {});
}

const ROLES: [(&str, &str); 4] = [
    ("", n_("No role")),
    ("zoom", n_("Zoom")),
    ("focus", n_("Focus")),
    ("fixed", n_("Fixed")),
];

fn role_combo(ui: &mut Ui, id: impl std::hash::Hash, role: &mut String) {
    let cur = ROLES
        .iter()
        .find(|r| r.0 == role.as_str())
        .map_or(role.clone(), |r| tr(r.1).to_string());
    egui::ComboBox::from_id_salt(id)
        .selected_text(cur)
        .show_ui(ui, |ui| {
            for (k, name) in ROLES {
                ui.selectable_value(role, k.to_string(), tr(name));
            }
        });
}

/// .zmx の取り込みの玉モードの群（玉の境を手で直す）
fn zgroups_ui(ui: &mut Ui, p: &mut Pick, weak: Color32) {
    ui.label(RichText::new(tr("Lens groups")).color(weak))
        .on_hover_text(tr(
        "Group .zmx surfaces from–to into one group (lens/group parameters). Without groups, one per lens automatically",
    ));
    let mut remove = None;
    for (i, g) in p.zgroups.iter_mut().enumerate() {
        ui.add(
            egui::DragValue::new(&mut g.from)
                .range(1..=9999)
                .prefix(tr("Surface ")),
        );
        ui.label("〜");
        ui.add(egui::DragValue::new(&mut g.to).range(1..=9999));
        ui.add(
            egui::TextEdit::singleline(&mut g.name)
                .hint_text(tr("Name"))
                .desired_width(50.0),
        );
        role_combo(ui, ("zg_role", p.path.as_str(), i), &mut g.role);
        if ui.small_button("✖").on_hover_text(tr("Remove")).clicked() {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        p.zgroups.remove(i);
    }
    if ui.small_button(tr("+ Group")).clicked() {
        let from = p.zgroups.last().map_or(p.range[0].max(1), |g| g.to + 1);
        p.zgroups.push(ZGroup {
            from,
            to: from,
            ..Default::default()
        });
    }
}

/// 群の役割と移動。群の名前は要素の設定の「群」で付ける。状態を選んでいれば移動はその状態の値。
/// only は 1 つの群だけ（面の表の要素の設定、#136）
fn groups_ui(ui: &mut Ui, combo: &mut Combo, weak: Color32, only: Option<&str>) {
    let mut names: Vec<String> = Vec::new();
    let ids: Vec<String> = combo.ids_all().into_iter().map(|x| x.1).collect();
    for p in combo.all().iter().filter_map(|l| combo.get(l)) {
        let g = p.group.trim();
        if !g.is_empty() && !names.iter().any(|n| n == g) {
            names.push(g.to_string());
        }
    }
    // 使わなくなった既定のままの群は残さない（名前を打つ途中の群など）
    combo.groups.retain(|d| {
        names.contains(&d.name) || !d.role.is_empty() || d.shift != 0.0 || !d.state_shift.is_empty()
    });
    if let Some(o) = only {
        names.retain(|n| n == o);
    }
    if names.is_empty() {
        return;
    }
    let st = combo.active_state().map(str::to_string);
    ui.label(RichText::new(tr("Group")).small().color(weak));
    for n in names {
        if ids.contains(&n) {
            ui.label(
                RichText::new(trf(
                    "{} is the same as an element name (not grouped)",
                    &[&n],
                ))
                .color(ui.visuals().warn_fg_color),
            );
            continue;
        }
        if !combo.groups.iter().any(|d| d.name == n) {
            combo.groups.push(GroupDef {
                name: n.clone(),
                ..Default::default()
            });
        }
        let d = combo.groups.iter_mut().find(|d| d.name == n).unwrap();
        ui.label(RichText::new(&n).strong());
        role_combo(ui, ("g_role", n.as_str()), &mut d.role);
        let cur = st
            .as_deref()
            .and_then(|s| d.state_shift.get(s).copied())
            .unwrap_or(d.shift);
        let mut x = cur;
        ui.add(
            egui::DragValue::new(&mut x)
                .speed(0.05)
                .max_decimals(4)
                .prefix(tr("Shift "))
                .suffix(" mm"),
        )
        .on_hover_text(tr(
            "Positive along the light. Added to the gap before and subtracted from the gap after",
        ));
        if x != cur {
            match st.as_deref() {
                Some(s) => {
                    d.state_shift.insert(s.to_string(), x);
                }
                None => d.shift = x,
            }
        }
        ui.add_space(8.0);
    }
}

/// 状態（多重構成に当たるプロジェクトの states）を選ぶ・足す・消す。状態を選ぶと、要素の ON/OFF と
/// 後の空気（図の寸法・要素の設定）はその状態の値を変える
fn states_ui(ui: &mut Ui, combo: &mut Combo, view: &mut View, weak: Color32) {
    ui.label(RichText::new(tr("State")).small().color(weak));
    let cur = combo.active_state().map(str::to_string);
    egui::ComboBox::from_id_salt("combo_state")
        .selected_text(cur.clone().unwrap_or_else(|| tr("Base").into()))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut combo.state, None, tr("Base"));
            for n in combo.states.clone() {
                ui.selectable_value(&mut combo.state, Some(n.clone()), n);
            }
        })
        .response
        .on_hover_text(tr(
            "Each state can change element ON/OFF, following air, decenter and tilt, the BS path and system settings (aperture, fields, wavelengths, object distance and so on), e.g. inserting a measurement system or zooming. Base is the system with no state selected",
        ));
    let r = ui.add(
        egui::TextEdit::singleline(&mut view.new_state)
            .hint_text(tr("State name"))
            .desired_width(90.0),
    );
    let name = view.new_state.trim().to_string();
    let ok = !name.is_empty() && !combo.states.contains(&name);
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    if (ui
        .add_enabled(ok, egui::Button::new(tr("+ State")))
        .clicked()
        || (enter && ok))
        && ok
    {
        combo.states.push(name.clone());
        combo.state = Some(name);
        view.new_state.clear();
    }
    if let Some(n) = cur {
        if ui
            .button("✖")
            .on_hover_text(trf("Delete state {}", &[&n]))
            .clicked()
        {
            combo.states.retain(|s| *s != n);
            for l in combo.all() {
                if let Some(p) = combo.get_mut(&l) {
                    p.clear_state(&n);
                }
            }
            combo.state_sys.remove(&n);
            combo.state_path.remove(&n);
            for g in &mut combo.groups {
                g.state_shift.remove(&n);
            }
            combo.state = None;
        }
    }
    // OFF の要素は図に出ないので、ここから選ぶ（ON に戻すなど）
    let st = combo.active_state().map(str::to_string);
    let off: Vec<usize> = (0..combo.picks.len())
        .filter(|&k| !combo.picks[k].on_in(st.as_deref()))
        .collect();
    if !off.is_empty() {
        ui.separator();
        ui.label(RichText::new(tr("OFF")).small().color(weak));
        for k in off {
            if ui
                .selectable_label(view.sel == Some(k), combo.picks[k].label())
                .on_hover_text(tr(
                    "Select an OFF element (turn it back ON in the settings below)",
                ))
                .clicked()
            {
                view.sel = Some(k);
                view.dim = None;
            }
        }
    }
}

/// 要素の ON/OFF・OFF の残し方・鏡の往路を戻る。状態 st を選んでいれば ON/OFF と後の空気はその状態の値。k は欄の id（GPN-18）
pub fn onoff_ui(ui: &mut Ui, p: &mut Pick, st: Option<&str>, k: usize, weak: Color32) {
    if p.kind.has_on() {
        let mut on = p.on_in(st);
        let hint = match st {
            Some(n) => trf(
                "ON/OFF in state {} (an OFF element is left out of the system)",
                &[&n],
            ),
            None => {
                tr("An OFF element is left out of the system (insert/remove optics)").to_string()
            }
        };
        if ui.checkbox(&mut on, "ON").on_hover_text(hint).changed() {
            match st {
                Some(n) => {
                    p.state_on.insert(n.to_string(), on);
                }
                None => p.on = on,
            }
        }
        if p.kind.has_off_mode() {
            egui::ComboBox::from_id_salt(("off_mode", k))
                .selected_text(if p.off_air {
                    tr("OFF leaves air")
                } else {
                    tr("OFF collapses")
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.off_air, false, tr("OFF collapses"));
                    ui.selectable_value(&mut p.off_air, true, tr("OFF leaves air"));
                });
        }
    }
    // 群
    ui.label(RichText::new(tr("Group")).color(weak));
    ui.add(
        egui::TextEdit::singleline(&mut p.group)
            .hint_text(tr("Off"))
            .desired_width(60.0),
    )
    .on_hover_text(tr(
        "Consecutive elements with the same name form one group (role and shift below, or under Groups in the Layout panel)",
    ));
    if p.kind == ElemKind::ZmxRange {
        zgroups_ui(ui, p, weak);
    }
    if let Some(n) = st {
        let own = p.has_state_values(n);
        if own {
            ui.label(
                RichText::new(trf("Values of state {}", &[&n]))
                    .small()
                    .color(weak),
            );
            if ui
                .small_button(tr("Use base values"))
                .on_hover_text(tr(
                    "Drop the ON/OFF, following air, decenter and tilt changed in this state",
                ))
                .clicked()
            {
                p.clear_state(n);
            }
        }
    }
}

/// 偏心（番号 dec）・傾き（番号 tilt）の欄。状態 st を選んでいれば、変えたときだけその状態の値にする
fn place_ui(ui: &mut Ui, p: &mut Pick, st: Option<&str>, dec: &[usize], tilt: &[usize]) {
    let was = p.place_in(st);
    let mut pl = was;
    const DEC: [&str; 2] = [n_("Decenter x"), n_("Decenter y")];
    const TILT: [&str; 3] = [n_("Tilt x"), n_("Tilt y"), n_("Tilt z")];
    for &i in dec {
        num(ui, tr(DEC[i]), &mut pl.decenter[i], " mm");
    }
    for &i in tilt {
        num(ui, tr(TILT[i]), &mut pl.tilt[i], "°");
    }
    if pl != was {
        let (d, t) = p.place_mut(st);
        *d = pl.decenter;
        *t = pl.tilt;
    }
}

/// 置き方（偏心・傾き）を持つ要素（プロジェクトのブロックの place）
fn place_fields(kind: ElemKind) -> bool {
    matches!(
        kind,
        ElemKind::Catalog
            | ElemKind::Zmx
            | ElemKind::Lens
            | ElemKind::Plate
            | ElemKind::Splitter
            | ElemKind::Surfaces
            | ElemKind::ZmxRange
    )
}

/// 要素の数の欄（名前と値）
fn num(ui: &mut Ui, label: &str, v: &mut f64, unit: &str) {
    num_in(ui, label, v, unit, f64::NEG_INFINITY);
}

/// 下限のある数の欄（厚さ ≥ 0・半径 > 0 など。負や 0 を入れて系を作る段で誤りにしない、GPN-10）
fn num_in(ui: &mut Ui, label: &str, v: &mut f64, unit: &str, min: f64) {
    let weak = theme::weak_text(ui.visuals());
    ui.label(RichText::new(label).color(weak));
    ui.add(
        egui::DragValue::new(v)
            .speed(0.05)
            .max_decimals(4)
            .range(min..=f64::INFINITY)
            .suffix(unit),
    );
}

/// 要素の種類ごとの欄（寸法・ガラス・膜など）
fn kind_fields(
    ui: &mut Ui,
    p: &mut Pick,
    st: Option<&str>,
    k: usize,
    weak: Color32,
    coats: &[String],
) {
    match p.kind {
        ElemKind::Lens if p.surfs.len() >= 2 => {
            ui.label(
                RichText::new(trf(
                    "{} surfaces (edit them in the Lens Data table)",
                    &[&p.surfs.len()],
                ))
                .color(weak),
            );
        }
        ElemKind::Lens => {
            num(ui, "R1", &mut p.r1, " mm");
            num(ui, "R2", &mut p.r2, " mm");
            num_in(ui, tr("Thickness"), &mut p.t, " mm", 0.0);
            ui.label(RichText::new(tr("Glass")).color(weak));
            ui.add(egui::TextEdit::singleline(&mut p.glass).desired_width(70.0));
            semi_edit(ui, &mut p.semi, &mut p.semi_auto);
            if ui
                .button(tr("+ Surface"))
                .on_hover_text(tr(
                    "Add surfaces for a cemented doublet, triplet, etc. (edit them in the Lens Data table)",
                ))
                .clicked()
            {
                let mut rows = p.lens_rows();
                rows[1].t = 2.0;
                rows[1].glass = "N-SF5".into();
                rows.push(SurfRow {
                    radius: -2.0 * p.r2,
                    semi: p.semi,
                    ..Default::default()
                });
                p.surfs = rows;
            }
        }
        ElemKind::Surfaces => {
            ui.label(
                RichText::new(trf("{} surfaces (table below)", &[&p.surfs.len()])).color(weak),
            );
        }
        ElemKind::ZmxRange => {
            ui.label(RichText::new(tr("Surface")).color(weak));
            ui.add(egui::DragValue::new(&mut p.range[0]).range(0..=9999))
                .on_hover_text(tr("First surface to import (0 is surface 1)"));
            ui.label("〜");
            ui.add(egui::DragValue::new(&mut p.range[1]).range(0..=9999))
                .on_hover_text(tr("Last surface to import (0 is the one before the image)"));
            ui.label(RichText::new(tr("Config")).color(weak));
            ui.add(egui::DragValue::new(&mut p.config).range(1..=999))
                .on_hover_text(tr("Multi-configuration number of the .zmx"));
        }
        ElemKind::Plate => {
            num_in(ui, tr("Thickness"), &mut p.t, " mm", 0.0);
            ui.label(RichText::new(tr("Glass")).color(weak));
            ui.add(egui::TextEdit::singleline(&mut p.glass).desired_width(70.0));
            semi_edit(ui, &mut p.semi, &mut p.semi_auto);
        }
        ElemKind::Mirror => {
            num(ui, "R", &mut p.r1, " mm");
            place_ui(ui, p, st, &[], &[0, 1]);
            num_in(ui, tr("Semi-diameter"), &mut p.semi, " mm", 0.001);
            crate::coat_defs::coat_edit(ui, &mut p.coat, k, weak, coats);
            ui.checkbox(&mut p.back, tr("Return along the path")).on_hover_text(tr(
                "Light reflected by the mirror returns along its path (double pass, end of an interferometer arm). Inside a BS branch, the returning light splits again at the BS",
            ));
        }
        ElemKind::Stop => num_in(ui, tr("Semi-diameter"), &mut p.semi, " mm", 0.001),
        ElemKind::Aperture => {
            egui::ComboBox::from_id_salt(("ap_shape", k))
                .selected_text(p.ap_mut().shape.label())
                .show_ui(ui, |ui| {
                    for a in [
                        ApShape::Circle,
                        ApShape::Rect,
                        ApShape::Ellipse,
                        ApShape::Obscuration,
                        ApShape::Spider,
                    ] {
                        ui.selectable_value(&mut p.ap_mut().shape, a, a.label());
                    }
                });
            match p.ap_mut().shape {
                ApShape::Circle | ApShape::Obscuration => {
                    num_in(ui, tr("Inner radius"), &mut p.ap_mut().min, " mm", 0.0);
                    num_in(ui, tr("Outer radius"), &mut p.semi, " mm", 0.001);
                }
                ApShape::Rect | ApShape::Ellipse => {
                    num_in(
                        ui,
                        tr("Half width x"),
                        &mut p.ap_mut().half[0],
                        " mm",
                        0.001,
                    );
                    num_in(
                        ui,
                        tr("Half width y"),
                        &mut p.ap_mut().half[1],
                        " mm",
                        0.001,
                    );
                }
                ApShape::Spider => {
                    num_in(
                        ui,
                        tr("Arm width (full)"),
                        &mut p.ap_mut().spider_width,
                        " mm",
                        0.0,
                    );
                    let mut n = p.ap_mut().spider_arms as f64;
                    num(ui, tr("Number of arms"), &mut n, "");
                    p.ap_mut().spider_arms = n.round().clamp(1.0, 360.0) as u32;
                    // 腕は無限に伸びる。図ではこの半径まで描く
                    num_in(ui, tr("Drawn radius"), &mut p.semi, " mm", 0.001);
                }
            }
            num(ui, tr("Decenter x"), &mut p.decenter[0], " mm");
            num(ui, tr("Decenter y"), &mut p.decenter[1], " mm");
        }
        ElemKind::Splitter => {
            let was = p.bs_mut().form;
            egui::ComboBox::from_id_salt(("bs_form", k))
                .selected_text(p.bs_mut().form.label())
                .show_ui(ui, |ui| {
                    for f in [BsForm::Cube, BsForm::Plate, BsForm::Ideal] {
                        ui.selectable_value(&mut p.bs_mut().form, f, f.label());
                    }
                });
            if p.bs_mut().form != was {
                // 形を変えたら寸法を形の初めの値に
                match p.bs_mut().form {
                    BsForm::Cube => p.t = 12.7,
                    BsForm::Plate => p.t = 1.0,
                    BsForm::Ideal => {}
                }
            }
            match p.bs_mut().form {
                BsForm::Cube => num_in(ui, tr("Size"), &mut p.t, " mm", 0.001),
                BsForm::Plate => num_in(ui, tr("Thickness"), &mut p.t, " mm", 0.0),
                BsForm::Ideal => {}
            }
            if p.bs_mut().form != BsForm::Ideal {
                ui.label(RichText::new(tr("Glass")).color(weak));
                ui.add(egui::TextEdit::singleline(&mut p.glass).desired_width(70.0));
            }
            if p.bs_mut().form != BsForm::Cube {
                num_in(ui, tr("Semi-diameter"), &mut p.semi, " mm", 0.001);
            }
            let mut mode = p.split_mode();
            egui::ComboBox::from_id_salt(("bs_split", k))
                .selected_text(mode.label())
                .show_ui(ui, |ui| {
                    for m in [BsSplit::Ratio, BsSplit::Coat, BsSplit::Pbs] {
                        ui.selectable_value(&mut mode, m, m.label());
                    }
                })
                .response
                .on_hover_text(tr("Splitting face: polarization-independent ratio, coating, or ideal polarizing BS"));
            p.bs_mut().split = Some(mode);
            match mode {
                BsSplit::Ratio => {
                    ui.add(
                        egui::DragValue::new(&mut p.bs_mut().ratio)
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .prefix("T "),
                    );
                }
                BsSplit::Coat => crate::coat_defs::coat_edit(ui, &mut p.coat, k, weak, coats),
                BsSplit::Pbs => {
                    ui.add(
                        egui::DragValue::new(&mut p.bs_mut().pbs[0])
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .prefix("Tp "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut p.bs_mut().pbs[1])
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .prefix("Ts "),
                    );
                }
            }
            ui.checkbox(&mut p.arm_r, tr("Follow reflected arm"))
                .on_hover_text(tr(
                    "Port to take when the path has no letter for this BS (default: transmitted)",
                ));
            ui.checkbox(&mut p.bs_mut().bs_flip, tr("Flip reflection"))
                .on_hover_text(tr(
                    "Swap the split face so the reflection goes to the other side (F key)",
                ));
        }
        ElemKind::Zmx => {
            ui.label(RichText::new(tr("Element no.")).color(weak));
            ui.add(egui::DragValue::new(&mut p.lens).range(1..=999));
            ui.label(RichText::new(tr("Config")).color(weak));
            ui.add(egui::DragValue::new(&mut p.config).range(1..=999))
                .on_hover_text(tr(
                    "Which multi-configuration of the .zmx to take the lens from",
                ));
        }
        ElemKind::Catalog => {}
        ElemKind::Raw => raw::raw_fields(ui, p, k),
    }
    if p.kind == ElemKind::ZmxRange {
        if p.range_has_stop() {
            ui.checkbox(&mut p.stop_front, tr("Use the .zmx stop")).on_hover_text(tr(
                "Make the stop of the imported surfaces the system stop. When off, the stop is the automatic stop or a stop element (one stop per system)",
            ));
        } else {
            ui.label(RichText::new(tr("The range has no .zmx stop surface")).color(weak));
        }
    } else if p.can_stop_front() {
        ui.checkbox(&mut p.stop_front, tr("Front is stop"))
            .on_hover_text(tr(
                "Make the front surface of the lens the stop (only one stop per system)",
            ));
    }
}

/// 選んだ寸法（入力欄とスライダー）・要素の設定。面の表で見る要素を返す
fn editor(ui: &mut Ui, combo: &mut Combo, view: &mut View) -> Option<usize> {
    let weak = theme::weak_text(ui.visuals());
    ui.add_space(4.0);
    let els = combo.chain_view().picks;
    let n = els.len();
    if let Some(g) = view.dim {
        if g != STOP_GAP && g >= n {
            view.dim = None;
            return None;
        }
        let name = if g == STOP_GAP {
            tr("Stop → first element").to_string()
        } else if g + 1 == n {
            trf("{} → image", &[&els[g].label()])
        } else {
            trf("{} → {}", &[&els[g].label(), &els[g + 1].label()])
        };
        let Some(v0) = gap_get(combo, g) else {
            view.dim = None;
            return None;
        };
        let mut x = v0;
        let v = &mut x;
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Gap")).color(weak));
            ui.label(RichText::new(name).strong());
            ui.add(
                egui::DragValue::new(v)
                    .speed(0.05)
                    .range(0.0..=1e5)
                    .suffix(" mm")
                    .max_decimals(4),
            );
            let top = (*v * 2.0).max(10.0);
            ui.add(egui::Slider::new(v, 0.0..=top).show_value(false));
        });
        if x != v0 {
            if let Some(v) = gap_mut(combo, g) {
                *v = x;
            }
        }
        ui.label(
            RichText::new(tr("← → also move it (Ctrl: 1 mm, Alt: 0.01 mm)"))
                .small()
                .color(weak),
        );
        return None;
    }
    let Some(k) = view.sel.filter(|&k| k < n) else {
        ui.label(
            RichText::new(tr("Click an element to select it, drag sideways to move it along the axis (Shift: decenter vertically). Click a dimension's value (or select it and press Enter) to type a new value; click the line to just select it. Keys: [ ] select, ← → move, ↑ ↓ decenter, Shift+↑ ↓ tilt, R reverse, F flip the BS reflection, Delete remove, Ctrl+L lock."))
                .small()
                .color(weak),
        );
        return None;
    };
    let where_ = combo
        .chain()
        .get(k)
        .map(|s| combo.where_label(&s.loc))
        .unwrap_or_default();
    let can_prev = combo.can_swap(k, -1);
    let can_next = combo.can_swap(k, 1);
    let mut remove = false;
    let mut swap: Option<i32> = None;
    let mut open = None;
    let mut save = false;
    element::element_settings(ui, combo, k, element::Detail::Compact, |ui, _| {
        if !where_.is_empty() {
            ui.label(RichText::new(trf("(beyond {})", &[&where_])).color(weak))
                .on_hover_text(tr("An element in the row beyond a BS port"));
        }
        if ui
            .button(tr("Lens Data"))
            .on_hover_text(tr(
                "View and edit this element's surfaces in the Lens Data table (or double-click the element in the figure)",
            ))
            .clicked()
        {
            open = Some(k);
        }
        if ui
            .button(tr("📚 Save to catalog"))
            .on_hover_text(tr(
                "Save this element (or a range) as an item of a user catalog",
            ))
            .clicked()
        {
            save = true;
        }
        if ui
            .add_enabled(can_prev, egui::Button::new("◀"))
            .on_hover_text(tr("Earlier"))
            .clicked()
        {
            swap = Some(-1);
        }
        if ui
            .add_enabled(can_next, egui::Button::new("▶"))
            .on_hover_text(tr("Later"))
            .clicked()
        {
            swap = Some(1);
        }
        if ui.button("✖").on_hover_text(tr("Remove")).clicked() {
            remove = true;
        }
    });
    if save {
        view.save = Some(k);
    }
    if remove {
        combo.remove_chain(k);
        view.sel = None;
    } else if let Some(d) = swap {
        view.sel = combo.swap_chain(k, d).or(Some(k));
    }
    open
}

/// BS の残りの項目（形ごとの欄・面ごとの膜）と、口 B・C・D の先の並び
pub fn bs_fields(ui: &mut Ui, combo: &mut Combo, loc: &[usize], weak: Color32) {
    let Some(p) = combo.get_mut(loc) else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        if p.bs_mut().form != BsForm::Cube {
            num(ui, tr("Splitting face tilt x"), &mut p.bs_mut().bs_angle[0], "°");
            num(ui, tr("Tilt y"), &mut p.bs_mut().bs_angle[1], "°");
        }
        if p.bs_mut().form == BsForm::Plate {
            num(ui, tr("Wedge x"), &mut p.bs_mut().wedge[0], "°");
            num(ui, tr("Wedge y"), &mut p.bs_mut().wedge[1], "°");
            ui.checkbox(&mut p.bs_mut().split_back, tr("Split at the back face"));
        }
        if p.bs_mut().form != BsForm::Ideal {
            ui.label(RichText::new(tr("Other faces' coating")).color(weak));
            ui.add(
                egui::TextEdit::singleline(&mut p.bs_mut().ar)
                    .hint_text(tr("no coating"))
                    .desired_width(70.0),
            )
            .on_hover_text(tr("Default coating of faces other than the splitting face (faces without their own coating)"));
            for f in p.bs_mut().form.faces() {
                let name = match *f {
                    "front" => tr("Table"),
                    "back" => tr("Back"),
                    x => x,
                };
                ui.label(RichText::new(trf("Surface {}", &[&name])).color(weak));
                let mut v = p.bs_mut().faces.get(*f).cloned().unwrap_or_default();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut v)
                            .hint_text(tr("Default"))
                            .desired_width(56.0),
                    )
                    .on_hover_text(if p.bs_mut().form == BsForm::Cube {
                        tr("Coating of cube face a (port A), b (B), c (C), d (D). Empty uses the other faces' coating")
                    } else {
                        tr("Coating of the plate's front/back (not applied to the splitting face). Empty uses the other faces' coating")
                    })
                    .changed()
                {
                    if v.trim().is_empty() {
                        p.bs_mut().faces.remove(*f);
                    } else {
                        p.bs_mut().faces.insert(f.to_string(), v);
                    }
                }
            }
        }
    });
    // 分岐
    let mut branched = p.branched;
    ui.horizontal_wrapped(|ui| {
        if ui
            .checkbox(&mut branched, tr("Elements beyond ports (branch)"))
            .on_hover_text(tr("Place elements beyond port B (transmitted), C (reflected) and D (reflection of light returning from B), e.g. interferometer arms. The BS becomes the last in its row and the elements after it move beyond the current port"))
            .changed()
        {
            combo.set_branched(loc, branched);
        }
    });
    let Some(p) = combo.get_mut(loc) else {
        return;
    };
    if !p.branched {
        return;
    }
    let names = [
        tr("Beyond port B (transmitted)"),
        tr("Beyond port C (reflected)"),
        tr("Beyond port D (returning reflection)"),
    ];
    let mut del: Option<(usize, usize)> = None;
    for (port, name) in names.iter().enumerate() {
        let b = &mut p.ports[port];
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(*name).color(weak));
            ui.add(
                egui::DragValue::new(&mut b.air)
                    .speed(0.05)
                    .range(0.0..=1e5)
                    .max_decimals(4)
                    .prefix(format!("{} ", tr("air")))
                    .suffix(" mm"),
            )
            .on_hover_text(tr("From the port to the first element (to the image plane if there is none)"));
            for (j, e) in b.picks.iter().enumerate() {
                ui.label("→");
                let mut l = e.label();
                if e.kind == ElemKind::Mirror && e.back {
                    l = trf("{} (returns)", &[&l]);
                }
                if ui
                    .small_button(format!("{l} ✖"))
                    .on_hover_text(tr("Remove"))
                    .clicked()
                {
                    del = Some((port, j));
                }
            }
            ui.menu_button("＋", |ui| {
                for kind in [
                    ElemKind::Lens,
                    ElemKind::Plate,
                    ElemKind::Mirror,
                    ElemKind::Stop,
                    ElemKind::Aperture,
                    ElemKind::Splitter,
                ] {
                    if ui.button(kind.label()).clicked() {
                        let mut e = Pick::new(kind);
                        if kind == ElemKind::Mirror {
                            // 腕の端の鏡: 光軸に垂直で往路を戻る
                            e.tilt = [0.0; 3];
                            e.back = true;
                        }
                        b.picks.push(e);
                        ui.close_menu();
                    }
                }
            })
            .response
            .on_hover_text(tr(
                "Add an element at the end of this port's row (to select and edit it in the figure, set the path through this port)",
            ));
        });
    }
    if let Some((port, j)) = del {
        p.ports[port].picks.remove(j);
    }
}

/// .zmx の絞りの面と像面の番号（読めなければ None）。ファイルの更新時刻ごとに覚えておく（図を描くたびに呼ぶので）。
/// 更新時刻の読み出し（stat）も 1 秒に 1 回まで（1 フレームに何十回も呼ばれ、ネットワークの上では遅い、GPN-12）
pub fn zmx_stop_info(path: &str) -> Option<(usize, usize)> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    /// 更新時刻、確かめた時刻、値
    type Memo = HashMap<
        String,
        (
            Option<std::time::SystemTime>,
            web_time::Instant,
            Option<(usize, usize)>,
        ),
    >;
    static MEMO: OnceLock<Mutex<Memo>> = OnceLock::new();
    if path.is_empty() {
        return None;
    }
    let memo = MEMO.get_or_init(Default::default);
    if let Some((_, at, v)) = memo.lock().ok()?.get(path) {
        if at.elapsed() < std::time::Duration::from_secs(1) {
            return *v;
        }
    }
    let mtime = lensbench_core::vfs::stamp(path).ok().and_then(|s| s.0);
    let now = web_time::Instant::now();
    if let Some((t, at, v)) = memo.lock().ok()?.get_mut(path) {
        if *t == mtime {
            *at = now;
            return *v;
        }
    }
    let v = lensbench_core::glass::read_text(std::path::Path::new(path))
        .ok()
        .and_then(|t| lensbench_core::zmx::parse_text(&t).ok())
        .and_then(|raw| {
            let stop = raw.surfs.iter().position(|x| x.stop)?;
            Some((stop, raw.surfs.len().checked_sub(1)?))
        });
    memo.lock().ok()?.insert(path.to_string(), (mtime, now, v));
    v
}

/// 面の並びの面の並びと種類（GPN-19）。半径・コーニック・厚さ・ガラス・半直径はレンズデータの表で編集し、
/// ここでは面の種類・足す・消すと、偏光子・波長板・Jones・座標ブレークの値（表の列に無いもの）だけを出す
fn surf_table(ui: &mut Ui, p: &mut Pick, k: usize, weak: Color32) {
    let n = p.surfs.len();
    let (mut del, mut add) = (None, None);
    let drag = |ui: &mut Ui, v: &mut f64, speed: f64| {
        ui.add(egui::DragValue::new(v).speed(speed).max_decimals(6));
    };
    ui.label(
        RichText::new(tr(
            "Radius, conic, thickness and semi-diameter are edited in the Lens Data table (surfaces in air only)",
        ))
        .small()
        .color(weak),
    );
    for (i, r) in p.surfs.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new((i + 1).to_string()).color(weak));
            egui::ComboBox::from_id_salt(("surf_kind", k, i))
                .selected_text(r.kind.label())
                .show_ui(ui, |ui| {
                    for x in [
                        SurfKind::Standard,
                        SurfKind::Polarizer,
                        SurfKind::Waveplate,
                        SurfKind::Jones,
                        SurfKind::Coordbrk,
                    ] {
                        ui.selectable_value(&mut r.kind, x, x.label());
                    }
                });
            if r.kind == SurfKind::Standard {
                let rad = if r.radius == 0.0 {
                    "∞".to_string()
                } else {
                    format!("{}", r.radius)
                };
                ui.label(
                    RichText::new(format!("R {rad}  t {}", r.t))
                        .small()
                        .color(weak),
                );
            }
            if ui
                .small_button("＋")
                .on_hover_text(tr("Add a surface after"))
                .clicked()
            {
                add = Some(i + 1);
            }
            if ui
                .add_enabled(n > 1, egui::Button::new("✖").small())
                .on_hover_text(tr("Remove surface"))
                .clicked()
            {
                del = Some(i);
            }
        });
        if r.kind == SurfKind::Standard {
            continue;
        }
        // 種類ごとの値は 1 行に 1 つ（GPN-19: 波長板の 11 個を横に並べない）
        ui.indent(("surf_vals", k, i), |ui| {
            egui::Grid::new(("surf_vals", k, i))
                .num_columns(2)
                .spacing([8.0, 2.0])
                .show(ui, |ui| {
                    let label = |ui: &mut Ui, t: &str| {
                        ui.label(RichText::new(t).small().color(weak));
                    };
                    match r.kind {
                        SurfKind::Standard => {}
                        SurfKind::Jones => {
                            for (j, l) in ["A", "B", "C", "D"].into_iter().enumerate() {
                                label(ui, l);
                                ui.horizontal(|ui| {
                                    drag(ui, &mut r.jones[2 * j], 0.01);
                                    ui.label("+");
                                    ui.add(
                                        egui::DragValue::new(&mut r.jones[2 * j + 1])
                                            .speed(0.01)
                                            .max_decimals(6)
                                            .suffix("i"),
                                    );
                                });
                                ui.end_row();
                            }
                        }
                        SurfKind::Polarizer => {
                            label(ui, tr("Transmission axis"));
                            ui.add(egui::DragValue::new(&mut r.axis_deg).speed(1.0).suffix("°"));
                            ui.end_row();
                            label(ui, tr("Extinction"));
                            ui.add(
                                egui::DragValue::new(&mut r.extinction_ratio)
                                    .speed(10.0)
                                    .range(0.0..=f64::MAX),
                            )
                            .on_hover_text(tr("0 means infinite (blocks the orthogonal direction)"));
                            ui.end_row();
                            label(ui, tr("Transmittance"));
                            ui.add(
                                egui::DragValue::new(&mut r.transmittance)
                                    .speed(0.01)
                                    .range(0.0..=1.0),
                            );
                            ui.end_row();
                        }
                        SurfKind::Waveplate => {
                            label(ui, tr("Retardance"));
                            ui.add(egui::DragValue::new(&mut r.retardance).speed(0.01).suffix(" λ"));
                            ui.end_row();
                            label(ui, tr("Design wavelength"));
                            ui.add(
                                egui::DragValue::new(&mut r.wavelength_um)
                                    .speed(0.001)
                                    .range(0.01..=100.0)
                                    .suffix(" µm"),
                            );
                            ui.end_row();
                            label(ui, tr("Fast axis"));
                            ui.add(egui::DragValue::new(&mut r.axis_deg).speed(1.0).suffix("°"));
                            ui.end_row();
                            label(ui, tr("Order"));
                            ui.add(egui::DragValue::new(&mut r.wp_order).speed(0.1));
                            ui.end_row();
                            ui.label(RichText::new(tr("Birefringent material")).small().color(weak))
                                .on_hover_text(tr("Ordinary index from the glass of this name, extraordinary from NAME-E (empty for an ideal retarder). A second material makes a two-material pair with crossed fast axes"));
                            let plate = |ui: &mut Ui, m: &mut String, t: &mut f64| {
                                ui.horizontal(|ui| {
                                    ui.add(egui::TextEdit::singleline(m).desired_width(80.0));
                                    ui.add(
                                        egui::DragValue::new(t)
                                            .speed(0.001)
                                            .range(0.0..=100.0)
                                            .suffix(" mm"),
                                    )
                                    .on_hover_text(tr("0 derives the thickness from the design retardance (single material)"));
                                });
                            };
                            plate(ui, &mut r.wp_material, &mut r.wp_thickness);
                            ui.end_row();
                            label(ui, "+");
                            plate(ui, &mut r.wp_material2, &mut r.wp_thickness2);
                            ui.end_row();
                        }
                        SurfKind::Coordbrk => {
                            for (j, l) in ["dx", "dy", "tx", "ty", "tz"].into_iter().enumerate() {
                                label(ui, l);
                                drag(ui, &mut r.cb[j], 0.05);
                                ui.end_row();
                            }
                            label(ui, tr("Thickness"));
                            drag(ui, &mut r.t, 0.05);
                            ui.end_row();
                            let mut tf = r.order == 1;
                            label(ui, "");
                            if ui.checkbox(&mut tf, tr("Tilt then decenter")).changed() {
                                r.order = tf as u8;
                            }
                            ui.end_row();
                        }
                    }
                });
        });
    }
    if let Some(i) = add {
        let semi = p.surfs.get(i.saturating_sub(1)).map_or(12.7, |r| r.semi);
        p.surfs.insert(
            i,
            SurfRow {
                semi,
                ..Default::default()
            },
        );
    }
    if let Some(i) = del {
        p.surfs.remove(i);
    }
}

/// 半径の JSON（自動なら "auto"）
fn semi_json(semi: f64, auto: bool) -> Value {
    if auto {
        json!("auto")
    } else {
        json!(semi)
    }
}

/// 自作の玉・平板の半径の欄と自動の切り替え
fn semi_edit(ui: &mut egui::Ui, semi: &mut f64, auto: &mut bool) {
    ui.add_enabled_ui(!*auto, |ui| {
        num_in(ui, tr("Semi-diameter"), semi, " mm", 0.001)
    });
    ui.checkbox(auto, tr("Auto")).on_hover_text(tr(
        "Compute the semi-diameter from rays of all fields and wavelengths",
    ));
}

#[cfg(test)]
#[path = "combo_tests.rs"]
mod tests;
