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
use crate::i18n::{tr, trf};
use crate::json_view::fmt_num;
use crate::sysset::{Aiming, ApKind, FieldKind, FieldPt, FieldSolve, WavePt};
use crate::theme;

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
            SurfKind::Standard => tr("面"),
            SurfKind::Jones => tr("Jones 行列"),
            SurfKind::Coordbrk => tr("座標ブレーク"),
            SurfKind::Polarizer => tr("偏光子"),
            SurfKind::Waveplate => tr("波長板"),
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
}

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
        }
    }
}

impl SurfRow {
    /// project の SurfSpec の JSON
    pub fn json(&self) -> Value {
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

/// 開口・遮蔽の形
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApShape {
    /// 円の開口（内径があれば輪帯）
    #[default]
    Circle,
    /// 矩形の開口
    Rect,
    /// 楕円の開口
    Ellipse,
    /// 円の遮蔽（内径〜外径を遮る）
    Obscuration,
    /// スパイダー（中心から外へ伸びる幅の腕を遮る。.zmx の SPID）
    Spider,
}

impl ApShape {
    pub fn label(self) -> &'static str {
        match self {
            ApShape::Circle => tr("円の開口"),
            ApShape::Rect => tr("矩形の開口"),
            ApShape::Ellipse => tr("楕円の開口"),
            ApShape::Obscuration => tr("円の遮蔽"),
            ApShape::Spider => tr("スパイダー"),
        }
    }
}

impl ElemKind {
    pub fn label(self) -> &'static str {
        match self {
            ElemKind::Catalog => tr("カタログの玉"),
            ElemKind::Lens => tr("自作の玉"),
            ElemKind::Plate => tr("平板"),
            ElemKind::Mirror => tr("鏡"),
            ElemKind::Stop => tr("絞り"),
            ElemKind::Splitter => tr("ビームスプリッター"),
            ElemKind::Zmx => tr(".zmx の玉"),
            ElemKind::Aperture => tr("開口・遮蔽"),
            ElemKind::Surfaces => tr("面の並び"),
            ElemKind::ZmxRange => tr(".zmx の取り込み"),
        }
    }
    /// ON/OFF を持つ（絞りは無い）
    pub fn has_on(self) -> bool {
        self != ElemKind::Stop
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
        }
    }
}

/// 組合せに置く機構部品: カタログの品を、要素（ブロック id）の面に置く
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MechPick {
    pub name: String,
    /// ユーザーカタログのファイル（絶対パス）と品の名前
    pub catalog: String,
    pub item: String,
    /// 置く要素のブロック id（空なら系の面 1）と、その要素の面（0 始まり）
    pub at: String,
    pub face: usize,
    /// 置いた面の局所座標でのずれ [mm] と傾き [deg]
    pub offset: [f64; 3],
    pub tilt: [f64; 2],
    /// 保持する要素のブロック id
    pub holds: Vec<String>,
    /// 部品の座を保持する要素の前面に合わせて置く（置く要素・面の代わり）
    pub seat: bool,
    /// 前の機構部品（名前）の後ろの端につなぐ（置く要素・面・座の代わり。z はそこからのずれ）
    pub after: String,
    /// 裏返して置く（光が部品の z の大きい端から入る）
    pub reversed: bool,
}

impl MechPick {
    /// プロジェクトの mechanics の項目
    pub fn json(&self) -> Value {
        let at = if self.at.is_empty() {
            json!({ "surface": 1 })
        } else {
            json!({ "block": self.at, "surface": self.face })
        };
        if !self.after.is_empty() {
            let mut v = json!({ "name": self.name, "catalog": self.catalog, "item": self.item,
                "after": self.after, "offset": self.offset });
            if !self.holds.is_empty() {
                v["holds"] = json!(self.holds);
            }
            if self.reversed {
                v["reversed"] = json!(true);
            }
            return v;
        }
        let mut v = json!({ "name": self.name, "catalog": self.catalog, "item": self.item, "at": at,
            "offset": self.offset, "tilt": self.tilt });
        if !self.holds.is_empty() {
            v["holds"] = json!(self.holds);
            if self.seat {
                v["seat"] = json!(true);
            }
        }
        if self.reversed {
            v["reversed"] = json!(true);
        }
        v
    }
}

/// 組合せの 1 つの要素（項目は種類ごとに使うものだけ。古い保存（vendor・part・reversed・air）も読める）
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
    /// BS: 透過率
    pub ratio: f64,
    /// .zmx の玉: ファイルと、何番目の玉か
    pub path: String,
    pub lens: usize,
    /// ロック: 中身（種類ごとの欄・面・反転）を変えない。位置・後の空気は変えられる
    pub locked: bool,
    /// ロックしたとき、位置（図のドラッグ・キー・偏心・傾き）と後の空気も変えない
    pub lock_place: bool,
    /// 鏡のコーニック
    pub conic: f64,
    /// BS: 分ける面の向きを逆に（反射を反対側へ。キューブの分ける面の傾き −45°）
    pub bs_flip: bool,
    /// 鏡の膜・BS の分ける面の膜（膜ファイル・LBCM の名前。空なら理想の鏡・ratio の分岐）
    pub coat: String,
    /// カタログの BS の、組合せの欄に無い項目（form・ar・faces・pbs・thickness・semi・wedge など）
    pub extra: Value,
    /// 開口・遮蔽の形。円・遮蔽の外径は semi、偏心は decenter（開口の dx・dy）
    pub ap: ApShape,
    /// 矩形・楕円の x・y の半幅 [mm]
    pub ap_half: [f64; 2],
    /// 円の開口・遮蔽の内径（半径）[mm]
    pub ap_min: f64,
    /// スパイダーの腕の幅（全幅）[mm]
    pub spider_width: f64,
    /// スパイダーの腕の本数（360° を等分、最初の腕は +x）
    pub spider_arms: u32,
    /// 面の表で変えた面の値（プロジェクトの surface_edits になる）
    pub edits: Vec<SurfEdit>,
    /// 玉の前面を絞りにする（カタログの玉・自作の玉・平板・.zmx の玉）
    pub stop_front: bool,
    /// .zmx の玉・取り込み: マルチコンフィグの番号（1 始まり）
    pub config: usize,
    /// 自作の玉（3 面以上・貼り合わせ）と面の並びの面。自作の玉は空なら r1・r2・t・glass の 2 面
    pub surfs: Vec<SurfRow>,
    /// .zmx の取り込み: 面 from〜to（0 は既定: 面 1〜像面の手前）
    pub range: [usize; 2],
    /// 面の絞り・ソルブ・ピックアップ（プロジェクトの surface_ops になる）。serve の edit の set_stop・add_solve・
    /// add_pickup の形で、この要素の面（surface・dst）は要素の面の番号（座標ブレークを除く）、ほかの要素の面（src・other）は
    /// {"block": id, "surface": i}
    pub ops: Vec<Value>,
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
    /// BS の形
    pub form: BsForm,
    /// BS（平板）: くさび。裏面を表面に対して傾ける角 x・y [deg]
    pub wedge: [f64; 2],
    /// BS（平板）: 裏の面で分ける
    pub split_back: bool,
    /// BS（平板・理想面）: 分ける面の傾き x・y [deg]。反射を逆向きにすると x の符号が変わる（キューブは 45° のまま）
    pub bs_angle: [f64; 2],
    /// BS: 分ける面の分け方（None は前の保存: 膜があれば膜、無ければ分岐比）
    pub split: Option<BsSplit>,
    /// BS: 理想の偏光 BS の p・s の強度の透過率
    pub pbs: [f64; 2],
    /// BS: 分ける面以外の面の膜の既定（空なら膜なし）
    pub ar: String,
    /// BS: 面ごとの膜（キューブは a〜d、平板は front・back）
    pub faces: std::collections::BTreeMap<String, String>,
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
}

/// BS の形
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BsForm {
    /// キューブ（斜面が 45°）
    #[default]
    Cube,
    /// 平板（表裏の 2 面、くさびも可）
    Plate,
    /// 厚さのない理想の面
    Ideal,
}

impl BsForm {
    pub fn label(self) -> &'static str {
        match self {
            BsForm::Cube => tr("キューブ"),
            BsForm::Plate => tr("平板"),
            BsForm::Ideal => tr("理想の面"),
        }
    }
    fn key(self) -> &'static str {
        match self {
            BsForm::Cube => "cube",
            BsForm::Plate => "plate",
            BsForm::Ideal => "ideal",
        }
    }
    /// 面ごとの膜を付ける面（分ける面を除く）
    pub fn faces(self) -> &'static [&'static str] {
        match self {
            BsForm::Cube => &["a", "b", "c", "d"],
            BsForm::Plate => &["front", "back"],
            BsForm::Ideal => &[],
        }
    }
}

/// BS の分ける面の分け方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BsSplit {
    /// 偏光によらない理想の分岐比
    Ratio,
    /// 膜（膜ファイル・膜モデル）
    Coat,
    /// 理想の偏光 BS
    Pbs,
}

impl BsSplit {
    pub fn label(self) -> &'static str {
        match self {
            BsSplit::Ratio => tr("分岐比"),
            BsSplit::Coat => tr("膜"),
            BsSplit::Pbs => tr("理想の PBS"),
        }
    }
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
            ratio: 0.5,
            path: String::new(),
            lens: 1,
            locked: false,
            lock_place: false,
            conic: 0.0,
            bs_flip: false,
            coat: String::new(),
            extra: Value::Null,
            ap: ApShape::Circle,
            ap_half: [5.0, 5.0],
            ap_min: 0.0,
            spider_width: 1.0,
            spider_arms: 4,
            edits: Vec::new(),
            form: BsForm::Cube,
            wedge: [0.0; 2],
            split_back: false,
            bs_angle: [45.0, 0.0],
            split: None,
            pbs: [1.0, 0.0],
            ar: String::new(),
            faces: Default::default(),
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
            self.form,
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
            }
            ElemKind::Aperture => {
                p.semi = 5.0;
                p.air = 10.0;
            }
            _ => {}
        }
        p
    }

    /// 面の値の上書きを足す（同じ面・同じ値の前の上書きは置き換える）
    pub fn set_edit(&mut self, surface: usize, field: &str, value: Value) {
        self.edits
            .retain(|e| !(e.surface == surface && e.field == field));
        self.edits.push(SurfEdit {
            surface,
            field: field.into(),
            value,
        });
    }

    /// BS の分ける面の分け方
    pub fn split_mode(&self) -> BsSplit {
        self.split.unwrap_or(if self.coat.trim().is_empty() {
            BsSplit::Ratio
        } else {
            BsSplit::Coat
        })
    }

    /// BS の分ける面の傾き（反射を逆向きにすると x の符号が変わる）
    pub fn split_angle(&self) -> [f64; 2] {
        let a = match self.form {
            BsForm::Cube => [45.0, 0.0],
            _ => self.bs_angle,
        };
        if self.bs_flip {
            [-a[0], a[1]]
        } else {
            a
        }
    }

    /// splitter ブロック（分岐の t・r・d は [`Combo::project`] が足す）
    fn splitter_block(&self, id: &str) -> Value {
        let mut b = json!({
            "type": "splitter", "id": id, "form": self.form.key(),
            "arm": if self.arm_r { "r" } else { "t" },
            "angle": self.split_angle(),
        });
        match self.form {
            BsForm::Cube => {
                b["size"] = json!(self.t);
                b["glass"] = json!(self.glass);
            }
            BsForm::Plate => {
                b["thickness"] = json!(self.t);
                b["glass"] = json!(self.glass);
                b["semi"] = json!(self.semi);
                if self.wedge != [0.0; 2] {
                    b["wedge"] = json!(self.wedge);
                }
                if self.split_back {
                    b["split_face"] = json!("back");
                }
            }
            BsForm::Ideal => {
                b["semi"] = json!(self.semi);
            }
        }
        match self.split_mode() {
            BsSplit::Coat if !self.coat.trim().is_empty() => b["coat"] = json!(self.coat.trim()),
            BsSplit::Pbs => b["pbs"] = json!({ "tp": self.pbs[0], "ts": self.pbs[1] }),
            _ => b["ratio"] = json!(self.ratio),
        }
        if self.form != BsForm::Ideal {
            if !self.ar.trim().is_empty() {
                b["ar"] = json!(self.ar.trim());
            }
            let faces: serde_json::Map<String, Value> = self
                .faces
                .iter()
                .filter(|(k, v)| !v.trim().is_empty() && self.form.faces().contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), json!(v.trim())))
                .collect();
            if !faces.is_empty() {
                b["faces"] = Value::Object(faces);
            }
        }
        b
    }

    /// カタログの BS の中身（[`Pick::extra`]）のうち、欄のある項目を欄へ移す
    pub fn absorb_extra(&mut self) {
        if self.kind != ElemKind::Splitter {
            return;
        }
        let Some(m) = self.extra.as_object_mut() else {
            return;
        };
        let num2 =
            |v: &Value| -> Option<[f64; 2]> { Some([v.get(0)?.as_f64()?, v.get(1)?.as_f64()?]) };
        if let Some(f) = m.remove("form") {
            self.form = match f.as_str() {
                Some("plate") => BsForm::Plate,
                Some("ideal") => BsForm::Ideal,
                _ => BsForm::Cube,
            };
        }
        if let Some(x) = m.remove("thickness").and_then(|v| v.as_f64()) {
            if self.form == BsForm::Plate {
                self.t = x;
            }
        }
        if let Some(x) = m.remove("size").and_then(|v| v.as_f64()) {
            if self.form == BsForm::Cube && x > 0.0 {
                self.t = x;
            }
        }
        if let Some(x) = m.remove("semi").and_then(|v| v.as_f64()) {
            self.semi = x;
        }
        if let Some(x) = m.remove("wedge").as_ref().and_then(num2) {
            self.wedge = x;
        }
        if let Some(f) = m.remove("split_face") {
            self.split_back = f == "back";
        }
        if let Some(a) = m.remove("angle").as_ref().and_then(num2) {
            self.bs_flip = a[0] < 0.0;
            self.bs_angle = [a[0].abs(), a[1]];
        }
        if let Some(a) = m.remove("arm") {
            self.arm_r = a == "r";
        }
        if let Some(p) = m.remove("pbs").filter(|p| p.is_object()) {
            self.pbs = [
                p["tp"].as_f64().unwrap_or(1.0),
                p["ts"].as_f64().unwrap_or(0.0),
            ];
            self.split = Some(BsSplit::Pbs);
        }
        if let Some(x) = m.remove("ratio").and_then(|v| v.as_f64()) {
            self.ratio = x;
        }
        if let Some(c) = m
            .remove("coat")
            .and_then(|v| v.as_str().map(str::to_string))
        {
            self.coat = c;
        }
        if let Some(a) = m.remove("ar").and_then(|v| v.as_str().map(str::to_string)) {
            self.ar = a;
        }
        if let Some(f) = m.remove("faces").and_then(|v| v.as_object().cloned()) {
            for (k, v) in f {
                if let Some(v) = v.as_str() {
                    self.faces.insert(k, v.to_string());
                }
            }
        }
        if let Some(g) = m
            .remove("glass")
            .and_then(|v| v.as_str().map(str::to_string))
        {
            self.glass = g;
        }
        for k in ["type", "id"] {
            m.remove(k);
        }
        if m.is_empty() {
            self.extra = Value::Null;
        }
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
            ElemKind::Aperture => self.ap.label().to_string(),
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
        match self.ap {
            ApShape::Circle => json!({
                "type": "circular", "min": self.ap_min.max(0.0), "max": self.semi, "dx": dx, "dy": dy,
            }),
            ApShape::Rect => json!({
                "type": "rect", "hx": self.ap_half[0], "hy": self.ap_half[1], "dx": dx, "dy": dy,
            }),
            ApShape::Ellipse => json!({
                "type": "ellipse", "hx": self.ap_half[0], "hy": self.ap_half[1], "dx": dx, "dy": dy,
            }),
            ApShape::Obscuration => json!({
                "type": "obscuration", "min": self.ap_min.max(0.0), "max": self.semi, "dx": dx, "dy": dy,
            }),
            ApShape::Spider => json!({
                "type": "spider", "width": self.spider_width.max(0.0), "arms": self.spider_arms.max(1),
                "dx": dx, "dy": dy,
            }),
        }
    }

    /// 開口・遮蔽の、図の縦の広がり（光軸から）。外の縁
    pub fn ap_extent(&self) -> f64 {
        let h = match self.ap {
            ApShape::Rect | ApShape::Ellipse => self.ap_half[0].hypot(self.ap_half[1]),
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
                let gs: Vec<Value> = self
                    .zgroups
                    .iter()
                    .filter(|g| g.from > 0 && g.to >= g.from)
                    .map(|g| {
                        let mut v = json!({ "from": g.from, "to": g.to });
                        if !g.name.trim().is_empty() {
                            v["id"] = json!(g.name.trim());
                            v["name"] = json!(g.name.trim());
                        }
                        if !g.role.is_empty() {
                            v["role"] = json!(g.role);
                        }
                        v
                    })
                    .collect();
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
        if self.kind == ElemKind::Catalog && !self.path.is_empty() {
            // ユーザーカタログの項目は群として入る（反転は無い）
            self.place(&mut b);
        } else if matches!(
            self.kind,
            ElemKind::Catalog | ElemKind::Zmx | ElemKind::Lens | ElemKind::Plate
        ) {
            if self.reversed {
                b["reversed"] = json!(true);
            }
            if self.stop_front {
                b["stop"] = json!(true);
            }
            self.place(&mut b);
        }
        if matches!(
            self.kind,
            ElemKind::Splitter | ElemKind::Surfaces | ElemKind::ZmxRange
        ) {
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
        if self.kind == ElemKind::Mirror && self.back {
            b["back"] = json!(true);
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
}

/// 組合せの最後の検出器のブロック: 種類と、欄の文字列（項目の名前 → 欄に書いた文字列、空は既定）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectorPick {
    pub kind: String,
    pub vals: BTreeMap<String, String>,
}

impl DetectorPick {
    /// プロジェクトのブロック（{"type": kind, 項目…}）。種類が登録されていなければ欄の値をそのまま入れ子に
    pub fn block(&self) -> Result<Value, String> {
        let mut v = match crate::ext::detector(&self.kind) {
            Some(d) => d.object.json(&self.vals, false)?,
            None => crate::argform::nest(
                &self
                    .vals
                    .iter()
                    .filter(|(_, t)| !t.trim().is_empty())
                    .map(|(k, t)| {
                        let x = serde_json::from_str(t).unwrap_or_else(|_| json!(t));
                        (k.clone(), x)
                    })
                    .collect(),
            ),
        };
        if !v.is_object() {
            v = json!({});
        }
        v["type"] = json!(self.kind);
        Ok(v)
    }
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

/// .zmx の取り込みの中の群（プロジェクトの zmx の groups）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ZGroup {
    pub from: usize,
    pub to: usize,
    pub name: String,
    pub role: String,
}

impl Default for Combo {
    fn default() -> Self {
        Combo {
            name: "カタログレンズの組合せ".into(),
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
            path: String::new(),
            glass_catalogs: vec!["SCHOTT".into()],
            states: Vec::new(),
            state: None,
            state_sys: BTreeMap::new(),
            state_path: BTreeMap::new(),
            coatings: BTreeMap::new(),
            groups: Vec::new(),
            detector: None,
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
            self.get(&l)?
                .ops
                .iter()
                .find(|o| o["op"] == "set_stop")
                .and_then(|o| o["surface"].as_u64())
                .map(|i| (l.clone(), i as usize))
        })
    }

    /// 絞りを場所 loc の要素の面 i に（None なら要素の面の絞りをやめる）
    pub fn set_stop_face(&mut self, at: Option<(Loc, usize)>) {
        for l in self.all() {
            if let Some(p) = self.get_mut(&l) {
                p.ops.retain(|o| o["op"] != "set_stop");
                // 玉の前面の絞りも面の絞りに置き換える
                if at.is_some() {
                    p.stop_front = false;
                }
            }
        }
        if let Some((l, i)) = at {
            if let Some(p) = self.get_mut(&l) {
                p.ops.push(json!({ "op": "set_stop", "surface": i }));
            }
        }
    }

    /// 今の経路に沿った要素のブロックの id（= 玉・群の名前、[`Combo::ids_all`]）
    pub fn ids(&self) -> Vec<String> {
        let all = self.ids_all();
        self.chain()
            .iter()
            .map(|s| {
                all.iter()
                    .find(|x| x.0 == s.loc)
                    .map(|x| x.1.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// プロジェクトファイル（JSON）にする
    pub fn project(&self) -> Value {
        let mut blocks = Vec::new();
        let mut edits = Vec::new();
        let mut ops = Vec::new();
        if self.lead_stop() {
            blocks.push(json!({ "type": "stop", "semi": self.epd / 2.0 }));
            blocks.push(json!({ "type": "air", "t": self.stop_gap }));
        }
        let grouped = self.blocks(&mut blocks, &mut edits);
        // 面の絞り・ソルブ・ピックアップと状態
        let mut states: Vec<(String, serde_json::Map<String, Value>)> = self
            .states
            .iter()
            .map(|n| (n.clone(), serde_json::Map::new()))
            .collect();
        for (loc, id) in self.ids_all() {
            let Some(p) = self.get(&loc) else { continue };
            for o in &p.ops {
                let mut o = o.clone();
                for key in ["surface", "dst"] {
                    if let Some(i) = o.get(key).and_then(Value::as_u64) {
                        o[key] = json!({ "block": id, "surface": i });
                    }
                }
                ops.push(o);
            }
            for (n, set) in states.iter_mut() {
                if let Some(&on) = p.state_on.get(n).filter(|_| p.kind.has_on()) {
                    set.insert(id.clone(), json!({ "on": on }));
                }
                if let Some(&t) = p.state_air.get(n) {
                    set.insert(format!("{id}_air"), json!({ "t": t }));
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
        if !edits.is_empty() {
            v["surface_edits"] = Value::Array(edits);
        }
        if !ops.is_empty() {
            v["surface_ops"] = Value::Array(ops);
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
            .zip(self.chain_picks())
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
    /// 玉・群の名前と面の範囲
    pub units: Vec<(String, Vec<[usize; 2]>)>,
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
            units: arr("units")
                .iter()
                .map(|u| {
                    (
                        u["name"].as_str().unwrap_or_default().to_string(),
                        u["spans"]
                            .as_array()
                            .map(|a| a.iter().map(|x| [us(&x[0]), us(&x[1])]).collect())
                            .unwrap_or_default(),
                    )
                })
                .collect(),
            xz: v["plane"] == "xz",
            unfold: v["unfold"] == true,
            d3: (v["plane"] == "3d")
                .then(|| std::sync::Arc::new(crate::view3d::Scene3::parse(v, by_cfg))),
        }
    }

    /// 要素ごとの面の範囲。玉・群として名前の無い要素（BS など）は、前後の要素の間の面とする
    pub fn spans_for(&self, ids: &[String], lead_stop: bool) -> Vec<Option<[usize; 2]>> {
        // 空の id（OFF の要素）は系に無いので除いて決める
        if ids.iter().any(String::is_empty) {
            let live: Vec<usize> = (0..ids.len()).filter(|&k| !ids[k].is_empty()).collect();
            let sub: Vec<String> = live.iter().map(|&k| ids[k].clone()).collect();
            let mut out = vec![None; ids.len()];
            for (j, sp) in self.spans_for(&sub, lead_stop).into_iter().enumerate() {
                out[live[j]] = sp;
            }
            return out;
        }
        // 名前の玉は前の要素より後の通り（往路を戻る系は同じ玉を 2 度通る。往きの通りを当てる）
        let mut v: Vec<Option<[usize; 2]>> = Vec::with_capacity(ids.len());
        let mut prev = if lead_stop { self.stop } else { None };
        for id in ids {
            let s = self.span_after(id, prev, ids);
            if let Some(s) = s {
                prev = Some(s[1]);
            }
            v.push(s);
        }
        let img = self.image_surface().unwrap_or(self.vertices.len());
        // 要素の名前に当たらない玉（serve が自動で付けた lens2 など。BS のキューブも玉になる）
        let mut spare: Vec<[usize; 2]> = self
            .units
            .iter()
            .filter(|(n, _)| !ids.contains(n))
            .filter_map(|(n, _)| self.span(n))
            .collect();
        spare.sort();
        let mut k = 0;
        while k < v.len() {
            if v[k].is_some() {
                k += 1;
                continue;
            }
            // 名前で決まらない要素の並び k..e と、その前後の面
            let e = (k..v.len()).find(|&j| v[j].is_some()).unwrap_or(v.len());
            let prev_end = if k == 0 {
                if lead_stop {
                    self.stop
                } else {
                    Some(0)
                }
            } else {
                v[k - 1].map(|s| s[1])
            };
            let next_start = if e == v.len() {
                Some(img)
            } else {
                v[e].map(|s| s[0])
            };
            if let (Some(a), Some(b)) = (prev_end, next_start) {
                let inside: Vec<[usize; 2]> = spare
                    .iter()
                    .copied()
                    .filter(|s| a < s[0] && s[1] < b)
                    .collect();
                if inside.len() == e - k {
                    // 並び（BS が続くときなど）の玉が要素と同じ数なら順に当てる
                    for (j, s) in (k..e).zip(inside) {
                        v[j] = Some(s);
                    }
                } else if e - k == 1 && a + 1 < b {
                    // 間がこの要素だけ
                    v[k] = Some([a + 1, b - 1]);
                }
            }
            k = e;
        }
        v
    }

    /// 名前の玉・群の面の範囲（最初・最後）
    fn span(&self, name: &str) -> Option<[usize; 2]> {
        let (_, s) = self.units.iter().find(|(n, _)| n == name)?;
        Some([s.iter().map(|x| x[0]).min()?, s.iter().map(|x| x[1]).max()?])
    }

    /// 名前の玉・群の、面 after より後の通りの範囲。ほかの要素（ids）の面が間に入ったら、そこまで（復路は入れない）
    fn span_after(&self, name: &str, after: Option<usize>, ids: &[String]) -> Option<[usize; 2]> {
        let (_, all) = self.units.iter().find(|(n, _)| n == name)?;
        let mut sp: Vec<[usize; 2]> = all
            .iter()
            .copied()
            .filter(|s| after.map_or(true, |a| s[0] > a))
            .collect();
        if sp.is_empty() {
            return self.span(name);
        }
        sp.sort();
        let first = sp[0];
        let limit = self
            .units
            .iter()
            .filter(|(n, _)| n != name && ids.contains(n))
            .flat_map(|(_, s)| s.iter().map(|x| x[0]))
            .filter(|&x| x > first[0])
            .min()
            .unwrap_or(usize::MAX);
        let end = sp
            .iter()
            .filter(|s| s[0] < limit)
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
    crate::coat_defs::set_names(ui.ctx(), combo.coatings.keys().cloned().collect());
    // 要素のパレット
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tr("足す")).small().color(weak));
        if ui
            .button(trf("＋ {}…", &[&tr("カタログの玉")]))
            .on_hover_text(tr(
                "カタログのパネルで行をダブルクリック、またはレイアウト図へドラッグ",
            ))
            .clicked()
        {
            act = Some(Act::Catalog);
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
            if ui.button(trf("＋ {}", &[&k.label()])).clicked() {
                insert(combo, view, Pick::new(k));
            }
        }
        for (kind, hint) in [
            (ElemKind::Zmx, tr(".zmx の k 番目の玉")),
            (
                ElemKind::ZmxRange,
                tr(".zmx の面の範囲（既定は面 1〜像面の手前）をまるごと入れる（測定系の中の被検レンズなど）"),
            ),
        ] {
            let start = ui
                .button(trf("＋ {}…", &[&kind.label()]))
                .on_hover_text(hint)
                .clicked();
            let key = format!("combo-zmx-{:?}", kind);
            {
                if let Some(p) = crate::files::open(&key, start, || {
                    // lensbench の系（系ファイル・プロジェクトの .json）も取り込める（#132）
                    crate::files::FileDialog::new()
                        .add_filter(tr("系（.zmx・.json）"), &["zmx", "json"])
                }) {
                    let mut e = Pick::new(kind);
                    e.path = p.display().to_string();
                    insert(combo, view, e);
                }
            }
        }
        ui.separator();
        if ui
            .add_enabled(
                !combo.is_empty(),
                egui::Button::new(tr("像面を近軸の焦点へ")),
            )
            .on_hover_text(tr("最後の空気を、軸上の細い光線が軸と交わる所までにする"))
            .clicked()
        {
            act = Some(Act::Focus);
        }
        if path_select(ui, combo, paths) {
            view.sel = None;
            view.dim = None;
        }
        ui.checkbox(auto, tr("変えたらすぐ作り直す"));
        if (!*auto || (data.is_none() && !building)) && ui.button(tr("▶ 系を作る")).clicked()
        {
            act = Some(Act::Build);
        }
    });
    if !combo.picks.is_empty() {
        ui.horizontal_wrapped(|ui| states_ui(ui, combo, view, weak));
        ui.horizontal_wrapped(|ui| groups_ui(ui, combo, weak, None));
    }
    ui.separator();
    if combo.picks.is_empty() {
        let r = ui
            .weak(tr(
                "要素を足すと、ここにレイアウト図が出ます（光は左から右へ）。",
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
    // 面の表（自作の玉の 3 面以上・面の並び）を出すときはその分も
    let rows = sel
        .filter(|p| p.kind == ElemKind::Surfaces || p.kind == ElemKind::Lens && p.surfs.len() >= 2)
        .map_or(0, |p| p.surfs.len() + 1);
    let editor_h = if bs_sel { 200.0 } else { 96.0 } + 24.0 * rows as f32;
    // 図と設定に残りを分ける（パネルが低くても設定が押し出されないように。横に広く低いと左右に並べる）
    let avail = ui.available_height().max(1.0);
    let (mut top, mut bottom) = crate::split::two(
        ui,
        "combo_fig",
        (((avail - editor_h) / avail).clamp(0.5, 0.85), 0.6),
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
        figure(ui, rect, &resp, combo, d, view, accent);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            if building {
                tr("系を作っています…")
            } else {
                tr("「▶ 系を作る」でレイアウト図が出ます")
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
    ui.label(RichText::new(tr("経路")).color(theme::weak_text(ui.visuals())))
        .on_hover_text(tr("BS を通る順の透過 T・反射 R（足りない文字は各 BS の既定の口）。図・解析はこの経路。ほかの経路は「重ねる構成」で重ねて描ける"));
    let shown = if path.is_empty() {
        tr("既定").to_string()
    } else {
        path.clone()
    };
    egui::ComboBox::from_id_salt("combo_path")
        .selected_text(shown)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut path, String::new(), tr("既定"));
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
                .small_button(tr("基本の経路に"))
                .on_hover_text(trf("状態 {} で変えた経路をやめる", &[n]))
                .clicked()
            {
                combo.state_path.remove(n);
                return true;
            }
        } else {
            ui.label(
                RichText::new(tr("（基本と同じ）"))
                    .small()
                    .color(theme::weak_text(ui.visuals())),
            )
            .on_hover_text(trf("変えると状態 {} だけの経路になる", &[n]));
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
    nav: Option<egui::Id>,
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
    let mut none = Combo {
        picks: Vec::new(),
        ..Default::default()
    };
    let mut view = View {
        hl_surf,
        ..Default::default()
    };
    if let Some(id) = nav {
        let (z, p): (f32, Vec2) = ui.data(|m| m.get_temp(id)).unwrap_or_default();
        view.nav = true;
        view.zoom_log = z;
        view.pan = p;
    }
    figure(ui, rect, &resp, &mut none, d, &mut view, accent);
    if let Some(id) = nav {
        ui.data_mut(|m| m.insert_temp(id, (view.zoom_log, view.pan)));
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
                        trf("面 {} の後に差し込む", &[&a]),
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
/// Shift+↑ ↓ で傾き、R で反転、Delete で外す、Esc で選ぶのをやめる。Ctrl で 10 倍、Alt で 1/10
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
            Key::L => {
                if let Some(p) = view.sel.and_then(|k| combo.el_mut(k)) {
                    p.locked = !p.locked;
                }
            }
            Key::R => {
                if let Some(p) = view
                    .sel
                    .filter(|&k| free(combo, k))
                    .and_then(|k| combo.el_mut(k))
                {
                    p.reversed = !p.reversed;
                }
            }
            Key::F => {
                if let Some(p) = view
                    .sel
                    .filter(|&k| free(combo, k))
                    .and_then(|k| combo.el_mut(k))
                    .filter(|p| p.kind == ElemKind::Splitter)
                {
                    p.bs_flip = !p.bs_flip;
                }
            }
            Key::Delete | Key::Backspace => {
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

/// キューブの半分（直角三角形の断面 [z, y]）から、キューブの正方形（4 隅）と分ける面（斜辺）
fn cube_of_prism(pts: &[[f64; 2]]) -> Option<[Vec<[f64; 2]>; 2]> {
    // 重なった点（断面の端で同じ角が 2 度出ることがある）は 1 つに
    let mut q: Vec<[f64; 2]> = Vec::new();
    for p in pts {
        let tol = 1e-6 * (1.0 + p[0].abs() + p[1].abs());
        if !q
            .iter()
            .any(|r| (r[0] - p[0]).abs() < tol && (r[1] - p[1]).abs() < tol)
        {
            q.push(*p);
        }
    }
    let pts = &q[..];
    if pts.len() != 3 {
        return None;
    }
    let sub = |a: [f64; 2], b: [f64; 2]| [a[0] - b[0], a[1] - b[1]];
    let len = |v: [f64; 2]| v[0].hypot(v[1]);
    // 直角の頂点: 残りの 2 点への向きの内積がいちばん小さい
    let (i, cosv) = (0..3)
        .map(|i| {
            let (c, a, b) = (pts[i], pts[(i + 1) % 3], pts[(i + 2) % 3]);
            let (u, v) = (sub(a, c), sub(b, c));
            (
                i,
                (u[0] * v[0] + u[1] * v[1]) / (len(u) * len(v)).max(1e-300),
            )
        })
        .min_by(|x, y| x.1.abs().total_cmp(&y.1.abs()))?;
    if cosv.abs() > 1e-3 {
        return None;
    }
    let (c, a, b) = (pts[i], pts[(i + 1) % 3], pts[(i + 2) % 3]);
    let o = [a[0] + b[0] - c[0], a[1] + b[1] - c[1]];
    Some([vec![a, c, b, o], vec![a, b]])
}

/// 図を動かすドラッグか（中ボタン・右ボタン、Space＋左ボタン）
fn panning(ui: &Ui, resp: &egui::Response) -> bool {
    resp.dragged_by(egui::PointerButton::Middle)
        || resp.dragged_by(egui::PointerButton::Secondary)
        || (resp.dragged_by(egui::PointerButton::Primary) && ui.input(|i| i.key_down(Key::Space)))
}

/// 2D の図の拡大縮小・移動・全体表示（#124）。ホイール・ピンチでカーソルの所を中心に拡大縮小、
/// 中ボタン・右ボタン・Space＋左のドラッグで移動、Home キー・何も無い所のダブルクリック・「全体」で全体表示。
/// fit_center は全体表示のときの図の中心（画面）
fn navigate(ui: &mut Ui, rect: Rect, fit_center: Pos2, resp: &egui::Response, view: &mut View) {
    if panning(ui, resp) {
        view.pan += resp.drag_delta();
    }
    if resp.hovered() {
        // ホイールは図の拡大縮小に使い、パネルのスクロールには渡さない（なめらかなスクロールは数フレームに
        // 分けて届くので、図の上にある間は毎フレーム取る）
        let (wheel, pinch, pos) = ui.input_mut(|i| {
            let w = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta = Vec2::ZERO;
            i.raw_scroll_delta = Vec2::ZERO;
            (w, i.zoom_delta(), i.pointer.hover_pos())
        });
        let f = pinch * (wheel / 200.0).exp();
        if (f - 1.0).abs() > 1e-4 {
            view.zoom_about(fit_center, pos.unwrap_or(fit_center), f);
        }
        if ui.input(|i| i.key_pressed(Key::Home)) {
            view.zoom_log = 0.0;
            view.pan = Vec2::ZERO;
        }
    }
    // 拡大・移動しているときは右上に「全体」（置き場は取らない: ui.put は下の欄の位置をずらす）
    if view.zoom_log != 0.0 || view.pan != Vec2::ZERO {
        let b = Rect::from_min_size(
            rect.right_top() + egui::vec2(-64.0, 8.0),
            egui::vec2(56.0, 22.0),
        );
        let r = ui
            .interact(b, resp.id.with("fit"), Sense::click())
            .on_hover_text(crate::i18n::tr(
                "全体を表示（Home・何も無い所のダブルクリックでも）",
            ));
        let w = ui.style().interact(&r);
        let p = ui.painter_at(rect);
        p.rect(
            b,
            4.0,
            w.weak_bg_fill,
            w.bg_stroke,
            egui::StrokeKind::Inside,
        );
        p.text(
            b.center(),
            egui::Align2::CENTER_CENTER,
            crate::i18n::tr("全体"),
            egui::FontId::proportional(12.0),
            w.text_color(),
        );
        if r.clicked() {
            view.zoom_log = 0.0;
            view.pan = Vec2::ZERO;
        }
    }
}

/// 図を描き、クリック・ドラッグを扱う
fn figure(
    ui: &mut Ui,
    rect: Rect,
    resp: &egui::Response,
    combo: &mut Combo,
    d: &LayoutData,
    view: &mut View,
    accent: Color32,
) {
    view.d3 = d.d3.is_some();
    view.fixed = d.unfold || view.d3;
    if let Some(sc) = &d.d3 {
        // 3D: 要素の選択だけ（寸法線・ドラッグでの移動はなし）
        let spans = d.spans_for(&combo.ids(), combo.lead_stop());
        let owner = |sf: [usize; 2]| {
            spans
                .iter()
                .position(|sp| sp.is_some_and(|sp| sp[0] <= sf[0] && sf[0] <= sp[1]))
        };
        view.axes.clear();
        view.mids.clear();
        if let Some(k) = crate::view3d::show(ui, resp, sc, &d.configs, &owner, view.sel, accent) {
            view.sel = k;
            view.dim = None;
        }
        return;
    }
    let vis = ui.visuals().clone();
    let weak = theme::weak_text(&vis);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, vis.extreme_bg_color);
    // 図の範囲（寸法線の分、下に余白）
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut grow = |p: &[f64; 2]| {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    };
    d.solids
        .iter()
        .flat_map(|s| s.pts.iter())
        .for_each(&mut grow);
    d.plates.iter().flat_map(|p| p.2.iter()).for_each(&mut grow);
    d.mechs
        .iter()
        .flat_map(|m| m.1.iter().flatten())
        .for_each(&mut grow);
    d.rays.iter().flat_map(|r| r.1.iter()).for_each(&mut grow);
    if !lo[0].is_finite() {
        return;
    }
    let inner = rect.shrink2(egui::vec2(24.0, 16.0));
    // 寸法線の分（要素が無ければ要らない）
    let dim_room = if combo.is_empty() { 0.0 } else { 44.0 };
    let inner = Rect::from_min_max(inner.min, inner.max - egui::vec2(0.0, dim_room));
    let w = (hi[0] - lo[0]).max(1e-6);
    let h = (hi[1] - lo[1]).max(1e-6);
    let fit = (inner.width() as f64 / w).min(inner.height() as f64 / h);
    let cz = (lo[0] + hi[0]) / 2.0;
    let cy = (lo[1] + hi[1]) / 2.0;
    if view.nav {
        navigate(ui, rect, inner.center(), resp, view);
    }
    let s = fit * (view.zoom_log as f64).exp();
    let center = inner.center() + view.pan;
    let to = |p: [f64; 2]| -> Pos2 {
        Pos2::new(
            center.x + ((p[0] - cz) * s) as f32,
            center.y - ((p[1] - cy) * s) as f32,
        )
    };
    view.surf_pos = d.vertices.iter().map(|v| v.map(to)).collect();
    // 光軸
    let axis_y = to([0.0, 0.0]).y;
    if rect.y_range().contains(axis_y) {
        painter.add(egui::Shape::dashed_line(
            &[
                Pos2::new(rect.left() + 8.0, axis_y),
                Pos2::new(rect.right() - 8.0, axis_y),
            ],
            Stroke::new(1.0_f32, vis.widgets.noninteractive.bg_stroke.color),
            6.0,
            4.0,
        ));
    }
    // 光線（視野ごとの色）
    for (f, p) in &d.rays {
        let line: Vec<Pos2> = p.iter().map(|&q| to(q)).collect();
        painter.add(egui::Shape::line(
            line,
            Stroke::new(
                1.0_f32,
                theme::series(f.saturating_sub(1)).gamma_multiply(0.85),
            ),
        ));
    }
    // 構成を重ねたときは構成の名前と色（左上）
    if d.configs.len() > 1 {
        let font = egui::FontId::proportional(11.0);
        for (k, name) in d.configs.iter().enumerate() {
            let p = rect.left_top() + egui::vec2(10.0, 10.0 + 15.0 * k as f32);
            let c = theme::series(k);
            painter.line_segment(
                [p + egui::vec2(0.0, 6.0), p + egui::vec2(14.0, 6.0)],
                (2.0, c),
            );
            painter.text(
                p + egui::vec2(18.0, 0.0),
                egui::Align2::LEFT_TOP,
                name,
                font.clone(),
                weak,
            );
        }
    }
    // 要素（今の経路に沿った並び）と玉の対応
    let ids = combo.span_ids();
    let chain = combo.chain();
    let els = combo.chain_picks();
    let spans = d.spans_for(&ids, combo.lead_stop());
    // 要素ごとの、画面の上での光軸の向き（矢印キー・ドラッグを画面の向きに合わせる）
    let vtx = |j: usize| d.vertices.get(j).copied().flatten().map(to);
    // 面 j へ入ってくる向き（前の面の頂点から）
    let incoming = |j: usize| {
        let a = vtx(j)?;
        (0..j).rev().find_map(|i| {
            let u = a - vtx(i)?;
            (u.length() > 1.0).then(|| u.normalized())
        })
    };
    view.xz = d.xz;
    view.axes = spans
        .iter()
        .zip(&els)
        .map(|(sp, p)| {
            let sp = (*sp)?;
            // BS は入ってくる向き（反射側へ進むと、前後の面の差は光軸の向きにならない）
            if p.kind == ElemKind::Splitter {
                return incoming(sp[0]);
            }
            let (a, b) = (vtx(sp[0])?, vtx(sp[1])?);
            let u = b - a;
            if u.length() > 1.0 {
                return Some(u.normalized());
            }
            // 面が 1 枚（鏡・絞り）: 入ってくる向き
            incoming(sp[0])
        })
        .map(|u| u.unwrap_or(Vec2::X))
        .collect();
    let owner = |sd: &Solid| {
        spans
            .iter()
            .position(|sp| sp.is_some_and(|sp| sp[0] <= sd.surfaces[0] && sd.surfaces[0] <= sp[1]))
    };
    let glass_fill = if vis.dark_mode {
        Color32::from_rgba_unmultiplied(0x2B, 0x4A, 0x66, 150)
    } else {
        Color32::from_rgba_unmultiplied(0xCD, 0xE4, 0xF8, 200)
    };
    let mirror_fill = Color32::from_rgba_unmultiplied(0x9A, 0xA3, 0xAF, 160);
    let mut hit: Option<usize> = None;
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    // BS: serve の断面（キューブの半分の直角プリズム = 直角三角形）から正方形と分ける面を作る。
    // 反射側へ進むと、通るのは片方の半分だけなので、もう半分は直角の頂点を斜辺の中点で折り返して補う
    let is_bs = |k: Option<usize>| k.is_some_and(|k| els[k].kind == ElemKind::Splitter);
    let mut bs_center: Vec<Option<Pos2>> = vec![None; els.len()];
    for (k, p) in els.iter().enumerate() {
        if p.kind != ElemKind::Splitter {
            continue;
        }
        let Some([sq, diag]) = d
            .solids
            .iter()
            .filter(|sd| owner(sd) == Some(k))
            .find_map(|sd| cube_of_prism(&sd.pts))
        else {
            continue;
        };
        let sq: Vec<Pos2> = sq.iter().map(|&q| to(q)).collect();
        let diag = [to(diag[0]), to(diag[1])];
        bs_center[k] = Some(diag[0] + (diag[1] - diag[0]) / 2.0);
        let selected = view.sel == Some(k);
        let stroke = if selected {
            Stroke::new(2.2_f32, accent)
        } else {
            Stroke::new(1.0_f32, accent.gamma_multiply(0.75))
        };
        painter.add(egui::Shape::convex_polygon(sq.clone(), glass_fill, stroke));
        painter.line_segment(diag, Stroke::new(1.5_f32, mirror_fill));
        if pointer.is_some_and(|q| inside(&sq, q)) {
            hit = Some(k);
        }
    }
    for sd in &d.solids {
        let k = owner(sd);
        if is_bs(k) {
            continue;
        }
        let poly: Vec<Pos2> = sd.pts.iter().map(|&q| to(q)).collect();
        let selected = k.is_some() && k == view.sel;
        let stroke = if selected {
            Stroke::new(2.2_f32, accent)
        } else {
            Stroke::new(1.0_f32, accent.gamma_multiply(0.75))
        };
        painter.add(egui::Shape::convex_polygon(
            poly.clone(),
            if sd.mirror { mirror_fill } else { glass_fill },
            Stroke::NONE,
        ));
        let mut closed = poly.clone();
        if let Some(f) = poly.first() {
            closed.push(*f);
        }
        painter.add(egui::Shape::line(closed, stroke));
        if pointer.is_some_and(|p| inside(&poly, p)) {
            view.hover_surfs = Some(sd.surfaces);
            if let Some(k) = k {
                hit = Some(k);
            }
        }
    }
    // 面の表で選んだ面: その面を持つ玉の縁と、面の頂点の印（#125）
    if let Some(j) = view.hl_surf {
        let hl = Color32::from_rgb(0xE0, 0x6C, 0x00);
        for sd in d
            .solids
            .iter()
            .filter(|sd| sd.surfaces[0] <= j && j <= sd.surfaces[1])
        {
            let mut closed: Vec<Pos2> = sd.pts.iter().map(|&q| to(q)).collect();
            if let Some(f) = closed.first().copied() {
                closed.push(f);
            }
            painter.add(egui::Shape::line(closed, Stroke::new(2.0_f32, hl)));
        }
        if let Some(Some(v)) = d.vertices.get(j) {
            let c = to(*v);
            painter.circle_stroke(c, 4.0, Stroke::new(2.0_f32, hl));
            painter.text(
                c + egui::vec2(5.0, -5.0),
                egui::Align2::LEFT_BOTTOM,
                format!("S{j}"),
                egui::FontId::proportional(11.0),
                hl,
            );
        }
    }
    // 機構部品: 壁の断面（選べない、名前は指したとき）
    let mech_fill = Color32::from_rgba_unmultiplied(0x9A, 0x8C, 0x7A, 0x60);
    let mech_line = Color32::from_rgb(0x7A, 0x6C, 0x5A);
    for (name, quads) in &d.mechs {
        for q in quads {
            let poly: Vec<Pos2> = q.iter().map(|&p| to(p)).collect();
            painter.add(egui::Shape::convex_polygon(
                poly.clone(),
                mech_fill,
                Stroke::new(1.0_f32, mech_line),
            ));
            if pointer.is_some_and(|p| inside(&poly, p)) {
                if let Some(c) = poly.first() {
                    painter.text(
                        *c,
                        egui::Align2::LEFT_BOTTOM,
                        name,
                        egui::FontId::proportional(11.0),
                        mech_line,
                    );
                }
            }
        }
    }
    // 像面・物体面
    for (kind, _, p) in &d.plates {
        let line: Vec<Pos2> = p.iter().map(|&q| to(q)).collect();
        let c = if kind == "image" {
            vis.strong_text_color()
        } else {
            weak
        };
        painter.add(egui::Shape::line(line, Stroke::new(1.5_f32, c)));
    }
    // 絞り（面の上下に短い線）
    if let Some(j) = d.stop {
        if let Some(Some(v)) = d.vertices.get(j) {
            let c = to(*v);
            let r = (d.stop_semi.unwrap_or(combo.epd / 2.0) * s) as f32;
            // 絞りの要素なら、その光軸に垂直に
            let u = spans
                .iter()
                .position(|sp| sp.is_some_and(|sp| sp[0] <= j && j <= sp[1]))
                .and_then(|k| view.axes.get(k).copied())
                .unwrap_or(Vec2::X);
            let n = Vec2::new(u.y, -u.x);
            for sgn in [-1.0f32, 1.0] {
                painter.line_segment(
                    [c + n * sgn * r, c + n * sgn * (r + 14.0)],
                    Stroke::new(2.0_f32, weak),
                );
            }
        }
    }
    // 要素の名前（鏡・絞り・BS など玉で描かれない要素は頂点の位置に）
    view.mids = vec![None; els.len()];
    for (k, p) in els.iter().enumerate() {
        let Some(sp) = spans[k] else { continue };
        let Some(front) = vtx(sp[0]) else {
            continue;
        };
        // 名前は要素の真ん中から
        let pos = if let Some(c) = bs_center[k] {
            // BS は立方体の中心（反射側へ進むと、出る面は光軸の先にない）
            c
        } else if p.kind == ElemKind::Splitter {
            front + view.axes.get(k).copied().unwrap_or(Vec2::X) * (p.t * s) as f32 / 2.0
        } else {
            vtx(sp[1]).map_or(front, |b| front + (b - front) / 2.0)
        };
        view.mids[k] = Some(pos);
        let sel = view.sel == Some(k);
        let r = (if p.kind == ElemKind::Aperture {
            p.ap_extent() + 14.0 / s
        } else {
            p.semi
        }
        .max(1.0)
            * s) as f32;
        // 名前は光軸に垂直に、要素の外へ（折り曲げなしなら上。鏡は折れた先と重ならないよう反対側）
        let u = view.axes.get(k).copied().unwrap_or(Vec2::X);
        let n = Vec2::new(u.y, -u.x)
            * if p.kind == ElemKind::Mirror {
                -1.0
            } else {
                1.0
            };
        // 開口・遮蔽: 断面（縦）で光を遮る所を太線で
        if p.kind == ElemKind::Aperture {
            let nn = Vec2::new(u.y, -u.x);
            let px = |mm: f64| (mm * s) as f32;
            let iv = if d.xz { 0 } else { 1 };
            let dy = p.decenter[iv];
            let col = if sel { accent } else { weak };
            let segs: Vec<(f64, f64)> = match p.ap {
                ApShape::Obscuration => {
                    let (lo, hi) = (p.ap_min.max(0.0), p.semi);
                    if lo <= 0.0 {
                        vec![(dy - hi, dy + hi)]
                    } else {
                        vec![(dy + lo, dy + hi), (dy - hi, dy - lo)]
                    }
                }
                ApShape::Spider => {
                    // 中心の腕の幅と、断面の向きに沿う腕（半径 semi まで）
                    let hw = p.spider_width.max(0.0) / 2.0;
                    let mut v = vec![(dy - hw, dy + hw)];
                    let n = p.spider_arms.max(1);
                    for k in 0..n {
                        let a = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
                        let (c, s) = (a.cos(), a.sin());
                        let (along, across) = if iv == 0 { (c, s) } else { (s, c) };
                        if across.abs() < 1e-9 {
                            v.push(if along > 0.0 {
                                (dy, dy + p.semi)
                            } else {
                                (dy - p.semi, dy)
                            });
                        }
                    }
                    v
                }
                _ => {
                    let h = match p.ap {
                        ApShape::Circle => p.semi,
                        _ => p.ap_half[iv],
                    };
                    let edge = 14.0 / s;
                    let mut v = vec![(dy + h, dy + h + edge), (dy - h - edge, dy - h)];
                    // 輪帯（内径）は真ん中を遮る
                    if p.ap == ApShape::Circle && p.ap_min > 0.0 {
                        v.push((dy - p.ap_min, dy + p.ap_min));
                    }
                    v
                }
            };
            for (a, b) in &segs {
                painter.line_segment(
                    [pos + nn * px(*a), pos + nn * px(*b)],
                    Stroke::new(if sel { 3.0_f32 } else { 2.5 }, col),
                );
            }
            if let Some(q) = pointer {
                let w = q - pos;
                let t = w.dot(nn);
                if w.dot(u).abs() < 6.0
                    && segs
                        .iter()
                        .any(|(a, b)| t >= px(*a) - 4.0 && t <= px(*b) + 4.0)
                {
                    hit = Some(k);
                }
            }
        }
        // 絞りは面の線（光軸に垂直）の近くでも選べる
        if p.kind == ElemKind::Stop {
            if let Some(q) = pointer {
                let w = q - pos;
                let along_n = w.dot(n).abs();
                let along_u = w.dot(u).abs();
                if along_u < 6.0 && along_n < r + 14.0 {
                    hit = Some(k);
                }
            }
        }
        let (label_pos, align) = if n.y.abs() >= n.x.abs() {
            let y = if n.y < 0.0 {
                pos.y - r - 10.0
            } else {
                pos.y + r + 10.0
            };
            (
                Pos2::new(pos.x, y.min(rect.bottom() - 10.0).max(rect.top() + 10.0)),
                if n.y < 0.0 {
                    egui::Align2::CENTER_BOTTOM
                } else {
                    egui::Align2::CENTER_TOP
                },
            )
        } else if n.x < 0.0 {
            (
                Pos2::new(pos.x - r - 10.0, pos.y),
                egui::Align2::RIGHT_CENTER,
            )
        } else {
            (
                Pos2::new(pos.x + r + 10.0, pos.y),
                egui::Align2::LEFT_CENTER,
            )
        };
        let g = painter.text(
            label_pos,
            align,
            if p.locked {
                format!("{} [{}]", p.label(), tr("ロック"))
            } else {
                p.label()
            },
            egui::FontId::proportional(11.0),
            if sel { accent } else { weak },
        );
        if pointer.is_some_and(|q| g.expand(3.0).contains(q)) {
            hit = Some(k);
        }
        if matches!(
            p.kind,
            ElemKind::Stop | ElemKind::Mirror | ElemKind::Aperture
        ) && sel
        {
            painter.circle_stroke(pos, 6.0, Stroke::new(2.0_f32, accent));
        }
    }
    // 寸法線（要素の後の空気。最後は像面まで）
    let dim_y = inner.bottom() + 26.0;
    let mut dims: Vec<(usize, Pos2, Pos2)> = Vec::new();
    if combo.lead_stop() {
        if let (Some(j), Some(sp)) = (d.stop, spans.first().copied().flatten()) {
            if let (Some(Some(a)), Some(Some(b))) = (d.vertices.get(j), d.vertices.get(sp[0])) {
                dims.push((STOP_GAP, to(*a), to(*b)));
            }
        }
    }
    for k in 0..els.len() {
        let Some(sp) = spans[k] else { continue };
        // 後の間隔の無い要素（往路を戻る鏡）・後の要素の前の間隔が別のもの（戻って口 D へ）は描かない
        if chain[k].after.is_none() || chain.get(k + 1).is_some_and(|n| n.before != chain[k].after)
        {
            continue;
        }
        let a = d.vertices.get(sp[1]).copied().flatten();
        let b = if k + 1 < els.len() {
            spans[k + 1].and_then(|n| d.vertices.get(n[0]).copied().flatten())
        } else {
            d.image_vertex()
        };
        if let (Some(a), Some(b)) = (a, b) {
            dims.push((k, to(a), to(b)));
        }
    }
    let mut dim_hit = None;
    // 縦の寸法線を光軸からどれだけ離すか（要素の半径の最大 + 余白）
    let vr = (els.iter().map(|p| p.semi).fold(combo.epd / 2.0, f64::max) * s) as f32 + 24.0;
    // 寸法の数字の上か
    let mut num_hit = None;
    let mut label_rects: Vec<(usize, Rect)> = Vec::new();
    for (g, a, b) in &dims {
        let val = gap_get(combo, *g).unwrap_or(0.0);
        let sel = view.dim == Some(*g);
        let col = if sel { accent } else { weak };
        let st = Stroke::new(if sel { 1.5_f32 } else { 1.0 }, col);
        let thin = Stroke::new(0.5_f32, col.gamma_multiply(0.5));
        let arrow = |tip: Pos2, dir: Vec2| {
            let side = Vec2::new(-dir.y, dir.x) * 3.0;
            painter.add(egui::Shape::convex_polygon(
                vec![tip, tip + dir * 6.0 + side, tip + dir * 6.0 - side],
                col,
                Stroke::NONE,
            ));
        };
        let text = fmt_num((val * 1e3).round() / 1e3);
        let font = egui::FontId::monospace(11.0);
        let (dx, dy) = ((b.x - a.x).abs(), (b.y - a.y).abs());
        let (g_rect, seg) = if dy < 1.0 || (dx > 1.0 && dy / dx < 0.02) {
            // 横の間隔: 図の下にそろえる
            let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
            painter.line_segment([*a, Pos2::new(a.x, dim_y + 4.0)], thin);
            painter.line_segment([*b, Pos2::new(b.x, dim_y + 4.0)], thin);
            painter.line_segment([Pos2::new(x0, dim_y), Pos2::new(x1, dim_y)], st);
            if x1 - x0 > 12.0 {
                arrow(Pos2::new(x0, dim_y), Vec2::X);
                arrow(Pos2::new(x1, dim_y), -Vec2::X);
            }
            let r = painter.text(
                Pos2::new((x0 + x1) / 2.0, dim_y - 3.0),
                egui::Align2::CENTER_BOTTOM,
                text,
                font,
                col,
            );
            (r, [Pos2::new(x0, dim_y), Pos2::new(x1, dim_y)])
        } else if dx < 1.0 || dx / dy < 0.02 {
            // 縦の間隔（折り曲げの後）: 光軸の右に
            let x = a.x.max(b.x) + vr;
            let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
            painter.line_segment([*a, Pos2::new(x + 4.0, a.y)], thin);
            painter.line_segment([*b, Pos2::new(x + 4.0, b.y)], thin);
            painter.line_segment([Pos2::new(x, y0), Pos2::new(x, y1)], st);
            if y1 - y0 > 12.0 {
                arrow(Pos2::new(x, y0), Vec2::Y);
                arrow(Pos2::new(x, y1), -Vec2::Y);
            }
            let r = painter.text(
                Pos2::new(x + 5.0, (y0 + y1) / 2.0),
                egui::Align2::LEFT_CENTER,
                text,
                font,
                col,
            );
            (r, [Pos2::new(x, y0), Pos2::new(x, y1)])
        } else {
            // 斜めの間隔: 光軸に平行に、右側へずらす（横・縦への射影ではなく、光軸に沿った長さ）
            let u = (*b - *a).normalized();
            let mut n = Vec2::new(-u.y, u.x);
            if n.x < 0.0 {
                n = -n;
            }
            let (a2, b2) = (*a + n * vr, *b + n * vr);
            painter.line_segment([*a, a2 + n * 4.0], thin);
            painter.line_segment([*b, b2 + n * 4.0], thin);
            painter.line_segment([a2, b2], st);
            if (b2 - a2).length() > 12.0 {
                arrow(a2, u);
                arrow(b2, -u);
            }
            let r = painter.text(
                a2 + (b2 - a2) / 2.0 + n * 5.0,
                egui::Align2::LEFT_CENTER,
                text,
                font,
                col,
            );
            (r, [a2, b2])
        };
        label_rects.push((*g, g_rect));
        if pointer.is_some_and(|q| g_rect.expand(4.0).contains(q)) {
            num_hit = Some(*g);
        }
        // 数字の近くか、寸法線から 5 px 以内
        let near_line = |q: Pos2| {
            let (p0, p1) = (seg[0], seg[1]);
            let w = p1 - p0;
            let t = ((q - p0).dot(w) / w.length_sq().max(1e-6)).clamp(0.0, 1.0);
            (q - (p0 + w * t)).length() <= 5.0
        };
        if pointer.is_some_and(|q| g_rect.expand(3.0).contains(q) || near_line(q)) {
            dim_hit = Some(*g);
        }
    }
    // クリック・ドラッグ
    if resp.double_clicked() {
        if let Some(g) = dim_hit {
            // 寸法線のダブルクリックでも数字を書き換える
            view.start_edit(combo, g);
        } else if let Some(k) = hit {
            // 要素のダブルクリックは、その要素の面を面の表で
            view.select(k);
            view.open = Some(k);
        } else if view.nav {
            // 何も無い所のダブルクリックは全体表示
            view.zoom_log = 0.0;
            view.pan = Vec2::ZERO;
        }
    } else if resp.clicked() {
        if let Some(g) = num_hit {
            // 数字のクリックはその場で書き換える（今の数字は全選択で、打てば置き換わる）
            view.start_edit(combo, g);
        } else if let Some(g) = dim_hit {
            // 寸法線（数字の外）のクリックは選ぶだけ（スライダー・← →）
            view.dim = Some(g);
            view.sel = None;
        } else {
            view.sel = hit;
            view.dim = None;
        }
    }
    let pan_drag = view.nav && panning(ui, resp);
    if resp.drag_started_by(egui::PointerButton::Primary) && !pan_drag {
        if let Some(k) = hit {
            view.sel = Some(k);
            view.dim = None;
        }
        if let Some(p) = resp.interact_pointer_pos() {
            view.drag0 = Some((p, combo.clone()));
        }
    }
    if resp.dragged_by(egui::PointerButton::Primary) && !pan_drag {
        if let (Some(k), Some((p0, c0)), Some(p)) = (
            view.sel.filter(|&k| els.get(k).is_some_and(|p| !p.fixed())),
            view.drag0.as_ref(),
            resp.interact_pointer_pos(),
        ) {
            // 画面の動きを、要素の光軸に沿う量と垂直な量に
            let (a, n) = view.along(k, p - *p0);
            let dz = (a as f64 / s * 100.0).round() / 100.0;
            let dy = (n as f64 / s * 100.0).round() / 100.0;
            let mut c = c0.clone();
            if ui.input(|i| i.modifiers.shift) {
                // Shift: 縦は偏心
                let st = c.active_state().map(str::to_string);
                if let Some(p) = c
                    .el_mut(k)
                    .filter(|p| !matches!(p.kind, ElemKind::Mirror | ElemKind::Stop))
                {
                    view.nudge(p, st.as_deref(), dy, 0.0);
                }
            } else {
                c.shift(k, dz);
            }
            *combo = c;
        }
    }
    if resp.drag_stopped() {
        view.drag0 = None;
    }
    // その場の入力欄（Enter・Tab・外をクリックで決める、Esc でやめる）
    let mut done = None;
    if let Some((g, text, focused)) = view.edit.as_mut() {
        if let Some((_, r)) = label_rects.iter().find(|(x, _)| x == g) {
            let r =
                Rect::from_center_size(r.center(), egui::vec2(r.width().max(40.0) + 36.0, 20.0));
            let te_id = egui::Id::new(("lensbench-dim-edit", *g));
            let te = ui.put(
                r,
                egui::TextEdit::singleline(text)
                    .id(te_id)
                    .font(egui::FontId::monospace(12.0))
                    .horizontal_align(egui::Align::Center)
                    .margin(egui::Margin::symmetric(4, 2)),
            );
            if !*focused || te.gained_focus() {
                te.request_focus();
                *focused = true;
                // 全選択（打てば今の数字が置き換わる）
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                    let n = text.chars().count();
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(n),
                    )));
                    st.store(ui.ctx(), te_id);
                }
            } else if te.lost_focus() {
                let (esc, enter) =
                    ui.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter)));
                let v = text
                    .trim()
                    .replace(',', ".")
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0);
                let err_id = te_id.with("err");
                if !esc && v.is_none() {
                    // 数でない・負の値は黙って捨てず、欄を残して赤枠にする（Esc でやめる、GPN-10）
                    ui.data_mut(|d| d.insert_temp(err_id, true));
                    *focused = false;
                } else {
                    ui.data_mut(|d| d.remove::<bool>(err_id));
                    // Enter・Esc で終えたら図にフォーカスを戻す（寸法は選んだまま、← → が効く）
                    if esc || enter {
                        resp.request_focus();
                    }
                    done = Some((*g, if esc { None } else { v }));
                }
            }
            if ui
                .data(|d| d.get_temp::<bool>(te_id.with("err")))
                .unwrap_or(false)
            {
                ui.painter().rect_stroke(
                    te.rect.expand(1.0),
                    2.0,
                    egui::Stroke::new(1.5_f32, ui.visuals().error_fg_color),
                    egui::StrokeKind::Outside,
                );
                te.on_hover_text(tr("0 以上の数（Esc でやめる）"));
            }
        } else {
            done = Some((*g, None));
        }
    }
    if let Some((g, v)) = done {
        view.edit = None;
        if let Some(v) = v.filter(|v| v.is_finite() && *v >= 0.0) {
            if let Some(x) = gap_mut(combo, g) {
                *x = v;
            }
        }
    }
    if hit.is_some() || dim_hit.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let _ = Vec2::ZERO;
}

/// 点が多角形の中か
fn inside(poly: &[Pos2], p: Pos2) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            c = !c;
        }
    }
    c
}

/// 面の表で選んだ要素 k（ブロック）の設定。要素の設定と同じ値を、置き方（偏心 x・y、傾き x・y・z）まで全部出す
pub fn block_settings(ui: &mut Ui, combo: &mut Combo, k: usize) {
    let chain = combo.chain();
    let last = k + 1 == chain.len();
    let Some(step) = chain.get(k).cloned() else {
        return;
    };
    let air_gap = matches!(step.after, Some(Gap::Air(_)));
    let st = combo.active_state().map(str::to_string);
    let st = st.as_deref();
    let Some(p) = combo.get_mut(&step.loc) else {
        return;
    };
    let weak = theme::weak_text(ui.visuals());
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{}. {}", k + 1, p.label())).strong());
        ui.label(RichText::new(p.kind.label()).color(weak));
        ui.checkbox(&mut p.locked, tr("ロック")).on_hover_text(tr(
            "ロックした要素は中身（面・硝材・反転など）を変えられない。位置・後の空気は変えられる（L キーでも切り替え）",
        ));
        if p.locked {
            ui.checkbox(&mut p.lock_place, tr("位置も")).on_hover_text(tr(
                "位置（図のドラッグ・キー・偏心・傾き）と後の空気も変えられないようにする",
            ));
        }
        if !p.edits.is_empty() || !p.ops.is_empty() {
            ui.label(
                RichText::new(trf(
                    "面の表で変えた値 {} 件",
                    &[&(p.edits.len() + p.ops.len())],
                ))
                .color(ui.visuals().warn_fg_color),
            );
            if ui
                .add_enabled(!p.locked, egui::Button::new(tr("元に戻す")))
                .on_hover_text(tr(
                    "面の表で変えた値をやめる（絞り・ソルブ・ピックアップも）",
                ))
                .clicked()
            {
                p.edits.clear();
                p.ops.clear();
            }
        }
    });
    // ロックは中身（種類ごとの欄・面・反転）だけを止める。位置・後の空気は「位置も」で止める。ON/OFF はいつも変えられる
    let locked = p.locked;
    ui.add_enabled_ui(!locked, |ui| {
        ui.horizontal_wrapped(|ui| kind_fields(ui, p, st, k, weak));
        if p.kind == ElemKind::Surfaces || p.kind == ElemKind::Lens && p.surfs.len() >= 2 {
            surf_table(ui, p, k, weak);
        }
    });
    ui.add_enabled_ui(!p.fixed(), |ui| {
        if place_fields(p.kind) {
            ui.horizontal_wrapped(|ui| {
                if matches!(p.kind, ElemKind::Catalog | ElemKind::Lens | ElemKind::Zmx)
                    && !(p.kind == ElemKind::Catalog && !p.path.is_empty())
                {
                    ui.add_enabled(!locked, egui::Checkbox::new(&mut p.reversed, tr("反転")));
                }
                place_ui(ui, p, st, &[0, 1], &[0, 1, 2]);
            });
        }
        let mut air = p.air_in(st);
        if air_gap {
            ui.horizontal_wrapped(|ui| {
                num(
                    ui,
                    if last {
                        tr("像面まで")
                    } else {
                        tr("後の空気")
                    },
                    &mut air,
                    " mm",
                );
                if air != p.air_in(st) {
                    *p.air_mut(st) = air;
                }
            });
        }
    });
    // 構成への入れる・外すは「位置も」のロックでも変えられる（動かすのではないので）
    ui.horizontal_wrapped(|ui| onoff_ui(ui, p, st, weak));
    let is_bs = p.kind == ElemKind::Splitter;
    if is_bs {
        ui.add_enabled_ui(!locked, |ui| bs_fields(ui, combo, &step.loc, weak));
    }
    // 要素の群の役割・移動もここで（レイアウトのパネルの「群」と同じ、#136）
    let g = combo
        .get(&step.loc)
        .map(|p| p.group.trim().to_string())
        .unwrap_or_default();
    if !g.is_empty() {
        ui.horizontal_wrapped(|ui| groups_ui(ui, combo, weak, Some(&g)));
    }
}

const ROLES: [(&str, &str); 4] = [
    ("", "役割なし"),
    ("zoom", "ズーム"),
    ("focus", "フォーカス"),
    ("fixed", "固定"),
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
    ui.label(RichText::new(tr("玉の群")).color(weak))
        .on_hover_text(tr(
        ".zmx の面 from〜to をまとめて 1 つの群（玉・群のパラメータ）にする。無ければ玉ごとに自動",
    ));
    let mut remove = None;
    for (i, g) in p.zgroups.iter_mut().enumerate() {
        ui.add(
            egui::DragValue::new(&mut g.from)
                .range(1..=9999)
                .prefix(tr("面 ")),
        );
        ui.label("〜");
        ui.add(egui::DragValue::new(&mut g.to).range(1..=9999));
        ui.add(
            egui::TextEdit::singleline(&mut g.name)
                .hint_text(tr("名前"))
                .desired_width(50.0),
        );
        role_combo(ui, ("zg_role", p.path.as_str(), i), &mut g.role);
        if ui.small_button("✖").on_hover_text(tr("外す")).clicked() {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        p.zgroups.remove(i);
    }
    if ui.small_button(tr("＋ 群")).clicked() {
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
    ui.label(RichText::new(tr("群")).small().color(weak));
    for n in names {
        if ids.contains(&n) {
            ui.label(
                RichText::new(trf("{} は要素の名前と同じ（群にしない）", &[&n]))
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
                .prefix(tr("移動 "))
                .suffix(" mm"),
        )
        .on_hover_text(tr("光の進む向きが正。前の間隔に足し、後の間隔から引く"));
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
    ui.label(RichText::new(tr("状態")).small().color(weak));
    let cur = combo.active_state().map(str::to_string);
    egui::ComboBox::from_id_salt("combo_state")
        .selected_text(cur.clone().unwrap_or_else(|| tr("基本").into()))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut combo.state, None, tr("基本"));
            for n in combo.states.clone() {
                ui.selectable_value(&mut combo.state, Some(n.clone()), n);
            }
        })
        .response
        .on_hover_text(tr(
            "状態ごとに要素の ON/OFF・後の空気・偏心・傾き、BS の経路、系の設定（開口・視野・波長・物体距離など）を変えられる（測定系の出し入れ・ズームなど）。基本は状態を選ばない系",
        ));
    let r = ui.add(
        egui::TextEdit::singleline(&mut view.new_state)
            .hint_text(tr("状態の名前"))
            .desired_width(90.0),
    );
    let name = view.new_state.trim().to_string();
    let ok = !name.is_empty() && !combo.states.contains(&name);
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    if (ui
        .add_enabled(ok, egui::Button::new(tr("＋ 状態")))
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
            .on_hover_text(trf("状態 {} を消す", &[&n]))
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
                .on_hover_text(tr("OFF の要素を選ぶ（下の設定で ON に戻せる）"))
                .clicked()
            {
                view.sel = Some(k);
                view.dim = None;
            }
        }
    }
}

/// 要素の ON/OFF・OFF の残し方・鏡の往路を戻る。状態 st を選んでいれば ON/OFF と後の空気はその状態の値
pub fn onoff_ui(ui: &mut Ui, p: &mut Pick, st: Option<&str>, weak: Color32) {
    if p.kind.has_on() {
        let mut on = p.on_in(st);
        let hint = match st {
            Some(n) => trf("状態 {} での ON/OFF（OFF の要素は系に入らない）", &[&n]),
            None => tr("OFF の要素は系に入らない（光学系の出し入れ）").to_string(),
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
            egui::ComboBox::from_id_salt(("off_mode", p as *const Pick as usize))
                .selected_text(if p.off_air {
                    tr("OFF は空気で残す")
                } else {
                    tr("OFF は詰める")
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.off_air, false, tr("OFF は詰める"));
                    ui.selectable_value(&mut p.off_air, true, tr("OFF は空気で残す"));
                });
        }
    }
    // 群
    ui.label(RichText::new(tr("群")).color(weak));
    ui.add(
        egui::TextEdit::singleline(&mut p.group)
            .hint_text(tr("なし"))
            .desired_width(60.0),
    )
    .on_hover_text(tr(
        "続く要素に同じ名前を付けると 1 つの群になる（役割・移動はこの下か、レイアウトのパネルの「群」で）",
    ));
    if p.kind == ElemKind::ZmxRange {
        zgroups_ui(ui, p, weak);
    }
    if let Some(n) = st {
        let own = p.has_state_values(n);
        if own {
            ui.label(
                RichText::new(trf("状態 {} の値", &[&n]))
                    .small()
                    .color(weak),
            );
            if ui
                .small_button(tr("基本の値に"))
                .on_hover_text(tr("この状態で変えた ON/OFF・後の空気・偏心・傾きをやめる"))
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
    const DEC: [&str; 2] = ["偏心 x", "偏心 y"];
    const TILT: [&str; 3] = ["傾き x", "傾き y", "傾き z"];
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
fn kind_fields(ui: &mut Ui, p: &mut Pick, st: Option<&str>, k: usize, weak: Color32) {
    match p.kind {
        ElemKind::Lens if p.surfs.len() >= 2 => {
            ui.label(RichText::new(trf("{} 面（下の表）", &[&p.surfs.len()])).color(weak));
        }
        ElemKind::Lens => {
            num(ui, "R1", &mut p.r1, " mm");
            num(ui, "R2", &mut p.r2, " mm");
            num_in(ui, tr("厚さ"), &mut p.t, " mm", 0.0);
            ui.label(RichText::new(tr("ガラス")).color(weak));
            ui.add(egui::TextEdit::singleline(&mut p.glass).desired_width(70.0));
            semi_edit(ui, &mut p.semi, &mut p.semi_auto);
            if ui
                .button(tr("＋ 面"))
                .on_hover_text(tr("面を足して貼り合わせ・3 枚玉などにする（面の表で編集）"))
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
            ui.label(RichText::new(trf("{} 面（下の表）", &[&p.surfs.len()])).color(weak));
        }
        ElemKind::ZmxRange => {
            ui.label(RichText::new(tr("面")).color(weak));
            ui.add(egui::DragValue::new(&mut p.range[0]).range(0..=9999))
                .on_hover_text(tr("取り込む最初の面（0 は面 1）"));
            ui.label("〜");
            ui.add(egui::DragValue::new(&mut p.range[1]).range(0..=9999))
                .on_hover_text(tr("取り込む最後の面（0 は像面の手前）"));
            ui.label(RichText::new(tr("構成")).color(weak));
            ui.add(egui::DragValue::new(&mut p.config).range(1..=999))
                .on_hover_text(tr(".zmx のマルチコンフィグの番号"));
        }
        ElemKind::Plate => {
            num_in(ui, tr("厚さ"), &mut p.t, " mm", 0.0);
            ui.label(RichText::new(tr("ガラス")).color(weak));
            ui.add(egui::TextEdit::singleline(&mut p.glass).desired_width(70.0));
            semi_edit(ui, &mut p.semi, &mut p.semi_auto);
        }
        ElemKind::Mirror => {
            num(ui, "R", &mut p.r1, " mm");
            place_ui(ui, p, st, &[], &[0, 1]);
            num_in(ui, tr("半径"), &mut p.semi, " mm", 0.001);
            coat_edit(ui, &mut p.coat, weak);
            ui.checkbox(&mut p.back, tr("往路を戻る")).on_hover_text(tr(
                "鏡で反射した光が来た道を戻る（ダブルパス・干渉計の腕の端）。BS の分岐の中なら、戻った光は BS で分かれる",
            ));
        }
        ElemKind::Stop => num_in(ui, tr("半径"), &mut p.semi, " mm", 0.001),
        ElemKind::Aperture => {
            egui::ComboBox::from_id_salt(("ap_shape", k))
                .selected_text(p.ap.label())
                .show_ui(ui, |ui| {
                    for a in [
                        ApShape::Circle,
                        ApShape::Rect,
                        ApShape::Ellipse,
                        ApShape::Obscuration,
                        ApShape::Spider,
                    ] {
                        ui.selectable_value(&mut p.ap, a, a.label());
                    }
                });
            match p.ap {
                ApShape::Circle | ApShape::Obscuration => {
                    num_in(ui, tr("内側の半径"), &mut p.ap_min, " mm", 0.0);
                    num_in(ui, tr("外側の半径"), &mut p.semi, " mm", 0.001);
                }
                ApShape::Rect | ApShape::Ellipse => {
                    num_in(ui, tr("半幅 x"), &mut p.ap_half[0], " mm", 0.001);
                    num_in(ui, tr("半幅 y"), &mut p.ap_half[1], " mm", 0.001);
                }
                ApShape::Spider => {
                    num_in(ui, tr("腕の幅（全幅）"), &mut p.spider_width, " mm", 0.0);
                    let mut n = p.spider_arms as f64;
                    num(ui, tr("腕の本数"), &mut n, "");
                    p.spider_arms = n.round().clamp(1.0, 360.0) as u32;
                    // 腕は無限に伸びる。図ではこの半径まで描く
                    num_in(ui, tr("図の半径"), &mut p.semi, " mm", 0.001);
                }
            }
            num(ui, tr("偏心 x"), &mut p.decenter[0], " mm");
            num(ui, tr("偏心 y"), &mut p.decenter[1], " mm");
        }
        ElemKind::Splitter => {
            let was = p.form;
            egui::ComboBox::from_id_salt(("bs_form", k))
                .selected_text(p.form.label())
                .show_ui(ui, |ui| {
                    for f in [BsForm::Cube, BsForm::Plate, BsForm::Ideal] {
                        ui.selectable_value(&mut p.form, f, f.label());
                    }
                });
            if p.form != was {
                // 形を変えたら寸法を形の初めの値に
                match p.form {
                    BsForm::Cube => p.t = 12.7,
                    BsForm::Plate => p.t = 1.0,
                    BsForm::Ideal => {}
                }
            }
            match p.form {
                BsForm::Cube => num_in(ui, tr("一辺"), &mut p.t, " mm", 0.001),
                BsForm::Plate => num_in(ui, tr("厚さ"), &mut p.t, " mm", 0.0),
                BsForm::Ideal => {}
            }
            if p.form != BsForm::Ideal {
                ui.label(RichText::new(tr("ガラス")).color(weak));
                ui.add(egui::TextEdit::singleline(&mut p.glass).desired_width(70.0));
            }
            if p.form != BsForm::Cube {
                num_in(ui, tr("半径"), &mut p.semi, " mm", 0.001);
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
                .on_hover_text(tr("分ける面: 偏光によらない分岐比・膜・理想の偏光 BS"));
            p.split = Some(mode);
            match mode {
                BsSplit::Ratio => {
                    ui.add(
                        egui::DragValue::new(&mut p.ratio)
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .prefix("T "),
                    );
                }
                BsSplit::Coat => coat_edit(ui, &mut p.coat, weak),
                BsSplit::Pbs => {
                    ui.add(
                        egui::DragValue::new(&mut p.pbs[0])
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .prefix("Tp "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut p.pbs[1])
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .prefix("Ts "),
                    );
                }
            }
            ui.checkbox(&mut p.arm_r, tr("反射側へ進む"))
                .on_hover_text(tr("経路の文字が足りないときに進む口（既定は透過）"));
            ui.checkbox(&mut p.bs_flip, tr("反射を逆向きに"))
                .on_hover_text(tr("分ける面の向きを入れ替え、反射を反対側へ出す（F キー）"));
        }
        ElemKind::Zmx => {
            ui.label(RichText::new(tr("何番目の玉")).color(weak));
            ui.add(egui::DragValue::new(&mut p.lens).range(1..=999));
            ui.label(RichText::new(tr("構成")).color(weak));
            ui.add(egui::DragValue::new(&mut p.config).range(1..=999))
                .on_hover_text(tr(".zmx のマルチコンフィグの何番目から玉を取るか"));
        }
        ElemKind::Catalog => {}
    }
    if p.kind == ElemKind::ZmxRange {
        if p.range_has_stop() {
            ui.checkbox(&mut p.stop_front, tr(".zmx の絞りを使う")).on_hover_text(tr(
                "取り込む面の絞りを系の絞りにする。外すと、絞りは自動の絞り・絞りの要素になる（絞りは系に 1 つだけ）",
            ));
        } else {
            ui.label(RichText::new(tr("範囲に .zmx の絞りの面が無い")).color(weak));
        }
    } else if p.can_stop_front() {
        ui.checkbox(&mut p.stop_front, tr("前面を絞りに"))
            .on_hover_text(tr("玉の前の面を絞りにする（絞りは系に 1 つだけ）"));
    }
}

/// 膜の名前の欄（鏡・BS の分ける面）
fn coat_edit(ui: &mut Ui, coat: &mut String, weak: Color32) {
    ui.label(RichText::new(tr("膜")).color(weak));
    ui.add(
        egui::TextEdit::singleline(coat)
            .hint_text(tr("膜なし"))
            .desired_width(70.0),
    )
    .on_hover_text(tr(
        "膜ファイル（COATING.DAT など）・膜モデルの膜の名前、系の設定で定義した膜の名前。空なら理想の鏡・分岐比のまま",
    ));
    // 系の設定で定義した膜から選ぶ
    let names = crate::coat_defs::names(ui.ctx());
    if !names.is_empty() {
        egui::ComboBox::from_id_salt(("coat_pick", coat as *const String as usize))
            .selected_text("▼")
            .width(24.0)
            .show_ui(ui, |ui| {
                for n in names {
                    if ui.selectable_label(*coat == n, n.as_str()).clicked() {
                        *coat = n;
                    }
                }
            })
            .response
            .on_hover_text(tr("定義した膜から選ぶ"));
    }
}

/// 選んだ寸法（入力欄とスライダー）・要素の設定。面の表で見る要素を返す
fn editor(ui: &mut Ui, combo: &mut Combo, view: &mut View) -> Option<usize> {
    let weak = theme::weak_text(ui.visuals());
    ui.add_space(4.0);
    let els = combo.chain_picks();
    let n = els.len();
    if let Some(g) = view.dim {
        if g != STOP_GAP && g >= n {
            view.dim = None;
            return None;
        }
        let name = if g == STOP_GAP {
            tr("絞り → 最初の要素").to_string()
        } else if g + 1 == n {
            trf("{} → 像面", &[&els[g].label()])
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
            ui.label(RichText::new(tr("間隔")).color(weak));
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
            RichText::new(tr("← → でも動かせます（Ctrl で 1 mm、Alt で 0.01 mm）"))
                .small()
                .color(weak),
        );
        return None;
    }
    let Some(k) = view.sel.filter(|&k| k < n) else {
        ui.label(
            RichText::new(tr("図の要素をクリックで選び、横にドラッグで光軸に沿って動かす（Shift で縦に偏心）。寸法の数字をクリック（または寸法を選んで Enter）でその場で入力、寸法線のクリックは選ぶだけ。キーは [ ] で選ぶ、← → で動かす、↑ ↓ で偏心、Shift+↑ ↓ で傾き、R で反転、F で BS の反射を逆向きに、Delete で外す、L でロック。"))
                .small()
                .color(weak),
        );
        return None;
    };
    let chain = combo.chain();
    let step = chain[k].clone();
    let where_ = combo.where_label(&step.loc);
    let can_prev = combo.can_swap(k, -1);
    let can_next = combo.can_swap(k, 1);
    let mut remove = false;
    let mut swap: Option<i32> = None;
    let mut open = None;
    let st = combo.active_state().map(str::to_string);
    let st = st.as_deref();
    let air_gap = matches!(step.after, Some(Gap::Air(_)));
    let p = combo.get_mut(&step.loc)?;
    let locked = p.locked;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{}. {}", k + 1, p.label())).strong());
        if !where_.is_empty() {
            ui.label(RichText::new(trf("（{} の先）", &[&where_])).color(weak))
                .on_hover_text(tr("BS の口の先の並びの要素"));
        }
        if ui
            .button(tr("面の表"))
            .on_hover_text(tr(
                "この要素の面をレンズデータの表で見る・変える（図の要素のダブルクリックでも）",
            ))
            .clicked()
        {
            open = Some(k);
        }
        if ui
            .button(tr("📚 カタログに保存"))
            .on_hover_text(tr("この要素（か範囲）をユーザーカタログの項目にする"))
            .clicked()
        {
            view.save = Some(k);
        }
        if !p.edits.is_empty() || !p.ops.is_empty() {
            ui.label(
                RichText::new(trf(
                    "面の表で変えた値 {} 件",
                    &[&(p.edits.len() + p.ops.len())],
                ))
                .color(ui.visuals().warn_fg_color),
            )
            .on_hover_text(
                p.edits
                    .iter()
                    .map(|e| format!("{} {} = {}", e.surface + 1, e.field, e.value))
                    .chain(p.ops.iter().map(|o| o.to_string()))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            if ui
                .add_enabled(!p.locked, egui::Button::new(tr("元に戻す")))
                .on_hover_text(tr(
                    "面の表で変えた値をやめる（絞り・ソルブ・ピックアップも）",
                ))
                .clicked()
            {
                p.edits.clear();
                p.ops.clear();
            }
        }
        ui.checkbox(&mut p.locked, tr("ロック")).on_hover_text(tr(
            "ロックした要素は中身（面・硝材・反転など）を変えられない。位置・後の空気は変えられる（L キーでも切り替え）",
        ));
        if p.locked {
            ui.checkbox(&mut p.lock_place, tr("位置も")).on_hover_text(tr(
                "位置（図のドラッグ・キー・偏心・傾き）と後の空気も変えられないようにする",
            ));
        }
        ui.add_enabled_ui(!p.locked, |ui| {
            kind_fields(ui, p, st, k, weak);
            if matches!(p.kind, ElemKind::Catalog | ElemKind::Lens | ElemKind::Zmx) {
                ui.checkbox(&mut p.reversed, tr("反転"));
            }
        });
        ui.add_enabled_ui(!p.fixed(), |ui| {
            if !matches!(
                p.kind,
                ElemKind::Mirror | ElemKind::Stop | ElemKind::Aperture
            ) {
                place_ui(ui, p, st, &[1], &[0]);
            }
            let mut air = p.air_in(st);
            if air_gap {
                num(
                    ui,
                    if k + 1 == n {
                        tr("像面まで")
                    } else {
                        tr("後の空気")
                    },
                    &mut air,
                    " mm",
                );
                if air != p.air_in(st) {
                    *p.air_mut(st) = air;
                }
            }
        });
        onoff_ui(ui, p, st, weak);
        if ui
            .add_enabled(can_prev, egui::Button::new("◀"))
            .on_hover_text(tr("前へ"))
            .clicked()
        {
            swap = Some(-1);
        }
        if ui
            .add_enabled(can_next, egui::Button::new("▶"))
            .on_hover_text(tr("後へ"))
            .clicked()
        {
            swap = Some(1);
        }
        if ui.button("✖").on_hover_text(tr("外す")).clicked() {
            remove = true;
        }
    });
    if combo
        .get(&step.loc)
        .is_some_and(|p| p.kind == ElemKind::Splitter)
    {
        ui.add_enabled_ui(!locked, |ui| bs_fields(ui, combo, &step.loc, weak));
    }
    if let Some(p) = combo
        .get_mut(&step.loc)
        .filter(|p| p.kind == ElemKind::Surfaces || p.kind == ElemKind::Lens && p.surfs.len() >= 2)
    {
        let locked = p.locked;
        ui.add_enabled_ui(!locked, |ui| surf_table(ui, p, k, weak));
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
        if p.form != BsForm::Cube {
            num(ui, tr("分ける面の傾き x"), &mut p.bs_angle[0], "°");
            num(ui, tr("傾き y"), &mut p.bs_angle[1], "°");
        }
        if p.form == BsForm::Plate {
            num(ui, tr("くさび x"), &mut p.wedge[0], "°");
            num(ui, tr("くさび y"), &mut p.wedge[1], "°");
            ui.checkbox(&mut p.split_back, tr("裏の面で分ける"));
        }
        if p.form != BsForm::Ideal {
            ui.label(RichText::new(tr("ほかの面の膜")).color(weak));
            ui.add(
                egui::TextEdit::singleline(&mut p.ar)
                    .hint_text(tr("膜なし"))
                    .desired_width(70.0),
            )
            .on_hover_text(tr("分ける面以外の面の膜の既定（面ごとの膜が無い面）"));
            for f in p.form.faces() {
                let name = match *f {
                    "front" => tr("表"),
                    "back" => tr("裏"),
                    x => x,
                };
                ui.label(RichText::new(trf("面 {}", &[&name])).color(weak));
                let mut v = p.faces.get(*f).cloned().unwrap_or_default();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut v)
                            .hint_text(tr("既定"))
                            .desired_width(56.0),
                    )
                    .on_hover_text(if p.form == BsForm::Cube {
                        tr("キューブの面 a（A の口）・b（B）・c（C）・d（D）の膜。空ならほかの面の膜")
                    } else {
                        tr("平板の表・裏の膜（分ける面には付かない）。空ならほかの面の膜")
                    })
                    .changed()
                {
                    if v.trim().is_empty() {
                        p.faces.remove(*f);
                    } else {
                        p.faces.insert(f.to_string(), v);
                    }
                }
            }
        }
    });
    // 分岐
    let mut branched = p.branched;
    ui.horizontal_wrapped(|ui| {
        if ui
            .checkbox(&mut branched, tr("口の先に並べる（分岐）"))
            .on_hover_text(tr("口 B（透過）・C（反射）・D（B から戻る光の反射）の先に、それぞれ要素を並べる。干渉計の腕など。BS は並びの最後になり、後ろの要素は今の口の先へ移る"))
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
        tr("口 B（透過）の先"),
        tr("口 C（反射）の先"),
        tr("口 D（戻りの反射）の先"),
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
                    .prefix(format!("{} ", tr("空気")))
                    .suffix(" mm"),
            )
            .on_hover_text(tr("口から最初の要素まで（要素が無ければ像面まで）"));
            for (j, e) in b.picks.iter().enumerate() {
                ui.label("→");
                let mut l = e.label();
                if e.kind == ElemKind::Mirror && e.back {
                    l = trf("{}（戻る）", &[&l]);
                }
                if ui
                    .small_button(format!("{l} ✖"))
                    .on_hover_text(tr("外す"))
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
                "この口の先の最後に要素を足す（図で選んで設定するには、経路をこの口へ）",
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

/// 自作の玉（3 面以上）・面の並びの面の表。自作の玉は面だけ（ガラスは面の後）、面の並びは種類を選べる
fn surf_table(ui: &mut Ui, p: &mut Pick, k: usize, weak: Color32) {
    let lens = p.kind == ElemKind::Lens;
    let n = p.surfs.len();
    let (mut del, mut add) = (None, None);
    let drag = |ui: &mut Ui, v: &mut f64| {
        ui.add(egui::DragValue::new(v).speed(0.05).max_decimals(6));
    };
    egui::Grid::new(("surf_table", k))
        .num_columns(8)
        .spacing([6.0, 2.0])
        .striped(true)
        .show(ui, |ui| {
            for h in [
                "#",
                if lens { "" } else { tr("種類") },
                "R",
                tr("コーニック"),
                tr("厚さ"),
                tr("ガラス"),
                tr("半径"),
                "",
            ] {
                ui.label(RichText::new(h).small().color(weak));
            }
            ui.end_row();
            for (i, r) in p.surfs.iter_mut().enumerate() {
                ui.label((i + 1).to_string());
                if lens {
                    ui.label("");
                } else {
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
                }
                let last_lens = lens && i + 1 == n;
                match r.kind {
                    SurfKind::Standard => {
                        drag(ui, &mut r.radius);
                        drag(ui, &mut r.conic);
                        if last_lens {
                            ui.label(RichText::new(tr("（後の空気）")).small().color(weak));
                            ui.label("");
                        } else {
                            drag(ui, &mut r.t);
                            ui.add(
                                egui::TextEdit::singleline(&mut r.glass)
                                    .desired_width(70.0)
                                    .hint_text(tr("空気")),
                            );
                        }
                        ui.horizontal(|ui| {
                            ui.add_enabled(!r.semi_auto, egui::DragValue::new(&mut r.semi).speed(0.1));
                            ui.checkbox(&mut r.semi_auto, tr("自動"))
                                .on_hover_text(tr("半径を全視野・全波長の光線から求める"));
                        });
                    }
                    SurfKind::Jones => {
                        ui.horizontal(|ui| {
                            for (j, l) in ["A", "B", "C", "D"].into_iter().enumerate() {
                                ui.label(RichText::new(l).small().color(weak));
                                ui.add(
                                    egui::DragValue::new(&mut r.jones[2 * j])
                                        .speed(0.01)
                                        .max_decimals(6),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut r.jones[2 * j + 1])
                                        .speed(0.01)
                                        .max_decimals(6)
                                        .suffix("i"),
                                );
                            }
                        });
                        ui.label("");
                        drag(ui, &mut r.t);
                        ui.label("");
                        drag(ui, &mut r.semi);
                    }
                    SurfKind::Polarizer => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tr("透過軸")).small().color(weak));
                            ui.add(egui::DragValue::new(&mut r.axis_deg).speed(1.0).suffix("°"));
                            ui.label(RichText::new(tr("消光比")).small().color(weak));
                            ui.add(
                                egui::DragValue::new(&mut r.extinction_ratio)
                                    .speed(10.0)
                                    .range(0.0..=f64::MAX),
                            )
                            .on_hover_text(tr("0 は無限（直交の向きを通さない）"));
                            ui.label(RichText::new(tr("透過率")).small().color(weak));
                            ui.add(
                                egui::DragValue::new(&mut r.transmittance)
                                    .speed(0.01)
                                    .range(0.0..=1.0),
                            );
                        });
                        ui.label("");
                        drag(ui, &mut r.t);
                        ui.label("");
                        drag(ui, &mut r.semi);
                    }
                    SurfKind::Waveplate => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tr("遅延")).small().color(weak));
                            ui.add(
                                egui::DragValue::new(&mut r.retardance)
                                    .speed(0.01)
                                    .suffix(" λ"),
                            );
                            ui.label(RichText::new(tr("設計波長")).small().color(weak));
                            ui.add(
                                egui::DragValue::new(&mut r.wavelength_um)
                                    .speed(0.001)
                                    .range(0.01..=100.0)
                                    .suffix(" µm"),
                            );
                            ui.label(RichText::new(tr("速い軸")).small().color(weak));
                            ui.add(egui::DragValue::new(&mut r.axis_deg).speed(1.0).suffix("°"));
                            ui.label(RichText::new(tr("次数")).small().color(weak));
                            ui.add(egui::DragValue::new(&mut r.wp_order).speed(0.1));
                            ui.label(RichText::new(tr("複屈折材")).small().color(weak))
                                .on_hover_text(tr("常光はこの名前、異常光は名前-E のガラス（空なら理想の遅延）。2 つ目があれば速い軸を直交させた 2 材の組"));
                            ui.add(egui::TextEdit::singleline(&mut r.wp_material).desired_width(70.0));
                            ui.add(
                                egui::DragValue::new(&mut r.wp_thickness)
                                    .speed(0.001)
                                    .range(0.0..=100.0)
                                    .suffix(" mm"),
                            )
                            .on_hover_text(tr("0 は設計波長の遅延から決める（1 材のとき）"));
                            ui.label("+");
                            ui.add(egui::TextEdit::singleline(&mut r.wp_material2).desired_width(70.0));
                            ui.add(
                                egui::DragValue::new(&mut r.wp_thickness2)
                                    .speed(0.001)
                                    .range(0.0..=100.0)
                                    .suffix(" mm"),
                            );
                        });
                        ui.label("");
                        drag(ui, &mut r.t);
                        ui.label("");
                        drag(ui, &mut r.semi);
                    }
                    SurfKind::Coordbrk => {
                        ui.horizontal(|ui| {
                            for (j, l) in ["dx", "dy", "tx", "ty", "tz"].into_iter().enumerate() {
                                ui.label(RichText::new(l).small().color(weak));
                                drag(ui, &mut r.cb[j]);
                            }
                            let mut tf = r.order == 1;
                            if ui.checkbox(&mut tf, tr("傾けてから偏心")).changed() {
                                r.order = tf as u8;
                            }
                        });
                        ui.label("");
                        drag(ui, &mut r.t);
                        ui.label("");
                        ui.label("");
                    }
                }
                ui.horizontal(|ui| {
                    if ui
                        .small_button("＋")
                        .on_hover_text(tr("後に面を足す"))
                        .clicked()
                    {
                        add = Some(i + 1);
                    }
                    let min = if lens { 2 } else { 1 };
                    if ui
                        .add_enabled(n > min, egui::Button::new("✖").small())
                        .on_hover_text(tr("面を消す"))
                        .clicked()
                    {
                        del = Some(i);
                    }
                });
                ui.end_row();
            }
        });
    if let Some(i) = add {
        let semi = p.surfs.get(i.saturating_sub(1)).map_or(12.7, |r| r.semi);
        p.surfs.insert(
            i,
            SurfRow {
                semi,
                ..Default::default()
            },
        );
        // 自作の玉の最後に足したら、前の最後の面の後はガラスにする
        if lens && i + 1 == p.surfs.len() {
            let prev = &mut p.surfs[i - 1];
            if prev.glass.trim().is_empty() {
                prev.glass = "N-BK7".into();
                prev.t = 2.0;
            }
        }
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
    ui.add_enabled_ui(!*auto, |ui| num_in(ui, tr("半径"), semi, " mm", 0.001));
    ui.checkbox(auto, tr("自動"))
        .on_hover_text(tr("半径を全視野・全波長の光線から求める"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ポータブル保存の相対パス: 機構部品のカタログのパスも写す
    #[test]
    fn map_paths_covers_mechanics() {
        let mut c = Combo {
            mechanics: vec![MechPick {
                catalog: "/a/items/tl.json".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        c.map_paths(&|p: &str| p.replace("/a/", ""));
        assert_eq!(c.mechanics[0].catalog, "items/tl.json");
    }

    #[test]
    fn ids_and_blocks() {
        let mut c = Combo::default();
        let mut cat = Pick::new(ElemKind::Catalog);
        cat.vendor = "LB".into();
        cat.part = "PCX-100".into();
        c.picks = vec![
            cat.clone(),
            cat,
            Pick::new(ElemKind::Lens),
            Pick::new(ElemKind::Mirror),
            Pick::new(ElemKind::Lens),
            Pick::new(ElemKind::Splitter),
        ];
        assert_eq!(
            c.ids(),
            vec!["PCX-100", "PCX-100#2", "L1", "M1", "L2", "BS1"]
        );
        let p = c.project();
        let b = p["blocks"].as_array().unwrap();
        // 前の絞り・空気 + 要素ごとにブロックと空気
        assert_eq!(b.len(), 2 + 2 * 6);
        assert_eq!(b[6]["source"]["surfaces"][0]["glass"], "N-BK7");
        assert_eq!(b[8]["type"], "mirror");
        assert_eq!(b[8]["tilt"], json!([45.0, 0.0]));
        assert_eq!(b[12]["type"], "splitter");
        assert_eq!(b[12]["angle"], json!([45.0, 0.0]));
        c.picks[5].bs_flip = true;
        assert_eq!(c.project()["blocks"][12]["angle"], json!([-45.0, 0.0]));
        // 絞りの要素があれば前の絞りは置かない
        c.picks.insert(0, Pick::new(ElemKind::Stop));
        assert_eq!(c.project()["blocks"][0]["type"], "stop");
        assert_eq!(c.project()["blocks"][0]["id"], "STO1");
    }

    /// 間に差し込むと前の空気を半分ずつ分け、後ろに足すときは分けない
    #[test]
    fn insert_pick_splits_air() {
        let mut c = Combo {
            picks: vec![Pick::new(ElemKind::Lens), Pick::new(ElemKind::Lens)],
            ..Default::default()
        };
        c.picks[0].air = 10.0;
        let mut p = Pick::new(ElemKind::Plate);
        p.air = 7.0;
        assert_eq!(insert_pick(&mut c, 1, p.clone()), 1);
        assert_eq!((c.picks[0].air, c.picks[1].air), (5.0, 5.0));
        assert_eq!(insert_pick(&mut c, 99, p), 3);
        assert_eq!(c.picks[3].air, 7.0);
    }

    /// 矢印キー: 光軸が斜め 45° でも ← → だけが光軸に沿い、↑ ↓ は偏心
    #[test]
    fn arrow_keys_follow_axis() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let v = View {
            axes: vec![Vec2::new(s, -s), Vec2::new(0.0, -1.0)],
            ..Default::default()
        };
        assert_eq!(v.along_key(0, Vec2::X), (1.0, 0.0));
        assert_eq!(v.along_key(0, -Vec2::X), (-1.0, 0.0));
        assert_eq!(v.along_key(0, -Vec2::Y), (0.0, 1.0));
        // 上向きの光軸は ↑ で進む、← → は偏心
        assert_eq!(v.along_key(1, -Vec2::Y), (1.0, 0.0));
        assert_eq!(v.along_key(1, Vec2::X).0, 0.0);
    }

    #[test]
    fn old_pick_is_read() {
        let p: Pick =
            serde_json::from_str(r#"{"vendor":"LB","part":"X","reversed":true,"air":3.0}"#)
                .unwrap();
        assert_eq!(p.kind, ElemKind::Catalog);
        assert!(p.reversed);
        assert_eq!(p.air, 3.0);
    }

    #[test]
    fn focus_distance() {
        // 軸から 1 の高さで、傾き -0.1 の光線は 10 先で軸に届く
        let t = paraxial_focus(1.0, 0.0, -0.1, (1.0f64 - 0.01).sqrt()).unwrap();
        assert!((t - 9.9498743710662).abs() < 1e-9, "{t}");
        assert!(paraxial_focus(1.0, 0.0, 0.0, 1.0).is_none());
    }

    #[test]
    fn shift_keeps_total() {
        let mut c = Combo {
            picks: vec![Pick::new(ElemKind::Lens), Pick::new(ElemKind::Lens)],
            ..Default::default()
        };
        c.picks[0].air = 10.0;
        c.picks[1].air = 20.0;
        c.shift(1, 2.0);
        assert_eq!((c.picks[0].air, c.picks[1].air), (12.0, 18.0));
        c.shift(0, 1.0);
        assert_eq!((c.stop_gap, c.picks[0].air), (3.0, 11.0));
    }

    #[test]
    fn point_in_polygon() {
        let sq = [
            Pos2::new(0.0, 0.0),
            Pos2::new(1.0, 0.0),
            Pos2::new(1.0, 1.0),
            Pos2::new(0.0, 1.0),
        ];
        assert!(inside(&sq, Pos2::new(0.5, 0.5)));
        assert!(!inside(&sq, Pos2::new(1.5, 0.5)));
    }

    #[test]
    fn surface_edits_follow_the_element() {
        let mut c = Combo::default();
        let mut a = Pick::new(ElemKind::Lens);
        a.set_edit(1, "r", json!(-40.0));
        a.set_edit(1, "r", json!(-30.0));
        a.set_edit(0, "semi", json!(8.0));
        c.picks = vec![Pick::new(ElemKind::Lens), a];
        let v = c.project();
        assert_eq!(
            v["surface_edits"],
            json!([
                {"block": "L2", "surface": 1, "fields": {"r": -30.0}},
                {"block": "L2", "surface": 0, "fields": {"semi": 8.0}},
            ])
        );
        // 前に要素を足して id が変わっても、上書きは同じ要素に付いて行く
        c.picks.insert(0, Pick::new(ElemKind::Lens));
        assert_eq!(c.project()["surface_edits"][0]["block"], "L3");
        // 上書きが無ければ書かない
        c.picks[2].edits.clear();
        assert!(c.project().get("surface_edits").is_none());
    }
    /// 面の数・順・元が変わったら、面番号の上書き（edits・ops）を外す。値だけの変更では残す（GPN-02）
    #[test]
    fn stale_surface_edits_are_dropped() {
        let mut c = Combo::default();
        let mut a = Pick::new(ElemKind::Surfaces);
        a.surfs = vec![SurfRow::default(), SurfRow::default(), SurfRow::default()];
        a.set_edit(1, "r", json!(-40.0));
        a.ops.push(json!({ "op": "set_stop", "surface": 2 }));
        c.picks = vec![a];
        // 印の無い上書き（前の保存）は今の並びのものとみなす
        assert_eq!(c.drop_stale_edits(), 0);
        assert!(!c.picks[0].edits_for.is_empty());
        // 値だけ変える（面の数・種類は同じ）: 残す
        c.picks[0].surfs[0].radius = 12.0;
        assert_eq!(c.drop_stale_edits(), 0);
        assert_eq!(c.picks[0].edits.len(), 1);
        // 面を足す: 番号がずれるので外す
        c.picks[0].surfs.insert(1, SurfRow::default());
        assert_eq!(c.drop_stale_edits(), 2);
        assert!(c.picks[0].edits.is_empty() && c.picks[0].ops.is_empty());
        assert!(c.picks[0].edits_for.is_empty());
        // 反転も番号の向きが変わるので外す
        let mut b = Pick::new(ElemKind::Lens);
        b.set_edit(0, "semi", json!(8.0));
        c.picks.push(b);
        assert_eq!(c.drop_stale_edits(), 0);
        c.picks[1].reversed = true;
        assert_eq!(c.drop_stale_edits(), 1);
        // 消す操作の後の数
        assert_eq!(c.census(), [2, 0, 0]);
    }

    /// 絞り・ソルブ・ピックアップ: 要素の面はその要素の id に、絞りを面に置けば前の絞りは無くなる
    #[test]
    fn surface_ops_follow_the_element() {
        let mut c = Combo {
            picks: vec![Pick::new(ElemKind::Lens), Pick::new(ElemKind::Lens)],
            ..Default::default()
        };
        assert!(c.lead_stop());
        c.picks[1].ops.push(json!({
            "op": "add_pickup", "dst": 1, "src": {"block": "L1", "surface": 0}, "what": "c", "scale": -1
        }));
        c.set_stop_face(Some((vec![0], 1)));
        assert!(!c.lead_stop());
        assert_eq!(c.stop_face(), Some((vec![0], 1)));
        let v = c.project();
        assert_eq!(v["blocks"][0]["type"], "lens");
        assert_eq!(
            v["surface_ops"],
            json!([
                {"op": "set_stop", "surface": {"block": "L1", "surface": 1}},
                {"op": "add_pickup", "dst": {"block": "L2", "surface": 1}, "src": {"block": "L1", "surface": 0}, "what": "c", "scale": -1},
            ])
        );
        c.set_stop_face(None);
        assert!(c.lead_stop());
        assert_eq!(c.project()["surface_ops"].as_array().map(Vec::len), Some(1));
    }
    /// ON/OFF・往路を戻る・状態
    #[test]
    fn on_off_back_and_states() {
        let mut c = Combo {
            picks: vec![
                Pick::new(ElemKind::Lens),
                Pick::new(ElemKind::Plate),
                Pick::new(ElemKind::Mirror),
            ],
            ..Default::default()
        };
        c.picks[1].on = false;
        c.picks[1].off_air = true;
        c.picks[2].back = true;
        let v = c.project();
        let b = &v["blocks"];
        assert_eq!((&b[4]["on"], &b[4]["off"]), (&json!(false), &json!("air")));
        assert_eq!(b[6]["back"], true);
        assert!(v.get("states").is_none());
        assert_eq!(c.span_ids(), vec!["L1", "", "M1"]);
        // 状態: P1 を入れ、L1 の後の空気を変える
        c.states.push("測定".into());
        c.state = Some("測定".into());
        c.picks[1].state_on.insert("測定".into(), true);
        *c.picks[0].air_mut(Some("測定")) = 12.0;
        assert_eq!(c.picks[0].air, 5.0);
        let v = c.project();
        assert_eq!(v["blocks"][3]["id"], "L1_air");
        assert_eq!(
            v["states"],
            json!([{ "name": "測定", "set": { "P1": { "on": true }, "L1_air": { "t": 12.0 } } }])
        );
        assert_eq!(c.span_ids(), vec!["L1", "P1", "M1"]);
        // 図の間隔を動かすと、状態の値が変わる
        c.shift(1, 1.0);
        assert_eq!(c.picks[0].air_in(Some("測定")), 13.0);
        assert_eq!(c.picks[0].air, 5.0);
        // 展開できる（基本は平板が空気、状態では平板が入る。鏡で往路を戻る）
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let pr = lensbench_project::Project::from_json(&c.project().to_string()).unwrap();
        let env = lensbench_project::Env {
            base: root.clone(),
            glass_dirs: vec![root.join("tests/glass")],
            catalog: None,
        };
        let n = |st: Option<&str>| {
            lensbench_project::expand_state(&pr, &env, st)
                .unwrap()
                .surfaces
                .len()
        };
        let (a, m) = (n(None), n(Some("測定")));
        assert_eq!(m, a + 2 * 2, "往復で平板の 2 面が 2 回");
    }
    /// 状態ごとの偏心・傾き（玉と鏡）・系の値・BS の経路がプロジェクトの状態になり、展開で当たる
    #[test]
    fn state_place_settings_and_path() {
        let mut c = Combo {
            picks: vec![Pick::new(ElemKind::Lens), Pick::new(ElemKind::Mirror)],
            states: vec!["S".into()],
            ..Default::default()
        };
        let st = Some("S");
        // 見るだけでは状態の値を作らない
        assert_eq!(c.picks[0].place_in(st).decenter, [0.0; 2]);
        assert!(!c.picks[0].has_state_values("S"));
        {
            let (d, t) = c.picks[0].place_mut(st);
            d[1] = 0.3;
            t[0] = 1.5;
        }
        c.picks[1].place_mut(st).1[1] = 2.0;
        assert_eq!(c.picks[0].decenter, [0.0; 2]);
        let mut s = c.snap();
        s.object_distance = Some(200.0);
        s.waves = vec![WavePt::um(0.55)];
        s.primary = 1;
        c.state_sys.insert("S".into(), s);
        c.state_path.insert("S".into(), "tr".into());
        let v = c.project();
        let ids = c.ids();
        let set = &v["states"][0]["set"];
        assert_eq!(set[&ids[0]]["decenter"], json!([0.0, 0.3]));
        assert_eq!(set[&ids[0]]["tilt"], json!([1.5, 0.0, 0.0]));
        assert_eq!(set[&ids[1]]["tilt"], json!([45.0, 2.0]));
        let pr = &v["states"][0]["project"];
        assert_eq!(pr["object_distance"], 200.0);
        assert_eq!(pr["wavelengths"], json!([{ "um": 0.55 }]));
        assert_eq!(pr["primary"], 1);
        assert_eq!(pr["path"], "TR");
        assert!(pr.get("aperture").is_none(), "変えていない項目は書かない");
        // プロジェクトとして読め、状態で値が変わる
        let p = lensbench_project::Project::from_json(&v.to_string()).unwrap();
        let q = p.with_state("S").unwrap();
        assert_eq!(q.object_distance, Some(200.0));
        assert_eq!(q.path.as_deref(), Some("TR"));
        // 状態の値をやめる
        c.picks[0].clear_state("S");
        assert!(!c.picks[0].has_state_values("S"));
        // 基本の値が無限遠で、状態で無限遠に戻すなど、書かない項目は既定の値で書く
        let mut c2 = c.clone();
        c2.object_distance = Some(50.0);
        c2.telecentric = true;
        c2.aiming = Aiming::Real;
        let mut s2 = c2.snap();
        s2.object_distance = None;
        s2.telecentric = false;
        s2.aiming = Aiming::Off;
        c2.state_sys.insert("S".into(), s2);
        let pr = c2.state_project("S");
        assert_eq!(pr["object_distance"], Value::Null);
        assert_eq!(pr["telecentric"], json!(false));
        assert_eq!(pr["aiming"], json!("off"));
        let v2 = c2.project();
        let p2 = lensbench_project::Project::from_json(&v2.to_string()).unwrap();
        assert_eq!(p2.with_state("S").unwrap().object_distance, None);
    }

    /// 群: 続く同じ群の要素を群のブロックに、状態で群を動かす
    #[test]
    fn groups_wrap_runs_and_move_by_state() {
        let mut c = Combo {
            picks: vec![
                Pick::new(ElemKind::Lens),
                Pick::new(ElemKind::Lens),
                Pick::new(ElemKind::Lens),
            ],
            ..Default::default()
        };
        c.picks[0].air = 10.0;
        c.picks[1].group = "G1".into();
        c.picks[2].group = "G1".into();
        c.picks[1].air = 3.0;
        c.picks[2].air = 30.0;
        c.states.push("tele".into());
        c.groups.push(GroupDef {
            name: "G1".into(),
            role: "zoom".into(),
            shift: 0.0,
            state_shift: [("tele".to_string(), 2.0)].into(),
        });
        let v = c.project();
        let b = &v["blocks"];
        // 絞り・空気、L1・空気、G1（L2・空気・L3）、空気
        assert_eq!(b[4]["type"], "group");
        assert_eq!(b[4]["role"], "zoom");
        assert_eq!(b[4]["blocks"].as_array().unwrap().len(), 3);
        assert_eq!(b[5]["t"], 30.0);
        assert_eq!(v["states"][0]["set"]["G1"]["shift"], 2.0);
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let pr = lensbench_project::Project::from_json(&v.to_string()).unwrap();
        let env = lensbench_project::Env {
            base: root.clone(),
            glass_dirs: vec![root.join("tests/glass")],
            catalog: None,
        };
        let a = lensbench_project::expand_state(&pr, &env, None).unwrap();
        let t = lensbench_project::expand_state(&pr, &env, Some("tele")).unwrap();
        // L1 の後の空気が 2 増え、群の後の空気が 2 減る
        assert!((t.surfaces[3].t - a.surfaces[3].t - 2.0).abs() < 1e-12);
        let n = a.surfaces.len();
        assert!((t.surfaces[n - 2].t - a.surfaces[n - 2].t + 2.0).abs() < 1e-12);
    }
    /// 群: 離れた同じ名前は最初の続きだけ群に、要素の名前と同じ名前は群にしない
    #[test]
    fn groups_split_runs_and_element_names() {
        let mut c = Combo {
            picks: (0..4).map(|_| Pick::new(ElemKind::Lens)).collect(),
            ..Default::default()
        };
        c.picks[0].group = "G".into();
        c.picks[2].group = "G".into();
        let v = c.project();
        let types: Vec<&str> = v["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["type"].as_str().unwrap())
            .collect();
        assert_eq!(types.iter().filter(|t| **t == "group").count(), 1);
        let id = c.ids()[1].clone();
        c.picks[2].group = String::new();
        c.picks[0].group = id;
        let v = c.project();
        assert!(!v["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["type"] == "group"));
    }
    /// 群: 要素から外した群・要素の名前と同じ群の状態の移動は書かない（状態で系が組める）
    #[test]
    fn stale_group_shift_is_not_written() {
        let mut c = Combo {
            picks: (0..2).map(|_| Pick::new(ElemKind::Lens)).collect(),
            ..Default::default()
        };
        c.states.push("tele".into());
        c.groups.push(GroupDef {
            name: "G1".into(),
            state_shift: [("tele".to_string(), 2.0)].into(),
            ..Default::default()
        });
        let id = c.ids()[0].clone();
        c.groups.push(GroupDef {
            name: id.clone(),
            state_shift: [("tele".to_string(), 1.0)].into(),
            ..Default::default()
        });
        c.picks[0].group = id;
        let v = c.project();
        assert!(v["states"][0]["set"].as_object().unwrap().is_empty());
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let pr = lensbench_project::Project::from_json(&v.to_string()).unwrap();
        let env = lensbench_project::Env {
            base: root.clone(),
            glass_dirs: vec![root.join("tests/glass")],
            catalog: None,
        };
        lensbench_project::expand_state(&pr, &env, Some("tele")).unwrap();
    }
    /// BS: キューブの半分の三角形から正方形と分ける面
    #[test]
    fn cube_from_half_prism() {
        let [sq, diag] = cube_of_prism(&[[16.0, -6.35], [28.7, 6.35], [16.0, 6.35]]).unwrap();
        assert_eq!(diag, vec![[16.0, -6.35], [28.7, 6.35]]);
        let has = |q: [f64; 2]| {
            sq.iter()
                .any(|p| (p[0] - q[0]).abs() < 1e-9 && (p[1] - q[1]).abs() < 1e-9)
        };
        assert!(has([28.7, -6.35]) && has([16.0, 6.35]) && sq.len() == 4);
        assert!(cube_of_prism(&[[0.0, 0.0], [2.0, 0.0], [1.0, 1.5]]).is_none());
        assert!(cube_of_prism(&[[0.0, 0.0], [1.0, 0.0]]).is_none());
    }
    /// 構成を重ねた layout では光線の色は構成ごと、1 つなら視野ごと
    #[test]
    fn ray_color_by_config() {
        let ray = |f: u64, c: u64| json!({ "field": f, "cfg": c, "pts": [[0.0, 0.0], [1.0, 0.0]] });
        let one = LayoutData::parse(&json!({ "rays": [ray(2, 0)], "configs": [] }));
        assert_eq!(one.rays[0].0, 2);
        let two = LayoutData::parse(&json!({
            "rays": [ray(2, 0), ray(2, 1)], "configs": ["TT", "RR"]
        }));
        assert_eq!(two.rays.iter().map(|r| r.0).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(two.configs, ["TT", "RR"]);
    }
    /// xz 面の図: plane で縦が x と分かり、矢印・ドラッグの偏心・傾きは x の向き
    #[test]
    fn xz_plane_moves_x() {
        let d = LayoutData::parse(&json!({ "plane": "xz" }));
        assert!(d.xz && !LayoutData::parse(&json!({})).xz);
        let mut p = Pick::new(ElemKind::Lens);
        let v = View {
            xz: true,
            ..Default::default()
        };
        v.nudge(&mut p, None, 0.5, 2.0);
        assert_eq!((p.decenter, p.tilt), ([0.5, 0.0], [0.0, -2.0, 0.0]));
        View::default().nudge(&mut p, None, 0.5, 2.0);
        assert_eq!((p.decenter, p.tilt), ([0.5, 0.5], [2.0, -2.0, 0.0]));
    }
    /// 名前の無い要素（BS）が続くとき、自動の玉を順に当てる
    #[test]
    fn spans_for_consecutive_splitters() {
        let d = LayoutData {
            vertices: vec![None; 17],
            plates: vec![("image".into(), 16, vec![])],
            units: vec![
                ("lens1".into(), vec![[2, 3]]),
                ("lens2".into(), vec![[4, 8]]),
                ("lens3".into(), vec![[10, 14]]),
                ("L1".into(), vec![[2, 3]]),
            ],
            ..Default::default()
        };
        let ids: Vec<String> = ["L1", "BS1", "BS2"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            d.spans_for(&ids, false),
            vec![Some([2, 3]), Some([4, 8]), Some([10, 14])]
        );
    }

    #[test]
    fn cube_from_prism_with_repeated_corner() {
        let p = [
            [16.0, -39.05],
            [28.7, -26.35],
            [16.0, -26.35],
            [16.0, -26.35 + 1e-12],
        ];
        assert!(cube_of_prism(&p).is_some());
    }
    /// 開口・遮蔽: 空気中の面の開口になり、serve で系にできて図の要素に当たる
    #[test]
    fn aperture_block_builds() {
        let mut c = Combo {
            epd: 4.0,
            stop_gap: 2.0,
            ..Default::default()
        };
        let mut l = Pick::new(ElemKind::Lens);
        l.air = 10.0;
        c.picks.push(l);
        for (shape, dy) in [
            (ApShape::Obscuration, 0.0),
            (ApShape::Rect, 0.5),
            (ApShape::Circle, 0.0),
            (ApShape::Spider, 0.0),
        ] {
            let mut a = Pick::new(ElemKind::Aperture);
            a.ap = shape;
            a.semi = 1.0;
            a.ap_half = [3.0, 1.5];
            a.decenter[1] = dy;
            a.air = 5.0;
            c.picks.push(a);
        }
        let mut proj = c.project();
        let b = proj["blocks"].as_array().unwrap();
        let ap: Vec<&Value> = b.iter().filter(|x| x["type"] == "surfaces").collect();
        assert_eq!(ap.len(), 4);
        assert_eq!(ap[0]["surfaces"][0]["aperture"]["type"], "obscuration");
        let sp = &ap[3]["surfaces"][0]["aperture"];
        assert_eq!(sp["type"], "spider");
        assert_eq!(
            (sp["width"].as_f64(), sp["arms"].as_u64()),
            (Some(1.0), Some(4))
        );
        assert_eq!(ap[1]["surfaces"][0]["aperture"]["dy"], 0.5);
        assert_eq!(ap[1]["surfaces"][0]["aperture"]["hx"], 3.0);
        proj["glass_catalogs"] = json!(["SCHOTT"]);
        let f = std::env::temp_dir().join(format!("lensbench-ap-{}.json", std::process::id()));
        lensbench_core::vfs::write(&f, proj.to_string()).unwrap();
        let gd = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/glass");
        let v = lensbench_engine::preview_layout(
            &json!({ "file": f.display().to_string(), "glass_dir": [gd.display().to_string()] }),
            &json!({}),
        )
        .unwrap();
        lensbench_core::vfs::remove_file(&f).ok();
        let d = LayoutData::parse(&v);
        let spans = d.spans_for(&c.ids(), c.lead_stop());
        assert!(spans.iter().all(Option::is_some), "{spans:?}");
    }

    /// 組合せの要素で作れなかったブロック: 3 面の自作の玉（貼り合わせ）、面の並び（Jones 面・座標ブレーク）、
    /// .zmx の面の範囲の取り込み。どれも系になり、要素ごとに図の面の範囲が当たる
    #[test]
    fn multi_surface_blocks_build() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut c = Combo {
            epd: 4.0,
            ..Default::default()
        };
        let mut l = Pick::new(ElemKind::Lens);
        l.r1 = 40.0;
        l.r2 = -30.0;
        l.t = 5.0;
        l.surfs = l.lens_rows();
        l.surfs[1].t = 2.0;
        l.surfs[1].glass = "F2".into();
        l.surfs.push(SurfRow {
            radius: -100.0,
            semi: 12.7,
            glass: "ignored".into(),
            t: 9.0,
            ..Default::default()
        });
        l.air = 10.0;
        let mut sf = Pick::new(ElemKind::Surfaces);
        sf.surfs = vec![
            SurfRow {
                kind: SurfKind::Jones,
                jones: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                t: 1.0,
                ..Default::default()
            },
            SurfRow {
                kind: SurfKind::Coordbrk,
                cb: [0.0, 0.0, 1.0, 0.0, 0.0],
                order: 1,
                ..Default::default()
            },
        ];
        sf.air = 10.0;
        let mut z = Pick::new(ElemKind::ZmxRange);
        z.path = root.join("tests/refs/A0/system.zmx").display().to_string();
        z.air = 30.0;
        c.picks = vec![l, sf, z];
        let mut proj = c.project();
        let b = proj["blocks"].as_array().unwrap();
        // .zmx の絞りを使うので、前に自動の絞りは無い
        assert!(!c.lead_stop());
        let lens = &b[0]["source"]["surfaces"];
        assert_eq!(lens.as_array().unwrap().len(), 3);
        assert_eq!(lens[1]["glass"], "F2");
        // 最後の面の後は空気（ガラス・厚さは書かない）
        assert!(lens[2]["glass"].is_null() && lens[2]["t"].is_null());
        assert_eq!(b[2]["surfaces"][0]["jones"][0], 1.0);
        assert_eq!(b[2]["surfaces"][1]["coordbrk"][2], 1.0);
        // 偏光子・波長板の行は project の書き方で読める
        let pol = SurfRow {
            kind: SurfKind::Polarizer,
            axis_deg: 30.0,
            extinction_ratio: 1000.0,
            ..Default::default()
        };
        let wp = SurfRow {
            kind: SurfKind::Waveplate,
            ..Default::default()
        };
        let none = SurfRow {
            kind: SurfKind::Polarizer,
            ..Default::default()
        };
        // 複屈折材の波長板（1 材・2 材の組）
        let mut quartz = SurfRow {
            kind: SurfKind::Waveplate,
            wp_material: "QUARTZ".into(),
            ..Default::default()
        };
        let q: lensbench_project::SurfSpec = serde_json::from_value(quartz.json()).unwrap();
        let w = q.waveplate.unwrap();
        assert_eq!(
            (w.material.as_deref(), w.thickness_mm),
            (Some("QUARTZ"), None)
        );
        quartz.wp_thickness = 0.4;
        quartz.wp_material2 = "MGF2".into();
        quartz.wp_thickness2 = 0.3;
        let q: lensbench_project::SurfSpec = serde_json::from_value(quartz.json()).unwrap();
        let w = q.waveplate.unwrap();
        assert!(w.material.is_none());
        assert_eq!(w.plates.len(), 2);
        assert_eq!(
            (w.plates[1].material.as_str(), w.plates[1].axis_deg),
            ("MGF2", 90.0)
        );
        assert_eq!(pol.json()["polarizer"]["extinction_ratio"], 1000.0);
        assert!(none.json()["polarizer"].get("extinction_ratio").is_none());
        for r in [&pol, &wp, &none] {
            let q: lensbench_project::SurfSpec = serde_json::from_value(r.json()).unwrap();
            assert!(
                q.polarizer.is_some() || q.waveplate.is_some(),
                "{:?}",
                r.json()
            );
        }
        assert_eq!(b[2]["surfaces"][1]["order"], 1);
        assert_eq!(b[4]["type"], "zmx");
        assert!(b[4]["from"].is_null() && b[4]["config"].is_null() && b[4]["stop"].is_null());
        // 範囲に .zmx の絞り（A0 は面 5）が無ければ、.zmx の絞りは使わず前に自動の絞りを置く（レビュー指摘）
        assert_eq!(zmx_stop_info(&c.picks[2].path).map(|x| x.0), Some(5));
        let mut short = c.clone();
        short.picks[2].range = [1, 3];
        assert!(!short.picks[2].range_has_stop() && short.lead_stop());
        let sp = short.project();
        assert_eq!(sp["blocks"][0]["type"], "stop");
        assert_eq!(sp["blocks"][6]["stop"], false);
        proj["glass_catalogs"] = json!(["SCHOTT"]);
        let f = std::env::temp_dir().join(format!("lensbench-multi-{}.json", std::process::id()));
        lensbench_core::vfs::write(&f, proj.to_string()).unwrap();
        let gd = root.join("tests/glass");
        let v = lensbench_engine::preview_layout(
            &json!({ "file": f.display().to_string(), "glass_dir": [gd.display().to_string()] }),
            &json!({}),
        )
        .unwrap();
        lensbench_core::vfs::remove_file(&f).ok();
        let d = LayoutData::parse(&v);
        let spans = d.spans_for(&c.ids(), c.lead_stop());
        assert!(spans.iter().all(Option::is_some), "{spans:?} {:?}", d.units);
        let mut sp = short.project();
        sp["glass_catalogs"] = json!(["SCHOTT"]);
        lensbench_core::vfs::write(&f, sp.to_string()).unwrap();
        let v2 = lensbench_engine::preview_layout(
            &json!({ "file": f.display().to_string(), "glass_dir": [gd.display().to_string()] }),
            &json!({}),
        );
        lensbench_core::vfs::remove_file(&f).ok();
        assert!(v2.is_ok(), "{v2:?}");
        // 貼り合わせの玉は 3 面
        let s0 = spans[0].unwrap();
        assert_eq!(s0[1] - s0[0], 2, "{spans:?}");
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    /// ロックは中身だけ、「位置も」を付けると後の間隔も変えられない
    #[test]
    fn lock_place_blocks_gap() {
        let mut c = Combo::default();
        c.picks.push(Pick::new(ElemKind::Lens));
        c.picks.push(Pick::new(ElemKind::Lens));
        c.picks[0].locked = true;
        assert!(gap_mut(&mut c, 0).is_some());
        c.picks[0].lock_place = true;
        assert!(c.picks[0].fixed());
        assert!(gap_mut(&mut c, 0).is_none());
        assert!(gap_mut(&mut c, 1).is_some());
    }
}

#[cfg(test)]
mod nav_tests {
    use super::*;

    /// カーソルの下の点は拡大縮小しても動かない（#124）
    #[test]
    fn zoom_keeps_point_under_cursor() {
        let mut v = View::default();
        let fc = Pos2::new(100.0, 50.0);
        let c = Pos2::new(160.0, 20.0);
        // 図の点 w（全体表示の中心からの図の上の差 × s）が画面の c に来ている
        let world = |v: &View, q: Pos2| (q - (fc + v.pan)) / v.zoom_log.exp();
        let w0 = world(&v, c);
        v.zoom_about(fc, c, 2.0);
        assert!((world(&v, c) - w0).length() < 1e-4);
        assert!((v.zoom_log - 2f32.ln()).abs() < 1e-6);
        v.zoom_about(fc, Pos2::new(10.0, 90.0), 0.5);
        assert!((v.zoom_log).abs() < 1e-6);
    }
}
