//! ユーザーカタログと機構部品の形、カタログのファイル（gzip を含む）の読み

use super::*;
use lensbench_core::LockExt;
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

/// ユーザーカタログの項目。中身（光学の面・機構部品・未完）は [`ItemBody`] の 1 つで、意味のない欄の組み合わせは
/// 読むときに誤り。JSON は欄を平たく並べる（[`RawItem`]）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(try_from = "RawItem", into = "RawItem")]
pub struct UserItem {
    pub description: String,
    /// 項目の玉のガラスのカタログ（使うときに足す）
    pub glass_catalogs: Vec<String>,
    /// 中身
    pub body: ItemBody,
    /// 品の種類（一覧の分類。省略時はブロックから: 鏡・BS のブロック 1 つならそれ、ほかは玉）
    pub kind: Option<catalog_spec::ItemKind>,
    /// 製品ページ
    pub url: String,
    /// CAD データ（STEP・図面）の公式 URL
    pub cad: CadLinks,
    /// 仕様（名前 → 数・[下限, 上限]・文字列。名前の決まったものは `catalog_spec::SPEC_NAMES`）
    pub specs: BTreeMap<String, serde_json::Value>,
    pub tags: Vec<String>,
    /// 仕様の出典（資料・ページ）
    pub source: String,
    /// 仕様を確かめた日（YYYY-MM-DD）
    pub checked: String,
    /// 品の名前（一覧のページの見出しなど）
    pub title: String,
    /// データを作ったときのメモ（読めなかった欄など）
    pub notes: Vec<String>,
}

/// 品の中身
#[derive(Debug, Clone)]
pub enum ItemBody {
    /// 光学の面（空なら面の無い光学の品）。枠付きの品（マウント済みのフィルタなど）は枠も
    Optic {
        blocks: Vec<Block>,
        frame: Option<Frame>,
    },
    /// 機構部品の寸法
    Mech(MechSpec),
    /// 未完の品（足りない値 `thickness`・`diameter`）。配置できず、品ページの仕様表か STEP で補ってから使う
    /// （[`catalog_fill`]）
    Incomplete { missing: Vec<String> },
}

impl Default for ItemBody {
    fn default() -> Self {
        ItemBody::Optic {
            blocks: Vec::new(),
            frame: None,
        }
    }
}

/// 枠付きの品の枠
#[derive(Debug, Clone)]
pub struct Frame {
    /// 枠の寸法。z は枠の端から光の進む向きで、`holds.seat_z` にガラスの前面の位置を書く。プロジェクトに置くと、
    /// ガラスの最初の面に機構部品として付く（ケラレ・当たりの確認と描画）
    pub spec: MechSpec,
    /// 枠の向き: false なら光は枠の z の小さい端から入り、true なら大きい端から入る（STEP から測った枠は STEP の
    /// 座標の向きなので、図面の矢印などで決める）
    pub reversed: bool,
}

/// ユーザーカタログの項目（JSON の形）。光学の面（blocks、枠付きなら frame も）・機構部品（mech）・未完（missing）の
/// どれか 1 つで、意味のない組み合わせは読むときに誤り（[`UserItem`] に直す）
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
#[schemars(rename = "UserItem")]
pub struct RawItem {
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
    /// CAD データ（STEP・図面）の公式 URL
    #[serde(default, skip_serializing_if = "CadLinks::is_empty")]
    pub cad: CadLinks,
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

impl TryFrom<RawItem> for UserItem {
    type Error = String;
    /// 意味のない組み合わせ（面と機構の寸法の両方、面の無い枠、枠の無い `frame_reversed`、面か機構の寸法のある品の
    /// `missing`）は誤り。どれも無い品は面の無い光学の品
    fn try_from(r: RawItem) -> Result<Self, String> {
        let bad = |m: &str| Err(m.to_string());
        if r.frame_reversed && r.frame.is_none() {
            return bad("frame_reversed without a frame");
        }
        let body = match (r.mech, r.blocks.is_empty()) {
            (Some(_), false) => {
                return bad("both surfaces (blocks) and mechanism dimensions (mech)")
            }
            (Some(_), true) if r.frame.is_some() => return bad("a frame on a mechanical part"),
            (Some(_), true) if !r.missing.is_empty() => {
                return bad("missing values on a mechanical part")
            }
            (Some(m), true) => ItemBody::Mech(m),
            (None, false) if !r.missing.is_empty() => {
                return bad("missing values on an item with surfaces")
            }
            (None, true) if r.frame.is_some() => return bad("a frame without surfaces (blocks)"),
            (None, true) if !r.missing.is_empty() => ItemBody::Incomplete { missing: r.missing },
            (None, _) => ItemBody::Optic {
                blocks: r.blocks,
                frame: r.frame.map(|spec| Frame {
                    spec,
                    reversed: r.frame_reversed,
                }),
            },
        };
        Ok(UserItem {
            description: r.description,
            glass_catalogs: r.glass_catalogs,
            body,
            kind: r.kind,
            url: r.url,
            cad: r.cad,
            specs: r.specs,
            tags: r.tags,
            source: r.source,
            checked: r.checked,
            title: r.title,
            notes: r.notes,
        })
    }
}

impl From<UserItem> for RawItem {
    fn from(u: UserItem) -> RawItem {
        let mut r = RawItem {
            description: u.description,
            glass_catalogs: u.glass_catalogs,
            kind: u.kind,
            url: u.url,
            cad: u.cad,
            specs: u.specs,
            tags: u.tags,
            source: u.source,
            checked: u.checked,
            title: u.title,
            notes: u.notes,
            ..Default::default()
        };
        match u.body {
            ItemBody::Optic { blocks, frame } => {
                r.blocks = blocks;
                if let Some(f) = frame {
                    (r.frame, r.frame_reversed) = (Some(f.spec), f.reversed);
                }
            }
            ItemBody::Mech(m) => r.mech = Some(m),
            ItemBody::Incomplete { missing } => r.missing = missing,
        }
        r
    }
}

impl JsonSchema for UserItem {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        RawItem::schema_name()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        RawItem::json_schema(g)
    }
}

impl UserItem {
    /// 光学の面 blocks の品（枠なし）
    pub fn optic(description: &str, glass_catalogs: Vec<String>, blocks: Vec<Block>) -> UserItem {
        UserItem {
            description: description.to_string(),
            glass_catalogs,
            body: ItemBody::Optic {
                blocks,
                frame: None,
            },
            ..Default::default()
        }
    }
    /// 光学の面を書き換える（機構部品・未完の品は None）
    pub fn blocks_mut(&mut self) -> Option<&mut Vec<Block>> {
        match &mut self.body {
            ItemBody::Optic { blocks, .. } => Some(blocks),
            _ => None,
        }
    }
    /// 光学の面（機構部品・未完の品は空）
    pub fn blocks(&self) -> &[Block] {
        match &self.body {
            ItemBody::Optic { blocks, .. } => blocks,
            _ => &[],
        }
    }
    /// 機構部品の寸法
    pub fn mech(&self) -> Option<&MechSpec> {
        match &self.body {
            ItemBody::Mech(m) => Some(m),
            _ => None,
        }
    }
    /// 枠付きの品の枠
    pub fn frame(&self) -> Option<&Frame> {
        match &self.body {
            ItemBody::Optic { frame, .. } => frame.as_ref(),
            _ => None,
        }
    }
    /// 未完の品の足りない値（ほかの品は空）
    pub fn missing(&self) -> &[String] {
        match &self.body {
            ItemBody::Incomplete { missing } => missing,
            _ => &[],
        }
    }
    /// 未完の品（足りない値があり、面も機構の寸法も無い）か
    pub fn is_incomplete(&self) -> bool {
        matches!(self.body, ItemBody::Incomplete { .. })
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
    /// 外形の形: 丸（既定、`outer_diameter` は外径）か角（`outer_diameter` は正方形の一辺。ケージ板など）。
    /// 配置図・3D・書き出しはこの簡易外形で描く
    #[serde(default, skip_serializing_if = "MechOutline::is_round")]
    pub outline: MechOutline,
}

/// 機構部品の簡易外形の形
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MechOutline {
    /// 丸（円柱）
    #[default]
    Round,
    /// 角（正方形の柱、辺は部品の局所の x・y 軸に沿う）
    Square,
}

impl MechOutline {
    pub fn is_round(&self) -> bool {
        *self == MechOutline::Round
    }
}

/// 品の CAD データの公式 URL（メーカーの配布元）。STEP はカタログに入れず、要るときに 1 件だけ取る
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CadLinks {
    /// STEP（.step・.stp。zip に入ったものも可）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub step: String,
    /// 図面の PDF
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pdf: String,
}

impl CadLinks {
    pub fn is_empty(&self) -> bool {
        self.step.is_empty() && self.pdf.is_empty()
    }
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
    /// 部品の端からねじの始まりまでの引っ込み [mm]（ケージ板のねじの穴の端が外形の端より奥にあるときなど）。
    /// つなぐとき、オスはこの分も奥へ入る
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub recess: f64,
}

/// 保持する素子
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HoldSpec {
    /// 保持する素子の径 [mm]（`max_diameter` があれば入る径の下限）
    pub diameter: f64,
    /// 素子の前面を当てる座の z [mm]
    #[serde(default)]
    pub seat_z: f64,
    /// 入る径の上限 [mm]。上下から挟むホルダー（バー型・ジョー型）のように径を選ばない部品は `diameter`〜これの
    /// 素子を挟む（素子の縁にだけ当たり、穴で切らない）。0 は径の決まった部品
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub max_diameter: f64,
}

impl HoldSpec {
    /// 径を選ばず挟む部品か（[`HoldSpec::max_diameter`]）
    pub fn is_range(&self) -> bool {
        self.max_diameter > self.diameter
    }
    /// 径 `d` の素子を挟めるか（挟む部品だけ。径の決まった部品は false）
    pub fn clamps(&self, d: f64) -> bool {
        self.is_range() && d >= self.diameter - 1e-6 && d <= self.max_diameter + 1e-6
    }
}

pub(crate) fn is_zero_f64(v: &f64) -> bool {
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
                seat_z: len - h.seat_z - t,
                ..h
            }),
            front_thread: self.back_thread.clone(),
            back_thread: self.front_thread.clone(),
            ..self.clone()
        }
    }

    pub fn validate(&self) -> lensbench_core::Result<()> {
        for (k, b) in self.bores.iter().enumerate() {
            if !(b.diameter > 0.0 && b.diameter.is_finite()) {
                return Err(
                    format!("mechanism bore {}: inner diameter {}", k + 1, b.diameter).into(),
                );
            }
            if !(b.z0.is_finite() && b.z1.is_finite() && b.z1 >= b.z0) {
                return Err(
                    format!("mechanism bore {}: z range {} to {}", k + 1, b.z0, b.z1).into(),
                );
            }
        }
        if !(self.length >= 0.0 && self.outer_diameter >= 0.0) {
            return Err("negative mechanism length or outer diameter".into());
        }
        if let Some(h) = &self.holds {
            if h.max_diameter != 0.0
                && !(h.max_diameter >= h.diameter && h.max_diameter.is_finite())
            {
                return Err(format!(
                    "mechanism holds: max_diameter {} is smaller than diameter {}",
                    h.max_diameter, h.diameter
                )
                .into());
            }
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
            // 挟む部品は素子の径に合わせて挟むので、保持径が太すぎる確認はしない
            hold_radius: self
                .holds
                .filter(|h| !h.is_range())
                .map_or(0.0, |h| h.diameter / 2.0),
            rod_spacing: self.rod_spacing,
            square: self.outline == MechOutline::Square,
            step: String::new(),
            step_flip: false,
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
    /// ねじ（`front_thread`・`back_thread`）があれば、同じねじでオスとメスか確かめ、オスのねじの長さ（とメスの引っ込み）だけ重ねる
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// 部品を裏返して置く（寸法の z の大きい端から光が入る。穴・座・両端のねじも入れ替わる）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reversed: bool,
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
    pub(crate) fn is_unset(&self) -> bool {
        self.block.is_none() && self.surface == 0
    }
}

impl UserCatalog {
    /// JSON から読む。品の欄の組み合わせも確かめる（[`UserItem`]）。組み合わせの誤りは `items.<名前>: <理由>`
    pub fn from_json(txt: &str) -> lensbench_core::Result<UserCatalog> {
        serde_json::from_str(txt).map_err(|e| {
            // どの品の誤りかを探す（serde_json の誤りには道が付かない）
            let v: serde_json::Value = serde_json::from_str(txt).unwrap_or_default();
            let named = v["items"].as_object().and_then(|m| {
                m.iter().find_map(|(k, x)| {
                    let r = serde_json::from_value::<RawItem>(x.clone());
                    let e = match r {
                        Ok(r) => UserItem::try_from(r).err()?,
                        Err(e) => e.to_string(),
                    };
                    Some(format!("user catalog: items.{k}: {e}"))
                })
            });
            lensbench_core::Error::Invalid(named.unwrap_or_else(|| format!("user catalog: {e}")))
        })
    }
    /// ユーザーカタログの JSON（先頭に `$schema` を付ける）
    pub fn to_json(&self) -> String {
        let txt = serde_json::to_string_pretty(self).expect("JSON of plain data");
        crate::schema::stamp_text(&txt, "user-catalog")
    }
    /// ファイルから読む。`.gz` は展開して読み、`x.json` が無くて `x.json.gz` があればそれを読む（[`read_text`]）
    pub fn load(path: &Path) -> lensbench_core::Result<UserCatalog> {
        let t = read_text(path)?;
        Ok(UserCatalog::from_json(&t).map_err(|e| format!("{}: {e}", path.display()))?)
    }
    /// [`Self::load`] と同じだが、読んだものをファイルの更新時刻・大きさと共に覚えておき、変わっていなければ
    /// 読み直さない（展開で品ごとに同じカタログを読むと、品の数の 2 乗の時間がかかるため）。読むだけの所で使う
    pub fn load_shared(path: &Path) -> lensbench_core::Result<std::sync::Arc<UserCatalog>> {
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
        if let Some((st, c)) = cache.lock_ok().get(&key) {
            if *st == stamp {
                return Ok(c.clone());
            }
        }
        let c = Arc::new(UserCatalog::load(path)?);
        cache.lock_ok().insert(key, (stamp, c.clone()));
        Ok(c)
    }
    /// ないファイル（`.gz` を足した名前も無い）なら空のカタログ。`.json.gz` だけあれば、それを読む
    /// （足して保存すると `.json` に書き、一覧では `.json` が勝つので、品は欠けない）
    pub fn load_or_new(path: &Path) -> lensbench_core::Result<UserCatalog> {
        let mut gz = path.as_os_str().to_owned();
        gz.push(".gz");
        if lensbench_core::vfs::exists(path) || lensbench_core::vfs::is_file(Path::new(&gz)) {
            UserCatalog::load(path)
        } else {
            Ok(UserCatalog::default())
        }
    }
    /// ファイルの今の中身を読み直して f で変え、すぐ書き戻す（読んでから時間のかかる処理の後に足すときに使う。
    /// 前に読んだ写しを書き戻すと、その間にほかの所（GUI・CLI）が足した品を消してしまう）。変えた後のカタログを返す
    pub fn update(
        path: &Path,
        f: impl FnOnce(&mut UserCatalog) -> lensbench_core::Result<()>,
    ) -> lensbench_core::Result<UserCatalog> {
        let mut c = UserCatalog::load_or_new(path)?;
        f(&mut c)?;
        c.save(path)?;
        Ok(c)
    }

    /// 保存する。名前が `.gz` で終われば gzip で圧縮して書く（一時ファイルに書いて名前を替える。途中で落ちても
    /// 前の中身が残る）
    pub fn save(&self, path: &Path) -> lensbench_core::Result<()> {
        use std::io::Write;
        let err = |e: std::io::Error| format!("{}: {e}", path.display());
        let text = self.to_json() + "\n";
        let gz = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gz"));
        if gz {
            let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            w.write_all(text.as_bytes()).map_err(err)?;
            Ok(lensbench_core::vfs::write_atomic(path, w.finish().map_err(err)?).map_err(err)?)
        } else {
            Ok(lensbench_core::vfs::write_atomic(path, text).map_err(err)?)
        }
    }
}

/// テキストのファイルを読む。拡張子 `.gz`（またはファイルの頭が gzip の印）なら展開する。`path` が無くて
/// `path` に `.gz` を足したファイルがあればそれを読む（リリースのカタログは .json.gz で配る）
pub fn read_text(path: &Path) -> lensbench_core::Result<String> {
    read_text_limit(path, GZ_LIMIT)
}

/// gzip を展開した大きさの上限 [byte]（展開爆弾で落ちないように。カタログは大きくても数十 MB）
pub const GZ_LIMIT: u64 = 256 << 20;

pub(crate) fn read_text_limit(path: &Path, limit: u64) -> lensbench_core::Result<String> {
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
            return Err(format!("{}: over {limit} bytes when inflated", p.display()).into());
        }
        out
    } else {
        b
    };
    Ok(String::from_utf8(b).map_err(|e| format!("{}: {e}", p.display()))?)
}

/// カタログのファイルか（`.json`・`.json.gz`）
pub fn is_catalog_file(p: &Path) -> bool {
    let n = p
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    n.ends_with(".json") || n.ends_with(".json.gz")
}

/// レンズカタログの品の .zmx の文（読んだもの raw）を系にする。展開・一覧の要約・差し替え探索で同じ設定にするための 1 か所
/// （読み直しは展開の [`SourceCache`] が省く）:
/// 構成 1、径はファイルの値で固定（カタログの玉の径は決まっている）。glasses を渡せば .zmx の GCAT のカタログの代わりに使う
pub fn catalog_system(
    raw: &zmx::Raw,
    glass_dirs: &[PathBuf],
    glasses: Option<HashMap<String, Glass>>,
) -> lensbench_core::Result<System> {
    zmx::build(
        raw,
        &zmx::LoadOptions {
            cfg: 1,
            fixed_semi: true,
            glasses,
            ..Default::default()
        },
        &glass::catalog_dirs(glass_dirs),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 品の欄の組み合わせ: 意味のあるものだけ通し、ほかは読むときに誤り
    #[test]
    fn item_body_rejects_meaningless_combinations() {
        let read = |items: &str| UserCatalog::from_json(&format!(r#"{{"items": {items}}}"#));
        let mech = r#"{"length": 10, "outer_diameter": 25}"#;
        let lens = r#"[{"type": "air", "t": 1}]"#;
        let c = read(&format!(
            r#"{{"m": {{"mech": {mech}}}, "i": {{"missing": ["thickness"]}}, "e": {{}}}}"#
        ))
        .unwrap();
        assert!(matches!(c.items["m"].body, ItemBody::Mech(_)));
        assert!(c.items["i"].is_incomplete());
        assert!(matches!(
            c.items["e"].body,
            ItemBody::Optic { frame: None, .. }
        ));
        let f = read(&format!(
            r#"{{"f": {{"blocks": {lens}, "frame": {mech}, "frame_reversed": true}}}}"#
        ))
        .unwrap();
        assert!(matches!(
            f.items["f"].body,
            ItemBody::Optic {
                frame: Some(Frame { reversed: true, .. }),
                ..
            }
        ));
        for (bad, why) in [
            (
                format!(r#"{{"blocks": {lens}, "mech": {mech}}}"#),
                "both surfaces",
            ),
            (
                format!(r#"{{"frame": {mech}}}"#),
                "a frame without surfaces",
            ),
            (
                r#"{"frame_reversed": true}"#.to_string(),
                "frame_reversed without a frame",
            ),
            (
                format!(r#"{{"mech": {mech}, "missing": ["diameter"]}}"#),
                "missing values on a mechanical part",
            ),
            (
                format!(r#"{{"blocks": {lens}, "missing": ["diameter"]}}"#),
                "missing values on an item with surfaces",
            ),
        ] {
            let e = read(&format!(r#"{{"x": {bad}}}"#)).unwrap_err().to_string();
            assert!(e.contains("items.x") && e.contains(why), "{e}");
        }
    }

    /// 書いて読み直すと同じ品で、JSON の欄は平たいまま
    #[test]
    fn item_round_trips_with_flat_fields() {
        let txt = r#"{"items": {"f": {"blocks": [{"type": "air", "t": 1}], "frame": {"length": 5},
            "frame_reversed": true, "url": "u"}, "m": {"mech": {"length": 10}}, "i": {"missing": ["thickness"]}}}"#;
        let c = UserCatalog::from_json(txt).unwrap();
        let v: serde_json::Value = serde_json::from_str(&c.to_json()).unwrap();
        assert_eq!(v["items"]["f"]["frame_reversed"], true);
        assert_eq!(v["items"]["f"]["frame"]["length"], 5.0);
        assert_eq!(v["items"]["m"]["mech"]["length"], 10.0);
        assert_eq!(v["items"]["i"]["missing"][0], "thickness");
        let d = UserCatalog::from_json(&c.to_json()).unwrap();
        assert_eq!(c.to_json(), d.to_json());
    }
}
