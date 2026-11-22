//! プロジェクトファイルの形（ブロック・面・状態・環境）と状態の当て方

use super::*;
use lensbench_core::error::{code, Error};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Wave {
    pub um: f64,
    #[serde(default = "wave_weight")]
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApertureSpec {
    #[serde(rename = "type")]
    pub kind: ApertureKind,
    #[serde(default)]
    pub value: f64,
}

/// 視野の種類（core の [`FieldKind`]）
pub use lensbench_core::system::FieldKind;

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
    /// 視野の正規化（瞳・視野の座標の 1 を何にとるか。.zmx の FTYP の 6 番目）
    #[serde(default, skip_serializing_if = "is_radial")]
    pub norm: FieldNorm,
    /// 視野ごとのケラレ係数 [VDX, VDY, VCX, VCY, VAN]（.zmx の VDXN〜VANN、VAN は度）。省略した視野は 0
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vignetting: Vec<[f64; 5]>,
}

fn is_radial(n: &FieldNorm) -> bool {
    *n == FieldNorm::Radial
}

impl Default for FieldsSpec {
    fn default() -> Self {
        FieldsSpec {
            kind: FieldKind::Angle,
            points: vec![[0.0, 0.0]],
            weights: Vec::new(),
            solve: None,
            norm: FieldNorm::Radial,
            vignetting: Vec::new(),
        }
    }
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
    pub(crate) fn moved(&self) -> bool {
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
    /// 面を無視する（OpticStudio の Ignore This Surface。座標ブレークにも書ける）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignore: bool,
    /// 複屈折材の板の入口・出口（OpticStudio の Birefringent In / Out）。入口の `glass` が常光の材料（異常光は 名前-E）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birefringent: Option<BireSpec>,
    /// 面の形（省略は標準面。`asph` があれば偶数次非球面）。`odd_asph` は `asph` が β1..β8（r¹..r⁸ の係数、
    /// OpticStudio の Odd Asphere）。`toroidal` は `radius`・`conic`・`asph` が y 方向の曲線で、x 方向の曲率半径が
    /// `radius_x`（省略・0 なら円柱面、OpticStudio の Toroidal）。`fresnel` は平面の上で、屈折だけ
    /// `radius`・`conic`・`asph` の面の傾きで決まる（OpticStudio の Fresnel）
    #[serde(default, skip_serializing_if = "SurfShape::is_standard")]
    pub shape: SurfShape,
    /// トロイダル面の x 方向の曲率半径 [mm]（省略・0 なら円柱面）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_x: Option<f64>,
    /// ほかの欄で書けない面（Zernike・Grid Sag・Irregular・Tilted・Binary 2・理想レンズ・GRIN の媒質・
    /// dn・disp など）を、システムファイル（版 2）の面の行で書く。書くときはほかの欄を書かない。絞りの印（stop）は
    /// 書けない（ブロックの set_stop で）。玉の最後の面の厚さ・媒質は使わない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<Box<crate::sysfile_v2::Row>>,
}

/// 面の形（[`SurfSpec::shape`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SurfShape {
    /// 標準面・偶数次非球面
    #[default]
    Standard,
    /// Odd Asphere
    OddAsph,
    /// トロイダル面（円柱面を含む）
    Toroidal,
    /// Fresnel（平面の上の屈折）
    Fresnel,
}

impl SurfShape {
    pub(crate) fn is_standard(&self) -> bool {
        *self == SurfShape::Standard
    }
}

/// 複屈折材の板の入口・出口の面。`{"in": {"axis": [x, y, z]}}` か `"out"`
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BireSpec {
    In {
        /// 2: 波長板（常光の道で、常光・異常光の両方の位相）、0: 常光の成分だけ、1: 異常光の道で異常光の成分だけ、3: 異常光の道で両方の位相（異常光の道は walk-off して進む）。OpticStudio の Mode
        #[serde(default = "two_u8")]
        mode: u8,
        /// 結晶軸の向き（面の局所座標）
        axis: [f64; 3],
    },
    Out,
}

pub(crate) fn two_u8() -> u8 {
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

pub(crate) fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

pub(crate) fn inf_f64() -> f64 {
    f64::INFINITY
}

pub(crate) fn is_inf(x: &f64) -> bool {
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
    pub fn retarder(&self) -> lensbench_core::Result<lensbench_core::surface::Retarder> {
        if !(self.transmittance >= 0.0 && self.transmittance <= 1.0) {
            return Err(format!("polarizer transmittance {} (0 to 1)", self.transmittance).into());
        }
        if self.extinction_ratio.is_nan() || self.extinction_ratio < 1.0 {
            return Err(format!(
                "polarizer extinction ratio {} (1 or more)",
                self.extinction_ratio
            )
            .into());
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
    pub fn retarder(&self) -> lensbench_core::Result<lensbench_core::surface::Retarder> {
        if !(self.wavelength_um > 0.0 && self.wavelength_um.is_finite()) {
            return Err(format!("waveplate design wavelength {} µm", self.wavelength_um).into());
        }
        if !(self.transmittance >= 0.0 && self.transmittance <= 1.0) {
            return Err(format!("waveplate transmittance {} (0 to 1)", self.transmittance).into());
        }
        if !self.retardance.is_finite() {
            return Err("the waveplate retardance is not a number".into());
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
            (Some(_), false) => {
                return Err("a waveplate can't have both material and plates".into())
            }
            (Some(m), true) => {
                let d = self.thickness_mm.unwrap_or(0.0);
                if self.thickness_mm.is_some() && !(d > 0.0 && d.is_finite()) {
                    return Err(format!("waveplate thickness {d} mm").into());
                }
                let total = self.order as f64 + self.retardance;
                if self.thickness_mm.is_none() && !(total > 0.0 && total.is_finite()) {
                    return Err("a waveplate without a thickness needs a positive retardance order + retardance".into());
                }
                vec![plate(m, d, 0.0)]
            }
            (None, false) => {
                if self.thickness_mm.is_some() {
                    return Err("a waveplate with plates takes a thickness for each plate".into());
                }
                let mut v = Vec::new();
                for p in &self.plates {
                    if !(p.thickness_mm > 0.0 && p.thickness_mm.is_finite()) {
                        return Err(format!(
                            "waveplate plate {} thickness {} mm",
                            p.material, p.thickness_mm
                        )
                        .into());
                    }
                    if !p.axis_deg.is_finite() {
                        return Err("the waveplate plate axis is not a number".into());
                    }
                    v.push(plate(&p.material, p.thickness_mm, p.axis_deg));
                }
                v
            }
            (None, true) => {
                if self.thickness_mm.is_some() {
                    return Err("write the waveplate thickness together with material".into());
                }
                Vec::new()
            }
        };
        if plates.iter().any(|p| p.material.is_empty()) {
            return Err("empty birefringent material name of the waveplate".into());
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
    /// .zmx の k 番目の玉（玉だけを数える、1 始まり）。lensbench の系の .json でもよい
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
        #[serde(flatten)]
        faces: FaceMods,
    },
    Air {
        #[serde(default)]
        id: String,
        t: f64,
        /// 間隔を満たす媒質（液浸の油・水など、硝材の名前か屈折率の数）。省略は空気。最後の間隔なら像空間の媒質
        #[serde(default, skip_serializing_if = "Option::is_none")]
        medium: Option<String>,
    },
    /// 絞り（空気中のダミー面）
    Stop {
        #[serde(default)]
        id: String,
        #[serde(default)]
        semi: f64,
        #[serde(flatten)]
        faces: FaceMods,
    },
    Mirror {
        #[serde(default)]
        id: String,
        /// 曲率半径（省略・0 なら平面）
        #[serde(default)]
        radius: Option<f64>,
        #[serde(default)]
        conic: f64,
        /// 傾き x・y [deg]。折り曲げでは鏡の前後で同じ角だけ傾ける（光軸が折れる。後ろは回す順を逆にして、2 軸でも後の軸が反射した光軸に乗る）。
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
        /// radius・conic で書けない鏡（Zernike・Grid Sag など）を、システムファイル（版 2）の面の行で書く。
        /// 働きは鏡（reflect、または mirror の付く働き）。書くときは radius・conic・semi・aperture・coat を書かない
        /// （行に書く）。厚さ・媒質・絞りの印は書かない（間隔は後の空気、絞りはブロックの set_stop で）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        row: Option<Box<crate::sysfile_v2::Row>>,
        #[serde(flatten)]
        faces: FaceMods,
    },
    /// 空気中の面の並び（玉に分けない）
    Surfaces {
        #[serde(default)]
        id: String,
        surfaces: Vec<SurfSpec>,
        #[serde(flatten)]
        place: Place,
        #[serde(flatten)]
        faces: FaceMods,
    },
    /// .zmx から取り込んだ系（面 from〜to、既定は面 1 から像面の手前まで。空気間隔も含む）。`path` は lensbench の系
    /// （系ファイル・プロジェクトの .json、構成は 1 だけ）でもよい
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
        /// 取り込むプロジェクト（.json）の状態（省略なら状態の変更なし）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<String>,
        /// 取り込むプロジェクト（.json）の BS の経路（T・R の文字列。省略なら状態・プロジェクトの path）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        beam_path: Option<String>,
        /// 玉モードの群（.zmx の面の範囲）
        #[serde(default)]
        groups: Vec<ZmxGroup>,
        #[serde(flatten)]
        place: Place,
        #[serde(flatten)]
        faces: FaceMods,
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
        #[serde(flatten)]
        faces: FaceMods,
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
        #[serde(flatten)]
        faces: FaceMods,
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
    pub fn check(&self) -> lensbench_core::Result<()> {
        let k = detectors::get(&self.kind).ok_or_else(|| {
            format!(
                "unknown block type {:?} ({} or a registered detector: {})",
                self.kind,
                BUILTIN_BLOCKS.join(", "),
                detectors::list()
                    .iter()
                    .map(|k| k.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        Ok((k.check)(&self.params).map_err(|e| format!("{} {}: {e}", self.kind, self.id))?)
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
pub(crate) fn check_detectors(blocks: &[Block]) -> lensbench_core::Result<()> {
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
    #[serde(flatten)]
    pub faces: FaceMods,
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

/// 光束の役割（[`Beam::role`]）。表示の色と既定の組み合わせだけに使い、計算は変えない
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BeamRole {
    /// 試料の光束（被検の系を通る）
    Sample,
    /// 参照の光束
    Reference,
    /// 迷光の経路
    Stray,
}

/// 光束（[`Project::beams`]）: ビームスプリッターの経路 1 本に名前を付けたもの。経路を選ぶだけで、分岐比・膜・偏光は
/// 追跡で決まる。名前は状態の名前と同じ所（`--state`、干渉の光束など）で引ける
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Beam {
    /// 光束の名前（状態の名前と重ならない）
    pub name: String,
    /// ビームスプリッターの経路（BS を通る順の T・R）。省略で状態・プロジェクトの path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// この光束に当てる状態の名前（経路ごとに系の設定を変える）。省略で状態を当てない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// 役割（`sample`・`reference`・`stray`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<BeamRole>,
    /// 解析の既定の組から外す
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub off: bool,
}

/// プロジェクトファイルの形式の印（[`Project::format`]）
pub const PROJECT_FORMAT: &str = "lensbench-project";
/// プロジェクトファイルの形式の版。これより新しい版のファイルは読まない（知らない項目を落として黙って計算しないよう）
pub const PROJECT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    /// 形式の印 "lensbench-project"。書くときはいつも付ける（省略したファイルは今の版として読む）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// 形式の版（今は 1）。読める版より新しければ誤り
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_version: Option<u32>,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_catalogs")]
    pub glass_catalogs: Vec<String>,
    /// 膜（名前 → フィルタの透過特性・表）。面の `coat` に名前を書いて使う。ユーザーカタログの同じ名前より優先
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub coatings: BTreeMap<String, filter::CoatSpec>,
    /// 膜ファイル（OpticStudio の COATING.DAT の形）。面の `coat` の名前を、`coatings` に無ければこのファイルで引く。
    /// 相対パスはプロジェクトのフォルダから、そこに無ければ材料の検索フォルダから。取り込んだ .zmx の膜ファイルより優先
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coating_file: Option<String>,
    /// ZPL のマクロ（名前 `name` と文 `source`、.ZPL の中身）。ブロックの `surface_ops` の `add_solve` で種類
    /// `macro_curv`・`macro_thick` の `param` にこの並びの番号（0 始まり）を書いて使う。マクロのファイルは読まない
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<lensbench_core::zpl_solve::Macro>,
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
    pub aiming: Aiming,
    /// 物体側テレセントリック（物体距離が有限のときだけ）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub telecentric: bool,
    /// 像空間をアフォーカルにする。値は角度の単位（`ur`・`mr`・`rr`・`as`・`am`・`dg`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub afocal: Option<String>,
    /// 浮動開口（半直径を固定した面）の外の光線をけるか（既定はける。OpticStudio と同じ）
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub floating_clip: bool,
    /// 瞳の振幅の分布（`"uniform"`・`{"gaussian": G}`・`"cosine_cubed"`。省略は一様）
    #[serde(default, skip_serializing_if = "is_uniform")]
    pub apodization: lensbench_core::pupil::Apodization,
    /// OPD の基準（.zmx の ROPD。省略は 2: 出射瞳の参照球）
    #[serde(default = "two_i64", skip_serializing_if = "is_two_i64")]
    pub ropd: i64,
    /// 光線エイミングの瞳のずれ・圧縮（.zmx の PUSH と RAIM の 10 番目。追跡には効かない）。省略は全部 0
    #[serde(
        default,
        skip_serializing_if = "lensbench_core::system::AimPupil::is_default"
    )]
    pub aim_pupil: lensbench_core::system::AimPupil,
    /// 系の入射偏光（.zmx の POLS。膜・偏光を入れる解析の入射の偏光）。省略は偏光なし
    #[serde(
        default,
        skip_serializing_if = "lensbench_core::system::InputPolarization::is_default"
    )]
    pub input_pol: lensbench_core::system::InputPolarization,
    /// 定義した視野以外の点のケラレ係数の求め方（`"continuous"`・`"opticstudio"`。省略は連続）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vig_interp: Option<lensbench_core::system::VigInterp>,
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub states: Vec<State>,
    /// ビームスプリッターの経路（BS を通る順の T・R）。省略で各 BS の arm。状態の project でも変えられる
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 光束の組（名前つきの経路）。光束の名前は状態の名前の代わりに使え、光束の状態を当てて経路を選んだ系になる
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beams: Vec<Beam>,
    /// 機構部品（チューブ・ケージ板・マウント）。光線の追跡は変えず、ケラレの確認（`mech-check`）と描画に使う
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mechanics: Vec<MechPlacement>,
    /// 大域座標の基準の面（.zmx の GLRS。書き出し・3D の座標の原点）。ブロックの面 `{"block": id, "surface": i}` か、
    /// 展開した系の面の番号か名前。省略で面 1
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glrs: Option<BlockRef>,
    /// 温度・気圧（ガラスの屈折率の計算）。省略で 20 °C・1 atm・補正なし
    #[serde(default, skip_serializing_if = "Environment::is_default")]
    pub environment: Environment,
    /// 像面（省略は平面で傾きなし）。システムファイル（版 2）の面の行で、形・面の前の傾き偏心・開口などを書く。
    /// 厚さ・媒質・絞りの印は書けない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Box<crate::sysfile_v2::Row>>,
    /// 物体面の形・開口・物体空間の媒質（省略は平面で開口なし・空気）。`image` と同じ面の行で、厚さは
    /// `object_distance`。`medium` は物体空間の媒質（液浸など）。厚さ・絞りの印・傾き偏心は書けない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<Box<crate::sysfile_v2::Row>>,
    /// GUI が自分の編集の状態（組合せの表など）を残す欄。展開には使わない
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gui: Option<Value>,
}

/// 系の環境（[`Project::environment`]）。.zmx の ENVD（温度・気圧・屈折率の補正）と同じ
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Environment {
    /// 温度 [°C]
    #[serde(default = "env_t")]
    pub temperature_c: f64,
    /// 気圧 [atm]
    #[serde(default = "env_p")]
    pub pressure_atm: f64,
    /// ガラスの屈折率を温度・気圧で補正する（OpticStudio の Adjust Index Data To Environment）
    #[serde(default)]
    pub adjust_index: bool,
}

pub(crate) fn env_t() -> f64 {
    20.0
}
pub(crate) fn env_p() -> f64 {
    1.0
}

impl Default for Environment {
    fn default() -> Self {
        Environment {
            temperature_c: env_t(),
            pressure_atm: env_p(),
            adjust_index: false,
        }
    }
}

impl Environment {
    pub(crate) fn is_default(&self) -> bool {
        *self == Environment::default()
    }
}

/// ブロックの面の上書き・絞り・ソルブ・ピックアップ（GUI の面の表で変えたもの）。ブロックと .zmx の取り込みの群が持ち、
/// 展開の最後に当てる（上書きの後に操作）。状態で外したブロックのものは当てない
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct FaceMods {
    /// 面の値の上書き
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surface_edits: Vec<SurfaceEdit>,
    /// 面の絞り・ソルブ・ピックアップ。serve の `edit` の `set_stop`・`add_solve`・`add_pickup`・`remove_solve`・
    /// `remove_pickup` と同じ形で、`surface`・`dst` の番号はこのブロックの面（[`SurfaceEdit`] と同じ数え方）。
    /// `src`・`other` はほかのブロックの面 `{"block": id, "surface": i}` か系の面の番号・名前。
    /// 往復するブロックは往路の面を指す
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surface_ops: Vec<SurfaceOp>,
    /// 展開の中での番号（1 始まり、0 は無し。[`SurfOrigin::mods`]）。展開がプロジェクトの写しに付ける
    /// （BS の腕などブロックを写して進んでも同じブロックとわかるように）。読み書きしない
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) site: usize,
}

impl PartialEq for FaceMods {
    fn eq(&self, o: &Self) -> bool {
        self.surface_edits == o.surface_edits && self.surface_ops == o.surface_ops
    }
}

impl FaceMods {
    pub fn is_empty(&self) -> bool {
        self.surface_edits.is_empty() && self.surface_ops.is_empty()
    }
}

impl Block {
    /// ブロックの面の上書き・操作（持てる種類だけ）
    pub fn faces(&self) -> Option<&FaceMods> {
        match self {
            Block::Lens { faces, .. }
            | Block::Stop { faces, .. }
            | Block::Mirror { faces, .. }
            | Block::Surfaces { faces, .. }
            | Block::Zmx { faces, .. }
            | Block::User { faces, .. }
            | Block::Group { faces, .. } => Some(faces),
            _ => None,
        }
    }
    pub fn faces_mut(&mut self) -> Option<&mut FaceMods> {
        match self {
            Block::Lens { faces, .. }
            | Block::Stop { faces, .. }
            | Block::Mirror { faces, .. }
            | Block::Surfaces { faces, .. }
            | Block::Zmx { faces, .. }
            | Block::User { faces, .. }
            | Block::Group { faces, .. } => Some(faces),
            _ => None,
        }
    }
}

/// ブロックの面の値の上書き（[`FaceMods::surface_edits`]）。面はブロックの面のうち座標ブレークを除いて数えた
/// 番号（0 始まり。偏心・傾きの座標ブレークの有無で番号が変わらないように）。値は系の面の値（順次の面の値。
/// 鏡の後は厚さ・曲率の符号が逆）で、名前は [`lensbench_core::edit::SURF_FIELDS`]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceEdit {
    pub surface: usize,
    /// 座標ブレークの面を指す（surface はブロックの面の座標ブレークだけを数えた番号。ブロックの偏心・傾きの座標ブレークは
    /// 数えない。往復するブロックでは使えない）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub coordbrk: bool,
    pub fields: Map<String, Value>,
}

impl Project {
    /// プロジェクトファイルを読む（相対パスの基準は [`Env::for_file`]）
    pub fn load(path: &std::path::Path) -> lensbench_core::Result<Project> {
        let txt = lensbench_core::vfs::read_to_string(path)
            .map_err(lensbench_core::Error::io_at(path.display()))?;
        Project::from_json(&txt).map_err(|e| format!("{}: {e}", path.display()).into())
    }

    pub fn from_json(txt: &str) -> lensbench_core::Result<Project> {
        let v: Value = serde_json::from_str(txt).map_err(|e| format!("project file: {e}"))?;
        let p: Project =
            serde_json::from_value(v.clone()).map_err(|e| format!("project file: {e}"))?;
        // "$schema" はエディタの検査用（docs/schema）
        let unknown: Vec<String> = unknown_keys::<Project>(&v, &p)
            .into_iter()
            .filter(|k| k != "$schema")
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "project file: unknown key {} (check the spelling)",
                unknown.join(", ")
            )
            .into());
        }
        if let Some(f) = p.format.as_deref().filter(|f| *f != PROJECT_FORMAT) {
            return Err(
                format!("project file: format {f} (a project file is {PROJECT_FORMAT})").into(),
            );
        }
        if let Some(n) = p.format_version.filter(|&n| n > PROJECT_FORMAT_VERSION) {
            return Err(format!(
                "project file: format version {n} is newer than this lensbench reads ({PROJECT_FORMAT_VERSION}); update lensbench"
            )
            .into());
        }
        check_detectors(&p.blocks)?;
        p.check_beams()?;
        Ok(p)
    }

    /// 光束の組の誤り: 名前の空・重なり（光束どうし・状態と）、無い状態、経路の文字
    pub fn check_beams(&self) -> lensbench_core::Result<()> {
        for (i, b) in self.beams.iter().enumerate() {
            let name = &b.name;
            if name.trim().is_empty() {
                return Err(format!("beam {}: the name is empty", i + 1).into());
            }
            if self.beams[..i].iter().any(|c| c.name == *name) {
                return Err(format!("two or more beams named {name}").into());
            }
            if self.states.iter().any(|s| s.name == *name) {
                return Err(format!("beam {name}: a state has the same name").into());
            }
            if let Some(st) = &b.state {
                if !self.states.iter().any(|s| s.name == *st) {
                    return Err(Error::Invalid(format!("beam {name}: no state {st}"))
                        .with_code(code::STATE_NOT_FOUND));
                }
            }
            if let Some(path) = &b.path {
                crate::splitter::parse_path(path).map_err(|e| format!("beam {name}: {e}"))?;
            }
        }
        Ok(())
    }

    /// 光束の名前（[`Project::beams`] の順）
    pub fn beam_names(&self) -> Vec<&str> {
        self.beams.iter().map(|b| b.name.as_str()).collect()
    }

    /// 名前が name の光束
    pub fn beam(&self, name: &str) -> Option<&Beam> {
        self.beams.iter().find(|b| b.name == name)
    }

    /// 状態・光束の名前 name と一緒に別の経路を渡してよいか（経路を持つ光束には渡せない）
    pub fn beam_takes_path(&self, name: &str, path_given: bool) -> lensbench_core::Result<()> {
        if path_given && self.beam(name).is_some_and(|b| b.path.is_some()) {
            return Err(Error::Invalid(format!(
                "beam {name} has its own path, so another path can't be given"
            ))
            .with_code(code::BAD_ARGUMENT));
        }
        Ok(())
    }

    /// [`Project::with_state`]。別の経路を渡すなら（path_given）、経路を持つ光束は誤り
    pub fn with_state_and_path(
        &self,
        name: &str,
        path_given: bool,
    ) -> lensbench_core::Result<Project> {
        self.beam_takes_path(name, path_given)?;
        self.with_state(name)
    }

    /// プロジェクトの JSON（形式の印と版、先頭に `$schema` をいつも付ける）
    pub fn to_json(&self) -> String {
        let mut p = self.clone();
        p.format = Some(PROJECT_FORMAT.into());
        p.format_version = Some(PROJECT_FORMAT_VERSION);
        let txt = serde_json::to_string_pretty(&p).expect("JSON of plain data");
        crate::schema::stamp_text(&txt, "project")
    }

    pub fn state_names(&self) -> Vec<&str> {
        self.states.iter().map(|s| s.name.as_str()).collect()
    }

    /// 状態 name の変更を当てたプロジェクト（状態の一覧は残す）。name が光束の名前なら、光束の状態を当て、経路を
    /// 光束の経路にする
    pub fn with_state(&self, name: &str) -> lensbench_core::Result<Project> {
        if let Some(b) = self.beam(name) {
            let mut p = match &b.state {
                Some(st) => self
                    .state_only(st)
                    .map_err(|e| format!("beam {name}: {e}"))?,
                None => self.clone(),
            };
            if b.path.is_some() {
                p.path = b.path.clone();
            }
            return Ok(p);
        }
        self.state_only(name)
    }

    /// 状態 name（光束は引かない）の変更を当てたプロジェクト（[`Project::with_state`]）
    fn state_only(&self, name: &str) -> lensbench_core::Result<Project> {
        let st: Vec<&State> = self.states.iter().filter(|s| s.name == name).collect();
        let st = match st.as_slice() {
            [s] => *s,
            [] if self.states.is_empty() && !self.beams.is_empty() => {
                return Err(Error::Invalid(format!(
                    "no state or beam {name} (beams: {})",
                    self.beam_names().join(", ")
                ))
                .with_code(code::STATE_NOT_FOUND))
            }
            [] if !self.beams.is_empty() => {
                return Err(Error::Invalid(format!(
                    "no state or beam {name} (states: {}; beams: {})",
                    self.state_names().join(", "),
                    self.beam_names().join(", ")
                ))
                .with_code(code::STATE_NOT_FOUND))
            }
            [] if self.states.is_empty() => {
                return Err(
                    Error::Invalid(format!("no state {name} (the project has no states)"))
                        .with_code(code::STATE_NOT_FOUND),
                )
            }
            [] => {
                return Err(Error::Invalid(format!(
                    "no state {name} ({})",
                    self.state_names().join(", ")
                ))
                .with_code(code::STATE_NOT_FOUND))
            }
            _ => return Err(format!("two or more states named {name}").into()),
        };
        // 当てるのは変える項目を持つ所（系の値、id のブロック・群）だけで、プロジェクト全体を JSON に往復させない。
        // 変えてよいかは型が読むか（unknown_keys）で決める（前は書き出した JSON にキーがあるかで決めたので、
        // 既定値で省かれる項目は変えられなかった）
        let mut p = self.clone();
        if !st.project.is_empty() {
            let blocks = std::mem::take(&mut p.blocks);
            let states = std::mem::take(&mut p.states);
            let beams = std::mem::take(&mut p.beams);
            patch(
                &mut p,
                &st.project,
                &["blocks", "states", "beams"],
                name,
                |k| format!("state {name}: system value {k} can't be changed"),
            )?;
            p.blocks = blocks;
            p.states = states;
            p.beams = beams;
        }
        for (id, fields) in &st.set {
            let n = count_id(&p.blocks, id);
            if n == 0 {
                return Err(Error::Invalid(format!("state {name}: no block {id}"))
                    .with_code(code::BLOCK_NOT_FOUND));
            }
            if n > 1 {
                return Err(format!("state {name}: two or more blocks with id {id}").into());
            }
            let fixed = ["type", "id", "blocks", "groups"];
            let msg = |k: &str| format!("state {name}: {k} of block {id} can't be changed");
            let r = match find_node_mut(&mut p.blocks, id).expect("counted above") {
                NodeMut::Block(b) => patch(b, fields, &fixed, name, msg),
                NodeMut::ZmxGroup(g) => patch(g, fields, &fixed, name, msg),
            };
            r?;
        }
        Ok(p)
    }
}

/// x の項目 fields を置き換える（x を JSON にして当て、読み直す）。fixed の項目と、型が読まない項目は誤り（msg）。
/// name は状態の名前（誤りの文）
fn patch<T: Serialize + serde::de::DeserializeOwned>(
    x: &mut T,
    fields: &Map<String, Value>,
    fixed: &[&str],
    name: &str,
    msg: impl Fn(&str) -> String,
) -> lensbench_core::Result<()> {
    let mut v = serde_json::to_value(&*x).map_err(|e| Error::Invalid(e.to_string()))?;
    let obj = v.as_object_mut().expect("block and project are objects");
    for (k, val) in fields {
        if fixed.contains(&k.as_str()) {
            return Err(msg(k).into());
        }
        obj.insert(k.clone(), val.clone());
    }
    let y: T = serde_json::from_value(v.clone()).map_err(|e| format!("state {name}: {e}"))?;
    // 型が読まない項目（書いても効かない）は誤り。fields の中のキーだけを見る（入れ子の中は値の誤り）
    let unknown = unknown_keys(&v, &y);
    if let Some(k) = fields.keys().find(|k| {
        unknown
            .iter()
            .any(|u| u == *k || u.starts_with(&format!("{k}/")))
    }) {
        if unknown.iter().any(|u| u == k) {
            return Err(msg(k).into());
        }
        let u: Vec<&str> = unknown.iter().map(String::as_str).collect();
        return Err(format!("state {name}: unknown key {}", u.join(", ")).into());
    }
    *x = y;
    Ok(())
}

/// id を持つブロック・群（.zmx の取り込みの群も）
enum NodeMut<'x> {
    Block(&'x mut Block),
    ZmxGroup(&'x mut ZmxGroup),
}

/// zmx ブロックの .zmx のフォルダ（.zmx を読むときと同じく、ガラスのカタログを探す先に足す）
pub(crate) fn zmx_dirs(blocks: &[Block], base: &Path, out: &mut Vec<PathBuf>) {
    for b in blocks {
        if let Block::Zmx { path, .. } = b {
            if let Some(d) = base.join(path).parent() {
                if !out.iter().any(|x| x == d) {
                    out.push(d.to_path_buf());
                }
            }
        }
        for k in kids(b) {
            zmx_dirs(k, base, out);
        }
    }
}

/// 子のブロックの並び（群・BS の枝）
fn kids(b: &Block) -> Vec<&Vec<Block>> {
    match b {
        Block::Group { blocks, .. } => vec![blocks],
        Block::Splitter { spec, .. } => vec![&spec.t, &spec.r, &spec.d],
        _ => Vec::new(),
    }
}

/// id のブロック・群の数（[`block_pointers`] と同じ所を探す）
fn count_id(blocks: &[Block], id: &str) -> usize {
    if id.is_empty() {
        return 0;
    }
    let mut n = 0;
    for b in blocks {
        n += usize::from(crate::blocks::block_id(b) == id);
        if let Block::Zmx { groups, .. } = b {
            n += groups.iter().filter(|g| g.id == id).count();
        }
        n += kids(b).into_iter().map(|k| count_id(k, id)).sum::<usize>();
    }
    n
}

/// id のブロック・群（[`count_id`] と同じ順で最初のもの）
fn find_node_mut<'x>(blocks: &'x mut [Block], id: &str) -> Option<NodeMut<'x>> {
    if id.is_empty() {
        return None;
    }
    for b in blocks {
        if crate::blocks::block_id(b) == id {
            return Some(NodeMut::Block(b));
        }
        match b {
            Block::Zmx { groups, .. } => {
                if let Some(g) = groups.iter_mut().find(|g| g.id == id) {
                    return Some(NodeMut::ZmxGroup(g));
                }
            }
            Block::Group { blocks, .. } => {
                if let Some(x) = find_node_mut(blocks, id) {
                    return Some(x);
                }
            }
            Block::Splitter { spec, .. } => {
                for k in [&mut spec.t, &mut spec.r, &mut spec.d] {
                    if let Some(x) = find_node_mut(k, id) {
                        return Some(x);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// プロジェクトの JSON（`project["blocks"]`）で、id のブロック・群の JSON ポインタ（`/blocks/2/blocks/0` の形）。
/// 入れ子のブロック・群も探す。id が空なら空
pub fn block_pointers(blocks: &Value, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !id.is_empty() {
        find_id(blocks, "/blocks", id, &mut out);
    }
    out
}

/// id のブロック（群・BS の枝の中も、[`block_pointers`] と同じ順で最初のもの）
pub fn find_block_mut<'x>(blocks: &'x mut [Block], id: &str) -> Option<&'x mut Block> {
    if id.is_empty() {
        return None;
    }
    for b in blocks {
        if crate::blocks::block_id(b) == id {
            return Some(b);
        }
        let kids: Vec<&mut Vec<Block>> = match b {
            Block::Group { blocks, .. } => vec![blocks],
            Block::Splitter { spec, .. } => vec![&mut spec.t, &mut spec.r, &mut spec.d],
            _ => Vec::new(),
        };
        for k in kids {
            if let Some(x) = find_block_mut(k, id) {
                return Some(x);
            }
        }
    }
    None
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

/// 読んだ JSON のうち、型に読まれずに捨てられたキーの場所（`blocks/3/reversd` の形）。
/// `#[serde(flatten)]`・untagged の型には `deny_unknown_fields` を付けられないので、読んだ値を書き戻した JSON と
/// 比べて、書き戻しに無いキーを探す。書き戻しで省かれる既定値（`"on": true` など）と区別するため、そのキーの値を
/// 型に合わない値に替えて読み直し、読めて結果が変わらなければ「捨てられた」とする
pub fn unknown_keys<T: serde::de::DeserializeOwned + Serialize>(
    input: &Value,
    parsed: &T,
) -> Vec<String> {
    fn walk(i: &Value, o: &Value, path: &mut Vec<String>, miss: &mut Vec<Vec<String>>) {
        match (i, o) {
            (Value::Object(a), Value::Object(b)) => {
                for (k, x) in a {
                    path.push(k.clone());
                    match b.get(k) {
                        Some(y) => walk(x, y, path, miss),
                        None => miss.push(path.clone()),
                    }
                    path.pop();
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                for (n, (x, y)) in a.iter().zip(b).enumerate() {
                    path.push(n.to_string());
                    walk(x, y, path, miss);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let Ok(out) = serde_json::to_value(parsed) else {
        return Vec::new();
    };
    let mut miss = Vec::new();
    walk(input, &out, &mut Vec::new(), &mut miss);
    // 読まれないキーは、どんな値に替えても読めて同じになる。中身がすべて既定の構造体（書き出しで省くもの）は
    // 知らないキーだけの表としても読めてしまうので、型の違う値（文字列）でも試す
    let ignored = |p: &[String], junk: Value| {
        let mut probe = input.clone();
        let mut at = &mut probe;
        for k in p {
            at = match at {
                Value::Object(m) => m.get_mut(k).expect("walk で見たキー"),
                Value::Array(a) => &mut a[k.parse::<usize>().expect("添字")],
                _ => unreachable!(),
            };
        }
        *at = junk;
        serde_json::from_value::<T>(probe)
            .ok()
            .and_then(|t| serde_json::to_value(t).ok())
            .is_some_and(|v| v == out)
    };
    fn at<'a>(v: &'a Value, p: &[String]) -> Option<&'a Value> {
        p.iter().try_fold(v, |v, k| match v {
            Value::Object(m) => m.get(k),
            Value::Array(a) => a.get(k.parse::<usize>().ok()?),
            _ => None,
        })
    }
    // 読むキーが書き出しに無い（既定なので省いた）ときは、中のキーを同じように調べる（既定の欄の中の綴りの違い）
    fn check(
        p: Vec<String>,
        input: &Value,
        ignored: &dyn Fn(&[String], Value) -> bool,
        found: &mut Vec<Vec<String>>,
    ) {
        if ignored(&p, serde_json::json!({ "\u{1}lensbench_probe": [true] }))
            && ignored(&p, Value::from("\u{1}lensbench_probe"))
        {
            found.push(p);
        } else if let Some(Value::Object(m)) = at(input, &p) {
            for k in m.keys() {
                let mut c = p.clone();
                c.push(k.clone());
                check(c, input, ignored, found);
            }
        }
    }
    let mut found = Vec::new();
    for p in miss {
        check(p, input, &ignored, &mut found);
    }
    found.into_iter().map(|p| p.join("/")).collect()
}

fn is_uniform(a: &lensbench_core::pupil::Apodization) -> bool {
    *a == lensbench_core::pupil::Apodization::Uniform
}

fn two_i64() -> i64 {
    2
}

fn is_two_i64(v: &i64) -> bool {
    *v == 2
}
