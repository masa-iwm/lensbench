//! 利用者が足す膜の口: 膜のファイル形式（拡張子ごとの読み手、[`CoatingSource`]）と、名前つきの膜の一覧
//! （[`CoatingCatalog`]）を拡張の表に足す（[`crate::ext::Ext::add_coating_source`]・
//! [`crate::ext::Ext::add_coating_catalog`]）。
//!
//! 膜の中身は今ある型のまま（[`CoatingSet`]: 層の並びとその材料・TABLE・特性行列のモデル・実測表）。膜の計算は
//! 同梱の膜と同じ経路を通る。
//!
//! 系の面の膜の名前は、次の順に引く（上ほど先。大文字小文字は問わない）:
//!
//! 1. 系に書いた表（フィルタ）
//! 2. 明示して読んだモデル・実測表（[`crate::coating::Coatings::load_file`]、CLI の `--coatings`）
//! 3. 登録した膜の一覧（[`CoatingCatalog`]）
//! 4. 系の膜ファイル（.zmx の COFN、COATING.DAT の形式）の COAT・TABLE
//! 5. 膜ファイルのフォルダの `<膜の名前>.lbcb`・`.lbcm`・`.lbct`・`.lbcs` と、登録した形式の `<膜の名前>.<拡張子>`
//! 6. 共有の置き場（[`crate::coating::set_remote_loader`]）
//!
//! 一覧の膜と同じ名前の膜が系の表・膜ファイル・フォルダにあれば、先のものを使い注意を返す
//! （[`crate::coating::Coatings::notes`]）。拡張を入れると、同じ .zmx でも膜ファイルの膜が一覧の膜に替わる。
//! 2 つの登録した一覧に同じ名前の膜があると、`Extensions` を入れる時の誤り（表に直接足したときは一覧の名前の順で先）。
//!
//! 約束:
//! - [`CoatingSource::load`] は状態を持たず、どのスレッドから呼んでもよい。ファイルの膜の名前はファイル名
//!   （拡張子なし）と同じにする（大文字小文字は問わない）
//! - 膜の名前に `|` を含めず、`-` そのものと `I.` で始まる名前にしない（境界の鍵・膜なし・理想膜と区別できない）
//! - 1 つの膜の名前は、層の並び・TABLE か、モデル・実測表のどちらか一方だけに使う（層・TABLE があるとモデル・実測表は使われない）
//! - 層の材料は、組の [`CoatingSet::mate`] か系のガラスの表で引く。系の膜ファイルに同じ名前の違う材料があれば、
//!   組の材料を `<材料>@<一覧の名前（ファイルなら名前）>` に名前を替えて入れる（系の膜ファイルの材料は変えない）

use std::collections::HashMap;
use std::path::Path;

use crate::coating::{CoatModel, CoatingFile, Layer, Measured, Table};
use crate::{invalid, Result};

/// 膜の組: 名前つきの膜（層の並び・TABLE・特性行列のモデル・実測表）と、層の材料。
/// 膜の種類は今後足すことがあるので（利用者が計算を書く膜など）、`CoatingSet::default()` から欄に足して作る
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CoatingSet {
    /// 層の材料: 波長 [µm]・n・k（COATING.DAT の MATE。k < 0 で吸収、波長の昇順）
    pub mate: HashMap<String, Vec<[f64; 3]>>,
    /// 膜の名前 → 層の並び（COATING.DAT の COAT。入射側から）
    pub coat: HashMap<String, Vec<Layer>>,
    /// 膜の名前 → 表（COATING.DAT の TABLE）
    pub table: HashMap<String, Table>,
    /// 膜の名前 → 特性行列のモデル（`.lbcm`・`.lbcb`）
    pub models: HashMap<String, CoatModel>,
    /// 境界の鍵（`膜|前の媒質|後の媒質`、鏡は後が `MIRROR`）→ 実測表（`.lbct`）
    pub measured: HashMap<String, Measured>,
}

impl CoatingSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// COATING.DAT の形式の文から（MATE・COAT・TABLE）
    pub fn parse_dat(txt: &str) -> Result<Self> {
        Ok(Self::from_file(CoatingFile::parse(txt)?))
    }

    /// 膜ファイルの中身から
    pub fn from_file(f: CoatingFile) -> Self {
        CoatingSet {
            mate: f.mate,
            coat: f.coat,
            table: f.table,
            ..Self::default()
        }
    }

    /// 層の並びの膜を足す（同じ名前は差し替える）
    pub fn add_layers(&mut self, name: &str, layers: Vec<Layer>) -> &mut Self {
        self.coat.insert(name.to_string(), layers);
        self
    }

    /// 層の材料を足す（行は 波長 [µm]・n・k、波長の昇順）
    pub fn add_material(&mut self, name: &str, rows: Vec<[f64; 3]>) -> &mut Self {
        self.mate.insert(name.to_string(), rows);
        self
    }

    /// 特性行列のモデルの文（`.lbcm`、[`crate::coating::parse_models`] の形式）を足す
    pub fn add_models_text(&mut self, txt: &str) -> Result<&mut Self> {
        self.models.extend(crate::coating::parse_models(txt)?);
        Ok(self)
    }

    /// 実測表の文（`.lbct`、[`crate::coating::parse_measured`] の形式）を足す
    pub fn add_measured_text(&mut self, txt: &str) -> Result<&mut Self> {
        self.measured.extend(crate::coating::parse_measured(txt)?);
        Ok(self)
    }

    /// 膜の名前（足したときの綴り、名前の順、重なりなし）
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .coat
            .keys()
            .chain(self.table.keys())
            .chain(self.models.keys())
            .cloned()
            .chain(
                self.measured
                    .keys()
                    .map(|k| k.split('|').next().unwrap_or("").to_string()),
            )
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// 名前の膜があるか（大文字小文字は問わない）
    pub fn contains(&self, name: &str) -> bool {
        let eq = |k: &String| k.eq_ignore_ascii_case(name);
        self.coat.keys().any(eq)
            || self.table.keys().any(eq)
            || self.models.keys().any(eq)
            || self.measured.keys().any(|k| {
                k.split('|')
                    .next()
                    .is_some_and(|c| c.eq_ignore_ascii_case(name))
            })
    }
}

/// 膜のファイル形式（拡張子ごとの読み手）。膜ファイルのフォルダの `<膜の名前>.<拡張子>` を膜の名前で読む。
/// 明示して読むファイル（[`crate::coating::Coatings::load_file`]）も拡張子でこの読み手を選ぶ
pub trait CoatingSource: Send + Sync {
    /// この実装が読める拡張子（小文字、ドットなし）。同梱の形式（lbcb・lbcm・lbct・lbcs）と同じなら、こちらが勝つ
    fn extensions(&self) -> &[&str];
    /// ファイルを読み、膜の組を返す（読めなければ `Error::Io`、中身の誤りは `Error::Invalid` など）
    fn load(&self, path: &Path) -> Result<CoatingSet>;
}

/// 名前つきの膜の一覧（カタログの単位）
#[derive(Debug, Clone)]
pub struct CoatingCatalog {
    pub name: String,
    set: CoatingSet,
}

impl CoatingCatalog {
    pub fn new(name: &str, set: CoatingSet) -> Self {
        CoatingCatalog {
            name: name.to_string(),
            set,
        }
    }

    /// 膜の組
    pub fn set(&self) -> &CoatingSet {
        &self.set
    }
}

/// 膜の名前が約束（`|` を含まない、`-` そのものでも `I.` で始まるのでもない、空でない）を守るか
fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('|') || name == "-" || name.starts_with("I.") {
        invalid!("coating name {name:?} is not allowed (empty, contains |, or is - or I.xx)");
    }
    Ok(())
}

/// 膜の組が約束を守るかを確かめる（拡張の試験から呼ぶ）: 膜の名前、材料の行（波長が正で昇順、n が正、k ≤ 0）、
/// 層の厚さが有限で 0 以上、TABLE の角度の昇順、実測表の大きさと角度の昇順、モデルに透過か鏡のデータがあること
pub fn check_coatings(set: &CoatingSet) -> Result<()> {
    for n in set.names() {
        check_name(&n)?;
    }
    for n in set.coat.keys().chain(set.table.keys()) {
        let pre = format!("{}|", n.to_ascii_uppercase());
        if set.models.keys().any(|m| m.eq_ignore_ascii_case(n))
            || set
                .measured
                .keys()
                .any(|k| k.to_ascii_uppercase().starts_with(&pre))
        {
            invalid!("coating {n} has layers or a table and also a model or measured table; use one of them");
        }
    }
    for (m, rows) in &set.mate {
        if rows.is_empty() {
            invalid!("coating material {m} has no data");
        }
        if rows.windows(2).any(|w| !(w[0][0] < w[1][0])) || rows.iter().any(|r| !(r[0] > 0.0)) {
            invalid!("coating material {m}: the wavelengths are not positive and ascending");
        }
        if rows
            .iter()
            .any(|r| !(r[1] > 0.0) || !(r[2] <= crate::coating::MATE_K_POS_TOL))
        {
            invalid!("coating material {m}: n must be positive and k ≤ 0 (absorption is k < 0)");
        }
    }
    for (c, ls) in &set.coat {
        if ls.is_empty() {
            invalid!("coating {c} has no layers");
        }
        for l in ls {
            if !(l.d.is_finite() && l.d >= 0.0) {
                invalid!(
                    "coating {c}: the thickness {} of layer {} is not a number ≥ 0",
                    l.d,
                    l.mat
                );
            }
        }
    }
    for (c, t) in &set.table {
        if t.is_empty() {
            invalid!("coating table {c} has no data");
        }
        if t.windows(2).any(|w| !(w[0].0 < w[1].0)) {
            invalid!("coating table {c}: the angles are not ascending");
        }
    }
    for (k, m) in &set.measured {
        if k.split('|').count() < 3 {
            invalid!("measured table key {k} is not coating|medium before|medium after");
        }
        if m.data.len() != m.wavelengths.len()
            || m.data.iter().any(|r| r.len() != m.angles_deg.len())
        {
            invalid!("size of measured table {k}");
        }
        if m.angles_deg.windows(2).any(|w| !(w[0] < w[1])) {
            invalid!("the angles of measured table {k} are not ascending");
        }
    }
    for (c, m) in &set.models {
        if m.wavelengths.is_empty() && m.mirror.is_none() {
            invalid!("coating model {c} has neither transmission nor mirror data");
        }
    }
    Ok(())
}
