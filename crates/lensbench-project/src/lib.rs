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
//! - 群（ズーム群・フォーカス群など）: 連続したブロック（.zmx の取り込みでは面の範囲）に名前と役割を付ける。
//!   移動 `shift` は前の間隔に足して後の間隔から引くので、群の外の合計長は変わらない（物体が無限遠で群が
//!   最初にあるときは、前の間隔は表さないので、後ろの間隔が縮むだけ）。
//! - ユーザーカタログ（[`UserCatalog`]、JSON）: 玉モードで得た玉や組んだブロックを名前で保存し、`"type": "user"` の
//!   ブロックで他のプロジェクトから使う。
//! - 検出器（[`DetectorBlock`]、`"type"` は登録した種類の名前）: 最後のブロックに置く。像面に置き、面は足さない。
//!   種類と項目の確かめ方は [`detectors`] に登録する（非公開版などの拡張が足す）。
//! - 状態（多重構成に当たる）: ブロックの値（ON/OFF、間隔、群の移動、偏心・傾き、.zmx の構成番号など）と
//!   系の値（物体距離など）の変更の組に名前を付ける。[`expand_state`] でその状態の系を展開する。

pub mod catalog_fill;
pub mod catalog_spec;
pub mod detectors;
pub mod edit_ops;
pub mod filter;
pub mod merit;
pub mod plugin_term;
pub mod plugins;
pub mod schema;
mod splitter;
mod step_geom;
pub mod swap;
pub mod sysfile;
pub mod tolerance;
pub use splitter::{Arm, PbsSpec, SplitFace, SplitForm, SplitterSpec};
use splitter::{Level, SplitState};

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use lensbench_core::elements::{self, Element, Kind};
use lensbench_core::glass::{self, Glass};
use lensbench_core::math::{mv, rot, M3};
use lensbench_core::param::{Pickup, ThickSolve};
use lensbench_core::surface::{After, Aperture, SurfType, Surface};
use lensbench_core::system::{
    AfocalUnit, Aiming, ApertureKind, Field, FieldCond, FieldNorm, FieldType, System,
};
use lensbench_core::zmx;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

fn yes() -> bool {
    true
}
fn one() -> usize {
    1
}
fn default_catalogs() -> Vec<String> {
    vec!["SCHOTT".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Wave {
    pub um: f64,
    #[serde(default = "wave_weight")]
    pub weight: f64,
}
fn wave_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApertureType {
    Epd,
    Fnum,
    Na,
    Float,
    Pwfn,
    Cone,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApertureSpec {
    #[serde(rename = "type")]
    pub kind: ApertureType,
    #[serde(default)]
    pub value: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    #[default]
    Angle,
    ObjectHeight,
    ParaxialImageHeight,
    RealImageHeight,
    /// 条件で定める視野（solve に条件、points が目標値）
    Solve,
}

/// 条件で定める視野の条件。面は番号か名前（面のコメント）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(tag = "cond", rename_all = "snake_case")]
pub enum FieldSolveSpec {
    /// 面 surface での主光線の位置 = 視野の値
    Position { surface: String },
    /// 面 s1・s2 での主光線の方向余弦 (l, m) の和 = 視野の値（法線入射で戻るなら 0）
    DirSum { s1: String, s2: String },
    /// 差（s1 − s2）= 視野の値
    DirDiff { s1: String, s2: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FieldsSpec {
    #[serde(rename = "type", default)]
    pub kind: FieldKind,
    /// 視野の (x, y)
    pub points: Vec<[f64; 2]>,
    #[serde(default)]
    pub weights: Vec<f64>,
    /// type = solve のときの条件
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solve: Option<FieldSolveSpec>,
}

impl Default for FieldsSpec {
    fn default() -> Self {
        FieldsSpec {
            kind: FieldKind::Angle,
            points: vec![[0.0, 0.0]],
            weights: Vec::new(),
            solve: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AimingSpec {
    #[default]
    Off,
    Paraxial,
    Real,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OffMode {
    /// 長さごと除く
    #[default]
    Collapse,
    /// 同じ長さの空気で残す
    Air,
}

/// ブロックの置き方（ON/OFF、偏心・傾き）
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
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
    /// 半直径 [mm]。"auto" なら光線から求める（OpticStudio の Automatic）
    #[serde(default)]
    pub semi: Semi,
    #[serde(default)]
    pub coat: String,
    #[serde(default)]
    pub comment: String,
    /// Jones 行列の面（平面、OpticStudio の JONESMAT）: [Ar, Ai, Br, Bi, Cr, Ci, Dr, Di]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jones: Option<[f64; 8]>,
    /// 直線偏光子の面（平面の Jones 面）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polarizer: Option<PolarizerSpec>,
    /// 波長板の面（平面の Jones 面、波長ごとに遅延を変える）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveplate: Option<WaveplateSpec>,
    /// 面だけの傾き [tx, ty] deg（頂点のまわり、後の面の座標は元に戻す。くさびのウィンドウなど）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt: Option<[f64; 2]>,
    /// 座標ブレークの面: [dx, dy, tx, ty, tz]（曲率・ガラスは書かない）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordbrk: Option<[f64; 5]>,
    /// 座標ブレークの順序（0: 偏心してから傾き、1: 傾けてから偏心）
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub order: u8,
    /// 開口（省略なら開口なし）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aperture: Option<SurfAperture>,
    /// 複屈折材の板の入口・出口（OpticStudio の Birefringent In / Out）。入口の `glass` が常光の材料（異常光は 名前-E）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birefringent: Option<BireSpec>,
}

/// 複屈折材の板の入口・出口の面。`{"in": {"axis": [x, y, z]}}` か `"out"`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BireSpec {
    In {
        /// 2: 波長板（常光の道で、常光・異常光の両方の位相）、0: 常光の成分だけ（OpticStudio の Mode）
        #[serde(default = "two_u8")]
        mode: u8,
        /// 結晶軸の向き（面の局所座標）
        axis: [f64; 3],
    },
    Out,
}

fn two_u8() -> u8 {
    2
}

/// 面の半直径: 数 [mm] か "auto"（全視野・全波長の主光線と瞳の縁の光線から求める）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Semi {
    Value(f64),
    Auto(AutoWord),
}

/// [`Semi::Auto`] の綴り "auto"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AutoWord {
    Auto,
}

impl Default for Semi {
    fn default() -> Self {
        Semi::Value(0.0)
    }
}

impl From<f64> for Semi {
    fn from(v: f64) -> Self {
        Semi::Value(v)
    }
}

impl Semi {
    pub const AUTO: Semi = Semi::Auto(AutoWord::Auto);

    /// 数の値（自動なら 0。系を作ると光線から求める）
    pub fn value(self) -> f64 {
        match self {
            Semi::Value(v) => v,
            Semi::Auto(_) => 0.0,
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, Semi::Auto(_))
    }

    /// 面の半直径（自動ならその印）
    pub fn of(d: &Surface) -> Semi {
        if d.semi_auto {
            Semi::AUTO
        } else {
            Semi::Value(d.semi)
        }
    }
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

fn one_f64() -> f64 {
    1.0
}

fn inf_f64() -> f64 {
    f64::INFINITY
}

fn is_inf(x: &f64) -> bool {
    x.is_infinite()
}

/// 直線偏光子（理想）: 透過軸の向きの強度透過率 `transmittance`、直交する向きは `transmittance / extinction_ratio`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PolarizerSpec {
    /// 透過軸の向き [deg]（x から y へ）
    #[serde(default)]
    pub axis_deg: f64,
    /// 消光比（1000 は 1000:1。省略は無限 = 直交の向きを通さない）
    #[serde(default = "inf_f64", skip_serializing_if = "is_inf")]
    pub extinction_ratio: f64,
    /// 透過軸の向きの強度透過率
    #[serde(default = "one_f64")]
    pub transmittance: f64,
}

/// 波長板: 速い軸 `axis_deg` に対する遅い軸の遅延。理想（材料なし）は (order + retardance)·wavelength_um/λ 波長。
/// `material`（複屈折材、常光はガラス material、異常光は material-E）か `plates`（2 材の組など）があれば
/// 遅延は板ごとの Δn(λ)·d/λ。`material` で厚さを省くと、設計波長の遅延 order + retardance から決める
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WaveplateSpec {
    /// 設計波長での遅延 [波長]（λ/4 板は 0.25、λ/2 板は 0.5）。厚さを書いた板では使わない
    #[serde(default)]
    pub retardance: f64,
    /// 設計波長 [µm]
    pub wavelength_um: f64,
    /// 速い軸の向き [deg]（x から y へ）
    #[serde(default)]
    pub axis_deg: f64,
    /// 多次の波長板の整数の次数（ゼロ次は 0）
    #[serde(default)]
    pub order: u32,
    /// 強度透過率
    #[serde(default = "one_f64")]
    pub transmittance: f64,
    /// 複屈折材（1 枚の板。速い軸が `axis_deg`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// `material` の板の厚さ [mm]（省略なら設計波長の遅延から）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness_mm: Option<f64>,
    /// 複屈折材の板の組（光の通る順。アクロマティック波長板は速い軸を直交させた 2 材など）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plates: Vec<PlateSpec>,
}

/// 波長板の複屈折材の板 1 枚
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlateSpec {
    /// 複屈折材（常光はガラス material、異常光は material-E）
    pub material: String,
    /// 厚さ [mm]
    pub thickness_mm: f64,
    /// 速い軸の向き [deg]（波長板の `axis_deg` から）
    #[serde(default)]
    pub axis_deg: f64,
}

impl PolarizerSpec {
    pub fn retarder(&self) -> Result<lensbench_core::surface::Retarder, String> {
        if !(self.transmittance >= 0.0 && self.transmittance <= 1.0) {
            return Err(format!("偏光子の透過率 {}（0〜1）", self.transmittance));
        }
        if self.extinction_ratio.is_nan() || self.extinction_ratio < 1.0 {
            return Err(format!(
                "偏光子の消光比 {}（1 以上）",
                self.extinction_ratio
            ));
        }
        Ok(lensbench_core::surface::Retarder {
            axis_deg: self.axis_deg,
            tx: self.transmittance.sqrt(),
            ty: (self.transmittance / self.extinction_ratio).sqrt(),
            retardance: 0.0,
            order: 0,
            wl0: 1.0,
            plates: Vec::new(),
        })
    }
}

impl WaveplateSpec {
    pub fn retarder(&self) -> Result<lensbench_core::surface::Retarder, String> {
        if !(self.wavelength_um > 0.0 && self.wavelength_um.is_finite()) {
            return Err(format!("波長板の設計波長 {} µm", self.wavelength_um));
        }
        if !(self.transmittance >= 0.0 && self.transmittance <= 1.0) {
            return Err(format!("波長板の透過率 {}（0〜1）", self.transmittance));
        }
        if !self.retardance.is_finite() {
            return Err("波長板の遅延が数でない".into());
        }
        use lensbench_core::surface::Plate;
        let plate = |m: &str, d: f64, a: f64| Plate {
            material: m.trim().to_string(),
            thickness: d,
            axis_deg: a,
            o: None,
            e: None,
        };
        let plates = match (&self.material, self.plates.is_empty()) {
            (Some(_), false) => return Err("波長板に material と plates は一緒に書けない".into()),
            (Some(m), true) => {
                let d = self.thickness_mm.unwrap_or(0.0);
                if self.thickness_mm.is_some() && !(d > 0.0 && d.is_finite()) {
                    return Err(format!("波長板の厚さ {d} mm"));
                }
                let total = self.order as f64 + self.retardance;
                if self.thickness_mm.is_none() && !(total > 0.0 && total.is_finite()) {
                    return Err("厚さを省いた波長板は遅延 order + retardance を正にする".into());
                }
                vec![plate(m, d, 0.0)]
            }
            (None, false) => {
                if self.thickness_mm.is_some() {
                    return Err("plates の波長板では厚さは板ごとに書く".into());
                }
                let mut v = Vec::new();
                for p in &self.plates {
                    if !(p.thickness_mm > 0.0 && p.thickness_mm.is_finite()) {
                        return Err(format!(
                            "波長板の板 {} の厚さ {} mm",
                            p.material, p.thickness_mm
                        ));
                    }
                    if !p.axis_deg.is_finite() {
                        return Err("波長板の板の軸が数でない".into());
                    }
                    v.push(plate(&p.material, p.thickness_mm, p.axis_deg));
                }
                v
            }
            (None, true) => {
                if self.thickness_mm.is_some() {
                    return Err("波長板の厚さは material と一緒に書く".into());
                }
                Vec::new()
            }
        };
        if plates.iter().any(|p| p.material.is_empty()) {
            return Err("波長板の複屈折材の名前が空".into());
        }
        let t = self.transmittance.sqrt();
        Ok(lensbench_core::surface::Retarder {
            axis_deg: self.axis_deg,
            tx: t,
            ty: t,
            retardance: self.retardance,
            order: self.order,
            wl0: self.wavelength_um,
            plates,
        })
    }
}

/// 面の開口（偏心 dx・dy は省略で 0）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SurfAperture {
    /// 円（最小・最大半径）
    Circular {
        #[serde(default)]
        min: f64,
        max: f64,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    /// 矩形（x・y の半幅）
    Rect {
        hx: f64,
        hy: f64,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    /// 楕円（x・y の半軸）
    Ellipse {
        hx: f64,
        hy: f64,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    /// 円形の遮蔽（最小〜最大半径の間を遮る）
    Obscuration {
        #[serde(default)]
        min: f64,
        max: f64,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    /// スパイダー（中心から外へ伸びる幅 width の腕 arms 本を遮る。最初の腕は +x、360° を等分）
    Spider {
        width: f64,
        arms: u32,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    /// 半径（semi）で切る
    Floating,
}

/// 玉の出どころ
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum LensSource {
    /// .zmx の k 番目の玉（玉だけを数える、1 始まり）
    Zmx {
        zmx: String,
        #[serde(default = "one")]
        lens: usize,
        /// .zmx のマルチコンフィグの番号（1 始まり）
        #[serde(default = "one")]
        config: usize,
    },
    /// レンズカタログ（メーカーと型番）
    Catalog { vendor: String, part: String },
    /// 面を直接
    Surfaces { surfaces: Vec<SurfSpec> },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(remote = "Self", tag = "type", rename_all = "snake_case")]
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
        /// 傾き x・y [deg]。折り曲げでは鏡の前後で同じ角だけ傾ける（光軸が折れる）。
        /// `back`（往路を戻る）では鏡だけを傾け、座標は元の軸に戻す（復路の光線が傾きの 2 倍の角で戻る）
        #[serde(default)]
        tilt: [f64; 2],
        #[serde(default)]
        semi: f64,
        /// 鏡の開口（省略なら半径 semi の円。四角い鏡は rect）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        aperture: Option<SurfAperture>,
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
        /// .zmx のマルチコンフィグの番号（1 始まり）
        #[serde(default = "one")]
        config: usize,
        /// .zmx の絞りを系の絞りにする（false なら取り込んだ面の絞りを外す。系の絞りはほかのブロックで）
        #[serde(default = "yes")]
        stop: bool,
        /// 玉モードの群（.zmx の面の範囲）
        #[serde(default)]
        groups: Vec<ZmxGroup>,
        #[serde(flatten)]
        place: Place,
    },
    /// ユーザーカタログの項目（群と同じに入れる）
    User {
        #[serde(default)]
        id: String,
        /// ユーザーカタログのファイル（プロジェクトのフォルダから）
        catalog: String,
        item: String,
        #[serde(flatten)]
        place: Place,
    },
    /// ビームスプリッター（[`SplitterSpec`]）。id が要る
    Splitter {
        #[serde(default)]
        id: String,
        #[serde(flatten)]
        spec: SplitterSpec,
        #[serde(flatten)]
        place: Place,
    },
    Group {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        role: Option<Role>,
        /// 群の移動 [mm]（光の進む向きが正）。前の間隔に足し、後の間隔から引く
        #[serde(default)]
        shift: f64,
        blocks: Vec<Block>,
        #[serde(flatten)]
        place: Place,
    },
    /// 検出器（登録した種類、[`DetectorBlock`]）。`"type"` は種類の名前（このクレートの組み込みの名前でないもの）。
    /// 読み書きは下の Deserialize・Serialize の実装で、スキーマは [`crate::schema`] が足す
    #[serde(skip)]
    Detector(DetectorBlock),
}

/// 組み込みのブロックの `"type"`（[`Block`] の検出器以外の種類）。これ以外の名前は検出器（[`detectors`]）
pub const BUILTIN_BLOCKS: [&str; 9] = [
    "lens", "air", "stop", "mirror", "surfaces", "zmx", "user", "splitter", "group",
];

/// 検出器のブロック（[`Block::Detector`]）。`"type"` は登録した種類（[`detectors`]）、ほかの項目は種類ごと
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct DetectorBlock {
    /// 種類（[`detectors::register`] で登録した名前）
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    /// 種類ごとの項目
    #[serde(flatten)]
    pub params: Map<String, Value>,
}

impl DetectorBlock {
    /// 項目を種類の登録で確かめる（登録していない種類は誤り）
    pub fn check(&self) -> Result<(), String> {
        let k = detectors::get(&self.kind).ok_or_else(|| {
            format!(
                "知らないブロックの種類 {:?}（{} か、登録した検出器: {}）",
                self.kind,
                BUILTIN_BLOCKS.join("・"),
                detectors::list()
                    .iter()
                    .map(|k| k.name)
                    .collect::<Vec<_>>()
                    .join("・")
            )
        })?;
        (k.check)(&self.params).map_err(|e| format!("{} {}: {e}", self.kind, self.id))
    }
}

impl<'de> Deserialize<'de> for Block {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let v = Value::deserialize(d)?;
        match v.get("type").and_then(Value::as_str) {
            Some(t) if !BUILTIN_BLOCKS.contains(&t) => serde_json::from_value(v)
                .map(Block::Detector)
                .map_err(D::Error::custom),
            _ => Block::deserialize(v).map_err(D::Error::custom),
        }
    }
}

impl Serialize for Block {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Block::Detector(d) => d.serialize(s),
            b => Block::serialize(b, s),
        }
    }
}

/// 検出器のブロックの種類・項目を確かめる（群の中も。置き場所は展開で確かめる）
fn check_detectors(blocks: &[Block]) -> Result<(), String> {
    for b in blocks {
        match b {
            Block::Detector(d) => d.check()?,
            Block::Group { blocks, .. } => check_detectors(blocks)?,
            _ => {}
        }
    }
    Ok(())
}

impl Project {
    /// 最後のブロックの検出器（なければ None）
    pub fn detector(&self) -> Option<&DetectorBlock> {
        match self.blocks.last() {
            Some(Block::Detector(d)) => Some(d),
            _ => None,
        }
    }
}

/// 群の役割
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Zoom,
    Focus,
    Fixed,
}

/// .zmx の取り込みの中の群（玉モード）: 面 from〜to に始まる玉・鏡・面と、その間の空気
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZmxGroup {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: Option<Role>,
    pub from: usize,
    pub to: usize,
    /// 群の移動 [mm]。前の間隔の値（.zmx の厚さの向き）に足し、後の間隔から引く
    #[serde(default)]
    pub shift: f64,
    #[serde(flatten)]
    pub place: Place,
}

/// 状態（多重構成に当たる）: 値の変更の組
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct State {
    pub name: String,
    /// ブロックの id → 変える値（ブロックの JSON の項目名と値）。群・.zmx の中の群も id で指す
    #[serde(default)]
    pub set: BTreeMap<String, Map<String, Value>>,
    /// 系の値（物体距離・開口・視野など、プロジェクトの JSON の項目名と値）
    #[serde(default)]
    pub project: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_catalogs")]
    pub glass_catalogs: Vec<String>,
    /// 膜（名前 → フィルタの透過特性・表）。面の `coat` に名前を書いて使う。ユーザーカタログの同じ名前より優先
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub coatings: BTreeMap<String, filter::CoatSpec>,
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
    /// 物体側テレセントリック（物体距離が有限のときだけ）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub telecentric: bool,
    /// 像空間をアフォーカルにする。値は角度の単位（`ur`・`mr`・`rr`・`as`・`am`・`dg`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub afocal: Option<String>,
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub states: Vec<State>,
    /// ビームスプリッターの経路（BS を通る順の T・R）。省略で各 BS の arm。状態の project でも変えられる
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// ブロックの面の値の上書き（GUI の面の表で変えた値）。展開の最後に当てる
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surface_edits: Vec<SurfaceEdit>,
    /// 面の絞り・ソルブ・ピックアップ。serve の `edit` の `set_stop`・`add_solve`・`add_pickup` と同じ形で、
    /// 面（`surface`・`dst`・`src`・`other`）は `{"block": id, "surface": i}`（[`SurfaceEdit`] と同じ数え方）か系の面の番号。
    /// 展開の最後（`surface_edits` の後）に当てる。ブロックが無い（状態で外したなど）操作は当てずに注意を出す。
    /// 往復するブロックは往路の面を指す
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surface_ops: Vec<Value>,
    /// 機構部品（チューブ・ケージ板・マウント）。光線の追跡は変えず、ケラレの確認（`mech-check`）と描画に使う
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mechanics: Vec<MechPlacement>,
}

/// ブロックの面の値の上書き（[`Project::surface_edits`]）。面はブロック id の面のうち座標ブレークを除いて数えた
/// 番号（0 始まり。偏心・傾きの座標ブレークの有無で番号が変わらないように）。値は系の面の値（順次の面の値。
/// 鏡の後は厚さ・曲率の符号が逆）で、名前は [`lensbench_core::edit::SURF_FIELDS`]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct SurfaceEdit {
    pub block: String,
    pub surface: usize,
    pub fields: Map<String, Value>,
}

impl Project {
    pub fn from_json(txt: &str) -> Result<Project, String> {
        let p: Project =
            serde_json::from_str(txt).map_err(|e| format!("プロジェクトファイル: {e}"))?;
        check_detectors(&p.blocks)?;
        Ok(p)
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap()
    }

    pub fn state_names(&self) -> Vec<&str> {
        self.states.iter().map(|s| s.name.as_str()).collect()
    }

    /// 状態 name の変更を当てたプロジェクト（状態の一覧は残す）
    pub fn with_state(&self, name: &str) -> Result<Project, String> {
        let st: Vec<&State> = self.states.iter().filter(|s| s.name == name).collect();
        let st = match st.as_slice() {
            [s] => *s,
            [] => {
                return Err(format!(
                    "状態 {name} はない（{}）",
                    self.state_names().join(", ")
                ))
            }
            _ => return Err(format!("状態 {name} が 2 つ以上ある")),
        };
        let mut v = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let root = v.as_object_mut().unwrap();
        for (k, x) in &st.project {
            if k == "blocks"
                || k == "states"
                || (!root.contains_key(k)
                    && !["path", "afocal", "telecentric"].contains(&k.as_str()))
            {
                return Err(format!("状態 {name}: 系の値 {k} は変えられない"));
            }
            root.insert(k.clone(), x.clone());
        }
        for (id, fields) in &st.set {
            let mut hits = Vec::new();
            if !id.is_empty() {
                find_id(&v["blocks"], "/blocks", id, &mut hits);
            }
            let ptr = match hits.as_slice() {
                [p] => p.clone(),
                [] => return Err(format!("状態 {name}: ブロック {id} はない")),
                _ => return Err(format!("状態 {name}: id {id} のブロックが 2 つ以上ある")),
            };
            let obj = v.pointer_mut(&ptr).unwrap().as_object_mut().unwrap();
            for (k, x) in fields {
                if ["type", "id", "blocks", "groups"].contains(&k.as_str()) || !obj.contains_key(k)
                {
                    return Err(format!("状態 {name}: ブロック {id} の {k} は変えられない"));
                }
                obj.insert(k.clone(), x.clone());
            }
        }
        serde_json::from_value(v).map_err(|e| format!("状態 {name}: {e}"))
    }
}

/// id が name の項目（blocks・groups の中）の JSON ポインタを集める
pub(crate) fn find_id(v: &Value, path: &str, id: &str, out: &mut Vec<String>) {
    let Some(a) = v.as_array() else { return };
    for (i, x) in a.iter().enumerate() {
        let p = format!("{path}/{i}");
        if x.get("id").and_then(Value::as_str) == Some(id) {
            out.push(p.clone());
        }
        for key in ["blocks", "groups", "t", "r", "d"] {
            if let Some(y) = x.get(key) {
                find_id(y, &format!("{p}/{key}"), id, out);
            }
        }
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
    /// 相対パスの基準（ユーザーカタログの項目の中ではカタログのフォルダ）
    base: PathBuf,
    /// 展開中のユーザーカタログの項目（正規化したパス, 項目）。循環を見つける
    users: Vec<(PathBuf, String)>,
    sf: Vec<Surface>,
    /// sf の各面の出どころ（zmx ブロックの取り込みの番号, 元の .zmx の面番号）。ピックアップを付け替える
    org: Vec<Option<(usize, usize)>>,
    /// zmx ブロックの取り込みごとの、元の系のピックアップ
    /// zmx ブロックごとのピックアップと厚さのソルブ
    zpicks: Vec<ZmxLinks>,
    /// 物体面の後の、最初の面までの距離（最初のブロックの前の空気）
    lead: f64,
    /// 鏡の数の偶奇（−1 なら光は −z へ進む）
    parity: f64,
    /// 系のガラス（プロジェクトのカタログ＋玉・ユーザーカタログで足したもの）。足す名前がなければ共有のまま
    glasses: std::sync::Arc<HashMap<String, Glass>>,
    /// ユーザーカタログの `coatings`（TABLE 膜）
    coats: HashMap<String, lensbench_core::coating::Table>,
    nstop: usize,
    /// id のあるブロック・群の面の範囲（id, sf の最初, 最後）。玉・群のパラメータにする
    spans: Vec<(String, usize, usize)>,
    /// spans のうち、鏡の back で復路にたどった範囲（面の値の上書きを往路から当てる）
    back_spans: Vec<(String, usize, usize)>,
    /// id のある空気のブロックが入った面（id, 面番号, 向き）。差し替え探索の間隔の変数にする
    gaps: Vec<Gap>,
    /// ビームスプリッターの経路
    split: SplitState,
    /// 枠付きのユーザーカタログの品（名前, 枠, sf の最初, 最後）。枠を機構部品として置く
    frames: Vec<(String, MechSpec, usize, usize)>,
}

/// 品の枠（向きを当てる前）と向き
type ItemFrame = Option<(MechSpec, bool)>;

fn is_air(m: &str) -> bool {
    m.is_empty() || m.eq_ignore_ascii_case("AIR")
}

/// 結晶板の面の元（波長板の面の半直径・開口を写した平面の標準面）
fn plate_face(s: &Surface) -> Surface {
    Surface {
        semi: s.semi,
        semi_auto: s.semi_auto,
        ap: s.ap,
        ..Default::default()
    }
}

/// cos・sin の丸め誤差（6e-17 など）を 0・±1 に
fn round_unit(v: f64) -> f64 {
    if v.abs() < 1e-15 {
        0.0
    } else if (v.abs() - 1.0).abs() < 1e-15 {
        v.signum()
    } else {
        v
    }
}

/// 強度透過率 T の板の外側の面 1 つの理想膜（√T。OpticStudio の I.xx と同じ書き方で、1 は I.99999999）
fn ideal_face(t: f64) -> String {
    let f = (t.sqrt() * 1e8).floor().min(99_999_999.0) as u64;
    format!("I.{f:08}")
}

/// 面の並びのブロックの面の後の媒質: 空気（座標ブレークも）のほかは、複屈折材の板の入口の後（結晶）と、
/// 貼り合わせの出口の後（次の入口へ厚さ 0）だけ。板の入口と出口は交互に並び、最後は空気
fn check_crystal_order(v: &[Surface]) -> Result<(), String> {
    use lensbench_core::surface::Bire;
    let mut inside = false;
    for (i, s) in v.iter().enumerate() {
        match (&s.bire, inside) {
            (Some(Bire::In { .. }), false) => inside = true,
            (Some(Bire::Out), true) => {
                inside = false;
                if !is_air(&s.mat) {
                    let next_in = matches!(
                        v.get(i + 1).and_then(|n| n.bire.as_ref()),
                        Some(Bire::In { .. })
                    );
                    if !next_in || s.t != 0.0 || v[i + 1].mat != s.mat {
                        return Err(
                            "複屈折材の板の出口の後のガラスは、厚さ 0 で同じ材料の次の板の入口へ"
                                .into(),
                        );
                    }
                }
            }
            (Some(Bire::In { .. }), true) => {
                return Err("複屈折材の板の入口が続く（出口が要る）".into())
            }
            (Some(Bire::Out), false) => return Err("複屈折材の板の出口の前に入口が無い".into()),
            (None, true) => return Err("複屈折材の板の入口と出口の間にほかの面は置けない".into()),
            (None, false) => {
                if s.typ != SurfType::CoordBrk && !is_air(&s.mat) {
                    return Err("面の並びのブロックは空気中の面だけ（ガラスは玉で）".into());
                }
            }
        }
    }
    if inside {
        return Err("複屈折材の板に出口が無い".into());
    }
    Ok(())
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

/// zmx ブロックで取り込んだ .zmx のピックアップと厚さのソルブ（面番号は取り込んだ .zmx の番号）
struct ZmxLinks {
    path: String,
    pickups: Vec<Pickup>,
    solves: Vec<ThickSolve>,
    /// 取り込んだ .zmx の像面の番号
    image: usize,
}

impl<'a, 'b> Ctx<'a, 'b> {
    /// 複屈折材の波長板（`waveplate` の `material`・`plates`）を、厚さのある結晶板の面（入口・出口）に置き換える。
    /// 板ごとに入口（後ろは常光の材料、厚さは板の厚さ、結晶軸は面内で遅い軸の向き）と出口（後ろは次の板の材料か空気）。
    /// 外側の 2 面の膜は波長板の面の `coat`、無ければ透過率から理想膜（1 面あたり √T、T = 1 は I.99999999）
    fn crystal_plates(&self, v: Vec<Surface>) -> Result<Vec<Surface>, String> {
        let mut out = Vec::with_capacity(v.len());
        for s in v {
            let Some(r0) = s.retarder.as_ref().filter(|r| !r.plates.is_empty()) else {
                out.push(s);
                continue;
            };
            if s.before.is_some() {
                return Err("複屈折材の波長板に tilt は未対応".into());
            }
            let mut r = r0.clone();
            r.resolve(&self.glasses)?;
            let coat = if !s.coat.is_empty() {
                s.coat.clone()
            } else {
                ideal_face(r.tx * r.tx)
            };
            let n = r.plates.len();
            for (i, p) in r.plates.iter().enumerate() {
                // 結晶軸（光学軸）は異常光の向き: ne > no なら遅い軸 = 速い軸 + 90°
                let fast = r.axis_deg + p.axis_deg;
                let oa = if p.dn(r.wl0) > 0.0 { fast + 90.0 } else { fast }.to_radians();
                let axis = [round_unit(oa.cos()), round_unit(oa.sin()), 0.0];
                out.push(Surface {
                    mat: p.material.clone(),
                    t: p.thickness,
                    stop: s.stop && i == 0,
                    coat: if i == 0 { coat.clone() } else { String::new() },
                    comment: if i == 0 {
                        s.comment.clone()
                    } else {
                        String::new()
                    },
                    bire: Some(lensbench_core::surface::Bire::In {
                        mode: 2,
                        axis,
                        ne: Vec::new(),
                    }),
                    ..plate_face(&s)
                });
                let last = i + 1 == n;
                out.push(Surface {
                    mat: if last {
                        String::new()
                    } else {
                        r.plates[i + 1].material.clone()
                    },
                    t: if last { s.t } else { 0.0 },
                    coat: if last { coat.clone() } else { String::new() },
                    bire: Some(lensbench_core::surface::Bire::Out),
                    ..plate_face(&s)
                });
            }
        }
        Ok(out)
    }

    /// 面をそのまま足す（出どころなし）
    fn put(&mut self, s: Surface) {
        self.sf.push(s);
        self.org.push(None);
    }

    /// id の空気が入る面（gap の前に呼ぶ）
    fn note_gap(&mut self, id: &str) {
        if !id.is_empty() {
            self.gaps.push((id.to_string(), self.sf.len(), self.parity));
        }
    }

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
        self.put(s);
    }

    /// sf の start 以降に鏡があるか。鏡を含む群の移動は枚数によらず断る:
    /// 偶数枚で向きが戻っても、出ていく軸が入ってくる軸からずれると間隔の補正では表せない
    fn has_mirror_since(&self, start: usize) -> bool {
        self.sf[start.min(self.sf.len())..].iter().any(|s| s.mirror)
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
                self.org.truncate(start);
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
        self.put(cb([dx2, dy2, -tx, -ty, -tz], 1, t2 + tail));
        Ok(())
    }

    /// OpticStudio の座標のままの要素（.zmx から取り込んだもの、鏡のあとは負の厚さ）を足す。
    /// sign は取り込み始めの向き（逆なら全部の符号を返す）
    fn element(&mut self, e: &Element, sign: f64) {
        self.element_from(e, sign, None)
    }

    /// element と同じ。inst は zmx ブロックの取り込みの番号（面の出どころ = 要素の元の面番号から）
    fn element_from(&mut self, e: &Element, sign: f64, inst: Option<usize>) {
        if e.kind == Kind::Air {
            match self.sf.last_mut() {
                Some(s) => s.t += e.t * sign,
                None => self.lead += e.t * sign,
            }
            return;
        }
        for (i, s) in e.surfaces.iter().enumerate() {
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
            self.org.push(inst.map(|k| (k, e.from + i)));
        }
    }

    /// id のあるブロックが sf の start から足した面を、玉・群の範囲として覚える
    fn note(&mut self, id: &str, start: usize) {
        if !id.is_empty() && self.sf.len() > start {
            let v = (id.to_string(), start, self.sf.len() - 1);
            if !self.spans.contains(&v) {
                self.spans.push(v);
            }
        }
    }

    /// 往路のブロックを復路でたどる（鏡のあと）
    fn back_block(&mut self, b: &Block) -> Result<(), String> {
        let start = self.sf.len();
        self.back_block_inner(b)?;
        self.note(block_id(b), start);
        let id = block_id(b);
        if !id.is_empty() && self.sf.len() > start {
            let v = (id.to_string(), start, self.sf.len() - 1);
            if !self.back_spans.contains(&v) {
                self.back_spans.push(v);
            }
        }
        Ok(())
    }

    fn back_block_inner(&mut self, b: &Block) -> Result<(), String> {
        match b {
            Block::Air { id, t } => {
                self.note_gap(id);
                self.gap(*t)
            }
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
                if v.iter().any(|s| {
                    matches!(s.typ, SurfType::Jones | SurfType::CoordBrk) || s.bire.is_some()
                }) {
                    return Err(
                        "往路を戻る範囲の面の並びに jones・coordbrk・複屈折材の面は未対応".into(),
                    );
                }
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
            Block::Group {
                blocks,
                place,
                shift,
                ..
            } => {
                if place.moved() {
                    return Err(
                        "往路を戻る範囲の、偏心・傾きをつけた群は未対応（群の中の玉に付ける）"
                            .into(),
                    );
                }
                if !place.on {
                    return Err("往路を戻る範囲の、OFF の群は未対応".into());
                }
                // 復路では後の間隔が先: 往路で引いた分を引き、群の後（往路の前の間隔）で足す
                self.gap(-shift);
                for x in blocks.iter().rev() {
                    self.back_block(x)?;
                    if self.split.stopped {
                        return Ok(());
                    }
                }
                self.gap(*shift);
            }
            Block::User { .. } => {
                let (g, dir, key) = self.user_group(b)?;
                let base = std::mem::replace(&mut self.base, dir);
                self.users.push(key);
                let r = self.back_block(&g);
                self.users.pop();
                self.base = base;
                r?;
            }
            Block::Mirror { .. } => return Err("往路を戻る範囲に鏡がある".into()),
            Block::Splitter { id, spec, .. } => {
                if spec.has_branches() {
                    return Err(format!(
                        "BS {id}: 分岐のある BS を後ろから戻ることはない（内部エラー）"
                    ));
                }
                self.splitter_back(id)?;
            }
            Block::Detector(_) => return Err("検出器は最後のブロックだけ".into()),
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
        self.put(cb([-dx2, -dy2, tx, ty, tz], 0, 0.0));
        for s in rev {
            self.push(s.clone());
        }
        if let Some(s) = self.sf.last_mut() {
            s.t = 0.0;
        }
        self.put(cb([-dx, -dy, -tx, -ty, -tz], 1, 0.0));
        Ok(())
    }

    fn load_zmx(&mut self, path: &str, cfg: usize) -> Result<System, String> {
        if cfg == 0 {
            return Err(format!("{path}: 構成の番号は 1 以上"));
        }
        let p = self.base.join(path);
        let opt = zmx::LoadOptions {
            cfg,
            catalog_dirs: self.env.glass_dirs.clone(),
            ..Default::default()
        };
        let s = zmx::load_with(&p, &opt).map_err(|e| format!("{}: {e}", p.display()))?;
        self.merge_glasses(&s);
        Ok(s)
    }

    /// ユーザーカタログの項目を群にする（と、その中の相対パスの基準）。項目のガラスカタログを足す
    fn user_group(&mut self, b: &Block) -> Result<(Block, PathBuf, (PathBuf, String)), String> {
        self.user_group_frame(b).map(|(g, d, k, _)| (g, d, k))
    }

    /// [`Self::user_group`] と、品の枠
    #[allow(clippy::type_complexity)]
    fn user_group_frame(
        &mut self,
        b: &Block,
    ) -> Result<(Block, PathBuf, (PathBuf, String), ItemFrame), String> {
        let Block::User {
            id,
            catalog,
            item,
            place,
        } = b
        else {
            unreachable!()
        };
        let path = self.base.join(catalog);
        let key = (
            lensbench_core::vfs::canonicalize(&path).unwrap_or_else(|_| path.clone()),
            item.clone(),
        );
        if self.users.contains(&key) {
            return Err(format!(
                "{}: 項目 {item} が自分自身を参照している（循環）",
                path.display()
            ));
        }
        let cat = UserCatalog::load_shared(&path)?;
        let it = cat
            .items
            .get(item)
            .ok_or_else(|| format!("{}: 項目 {item} はない", path.display()))?;
        let dirs = glass::catalog_dirs(&self.env.glass_dirs);
        let (g, missing) = glass::load_catalogs_shared(&it.glass_catalogs, &dirs);
        if !missing.is_empty() {
            return Err(format!(
                "{item}: ガラスカタログが見つからない: {}",
                missing.join(", ")
            ));
        }
        self.add_glasses(&g);
        // カタログの膜（フィルタの透過特性）。同じ名前で中身の違う膜は誤り
        for (k, spec) in &cat.coatings {
            let t = spec
                .table()
                .map_err(|e| format!("{}: 膜 {k}: {e}", path.display()))?;
            match self.coats.get(k) {
                Some(old) if *old != t => {
                    return Err(format!("{}: 膜 {k} がほかのカタログと違う", path.display()))
                }
                Some(_) => {}
                None => {
                    self.coats.insert(k.clone(), t);
                }
            }
        }
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok((
            Block::Group {
                id: id.clone(),
                name: item.clone(),
                role: None,
                shift: 0.0,
                blocks: it.blocks.clone(),
                place: place.clone(),
            },
            dir,
            key,
            it.frame.clone().map(|f| (f, it.frame_reversed)),
        ))
    }

    fn merge_glasses(&mut self, s: &System) {
        self.add_glasses(&s.glasses);
    }

    /// 無い名前のガラスだけ足す（全部あれば写さない）
    fn add_glasses(&mut self, g: &std::sync::Arc<HashMap<String, Glass>>) {
        if std::sync::Arc::ptr_eq(&self.glasses, g)
            || g.keys().all(|k| self.glasses.contains_key(k))
        {
            return;
        }
        if self.glasses.is_empty() {
            self.glasses = g.clone();
            return;
        }
        let m = std::sync::Arc::make_mut(&mut self.glasses);
        for (k, v) in g.iter() {
            m.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    fn lens_element(&mut self, src: &LensSource) -> Result<Element, String> {
        match src {
            LensSource::Zmx { zmx, lens, config } => {
                let s = self.load_zmx(zmx, *config)?;
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
                        fixed_semi: true,
                        ..Default::default()
                    },
                    &glass::catalog_dirs(&self.env.glass_dirs),
                    None,
                )
                .map_err(|e| format!("{vendor} {part}: {e}"))?;
                self.merge_glasses(&s);
                // カタログの玉は径が決まっているので半直径は固定（fixed_semi）
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
        let start = self.sf.len();
        self.block_inner(b, siblings, idx)?;
        // 鏡は鏡の面だけ（往路を戻る面は除く）。中で覚える
        if !matches!(b, Block::Mirror { .. } | Block::Splitter { .. }) {
            self.note(block_id(b), start);
        }
        Ok(())
    }

    fn block_inner(&mut self, b: &Block, siblings: &[Block], idx: usize) -> Result<(), String> {
        match b {
            Block::Air { id, t } => {
                self.note_gap(id);
                self.gap(*t)
            }
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
                let v = self.crystal_plates(v)?;
                check_crystal_order(&v)?;
                self.placed(place, &mut |c: &mut Self| {
                    for s in &v {
                        c.push(s.clone());
                    }
                    Ok(())
                })?;
            }
            Block::Mirror {
                id,
                radius,
                conic,
                tilt,
                semi,
                aperture,
                coat,
                on,
                back,
            } => {
                if !on {
                    return Ok(());
                }
                let start = self.sf.len();
                let back = *back;
                let fold = tilt[0] != 0.0 || tilt[1] != 0.0;
                if fold {
                    self.put(cb([0.0, 0.0, tilt[0], tilt[1], 0.0], 0, 0.0));
                }
                let mut m = Surface {
                    mirror: true,
                    mat: "MIRROR".into(),
                    c: curv(*radius),
                    k: *conic,
                    semi: *semi,
                    coat: coat.clone(),
                    ap: surf_ap(*aperture, *semi),
                    ..Default::default()
                };
                if self.parity < 0.0 {
                    m.c = -m.c;
                }
                self.put(m);
                self.parity = -self.parity;
                if fold && back {
                    // 往路を戻る鏡の傾き（据え付けの誤差など）: 鏡だけを傾け、座標は元の軸へ戻す（順序 1 で逆の傾き）。
                    // 復路の面は往路と同じ位置にあり、光線は鏡の傾きの 2 倍の角でそこを戻る
                    self.put(cb([0.0, 0.0, -tilt[0], -tilt[1], 0.0], 1, 0.0));
                } else if fold {
                    self.put(cb([0.0, 0.0, tilt[0], tilt[1], 0.0], 0, 0.0));
                }
                self.note(id, start);
                if back {
                    // 前のブロックを戻る。BS で A 以外の口へ出たら止まり、並びの初めまで戻れば（BS の分岐なら）BS を通る
                    self.split.stopped = false;
                    for x in siblings[..idx].iter().rev() {
                        self.back_block(x)?;
                        if self.split.stopped {
                            break;
                        }
                    }
                    if !self.split.stopped {
                        let lv = self.split.levels.last().cloned().unwrap_or(Level::Other);
                        self.return_at(lv)?;
                    }
                    self.split.stopped = false;
                    if self.split.done && idx + 1 < siblings.len() {
                        return Err(
                            "鏡の後のブロックと、BS の分岐の両方がある（復路が BS の分岐へ進む経路）".into(),
                        );
                    }
                }
            }
            Block::Zmx {
                path,
                from,
                to,
                config,
                stop,
                groups,
                place,
                ..
            } => {
                let s = self.load_zmx(path, *config)?;
                let els = elements::split(&s).map_err(|e| format!("{path}: {e}"))?;
                let inst = self.zpicks.len();
                self.zpicks.push(ZmxLinks {
                    path: path.clone(),
                    pickups: s.pickups.clone(),
                    solves: s.solves.clone(),
                    image: s.image(),
                });
                let (a, z) = (from.unwrap_or(1), to.unwrap_or(s.image() - 1));
                // 範囲の中の要素と、その直後の空気だけを取る（範囲の外の要素の後の空気は除く）
                let mut keep = Vec::new();
                let mut last_in = false;
                for e in els {
                    if e.kind == Kind::Air {
                        if last_in {
                            keep.push(e);
                        }
                    } else {
                        last_in = e.from >= a && e.from <= z;
                        if last_in {
                            keep.push(e);
                        }
                    }
                }
                if !*stop {
                    for e in &mut keep {
                        e.surfaces.iter_mut().for_each(|s| s.stop = false);
                    }
                }
                let spans = zmx_group_spans(&keep, groups).map_err(|e| format!("{path}: {e}"))?;
                self.placed(place, &mut |c: &mut Self| {
                    let sign = c.parity;
                    let mut i = 0;
                    while i < keep.len() {
                        let Some(&(g, a, z)) = spans.iter().find(|q| q.1 == i) else {
                            c.element_from(&keep[i], sign, Some(inst));
                            i += 1;
                            continue;
                        };
                        let gr = &groups[g];
                        c.element(&Element::air(gr.shift), sign);
                        let start = c.sf.len();
                        c.placed(&gr.place, &mut |c: &mut Self| {
                            for e in &keep[a..=z] {
                                c.element_from(e, sign, Some(inst));
                            }
                            Ok(())
                        })?;
                        c.note(&gr.id, start);
                        if gr.shift != 0.0 && c.has_mirror_since(start) {
                            return Err("鏡を含む群は動かせない（shift は 0 に）".into());
                        }
                        c.element(&Element::air(-gr.shift), sign);
                        i = z + 1;
                    }
                    Ok(())
                })?;
            }
            Block::User { id, item, .. } => {
                let (g, dir, key, frame) = self.user_group_frame(b)?;
                let base = std::mem::replace(&mut self.base, dir);
                self.users.push(key);
                let start = self.sf.len();
                let r = self.block(&g, &[], 0);
                self.users.pop();
                self.base = base;
                r?;
                // 品の枠は、外側の品（項目の中の項目でない）だけ置く
                if let Some((f, rev)) =
                    frame.filter(|_| self.users.is_empty() && self.sf.len() > start)
                {
                    let name = if id.is_empty() {
                        item.clone()
                    } else {
                        id.clone()
                    };
                    // 逆向きの枠は、品のガラスの厚さ（最初の面から最後の面まで）で座を移す
                    let f = if rev {
                        let t: f64 = self.sf[start..self.sf.len() - 1]
                            .iter()
                            .filter(|q| q.typ != SurfType::CoordBrk)
                            .map(|q| q.t)
                            .sum();
                        f.flipped(t)
                    } else {
                        f
                    };
                    self.frames.push((name, f, start, self.sf.len() - 1));
                }
            }
            Block::Splitter { id, spec, place } => {
                self.splitter_forward(id, spec, place, siblings, idx)?;
            }
            Block::Detector(_) => {
                return Err(
                    "検出器は最後のブロックだけ（群・ユーザーカタログの中には置けない）".into(),
                )
            }
            Block::Group {
                blocks,
                place,
                shift,
                ..
            } => {
                let bl = blocks.clone();
                self.gap(*shift);
                let start = self.sf.len();
                self.split.levels.push(Level::Other);
                let r = self.placed(place, &mut |c: &mut Self| {
                    for (i, x) in bl.iter().enumerate() {
                        if c.split.done {
                            break;
                        }
                        c.block(x, &bl, i)?;
                    }
                    Ok(())
                });
                self.split.levels.pop();
                r?;
                if *shift != 0.0 && self.has_mirror_since(start) {
                    return Err("鏡を含む群は動かせない（shift は 0 に）".into());
                }
                self.gap(-shift);
            }
        }
        Ok(())
    }
}

/// ブロックの id
fn block_id(b: &Block) -> &str {
    match b {
        Block::Lens { id, .. }
        | Block::Air { id, .. }
        | Block::Stop { id, .. }
        | Block::Mirror { id, .. }
        | Block::Surfaces { id, .. }
        | Block::Zmx { id, .. }
        | Block::User { id, .. }
        | Block::Detector(DetectorBlock { id, .. })
        | Block::Splitter { id, .. }
        | Block::Group { id, .. } => id,
    }
}

/// ユーザーカタログ（JSON）: 名前 → 保存した玉・ブロックの並び
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserCatalog {
    #[serde(default = "one")]
    pub version: usize,
    /// カタログブラウザに出すメーカー（空ならファイルのあるフォルダの名前）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vendor: String,
    #[serde(default)]
    pub items: BTreeMap<String, UserItem>,
    /// 品の url が無いときの製品ページ（`{part}` を型番に置き換える）。同じメーカーのほかのカタログ（ZMF・.zmx）の品にも当てる
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url_template: String,
    /// 同じメーカーのほかのカタログの品へ足す仕様・リンク: 型番のパターン（`*`・`?`）→ 中身
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overlay: BTreeMap<String, catalog_spec::Overlay>,
    /// カタログの品が使う膜（名前 → フィルタの透過特性・表）。品の面の `coat` に名前を書く
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub coatings: BTreeMap<String, filter::CoatSpec>,
}

impl Default for UserCatalog {
    fn default() -> Self {
        UserCatalog {
            version: 1,
            vendor: String::new(),
            items: BTreeMap::new(),
            url_template: String::new(),
            overlay: BTreeMap::new(),
            coatings: BTreeMap::new(),
        }
    }
}

/// ユーザーカタログの項目
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct UserItem {
    #[serde(default)]
    pub description: String,
    /// 項目の玉のガラスのカタログ（使うときに足す）
    #[serde(default)]
    pub glass_catalogs: Vec<String>,
    /// 光学の面（機構部品は空）
    #[serde(default)]
    pub blocks: Vec<Block>,
    /// 機構部品の寸法
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mech: Option<MechSpec>,
    /// 枠付きの品（マウント済みのフィルタなど）の枠。`blocks` はガラスだけで、枠は z を枠の端から光の進む
    /// 向きに書き、`holds.seat_z` にガラスの前面の位置を書く。プロジェクトに置くと、ガラスの最初の面に
    /// 機構部品として付く（ケラレ・当たりの確認と描画）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<MechSpec>,
    /// 枠の向き: false なら光は枠の z の小さい端から入り、true なら大きい端から入る（STEP から測った枠は STEP の
    /// 座標の向きなので、図面の矢印などで決める）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub frame_reversed: bool,
    /// 品の種類（一覧の分類。省略時はブロックから: 鏡・BS のブロック 1 つならそれ、ほかは玉）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<catalog_spec::ItemKind>,
    /// 製品ページ
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// 仕様（名前 → 数・[下限, 上限]・文字列。名前の決まったものは `catalog_spec::SPEC_NAMES`）
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub specs: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// 仕様の出典（資料・ページ）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// 仕様を確かめた日（YYYY-MM-DD）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checked: String,
    /// 未完の品の足りない値（`thickness`・`diameter`）。blocks の無い品で空でなければ、配置できない「未完の品」
    /// として並べ、品ページの仕様表か STEP で補ってから使う（[`catalog_fill`]）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// 品の名前（一覧のページの見出しなど）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// データを作ったときのメモ（読めなかった欄など）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl UserItem {
    /// 未完の品（足りない値があり、面も機構の寸法も無い）か
    pub fn is_incomplete(&self) -> bool {
        !self.missing.is_empty() && self.blocks.is_empty() && self.mech.is_none()
    }
}

/// 機構部品の穴（部品の基準面からの z の範囲と内径）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BoreSpec {
    #[serde(default)]
    pub z0: f64,
    pub z1: f64,
    /// 内径・クリア開口 [mm]
    pub diameter: f64,
}

/// 機構部品の寸法。z は部品の基準面（チューブの入口の端など）から光の進む向き
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, JsonSchema)]
pub struct MechSpec {
    /// 光の通る穴（チューブの内径、ケージ板・マウントのクリア開口）
    #[serde(default)]
    pub bores: Vec<BoreSpec>,
    /// 長さ [mm]（外形の z の範囲 0〜length。0 なら穴の範囲）
    #[serde(default)]
    pub length: f64,
    /// 外径 [mm]（ケージ板は外形の一辺）
    #[serde(default)]
    pub outer_diameter: f64,
    /// ねじ（"SM1" など）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thread: String,
    /// ケージのロッドの間隔 [mm]
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub rod_spacing: f64,
    /// 保持する素子の径 [mm] と座の z [mm]（マウント・チューブ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holds: Option<HoldSpec>,
    /// z 0 の端（光の入る側）のねじ。機構部品をつなぐ（[`MechPlacement::after`]）とき、合うかを確かめる
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front_thread: Option<ThreadEnd>,
    /// z の大きい端のねじ
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_thread: Option<ThreadEnd>,
}

/// 機構部品の端のねじ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ThreadEnd {
    /// ねじの名前（"SM1" など）。つなぐ相手と同じ名前なら合う
    pub thread: String,
    /// オス（外側のねじ）なら true、メス（内側のねじ）なら false
    pub male: bool,
    /// ねじの長さ [mm]。オスのねじは相手のメスにこの長さだけねじ込む（部品が重なる）
    #[serde(default)]
    pub length: f64,
}

/// 保持する素子
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HoldSpec {
    pub diameter: f64,
    /// 素子の前面を当てる座の z [mm]
    #[serde(default)]
    pub seat_z: f64,
}

fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

impl MechSpec {
    /// 光の進む向きを逆にした寸法（z を長さから引く）。`t` は保持する素子の厚さ（座はその前面）
    pub fn flipped(&self, t: f64) -> MechSpec {
        let len = if self.length > 0.0 {
            self.length
        } else {
            self.bores.iter().map(|b| b.z1).fold(0.0, f64::max)
        };
        let mut bores: Vec<BoreSpec> = self
            .bores
            .iter()
            .map(|b| BoreSpec {
                z0: len - b.z1,
                z1: len - b.z0,
                diameter: b.diameter,
            })
            .collect();
        bores.sort_by(|a, b| a.z0.total_cmp(&b.z0));
        MechSpec {
            bores,
            holds: self.holds.map(|h| HoldSpec {
                diameter: h.diameter,
                seat_z: len - h.seat_z - t,
            }),
            front_thread: self.back_thread.clone(),
            back_thread: self.front_thread.clone(),
            ..self.clone()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for (k, b) in self.bores.iter().enumerate() {
            if !(b.diameter > 0.0 && b.diameter.is_finite()) {
                return Err(format!("機構の穴 {}: 内径 {}", k + 1, b.diameter));
            }
            if !(b.z0.is_finite() && b.z1.is_finite() && b.z1 >= b.z0) {
                return Err(format!("機構の穴 {}: z の範囲 {}〜{}", k + 1, b.z0, b.z1));
            }
        }
        if !(self.length >= 0.0 && self.outer_diameter >= 0.0) {
            return Err("機構の長さ・外径が負".into());
        }
        Ok(())
    }

    /// 系に置いた部品（[`lensbench_core::mech::MechPart`]）
    pub fn part(
        &self,
        name: &str,
        surface: usize,
        offset: [f64; 3],
        tilt: [f64; 2],
    ) -> lensbench_core::mech::MechPart {
        lensbench_core::mech::MechPart {
            name: name.to_string(),
            surface,
            offset,
            tilt,
            bores: self
                .bores
                .iter()
                .map(|b| lensbench_core::mech::Bore {
                    z0: b.z0,
                    z1: b.z1,
                    radius: b.diameter / 2.0,
                })
                .collect(),
            length: self.length,
            outer_radius: self.outer_diameter / 2.0,
            hold_radius: self.holds.map_or(0.0, |h| h.diameter / 2.0),
            rod_spacing: self.rod_spacing,
            holds: Vec::new(),
        }
    }
}

/// 系に置く機構部品（[`Project::mechanics`]）。部品はユーザーカタログの品（`catalog`・`item`）か、
/// その場で書いた寸法（`mech`）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MechPlacement {
    /// 名前（省略で品の名前か mech1, mech2, …）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// ユーザーカタログのファイル（プロジェクトのフォルダから）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mech: Option<MechSpec>,
    /// 置く基準: ブロック id の面（座標ブレークを除いた 0 始まりの番号、省略で 0）か、系の面の番号（`surface` だけ）。
    /// `seat` なら省略できる（使わない）
    #[serde(default, skip_serializing_if = "MechAt::is_unset")]
    pub at: MechAt,
    /// 基準の面の局所座標でのずれ [dx, dy, dz] [mm]
    #[serde(default)]
    pub offset: [f64; 3],
    /// 傾き [tx, ty] [deg]（ずらした後、x → y の順）
    #[serde(default)]
    pub tilt: [f64; 2],
    /// 保持する素子のブロック id。はみ出し・保持径を確かめる
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<String>,
    /// 部品の座（寸法の `holds.seat_z`）を、保持する最初の素子の前面に合わせて置く（`at` の代わり。`offset` は
    /// そこからのずれ）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub seat: bool,
    /// 前に書いた機構部品（名前）の z の大きい端につなぐ（`at`・`seat` の代わり。同じ名前が複数あれば最後のもの）。
    /// 前の部品と同じ面・傾きに置き、`offset` は前の部品の座標でのずれ、`tilt` は使えない。両方の部品の向き合う端に
    /// ねじ（`front_thread`・`back_thread`）があれば、同じねじでオスとメスか確かめ、オスのねじの長さだけ重ねる
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// 機構を置く基準の面（[`MechPlacement::at`]）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MechAt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    #[serde(default)]
    pub surface: usize,
}

impl MechAt {
    fn is_unset(&self) -> bool {
        self.block.is_none() && self.surface == 0
    }
}

impl UserCatalog {
    pub fn from_json(txt: &str) -> Result<UserCatalog, String> {
        serde_json::from_str(txt).map_err(|e| format!("ユーザーカタログ: {e}"))
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap()
    }
    /// ファイルから読む。`.gz` は展開して読み、`x.json` が無くて `x.json.gz` があればそれを読む（[`read_text`]）
    pub fn load(path: &Path) -> Result<UserCatalog, String> {
        let t = read_text(path)?;
        UserCatalog::from_json(&t).map_err(|e| format!("{}: {e}", path.display()))
    }
    /// [`Self::load`] と同じだが、読んだものをファイルの更新時刻・大きさと共に覚えておき、変わっていなければ
    /// 読み直さない（展開で品ごとに同じカタログを読むと、品の数の 2 乗の時間がかかるため）。読むだけの所で使う
    pub fn load_shared(path: &Path) -> Result<std::sync::Arc<UserCatalog>, String> {
        use std::sync::{Arc, Mutex, OnceLock};
        type Stamp = (Option<std::time::SystemTime>, u64);
        type Cache = HashMap<PathBuf, (Stamp, Arc<UserCatalog>)>;
        static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
        // `.gz` だけある場合など、更新時刻が取れなければ覚えずに読む
        let Ok(stamp) = lensbench_core::vfs::stamp(path) else {
            return UserCatalog::load(path).map(Arc::new);
        };
        let key = lensbench_core::vfs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let cache = CACHE.get_or_init(Default::default);
        if let Some((st, c)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            if *st == stamp {
                return Ok(c.clone());
            }
        }
        let c = Arc::new(UserCatalog::load(path)?);
        cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, (stamp, c.clone()));
        Ok(c)
    }
    /// ないファイル（`.gz` を足した名前も無い）なら空のカタログ。`.json.gz` だけあれば、それを読む
    /// （足して保存すると `.json` に書き、一覧では `.json` が勝つので、品は欠けない）
    pub fn load_or_new(path: &Path) -> Result<UserCatalog, String> {
        let mut gz = path.as_os_str().to_owned();
        gz.push(".gz");
        if lensbench_core::vfs::exists(path) || lensbench_core::vfs::is_file(Path::new(&gz)) {
            UserCatalog::load(path)
        } else {
            Ok(UserCatalog::default())
        }
    }
    /// 保存する。名前が `.gz` で終われば gzip で圧縮して書く
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        let err = |e: std::io::Error| format!("{}: {e}", path.display());
        let text = self.to_json() + "\n";
        let gz = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gz"));
        if gz {
            let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            w.write_all(text.as_bytes()).map_err(err)?;
            lensbench_core::vfs::write(path, w.finish().map_err(err)?).map_err(err)
        } else {
            lensbench_core::vfs::write(path, text).map_err(err)
        }
    }
}

/// テキストのファイルを読む。拡張子 `.gz`（またはファイルの頭が gzip の印）なら展開する。`path` が無くて
/// `path` に `.gz` を足したファイルがあればそれを読む（リリースのカタログは .json.gz で配る）
pub fn read_text(path: &Path) -> Result<String, String> {
    read_text_limit(path, GZ_LIMIT)
}

/// gzip を展開した大きさの上限 [byte]（展開爆弾で落ちないように。カタログは大きくても数十 MB）
pub const GZ_LIMIT: u64 = 256 << 20;

fn read_text_limit(path: &Path, limit: u64) -> Result<String, String> {
    use std::io::Read;
    let p = if !lensbench_core::vfs::exists(path) {
        let mut g = path.as_os_str().to_owned();
        g.push(".gz");
        let g = PathBuf::from(g);
        if lensbench_core::vfs::is_file(&g) {
            g
        } else {
            path.to_path_buf()
        }
    } else {
        path.to_path_buf()
    };
    let err = |e: std::io::Error| format!("{}: {e}", p.display());
    let b = lensbench_core::vfs::read(&p).map_err(err)?;
    let b = if b.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(&b[..])
            .take(limit + 1)
            .read_to_end(&mut out)
            .map_err(err)?;
        if out.len() as u64 > limit {
            return Err(format!(
                "{}: 展開すると {limit} バイトを超える",
                p.display()
            ));
        }
        out
    } else {
        b
    };
    String::from_utf8(b).map_err(|e| format!("{}: {e}", p.display()))
}

/// カタログのファイルか（`.json`・`.json.gz`）
pub fn is_catalog_file(p: &Path) -> bool {
    let n = p
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    n.ends_with(".json") || n.ends_with(".json.gz")
}

/// 玉（玉モードの要素）を、面を直接書いた玉のブロックにする（ほかのプロジェクトへ持ち出せる形）
pub fn lens_to_block(e: &Element, id: &str) -> Result<Block, String> {
    if e.kind != Kind::Lens {
        return Err("玉でない要素".into());
    }
    let n = e.surfaces.len();
    let mut v = Vec::new();
    for (i, s) in e.surfaces.iter().enumerate() {
        if !matches!(s.typ, SurfType::Standard | SurfType::EvenAsph) {
            return Err(format!(
                "玉の面 {}: 標準面・偶数次非球面だけ保存できる",
                i + 1
            ));
        }
        if s.mirror {
            return Err(format!("玉の面 {}: 鏡の面のある玉は保存できない", i + 1));
        }
        let glass = if i + 1 == n || is_air(&s.mat) {
            None
        } else if let Some((nd, vd)) = s.mat.split_once(',') {
            // モデルガラス（.zmx 読込の名前）: 分散なしだけ
            match (nd.trim().parse::<f64>(), vd.trim().parse::<f64>()) {
                (Ok(_), Ok(0.0)) => Some(format!("{}", s.n.first().copied().unwrap_or(0.0))),
                _ => {
                    return Err(format!(
                        "玉の面 {}: 分散のあるモデルガラス {} は保存できない",
                        i + 1,
                        s.mat
                    ))
                }
            }
        } else {
            Some(s.mat.clone())
        };
        v.push(SurfSpec {
            radius: (s.c != 0.0).then(|| 1.0 / s.c),
            conic: s.k,
            asph: if s.typ == SurfType::EvenAsph {
                s.asph.clone()
            } else {
                Vec::new()
            },
            t: if i + 1 == n { 0.0 } else { s.t },
            glass,
            // カタログの玉は径が決まっているので固定（自動ならいまの値）
            semi: Semi::Value(s.semi),
            coat: s.coat.clone(),
            comment: s.comment.clone(),
            ..Default::default()
        });
    }
    Ok(Block::Lens {
        id: id.to_string(),
        source: LensSource::Surfaces { surfaces: v },
        reversed: false,
        stop: false,
        place: Place::default(),
    })
}

/// 系の玉 k（1 始まり、玉だけを数える）をユーザーカタログの項目にする
pub fn lens_item(s: &System, k: usize, description: &str) -> Result<UserItem, String> {
    let e = pick_lens(s, k).map_err(|e| e.to_string())?;
    let mut cats: Vec<String> = Vec::new();
    for d in &e.surfaces {
        if let Some(g) = s.glasses.get(&d.mat) {
            if !g.catalog.is_empty() && !cats.contains(&g.catalog) {
                cats.push(g.catalog.clone());
            }
        }
    }
    Ok(UserItem {
        description: description.to_string(),
        glass_catalogs: cats,
        blocks: vec![lens_to_block(&e, "")?],
        ..Default::default()
    })
}

/// プロジェクトのブロック id（入れ子も探す）をユーザーカタログの項目にする。
/// .zmx の玉は面を直接書いた玉に、.zmx の取り込み・ユーザーカタログの参照のパスは絶対パスにする
pub fn block_item(p: &Project, env: &Env, id: &str, description: &str) -> Result<UserItem, String> {
    fn find<'x>(v: &'x [Block], id: &str, out: &mut Vec<&'x Block>) {
        for b in v {
            let bid = match b {
                Block::Lens { id, .. }
                | Block::Air { id, .. }
                | Block::Stop { id, .. }
                | Block::Mirror { id, .. }
                | Block::Surfaces { id, .. }
                | Block::Zmx { id, .. }
                | Block::User { id, .. }
                | Block::Detector(DetectorBlock { id, .. })
                | Block::Splitter { id, .. }
                | Block::Group { id, .. } => id,
            };
            if !id.is_empty() && bid == id {
                out.push(b);
            }
            if let Block::Group { blocks, .. } = b {
                find(blocks, id, out);
            }
            if let Block::Splitter { spec, .. } = b {
                for l in [&spec.t, &spec.r, &spec.d] {
                    find(l, id, out);
                }
            }
        }
    }
    let mut hits = Vec::new();
    find(&p.blocks, id, &mut hits);
    let b = match hits.as_slice() {
        [b] => (*b).clone(),
        [] => return Err(format!("ブロック {id} はない")),
        _ => return Err(format!("id {id} のブロックが 2 つ以上ある")),
    };
    let mut c = Ctx {
        env,
        base: env.base.clone(),
        users: Vec::new(),
        sf: Vec::new(),
        org: Vec::new(),
        zpicks: Vec::new(),
        lead: 0.0,
        parity: 1.0,
        glasses: Default::default(),
        coats: HashMap::new(),
        nstop: 0,
        spans: Vec::new(),
        back_spans: Vec::new(),
        gaps: Vec::new(),
        split: SplitState::default(),
        frames: Vec::new(),
    };
    let mut cats = p.glass_catalogs.clone();
    let b = portable(&mut c, b, &mut cats)?;
    Ok(UserItem {
        description: description.to_string(),
        glass_catalogs: cats,
        blocks: vec![b],
        ..Default::default()
    })
}

/// プロジェクトの最上位の並びの、id `ids[0]` から `ids[last]` までのブロック（間の空気も含む）をユーザーカタログの
/// 項目にする（GUI の組合せの要素・範囲の保存）。`ids` が 1 つなら [`block_item`] と同じ（入れ子も探す）。
/// 面の表で変えた値（[`Project::surface_edits`]）は中身に焼き込む: 変えた玉は展開した系の面を直接書いた玉にする
/// （玉のほかのブロックの変更は焼き込めないので誤り）
pub fn blocks_item(
    p: &Project,
    env: &Env,
    ids: &[String],
    description: &str,
) -> Result<UserItem, String> {
    let (first, last) = match ids {
        [] => return Err("保存するブロックの id がない".into()),
        [a] => (a, a),
        [a, .., b] => (a, b),
    };
    // 範囲（最上位の並び）
    let top = |id: &str| -> Result<usize, String> {
        let v: Vec<usize> = p
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| !id.is_empty() && block_id(b) == id)
            .map(|(i, _)| i)
            .collect();
        match v.as_slice() {
            [i] => Ok(*i),
            [] => Err(format!("ブロック {id} は最上位の並びにない")),
            _ => Err(format!("id {id} のブロックが 2 つ以上ある")),
        }
    };
    let mut item = if ids.len() == 1 {
        block_item(p, env, first, description)?
    } else {
        let (i, j) = (top(first)?, top(last)?);
        if i > j {
            return Err(format!("{first} は {last} より後ろ"));
        }
        let mut blocks = Vec::new();
        let mut cats = Vec::new();
        for b in &p.blocks[i..=j] {
            let id = block_id(b);
            if id.is_empty() {
                // 空気・絞りなど id の無いブロックはそのまま
                blocks.push(b.clone());
                continue;
            }
            let it = block_item(p, env, id, "")?;
            for c in it.glass_catalogs {
                if !cats.contains(&c) {
                    cats.push(c);
                }
            }
            blocks.extend(it.blocks);
        }
        UserItem {
            description: description.to_string(),
            glass_catalogs: cats,
            blocks,
            ..Default::default()
        }
    };
    // 面の表の変更を焼き込む
    let edited: Vec<&str> = p
        .surface_edits
        .iter()
        .map(|e| e.block.as_str())
        .filter(|b| item_has_id(&item.blocks, b))
        .collect();
    if edited.is_empty() {
        return Ok(item);
    }
    let s = expand(p, env)?;
    for id in edited {
        bake_lens(&s, &mut item, id)?;
    }
    Ok(item)
}

fn item_has_id(blocks: &[Block], id: &str) -> bool {
    blocks.iter().any(|b| {
        block_id(b) == id || matches!(b, Block::Group { blocks, .. } if item_has_id(blocks, id))
    })
}

/// 展開した系 s の玉 id の面（面の表の変更を当てた後）で、項目の玉 id を面を直接書いた玉にする
fn bake_lens(s: &System, item: &mut UserItem, id: &str) -> Result<(), String> {
    fn find<'x>(v: &'x mut [Block], id: &str) -> Option<&'x mut Block> {
        for b in v {
            if block_id(b) == id {
                return Some(b);
            }
            if let Block::Group { blocks, .. } = b {
                if let Some(x) = find(blocks, id) {
                    return Some(x);
                }
            }
        }
        None
    }
    let nope = || {
        format!(
            "ブロック {id}: 面の表で変えた値を焼き込めるのは玉だけ（ほかは変更を戻してから保存）"
        )
    };
    let b = find(&mut item.blocks, id).ok_or_else(nope)?;
    let Block::Lens {
        reversed,
        source,
        place,
        ..
    } = b
    else {
        return Err(nope());
    };
    let u = s
        .units
        .iter()
        .find(|u| u.name == id)
        .ok_or_else(|| format!("ブロック {id} の面が系に無い"))?;
    let [a, z] = u.spans[0];
    if s.surfaces[..a].iter().any(|x| x.mirror) {
        return Err(format!(
            "ブロック {id}: 鏡の後の玉の変更は焼き込めない（未対応）"
        ));
    }
    // 面の表の番号と同じく、座標ブレークを除いた面（偏心・傾きの座標ブレークは place のまま残る）
    let faces: Vec<&Surface> = (a..=z)
        .map(|j| &s.surfaces[j])
        .filter(|x| x.typ != SurfType::CoordBrk)
        .collect();
    let span_cbs = (a..=z)
        .filter(|&j| s.surfaces[j].typ == SurfType::CoordBrk)
        .count();
    let place_cbs = if place.moved() { 2 } else { 0 };
    // 元の面の並び（面を直接書いた玉なら、中の座標ブレークはそのまま残す）
    let orig: Option<Vec<SurfSpec>> = match &*source {
        LensSource::Surfaces { surfaces } => Some(surfaces.clone()),
        _ => None,
    };
    let inner_cbs = orig
        .as_ref()
        .map_or(0, |v| v.iter().filter(|q| q.coordbrk.is_some()).count());
    if span_cbs != place_cbs + inner_cbs || (*reversed && inner_cbs > 0) {
        return Err(format!(
            "ブロック {id}: 中に座標ブレークのある玉の変更は焼き込めない（未対応）"
        ));
    }
    let n = faces.len();
    let mut specs = Vec::with_capacity(n + inner_cbs);
    let mut k = 0;
    let take = |k: &mut usize| -> Result<SurfSpec, String> {
        let f = faces
            .get(*k)
            .ok_or_else(|| format!("ブロック {id}: 面の数が元の玉と違う"))?;
        *k += 1;
        let mut q = surface_spec(f, *k == n).map_err(|e| format!("ブロック {id} の面 {k}: {e}"))?;
        if *k == n {
            q.t = 0.0;
        }
        Ok(q)
    };
    match &orig {
        Some(v) if !*reversed => {
            for q in v {
                if q.coordbrk.is_some() {
                    specs.push(q.clone());
                } else {
                    specs.push(take(&mut k)?);
                }
            }
        }
        _ => {
            for _ in 0..n {
                specs.push(take(&mut k)?);
            }
        }
    }
    if k != n {
        return Err(format!("ブロック {id}: 面の数が元の玉と違う"));
    }
    for d in &faces {
        if let Some(g) = s.glasses.get(&d.mat) {
            if !g.catalog.is_empty() && !item.glass_catalogs.contains(&g.catalog) {
                item.glass_catalogs.push(g.catalog.clone());
            }
        }
    }
    // 展開した系の面はもう反転の後の向き
    *source = LensSource::Surfaces { surfaces: specs };
    *reversed = false;
    Ok(())
}

/// 展開した系の面を、面を直接書いた玉の面（[`SurfSpec`]）に戻す。書けない値（面の前後の傾き・偏心、
/// 標準面・偶数次非球面・Jones 面のほかの型、鏡、ignore、dn・disp、分散のあるモデルガラス）は誤り
fn surface_spec(f: &Surface, last: bool) -> Result<SurfSpec, String> {
    if !matches!(
        f.typ,
        SurfType::Standard | SurfType::EvenAsph | SurfType::Jones
    ) {
        return Err(format!("面の型 {:?} は書けない", f.typ));
    }
    if f.mirror {
        return Err("鏡の面は書けない".into());
    }
    // 頂点のまわりの傾きだけで後で戻す面は tilt で書ける（tilt は tx → ty の順。順序 1 は tx・ty の片方が 0 のときだけ同じ）
    let tilt = match (&f.before, &f.after) {
        (None, After::None) => None,
        (Some(t), After::Reverse)
            if t.d[0] == 0.0
                && t.d[1] == 0.0
                && t.d[4] == 0.0
                && (t.order == 0 || t.d[2] == 0.0 || t.d[3] == 0.0) =>
        {
            Some([t.d[2], t.d[3]])
        }
        _ => return Err("面の前後の傾き・偏心（before・after）は書けない".into()),
    };
    if f.ignore || f.dn != 0.0 || f.disp != 0.0 || f.cx != 0.0 {
        return Err("ignore・dn・disp・cx は書けない".into());
    }
    let glass = if last || is_air(&f.mat) {
        None
    } else if let Some((nd, vd)) = f.mat.split_once(',') {
        match (nd.trim().parse::<f64>(), vd.trim().parse::<f64>()) {
            (Ok(_), Ok(0.0)) => Some(format!("{}", f.n.first().copied().unwrap_or(0.0))),
            _ => return Err(format!("分散のあるモデルガラス {} は書けない", f.mat)),
        }
    } else {
        Some(f.mat.clone())
    };
    let aperture = match f.ap {
        Aperture::None => None,
        Aperture::Circular { min, max, dx, dy } => {
            Some(SurfAperture::Circular { min, max, dx, dy })
        }
        Aperture::Rect { hx, hy, dx, dy } => Some(SurfAperture::Rect { hx, hy, dx, dy }),
        Aperture::Ellipse { hx, hy, dx, dy } => Some(SurfAperture::Ellipse { hx, hy, dx, dy }),
        Aperture::Obsc { min, max, dx, dy } => Some(SurfAperture::Obscuration { min, max, dx, dy }),
        Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        } => Some(SurfAperture::Spider {
            width,
            arms,
            dx,
            dy,
        }),
        Aperture::Floating { .. } => Some(SurfAperture::Floating),
    };
    // 偏光子・波長板の面はその書き方で戻す
    let (mut polarizer, mut waveplate) = (None, None);
    if let (SurfType::Jones, Some(r)) = (f.typ, &f.retarder) {
        // tx = ty は波長板（遅延 0 でも設計波長を残す）、tx > ty の遅延なしは偏光子
        if r.retardance == 0.0 && r.order == 0 && r.tx > r.ty {
            let t = r.tx * r.tx;
            polarizer = Some(PolarizerSpec {
                axis_deg: r.axis_deg,
                extinction_ratio: if r.ty == 0.0 {
                    f64::INFINITY
                } else {
                    t / (r.ty * r.ty)
                },
                transmittance: t,
            });
        } else if r.tx == r.ty {
            // 板は厚さを書いて戻す（1 枚で軸 0 は material の書き方）
            let (material, thickness_mm, plates) = match r.plates.as_slice() {
                [] => (None, None, Vec::new()),
                [p] if p.axis_deg == 0.0 => {
                    (Some(p.material.clone()), Some(p.thickness), Vec::new())
                }
                ps => (
                    None,
                    None,
                    ps.iter()
                        .map(|p| PlateSpec {
                            material: p.material.clone(),
                            thickness_mm: p.thickness,
                            axis_deg: p.axis_deg,
                        })
                        .collect(),
                ),
            };
            waveplate = Some(WaveplateSpec {
                retardance: r.retardance,
                wavelength_um: r.wl0,
                axis_deg: r.axis_deg,
                order: r.order,
                transmittance: r.tx * r.tx,
                material,
                thickness_mm,
                plates,
            });
        }
    }
    let jones =
        (f.typ == SurfType::Jones && polarizer.is_none() && waveplate.is_none()).then_some(f.jones);
    Ok(SurfSpec {
        radius: (f.c != 0.0).then(|| 1.0 / f.c),
        conic: f.k,
        asph: if f.typ == SurfType::EvenAsph {
            f.asph.clone()
        } else {
            Vec::new()
        },
        t: f.t,
        glass,
        semi: Semi::of(f),
        coat: f.coat.clone(),
        comment: f.comment.clone(),
        jones,
        tilt,
        polarizer,
        waveplate,
        aperture,
        birefringent: f.bire.as_ref().map(|b| match b {
            lensbench_core::surface::Bire::In { mode, axis, .. } => BireSpec::In {
                mode: *mode,
                axis: *axis,
            },
            lensbench_core::surface::Bire::Out => BireSpec::Out,
        }),
        ..Default::default()
    })
}

/// ブロックを持ち出せる形に（.zmx の玉は面に、パスは絶対パスに）
fn portable(c: &mut Ctx, b: Block, cats: &mut Vec<String>) -> Result<Block, String> {
    let abs = |c: &Ctx, p: &str| -> String {
        let q = c.base.join(p);
        lensbench_core::vfs::canonicalize(&q)
            .unwrap_or(q)
            .to_string_lossy()
            .into_owned()
    };
    Ok(match b {
        Block::Lens {
            id,
            source: source @ LensSource::Zmx { .. },
            reversed,
            stop,
            place,
        } => {
            let e = c.lens_element(&source)?;
            for d in &e.surfaces {
                if let Some(g) = c.glasses.get(&d.mat) {
                    if !g.catalog.is_empty() && !cats.contains(&g.catalog) {
                        cats.push(g.catalog.clone());
                    }
                }
            }
            let Block::Lens { source, .. } = lens_to_block(&e, &id)? else {
                unreachable!()
            };
            Block::Lens {
                id,
                source,
                reversed,
                stop,
                place,
            }
        }
        Block::Zmx {
            id,
            path,
            from,
            to,
            config,
            stop,
            groups,
            place,
        } => Block::Zmx {
            id,
            path: abs(c, &path),
            from,
            to,
            config,
            stop,
            groups,
            place,
        },
        Block::User {
            id,
            catalog,
            item,
            place,
        } => Block::User {
            id,
            catalog: abs(c, &catalog),
            item,
            place,
        },
        Block::Group {
            id,
            name,
            role,
            shift,
            blocks,
            place,
        } => Block::Group {
            id,
            name,
            role,
            shift,
            blocks: blocks
                .into_iter()
                .map(|x| portable(c, x, cats))
                .collect::<Result<_, _>>()?,
            place,
        },
        Block::Splitter { id, spec, place } => {
            let mut spec = spec;
            for l in [&mut spec.t, &mut spec.r, &mut spec.d] {
                *l = std::mem::take(l)
                    .into_iter()
                    .map(|x| portable(c, x, cats))
                    .collect::<Result<_, _>>()?;
            }
            Block::Splitter { id, spec, place }
        }
        x => x,
    })
}

/// .zmx の取り込みの群の、要素の範囲 (群の番号, 最初, 最後)。最初・最後は面が from〜to に始まる要素（空気でない）
fn zmx_group_spans(
    keep: &[Element],
    groups: &[ZmxGroup],
) -> Result<Vec<(usize, usize, usize)>, String> {
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    for (g, gr) in groups.iter().enumerate() {
        let label = if gr.id.is_empty() { &gr.name } else { &gr.id };
        if gr.from > gr.to {
            return Err(format!("群 {label}: 面 {}〜{} の順が逆", gr.from, gr.to));
        }
        let inside: Vec<usize> = (0..keep.len())
            .filter(|&i| keep[i].kind != Kind::Air && (gr.from..=gr.to).contains(&keep[i].from))
            .collect();
        let (Some(&a), Some(&z)) = (inside.first(), inside.last()) else {
            return Err(format!(
                "群 {label}: 面 {}〜{} に取り込んだ玉・面がない",
                gr.from, gr.to
            ));
        };
        if let Some(o) = out.iter().find(|o| a <= o.2 && o.1 <= z) {
            return Err(format!("群 {label} が群 {} と重なる", groups[o.0].name));
        }
        out.push((g, a, z));
    }
    Ok(out)
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

/// 面の開口（JSON）を面の開口に。浮動の開口は半径 semi
fn surf_ap(ap: Option<SurfAperture>, semi: f64) -> Aperture {
    match ap {
        None => Aperture::None,
        Some(SurfAperture::Circular { min, max, dx, dy }) => {
            Aperture::Circular { min, max, dx, dy }
        }
        Some(SurfAperture::Rect { hx, hy, dx, dy }) => Aperture::Rect { hx, hy, dx, dy },
        Some(SurfAperture::Ellipse { hx, hy, dx, dy }) => Aperture::Ellipse { hx, hy, dx, dy },
        Some(SurfAperture::Obscuration { min, max, dx, dy }) => Aperture::Obsc { min, max, dx, dy },
        Some(SurfAperture::Spider {
            width,
            arms,
            dx,
            dy,
        }) => Aperture::Spider {
            width,
            arms,
            dx,
            dy,
        },
        Some(SurfAperture::Floating) => Aperture::Floating { semi },
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
        semi: q.semi.value(),
        semi_auto: q.semi.is_auto(),
        coat: q.coat.clone(),
        comment: q.comment.clone(),
        ..Default::default()
    };
    s.ap = surf_ap(q.aperture, q.semi.value());
    let special = q.jones.is_some() as u8
        + q.coordbrk.is_some() as u8
        + q.polarizer.is_some() as u8
        + q.waveplate.is_some() as u8
        + q.birefringent.is_some() as u8;
    if special > 1 {
        return Err(
            "面に jones・polarizer・waveplate・coordbrk・birefringent のうち 2 つ以上は書けない"
                .into(),
        );
    }
    if let Some(b) = q.birefringent {
        s.bire = Some(match b {
            BireSpec::In { mode, axis } => {
                if !matches!(mode, 0 | 2) {
                    return Err(format!("複屈折材の板の mode {mode}（0 か 2）"));
                }
                if !(axis.iter().all(|v| v.is_finite()) && axis.iter().any(|v| *v != 0.0)) {
                    return Err("複屈折材の板の結晶軸 axis が 0 か数でない".into());
                }
                if q.glass.as_deref().map_or(true, |g| is_air(g.trim())) {
                    return Err("複屈折材の板の入口の面には glass（常光の材料）を書く".into());
                }
                lensbench_core::surface::Bire::In {
                    mode,
                    axis,
                    ne: Vec::new(),
                }
            }
            BireSpec::Out => lensbench_core::surface::Bire::Out,
        });
    }
    if special == 1 && q.birefringent.is_none() {
        let what = if q.jones.is_some() {
            "jones"
        } else if q.polarizer.is_some() {
            "polarizer"
        } else if q.waveplate.is_some() {
            "waveplate"
        } else {
            "coordbrk"
        };
        if curv(q.radius) != 0.0 || q.conic != 0.0 || s.typ == SurfType::EvenAsph {
            return Err(format!("{what} の面に曲率・円錐係数・非球面は書けない"));
        }
        if q.glass.as_deref().is_some_and(|g| !is_air(g.trim())) {
            return Err(format!("{what} の面の後にガラスは書けない"));
        }
    }
    if let Some(j) = q.jones {
        s.typ = SurfType::Jones;
        s.jones = j;
    }
    let ret = match (&q.polarizer, &q.waveplate) {
        (Some(p), _) => Some(p.retarder()?),
        (_, Some(w)) => Some(w.retarder()?),
        _ => None,
    };
    if let Some(r) = ret {
        // jones は設計波長（偏光子は波長によらない）の値。偏光の追跡は波長ごとに retarder から
        s.typ = SurfType::Jones;
        s.jones = r.jones8(r.wl0);
        s.retarder = Some(r);
    }
    if let Some([tx, ty]) = q.tilt {
        if q.coordbrk.is_some() {
            return Err("座標ブレークの面に tilt は書けない".into());
        }
        if !(tx.is_finite() && ty.is_finite()) {
            return Err("tilt が数でない".into());
        }
        if tx != 0.0 || ty != 0.0 {
            s.before = Some(lensbench_core::surface::Tilt {
                d: [0.0, 0.0, tx, ty, 0.0],
                order: 0,
            });
            s.after = After::Reverse;
        }
    }
    if let Some(d) = q.coordbrk {
        if q.order > 1 {
            return Err(format!("座標ブレークの順序 {}（0 か 1）", q.order));
        }
        if q.aperture.is_some() {
            return Err("座標ブレークの面に開口は書けない".into());
        }
        s.typ = SurfType::CoordBrk;
        s.mat = "-".into();
        s.cb = d;
        s.cb_order = q.order;
        return Ok(s);
    }
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

/// プロジェクトの状態 state（None なら変更なし）を順次の面の列の系に展開する
pub fn expand_state(p: &Project, env: &Env, state: Option<&str>) -> Result<System, String> {
    match state {
        Some(n) => expand(&p.with_state(n)?, env),
        None => expand(p, env),
    }
}

/// プロジェクトを順次の面の列の系に展開する
pub fn expand(p: &Project, env: &Env) -> Result<System, String> {
    expand_report(p, env).map(|r| r.0)
}

/// expand と同じ。展開で落としたもの（取り込んだ範囲の外を参照するピックアップなど）の説明も返す
pub fn expand_report(p: &Project, env: &Env) -> Result<(System, Vec<String>), String> {
    expand_inner(p, env, p.path.as_deref(), false).map(|r| (r.0, r.1))
}

/// id のある空気のブロックが入る面: (id, 面番号, 向き)。[`expand_gaps`]
pub type Gap = (String, usize, f64);

/// id のある空気のブロックの間隔（差し替え探索の変数）: (id, その空気が入る面の番号, 向き)。
/// 向きは鏡の後（光が −z へ進む）で −1（面の厚さは −t）。往復で 2 回通る空気は 2 つ並ぶ。
/// 面の厚さには、同じ面に入る他の空気・群の移動も足されている
pub fn expand_gaps(p: &Project, env: &Env) -> Result<(System, Vec<Gap>), String> {
    expand_inner(p, env, p.path.as_deref(), false).map(|r| (r.0, r.3))
}

/// ビームスプリッターの経路 path（BS を通る順の T・R）を展開する。None ならプロジェクトの path
pub fn expand_path(
    p: &Project,
    env: &Env,
    path: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    expand_inner(p, env, path.or(p.path.as_deref()), false).map(|r| (r.0, r.1))
}

/// プロジェクトでとれるビームスプリッターの経路の全部（BS を通る順の T・R の文字列）。BS がなければ空。
/// 経路は BS ごとに透過・反射を選んで、BS を通らなくなるまで（像面まで）たどったもの
pub fn paths(p: &Project, env: &Env) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut todo = vec![String::new()];
    while let Some(q) = todo.pop() {
        match expand_inner(p, env, Some(&q), true) {
            Ok(_) => out.push(q),
            Err(e) if e == splitter::NEED => {
                if out.len() + todo.len() > 1024 {
                    return Err("経路が 1024 より多い".into());
                }
                todo.push(format!("{q}R"));
                todo.push(format!("{q}T"));
            }
            Err(e) => {
                return Err(if q.is_empty() {
                    e
                } else {
                    format!("経路 {q}: {e}")
                })
            }
        }
    }
    if out == [""] {
        out.clear();
    }
    out.sort_by(|a, b| a.len().cmp(&b.len()).then(b.cmp(a)));
    Ok(out)
}

/// BS の id を集める（重なりを調べる）
fn splitter_ids<'x>(v: &'x [Block], out: &mut Vec<&'x str>) {
    for b in v {
        match b {
            Block::Splitter { id, spec, .. } => {
                out.push(id);
                for l in [&spec.t, &spec.r, &spec.d] {
                    splitter_ids(l, out);
                }
            }
            Block::Group { blocks, .. } => splitter_ids(blocks, out),
            _ => {}
        }
    }
}

fn expand_inner(
    p: &Project,
    env: &Env,
    path: Option<&str>,
    strict: bool,
) -> Result<(System, Vec<String>, usize, Vec<Gap>), String> {
    {
        let mut ids = Vec::new();
        splitter_ids(&p.blocks, &mut ids);
        for (i, id) in ids.iter().enumerate() {
            if id.is_empty() {
                return Err("BS（splitter）には id が要る".into());
            }
            if ids[..i].contains(id) {
                return Err(format!("BS の id {id} が 2 つ以上ある"));
            }
        }
    }
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
    let (glasses, missing) = glass::load_catalogs_shared(&p.glass_catalogs, &dirs);
    if !missing.is_empty() {
        return Err(format!(
            "ガラスカタログが見つからない: {}",
            missing.join(", ")
        ));
    }
    let mut c = Ctx {
        env,
        base: env.base.clone(),
        users: Vec::new(),
        sf: Vec::new(),
        org: Vec::new(),
        zpicks: Vec::new(),
        lead: 0.0,
        parity: 1.0,
        glasses,
        coats: HashMap::new(),
        nstop: 0,
        spans: Vec::new(),
        back_spans: Vec::new(),
        gaps: Vec::new(),
        split: SplitState {
            path: splitter::parse_path(path.unwrap_or(""))?,
            strict,
            ..Default::default()
        },
        frames: Vec::new(),
    };
    for (i, b) in p.blocks.iter().enumerate() {
        if c.split.done {
            break;
        }
        // 検出器（最後のブロック）は面を足さない。像面に置く
        if matches!(b, Block::Detector(_)) && i + 1 == p.blocks.len() {
            continue;
        }
        c.block(b, &p.blocks, i)?;
    }
    if c.split.pos < c.split.path.len() {
        return Err(format!(
            "経路 {}: 文字が多い（BS を通るのは {} 回）",
            path.unwrap_or(""),
            c.split.pos
        ));
    }
    let npass = c.split.pos;
    // 絞りを面に置く操作（surface_ops の set_stop）があれば、ブロックの絞りは無くてよい
    let stop_op = p
        .surface_ops
        .iter()
        .any(|o| o.get("op").and_then(Value::as_str) == Some("set_stop"));
    if c.nstop > 1 || (c.nstop == 0 && !stop_op) {
        return Err(format!("絞りが {} 個（ちょうど 1 つ要る）", c.nstop));
    }
    if c.nstop == 0 {
        // 仮の絞り（set_stop で移す）
        if let Some(f) = c.sf.first_mut() {
            f.stop = true;
        }
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
    if c.sf.len() != c.org.len() {
        // 面は put か element_from で足す（出どころの表を面と揃える）
        return Err(format!(
            "内部エラー: 面 {} 個に出どころ {} 個",
            c.sf.len(),
            c.org.len()
        ));
    }
    sf.append(&mut c.sf);
    let org = std::mem::take(&mut c.org);
    let zpicks = std::mem::take(&mut c.zpicks);
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
        // 複屈折材の板の入口: 異常光の屈折率
        if let Some(lensbench_core::surface::Bire::In { ne, .. }) = d.bire.as_mut() {
            let en = lensbench_core::surface::Bire::e_name(&d.mat);
            let g = c.glasses.get(&en).ok_or_else(|| {
                format!(
                    "面 {j}: 複屈折材 {} の異常光のガラス {en} がカタログにない",
                    d.mat
                )
            })?;
            *ne = wl.iter().map(|&l| g.index(l, 20.0, 1.0, false)).collect();
        }
        // 複屈折材の波長板: 板のガラスを引き、設計波長の行列を作り直す
        if let Some(r) = d.retarder.as_mut().filter(|r| !r.plates.is_empty()) {
            r.resolve(&c.glasses).map_err(|e| format!("面 {j}: {e}"))?;
            d.jones = r.jones8(r.wl0);
        }
        // 屈折面での反射（ビームスプリッター）: 反対側の媒質の屈折率
        if let Some(r) = &mut d.reflect {
            r.n = if is_air(&r.mat) {
                vec![1.0; wl.len()]
            } else if let Some(g) = c.glasses.get(&r.mat) {
                wl.iter().map(|&l| g.index(l, 20.0, 1.0, false)).collect()
            } else {
                return Err(format!("面 {j}: ガラス {} がカタログにない", r.mat));
            };
        }
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
        // 面の名前は展開した後で引く
        FieldKind::Solve => FieldType::Angle,
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
        p.telecentric,
        aim,
        c.glasses,
    )
    .map_err(|e| e.to_string())?;
    s.name = p.name.clone();
    // 膜: プロジェクトの coatings はカタログの同じ名前より優先
    s.coat_tables = c.coats;
    for (k, spec) in &p.coatings {
        let t = spec.table().map_err(|e| format!("膜 {k}: {e}"))?;
        s.coat_tables.insert(k.clone(), t);
    }
    if let Some(u) = &p.afocal {
        s.afocal = true;
        s.afocal_unit = AfocalUnit::from_code(u)
            .ok_or_else(|| format!("afocal の単位 {u}（ur・mr・rr・as・am・dg のどれか）"))?;
    }
    if p.fields.kind == FieldKind::Solve {
        let Some(c) = &p.fields.solve else {
            return Err("fields の type が solve なら solve に条件を書く".into());
        };
        let f = |t: &str| s.find_surface(t).map_err(|e| format!("視野の条件: {e}"));
        let cond = match c {
            FieldSolveSpec::Position { surface } => FieldCond::Position { surf: f(surface)? },
            FieldSolveSpec::DirSum { s1, s2 } => FieldCond::DirSum {
                s1: f(s1)?,
                s2: f(s2)?,
            },
            FieldSolveSpec::DirDiff { s1, s2 } => FieldCond::DirDiff {
                s1: f(s1)?,
                s2: f(s2)?,
            },
        };
        s.field_type = FieldType::Solve(cond);
        s.changed().map_err(|e| e.to_string())?;
    } else if p.fields.solve.is_some() {
        return Err("fields の solve は type が solve のときだけ".into());
    }
    let mut warn = keep_pickups(&mut s, &org, &zpicks)?;
    // 玉・群のパラメータ: 玉モードの玉 lens1 … と、id のあるブロック・群（同じ id の範囲は 1 つの玉・群）
    s.units = s.lens_units();
    let mut ids: Vec<&str> = Vec::new();
    for (id, _, _) in &c.spans {
        if !ids.contains(&id.as_str()) {
            ids.push(id);
        }
    }
    for id in ids {
        let spans: Vec<[usize; 2]> = c
            .spans
            .iter()
            .filter(|x| x.0 == id)
            .map(|x| [x.1 + 1, x.2 + 1])
            .collect();
        match s.define_unit(id, &spans, [0.0; 3]) {
            // 群の移動はもう厚さに入っているので、その分を玉・群の shift の今の値にする（状態ごとの群の位置を
            // `id.shift` で比べられるように）
            Ok(u) => s.units[u].shift = group_shift(&p.blocks, id).unwrap_or(0.0),
            Err(e) => warn.push(format!("ブロック {id} を玉・群のパラメータにしない: {e}")),
        }
    }
    apply_surface_edits(&mut s, &p.surface_edits, &c.spans, &c.back_spans)?;
    let (w, stopped) = apply_surface_ops(&mut s, &p.surface_ops, &c.spans, &c.back_spans)?;
    if c.nstop == 0 && !stopped {
        // 仮の絞りのまま（set_stop のブロックが無い）にはしない
        return Err(
            "絞りが 0 個（surface_ops の set_stop のブロックが無い。ちょうど 1 つ要る）".into(),
        );
    }
    warn.extend(w);
    // 自動の半直径を光線から求める。機構部品の検査・描画・書き出しが使う
    s.update_semi();
    s.mechanics = place_mechanics(
        &s,
        &p.mechanics,
        &env.base,
        &c.spans,
        &c.back_spans,
        &mut warn,
    )?;
    s.mechanics.extend(place_frames(&s, &c.frames, &mut warn));
    s.validate().map_err(|e| e.to_string())?;
    Ok((s, warn, npass, c.gaps))
}

/// 機構部品を系の面に置く（[`Project::mechanics`]）。ブロックが無い（状態で外したなど）部品は置かずに注意を出す
fn place_mechanics(
    s: &System,
    ms: &[MechPlacement],
    base: &Path,
    spans: &[(String, usize, usize)],
    back_spans: &[(String, usize, usize)],
    warn: &mut Vec<String>,
) -> Result<Vec<lensbench_core::mech::MechPart>, String> {
    let mut out = Vec::new();
    // 置いた部品の寸法（after でねじを見る）
    let mut specs: Vec<MechSpec> = Vec::new();
    for (k, m) in ms.iter().enumerate() {
        let tag = format!("mechanics {}", k + 1);
        let (spec, item_name) = match (&m.mech, &m.catalog, &m.item) {
            (Some(x), None, None) => (x.clone(), String::new()),
            (None, Some(cat), Some(item)) => {
                let path = base.join(cat);
                let c = UserCatalog::load_shared(&path)?;
                let it = c
                    .items
                    .get(item)
                    .ok_or_else(|| format!("{tag}: {} に品 {item} はない", path.display()))?;
                let x = it
                    .mech
                    .clone()
                    .ok_or_else(|| format!("{tag}: 品 {item} は機構部品でない（mech がない）"))?;
                (x, item.clone())
            }
            _ => {
                return Err(format!(
                    "{tag}: 部品は mech か、catalog と item のどちらか一方で書く"
                ))
            }
        };
        spec.validate().map_err(|e| format!("{tag}: {e}"))?;
        let name = if !m.name.is_empty() {
            m.name.clone()
        } else if !item_name.is_empty() {
            item_name.clone()
        } else {
            format!("mech{}", k + 1)
        };
        // 前の部品の後ろの端につなぐ
        if let Some(prev_name) = &m.after {
            if m.seat || !m.at.is_unset() {
                return Err(format!("{tag}: after は at・seat と一緒に使えない"));
            }
            if m.tilt != [0.0, 0.0] {
                return Err(format!(
                    "{tag}: after では tilt は使えない（前の部品の傾きになる）"
                ));
            }
            let Some((prev, prev_spec)) = out
                .iter()
                .zip(&specs)
                .rev()
                .find(|(p, _): &(&lensbench_core::mech::MechPart, _)| &p.name == prev_name)
            else {
                return Err(format!("{tag}: after の部品 {prev_name} が前に無い"));
            };
            let (prev, prev_spec): (&lensbench_core::mech::MechPart, &MechSpec) = (prev, prev_spec);
            let overlap = match (&prev_spec.back_thread, &spec.front_thread) {
                (Some(a), Some(b)) => {
                    if a.thread != b.thread || a.male == b.male {
                        let kind = |t: &ThreadEnd| {
                            format!("{} の{}", t.thread, if t.male { "オス" } else { "メス" })
                        };
                        warn.push(format!(
                            "{tag}: {prev_name} の後ろの端（{}）と、{name} の前の端（{}）のねじが合わない",
                            kind(a),
                            kind(b)
                        ));
                        0.0
                    } else if b.male {
                        b.length
                    } else {
                        a.length
                    }
                }
                _ => 0.0,
            };
            let mut part0 = spec.part(&name, prev.surface, [0.0; 3], prev.tilt);
            let z = prev.z_range()[1] - overlap - part0.z_range()[0];
            let r = lensbench_core::math::mm(
                &lensbench_core::math::rot(0, prev.tilt[0]),
                &lensbench_core::math::rot(1, prev.tilt[1]),
            );
            let d = lensbench_core::math::mv(&r, [m.offset[0], m.offset[1], m.offset[2] + z]);
            part0.offset = [
                prev.offset[0] + d[0],
                prev.offset[1] + d[1],
                prev.offset[2] + d[2],
            ];
            if !part0.offset.iter().all(|v| v.is_finite()) {
                return Err(format!("{tag}: offset が数でない"));
            }
            for b in &m.holds {
                match spans.iter().find(|x| &x.0 == b && !back_spans.contains(x)) {
                    Some(x) => part0.holds.extend(
                        (x.1 + 1..=x.2 + 1).filter(|&j| s.surfaces[j].typ != SurfType::CoordBrk),
                    ),
                    None => warn.push(format!("{tag}: holds のブロック {b} が無い")),
                }
            }
            out.push(part0);
            specs.push(spec);
            continue;
        }
        let mut offset = m.offset;
        // 座に合わせる: 保持する最初の素子の前面に、部品の座の z を合わせる
        let seat_at = if m.seat {
            let seat = spec
                .holds
                .ok_or_else(|| format!("{tag}: seat には部品の寸法の holds（座の位置）が要る"))?
                .seat_z;
            let b = m
                .holds
                .first()
                .ok_or_else(|| format!("{tag}: seat には holds（保持する素子）が要る"))?;
            offset[2] -= seat;
            match block_face(s, b, 0, spans, back_spans).map_err(|e| format!("{tag}: {e}"))? {
                Some(j) => Some(j),
                None => {
                    warn.push(format!("{tag}: ブロック {b} が無いので置かない"));
                    continue;
                }
            }
        } else {
            None
        };
        let surface = match (seat_at, &m.at.block) {
            (Some(j), _) => j,
            (None, block) => match block {
                Some(b) => match block_face(s, b, m.at.surface, spans, back_spans)
                    .map_err(|e| format!("{tag}: {e}"))?
                {
                    Some(j) => j,
                    None => {
                        warn.push(format!("{tag}: ブロック {b} が無いので置かない"));
                        continue;
                    }
                },
                None => {
                    if m.at.is_unset() {
                        return Err(format!("{tag}: 置く所（at）か seat が要る"));
                    }
                    if m.at.surface == 0 || m.at.surface > s.image() {
                        return Err(format!("{tag}: 面 {}（1〜{}）", m.at.surface, s.image()));
                    }
                    m.at.surface
                }
            },
        };
        if !offset.iter().chain(&m.tilt).all(|v| v.is_finite()) {
            return Err(format!("{tag}: offset・tilt が数でない"));
        }
        let mut part = spec.part(&name, surface, offset, m.tilt);
        for b in &m.holds {
            match spans.iter().find(|x| &x.0 == b && !back_spans.contains(x)) {
                Some(x) => part.holds.extend(
                    (x.1 + 1..=x.2 + 1).filter(|&j| s.surfaces[j].typ != SurfType::CoordBrk),
                ),
                None => warn.push(format!("{tag}: holds のブロック {b} が無い")),
            }
        }
        out.push(part);
        specs.push(spec);
    }
    Ok(out)
}

/// 枠付きのユーザーカタログの品の枠を、ガラスの最初の面に置く（枠の座 `holds.seat_z` をガラスの前面に合わせる）。
/// 保持する素子はその品の面
fn place_frames(
    s: &System,
    frames: &[(String, MechSpec, usize, usize)],
    warn: &mut Vec<String>,
) -> Vec<lensbench_core::mech::MechPart> {
    let mut out = Vec::new();
    for (name, f, a, b) in frames {
        let tag = format!("{name} の枠");
        if let Err(e) = f.validate() {
            warn.push(format!("{tag}: {e}（置かない）"));
            continue;
        }
        let faces: Vec<usize> = (a + 1..=b + 1)
            .filter(|&j| j < s.surfaces.len() && s.surfaces[j].typ != SurfType::CoordBrk)
            .collect();
        let Some(&first) = faces.first() else {
            continue;
        };
        let seat = f.holds.map_or(0.0, |h| h.seat_z);
        let mut part = f.part(&tag, first, [0.0, 0.0, -seat], [0.0, 0.0]);
        part.holds = faces;
        out.push(part);
    }
    out
}

/// ブロック block の面 i（座標ブレークを除いた番号、往路）の系の面の番号。ブロックが無ければ None
fn block_face(
    s: &System,
    block: &str,
    i: usize,
    spans: &[(String, usize, usize)],
    back_spans: &[(String, usize, usize)],
) -> Result<Option<usize>, String> {
    let Some(x) = spans
        .iter()
        .find(|x| x.0 == block && !back_spans.contains(x))
    else {
        return Ok(None);
    };
    let faces: Vec<usize> = (x.1 + 1..=x.2 + 1)
        .filter(|&j| s.surfaces[j].typ != SurfType::CoordBrk)
        .collect();
    faces.get(i).copied().map(Some).ok_or(format!(
        "ブロック {block} の面 {i}（面は {} 枚）",
        faces.len()
    ))
}

/// 面の絞り・ソルブ・ピックアップを当てる（[`Project::surface_ops`]）。当てなかった操作の注意と、set_stop を
/// 当てたかを返す
fn apply_surface_ops(
    s: &mut System,
    ops: &[Value],
    spans: &[(String, usize, usize)],
    back_spans: &[(String, usize, usize)],
) -> Result<(Vec<String>, bool), String> {
    let mut warn = Vec::new();
    let mut out = Vec::new();
    'ops: for (k, op) in ops.iter().enumerate() {
        let name = op.get("op").and_then(Value::as_str).unwrap_or_default();
        if !["set_stop", "add_solve", "add_pickup"].contains(&name) {
            return Err(format!(
                "surface_ops {}: 操作は set_stop・add_solve・add_pickup（{name}）",
                k + 1
            ));
        }
        let mut o = op.clone();
        for key in ["surface", "dst", "src", "other"] {
            let Some(r) = op.get(key).and_then(Value::as_object) else {
                continue;
            };
            let (Some(b), Some(i)) = (
                r.get("block").and_then(Value::as_str),
                r.get("surface").and_then(Value::as_u64),
            ) else {
                return Err(format!(
                    "surface_ops {}: {key} は {{\"block\": id, \"surface\": i}} か面の番号",
                    k + 1
                ));
            };
            match block_face(s, b, i as usize, spans, back_spans)
                .map_err(|x| format!("surface_ops {}: {x}", k + 1))?
            {
                Some(j) => o[key] = Value::from(j),
                None => {
                    warn.push(format!(
                        "surface_ops {}: ブロック {b} が無いので {name} を当てない",
                        k + 1
                    ));
                    continue 'ops;
                }
            }
        }
        out.push(o);
    }
    let stopped = out.iter().any(|o| o["op"] == "set_stop");
    if !out.is_empty() {
        warn.extend(
            edit_ops::apply(s, &Value::Array(out)).map_err(|x| format!("surface_ops: {x}"))?,
        );
    }
    Ok((warn, stopped))
}

/// 面の値の上書きを当てる（ブロック id の面のうち座標ブレークを除いた番号）
fn apply_surface_edits(
    s: &mut System,
    edits: &[SurfaceEdit],
    spans: &[(String, usize, usize)],
    back_spans: &[(String, usize, usize)],
) -> Result<(), String> {
    // 範囲の面（座標ブレークを除く）
    let faces = |s: &System, a: usize, b: usize| -> Vec<usize> {
        (a + 1..=b + 1)
            .filter(|&j| s.surfaces[j].typ != SurfType::CoordBrk)
            .collect()
    };
    for e in edits {
        let err = |x: String| {
            format!(
                "surface_edits: ブロック {} の面 {}: {x}",
                e.block, e.surface
            )
        };
        let mine = |x: &&(String, usize, usize)| x.0 == e.block;
        // 往路（と、同じ向きに通るほかの範囲）・復路
        let fwd: Vec<Vec<usize>> = spans
            .iter()
            .filter(mine)
            .filter(|x| !back_spans.contains(x))
            .map(|x| faces(s, x.1, x.2))
            .collect();
        let back: Vec<Vec<usize>> = back_spans
            .iter()
            .filter(mine)
            .map(|x| faces(s, x.1, x.2))
            .collect();
        let Some(n) = fwd.first().map(Vec::len).filter(|&n| n > 0) else {
            return Err(format!("surface_edits: ブロック {} はない", e.block));
        };
        let i = e.surface;
        if i >= n {
            return Err(format!(
                "surface_edits: ブロック {} の面 {}（面は {n} 枚）",
                e.block, e.surface
            ));
        }
        // 面の値の名前（SURF_FIELDS）と、それ以外の面のデータの項目（set_surface_data）
        let (named, data): (Map<String, Value>, Map<String, Value>) = e
            .fields
            .clone()
            .into_iter()
            .partition(|(k, _)| lensbench_core::edit::SURF_FIELDS.contains(&k.as_str()));
        let data = Value::Object(data);
        let has_data = data.as_object().is_some_and(|m| !m.is_empty());
        let f = edit_ops::surf_fields(&Value::Object(named))
            .map_err(|x| format!("surface_edits: {x}"))?;
        for v in &fwd {
            let j = *v
                .get(i)
                .ok_or_else(|| err("同じ向きの範囲の面の数が違う".into()))?;
            if !f.is_empty() {
                s.set_surface_fields(j, &f)
                    .map_err(|x| err(x.to_string()))?;
            }
            if has_data {
                edit_ops::set_surface_data(s, j, &data).map_err(err)?;
            }
        }
        if has_data && !back.is_empty() {
            if ["t", "mat", "dn", "disp"]
                .iter()
                .any(|k| data.get(*k).is_some())
            {
                return Err(err(
                    "往復するブロックの厚さ・材料は t・material の名前で変える".into(),
                ));
            }
            // 復路では面の前後が入れ替わり向きも逆になるので、前後の傾き偏心はそのまま当てられない
            if ["before", "after"].iter().any(|k| data.get(*k).is_some()) {
                return Err(err(
                    "往復するブロックの面の前後の傾き偏心は変えられない（ブロックの偏心・傾きで）"
                        .into(),
                ));
            }
            for v in &back {
                if v.len() == n {
                    edit_ops::set_surface_data(s, v[n - 1 - i], &data).map_err(err)?;
                }
            }
        }
        // 復路は逆の順にたどる: 面の値（曲率・コーニック・半径など）は同じ面 n−1−i に同じ値、面の後の媒質の値
        // （厚さ・材料）は n−2−i に（厚さは符号が逆）。往路の最後の面の後はブロックの外（後の空気）なので、往復する
        // ブロックでは変えない
        for v in &back {
            if v.len() != n {
                return Err(err(format!("復路の面の数 {}（往路は {n}）", v.len())));
            }
            let (face, gap): (Vec<_>, Vec<_>) = f
                .iter()
                .cloned()
                .partition(|(k, _)| !matches!(k.as_str(), "t" | "material" | "dn" | "disp"));
            if !face.is_empty() {
                s.set_surface_fields(v[n - 1 - i], &face)
                    .map_err(|x| err(x.to_string()))?;
            }
            if !gap.is_empty() {
                if i + 1 == n {
                    return Err(err(
                        "往復するブロックの最後の面の厚さ・材料は、後の空気のブロックで変える"
                            .into(),
                    ));
                }
                let gap: Vec<_> = gap
                    .into_iter()
                    .map(|(k, x)| match (k.as_str(), x) {
                        ("t", lensbench_core::edit::SurfValue::Num(t)) => {
                            (k, lensbench_core::edit::SurfValue::Num(-t))
                        }
                        (_, x) => (k, x),
                    })
                    .collect();
                s.set_surface_fields(v[n - 2 - i], &gap)
                    .map_err(|x| err(x.to_string()))?;
            }
        }
    }
    Ok(())
}

/// id の群（ブロックの群・.zmx の取り込みの中の群）の移動
fn group_shift(blocks: &[Block], id: &str) -> Option<f64> {
    for b in blocks {
        match b {
            Block::Group {
                id: g,
                shift,
                blocks,
                ..
            } => {
                if g == id {
                    return Some(*shift);
                }
                if let Some(v) = group_shift(blocks, id) {
                    return Some(v);
                }
            }
            Block::Zmx { groups, .. } => {
                if let Some(g) = groups.iter().find(|g| g.id == id) {
                    return Some(g.shift);
                }
            }
            Block::Splitter { spec, .. } => {
                for arm in [&spec.t, &spec.r, &spec.d] {
                    if let Some(v) = group_shift(arm, id) {
                        return Some(v);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// zmx ブロックで取り込んだ面のピックアップと厚さのソルブを、展開した系の面番号に付け替えて残す。
/// 元か先が取り込んだ範囲の外にあるもの、展開で値の関係が崩れたもの（群の移動で厚さが変わったなど）は
/// 値で固定し、説明を返す。org[i] は展開した系の面 i + 1 の出どころ
fn keep_pickups(
    s: &mut System,
    org: &[Option<(usize, usize)>],
    zpicks: &[ZmxLinks],
) -> Result<Vec<String>, String> {
    let mut at: HashMap<(usize, usize), usize> = HashMap::new();
    for (i, o) in org.iter().enumerate() {
        if let Some(k) = o {
            // 同じ取り込みで同じ面が 2 回出ることはない
            at.insert(*k, i + 1);
        }
    }
    let mut warn = Vec::new();
    for (inst, z) in zpicks.iter().enumerate() {
        let (path, picks) = (&z.path, &z.pickups);
        for q in picks {
            let (Some(&dst), src) = (at.get(&(inst, q.dst)), at.get(&(inst, q.src))) else {
                continue; // 先が取り込んでいない面
            };
            let Some(&src) = src else {
                warn.push(format!(
                    "{path}: 面 {} の {:?} は範囲の外の面 {} からのピックアップ（値で固定）",
                    q.dst, q.what, q.src
                ));
                continue;
            };
            // 鏡のあと（向きが逆）では厚さ・非球面係数の符号が変わるので、オフセットの符号も試す
            let cands = [
                Pickup { dst, src, ..*q },
                Pickup {
                    dst,
                    src,
                    offset: -q.offset,
                    ..*q
                },
            ];
            match cands.iter().find(|c| c.holds(s)) {
                Some(c) => s.pickups.push(*c),
                None => warn.push(format!(
                    "{path}: 面 {} の {:?}（面 {} から）は展開で値の関係が変わった（値で固定）",
                    q.dst, q.what, q.src
                )),
            }
        }
    }
    if !s.pickups.is_empty() {
        s.resolve_pickups().map_err(|e| e.to_string())?;
    }
    // ソルブ（厚さ・曲率）: 関わる面（厚さは次の面、玉のパワーは前の面、基準の面）が同じ並びで取り込まれていれば
    // 残し、展開した系で解き直す（雛形の像面の手前の面の厚さのソルブは、展開した系でも像面の手前にあれば残す）
    for (inst, z) in zpicks.iter().enumerate() {
        for q in &z.solves {
            let Some(&k) = at.get(&(inst, q.surf)) else {
                continue;
            };
            // 雛形の面 j が、展開した系で面 k からの同じずれにあるか
            let same = |j: usize| -> bool {
                at.get(&(inst, j))
                    .is_some_and(|&n| n as i64 - k as i64 == j as i64 - q.surf as i64)
            };
            use lensbench_core::param::SolveKind as Sk;
            let mut ok = true;
            // 位置ソルブは高さを見ないので次の面は要らない（基準の面との間が並んでいればよい）
            if !q.kind.is_curv() && !matches!(q.kind, Sk::Position(_)) {
                ok &= same(q.surf + 1) || (q.surf + 1 == z.image && k + 1 == s.image());
            }
            if q.kind == Sk::ElementPower {
                ok &= q.surf >= 1 && same(q.surf - 1);
            }
            let mut kind = q.kind;
            if let Some(o) = q.kind.other_surface() {
                ok &= same(o);
                // 基準の面との距離で決まるものは、間の面（基準の面が後ろでも）も並んで取り込まれていること
                if matches!(
                    q.kind,
                    Sk::Position(_)
                        | Sk::CenterOfCurvature(_)
                        | Sk::ConcentricSurface(_)
                        | Sk::ConcentricRadius(_)
                ) {
                    ok &= (o.min(q.surf)..=o.max(q.surf)).all(same);
                }
                if let Some(&n) = at.get(&(inst, o)) {
                    kind = kind.with_other_surface(n);
                }
            }
            if ok {
                s.solves.push(ThickSolve {
                    surf: k,
                    kind,
                    ..*q
                });
            } else {
                warn.push(format!(
                    "{}: 面 {} のソルブ（{:?}）は、関わる面が範囲の外か離れている（値で固定）",
                    z.path, q.surf, q.kind
                ));
            }
        }
    }
    if !s.solves.is_empty() {
        let val = |s: &System, q: &ThickSolve| {
            let d = &s.surfaces[q.surf];
            if q.kind.is_curv() {
                d.c
            } else {
                d.t
            }
        };
        let before: Vec<f64> = s.solves.iter().map(|q| val(s, q)).collect();
        s.solve_thickness().map_err(|e| e.to_string())?;
        for (q, b) in s.solves.iter().zip(before) {
            let a = val(s, q);
            if (a - b).abs() > 1e-9 * (1.0 + b.abs()) {
                let what = if q.kind.is_curv() { "曲率" } else { "厚さ" };
                warn.push(format!(
                    "面 {} の{what}をソルブ（{:?}、値 {}）で解き直した: {b} → {a}",
                    q.surf, q.kind, q.height
                ));
            }
        }
    }
    Ok(warn)
}

/// プロジェクトファイルを読んで展開する
pub fn load(
    path: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
) -> Result<System, String> {
    load_state(path, glass_dirs, catalog, None)
}

/// プロジェクトファイルを読んで、状態 state（None なら変更なし）を展開する
pub fn load_state(
    path: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
) -> Result<System, String> {
    load_state_report(path, glass_dirs, catalog, state).map(|r| r.0)
}

/// load_state と同じ。展開で落としたもの（[`expand_report`]）の説明も返す
pub fn load_state_report(
    path: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    load_path_report(path, glass_dirs, catalog, state, None)
}

/// load_state_report と同じ。split_path はビームスプリッターの経路（None ならプロジェクト・状態の path）
pub fn load_path_report(
    path: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
    split_path: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    let txt = lensbench_core::vfs::read_to_string(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    expand_json_path(&txt, &base, glass_dirs, catalog, state, split_path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// プロジェクトの JSON（ファイルの中身と同じ形）を展開する。相対パスは base から。
/// 展開で落としたもの（[`expand_report`]）の説明も返す
pub fn expand_json(
    txt: &str,
    base: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    expand_json_path(txt, base, glass_dirs, catalog, state, None)
}

/// expand_json と同じ。split_path はビームスプリッターの経路（None ならプロジェクト・状態の path）
pub fn expand_json_path(
    txt: &str,
    base: &Path,
    glass_dirs: &[PathBuf],
    catalog: Option<CatalogLookup>,
    state: Option<&str>,
    split_path: Option<&str>,
) -> Result<(System, Vec<String>), String> {
    let p = Project::from_json(txt)?;
    let p = match state {
        Some(n) => p.with_state(n)?,
        None => p,
    };
    let env = Env {
        base: base.to_path_buf(),
        glass_dirs: glass_dirs.to_vec(),
        catalog,
    };
    expand_path(&p, &env, split_path)
}

#[cfg(test)]
mod retarder_spec_tests {
    use super::*;

    /// 偏光子・波長板の面を書き戻すと同じ書き方に。遅延 0 の波長板は波長板のまま（設計波長を残す）
    #[test]
    fn surface_spec_round_trip() {
        for q in [
            SurfSpec {
                waveplate: Some(WaveplateSpec {
                    retardance: 0.0,
                    wavelength_um: 0.55,
                    axis_deg: 10.0,
                    order: 0,
                    transmittance: 1.0,
                    material: None,
                    thickness_mm: None,
                    plates: Vec::new(),
                }),
                ..Default::default()
            },
            // 複屈折材の板（1 枚・2 枚の組）は厚さのまま戻る
            SurfSpec {
                waveplate: Some(WaveplateSpec {
                    retardance: 0.25,
                    wavelength_um: 0.633,
                    axis_deg: 45.0,
                    order: 0,
                    transmittance: 1.0,
                    material: Some("QUARTZ".into()),
                    thickness_mm: Some(0.5),
                    plates: Vec::new(),
                }),
                ..Default::default()
            },
            SurfSpec {
                waveplate: Some(WaveplateSpec {
                    retardance: 0.0,
                    wavelength_um: 0.633,
                    axis_deg: 0.0,
                    order: 0,
                    transmittance: 1.0,
                    material: None,
                    thickness_mm: None,
                    plates: vec![
                        PlateSpec {
                            material: "QUARTZ".into(),
                            thickness_mm: 0.4,
                            axis_deg: 0.0,
                        },
                        PlateSpec {
                            material: "MGF2".into(),
                            thickness_mm: 0.3,
                            axis_deg: 90.0,
                        },
                    ],
                }),
                ..Default::default()
            },
            SurfSpec {
                polarizer: Some(PolarizerSpec {
                    axis_deg: 30.0,
                    extinction_ratio: 500.0,
                    transmittance: 0.9,
                }),
                ..Default::default()
            },
        ] {
            let s = surf(&q).unwrap();
            let back = surface_spec(&s, false).unwrap();
            assert_eq!(back.waveplate, q.waveplate);
            if let (Some(a), Some(b)) = (back.polarizer, q.polarizer) {
                assert!((a.extinction_ratio - b.extinction_ratio).abs() < 1e-9);
                assert!((a.transmittance - b.transmittance).abs() < 1e-12);
                assert_eq!(a.axis_deg, b.axis_deg);
            } else {
                assert_eq!(back.polarizer, q.polarizer);
            }
            assert!(back.jones.is_none());
        }
        // 面の tilt（くさび）は tilt のまま戻る
        let q = SurfSpec {
            tilt: Some([0.5, -0.25]),
            semi: Semi::Value(5.0),
            ..Default::default()
        };
        let back = surface_spec(&surf(&q).unwrap(), true).unwrap();
        assert_eq!(back.tilt, Some([0.5, -0.25]));
        // 順序 1（ty → tx）の傾きは、両方 0 でないと tilt では書けない（片方だけなら同じ向き）
        let mut s = surf(&q).unwrap();
        s.before.as_mut().unwrap().order = 1;
        assert!(surface_spec(&s, true).is_err());
        let q1 = SurfSpec {
            tilt: Some([0.0, -0.25]),
            ..Default::default()
        };
        let mut s = surf(&q1).unwrap();
        s.before.as_mut().unwrap().order = 1;
        assert_eq!(surface_spec(&s, true).unwrap().tilt, Some([0.0, -0.25]));
        // 座標ブレークと一緒は誤り
        let bad = SurfSpec {
            tilt: Some([1.0, 0.0]),
            coordbrk: Some([0.0; 5]),
            ..Default::default()
        };
        assert!(surf(&bad).is_err());
    }
}

#[cfg(test)]
mod read_text_tests {
    use super::*;
    use std::io::Write;

    /// gzip の展開は上限まで（展開爆弾はエラー）、上限以内ならそのまま読む
    #[test]
    fn gzip_expansion_is_limited() {
        let dir = std::env::temp_dir().join(format!("lbgzlim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("big.json.gz");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&vec![b' '; 1 << 20]).unwrap();
        std::fs::write(&f, gz.finish().unwrap()).unwrap();
        let e = read_text_limit(&f, 1000).unwrap_err();
        assert!(e.contains("超える"), "{e}");
        assert_eq!(read_text_limit(&f, 1 << 20).unwrap().len(), 1 << 20);
        // .json を指すと .json.gz を読む
        assert_eq!(read_text(&dir.join("big.json")).unwrap().len(), 1 << 20);
        // ユーザーカタログ: .gz の名前へは圧縮して保存、.json.gz だけあれば load_or_new はそれを読む
        let c = UserCatalog::default();
        let g = dir.join("cat.json.gz");
        c.save(&g).unwrap();
        assert!(std::fs::read(&g).unwrap().starts_with(&[0x1f, 0x8b]));
        assert_eq!(
            UserCatalog::load_or_new(&dir.join("cat.json"))
                .unwrap()
                .to_json(),
            c.to_json()
        );
        assert!(UserCatalog::load_or_new(&dir.join("none.json")).is_ok());
        // load_shared は同じファイルなら覚えたものを返し、書き換えれば読み直す（.json.gz だけでも読める）
        assert_eq!(
            UserCatalog::load_shared(&dir.join("cat.json"))
                .unwrap()
                .to_json(),
            c.to_json()
        );
        let p = dir.join("shared.json");
        let mut a = UserCatalog {
            vendor: "A".into(),
            ..Default::default()
        };
        a.save(&p).unwrap();
        let x = UserCatalog::load_shared(&p).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &x,
            &UserCatalog::load_shared(&p).unwrap()
        ));
        a.vendor = "BB".into();
        a.save(&p).unwrap();
        assert_eq!(UserCatalog::load_shared(&p).unwrap().vendor, "BB");
        std::fs::remove_dir_all(&dir).ok();
    }
}
