//! プロジェクトファイルの形（ブロック・面・状態・環境）と状態の当て方

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Wave {
    pub um: f64,
    #[serde(default = "wave_weight")]
    pub weight: f64,
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
        /// 2: 波長板（常光の道で、常光・異常光の両方の位相）、0: 常光の成分だけ（OpticStudio の Mode）
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
    },
    /// 空気中の面の並び（玉に分けない）
    Surfaces {
        #[serde(default)]
        id: String,
        surfaces: Vec<SurfSpec>,
        #[serde(flatten)]
        place: Place,
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
pub(crate) fn check_detectors(blocks: &[Block]) -> Result<(), String> {
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
    /// 浮動開口（半直径を固定した面）の外の光線をけるか（既定はける。OpticStudio と同じ）
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub floating_clip: bool,
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
    /// 温度・気圧（ガラスの屈折率の計算）。省略で 20 °C・1 atm・補正なし
    #[serde(default, skip_serializing_if = "Environment::is_default")]
    pub environment: Environment,
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
        let v: Value =
            serde_json::from_str(txt).map_err(|e| format!("プロジェクトファイル: {e}"))?;
        let p: Project =
            serde_json::from_value(v.clone()).map_err(|e| format!("プロジェクトファイル: {e}"))?;
        // "$schema" はエディタの検査用（docs/schema）
        let unknown: Vec<String> = unknown_keys::<Project>(&v, &p)
            .into_iter()
            .filter(|k| k != "$schema")
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "プロジェクトファイル: 知らないキー {}（綴りを確かめる）",
                unknown.join("・")
            ));
        }
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
        let p: Project =
            serde_json::from_value(v.clone()).map_err(|e| format!("状態 {name}: {e}"))?;
        let unknown = unknown_keys::<Project>(&v, &p);
        if !unknown.is_empty() {
            return Err(format!("状態 {name}: 知らないキー {}", unknown.join("・")));
        }
        Ok(p)
    }
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
    miss.into_iter()
        .filter(|p| {
            let mut probe = input.clone();
            let mut at = &mut probe;
            for k in p {
                at = match at {
                    Value::Object(m) => m.get_mut(k).expect("walk で見たキー"),
                    Value::Array(a) => &mut a[k.parse::<usize>().expect("添字")],
                    _ => unreachable!(),
                };
            }
            *at = serde_json::json!({ "\u{1}lensbench_probe": [true] });
            serde_json::from_value::<T>(probe)
                .ok()
                .and_then(|t| serde_json::to_value(t).ok())
                .is_some_and(|v| v == out)
        })
        .map(|p| p.join("/"))
        .collect()
}
