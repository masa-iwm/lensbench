//! 利用者が足す膜の口: 膜のファイル形式（拡張子ごとの読み手、[`CoatingSource`]）と、名前つきの膜の一覧
//! （[`CoatingCatalog`]）を拡張の表に足す（[`crate::ext::Ext::add_coating_source`]・
//! [`crate::ext::Ext::add_coating_catalog`]）。
//!
//! 膜の中身は今ある型（[`CoatingSet`]: 層の並びとその材料・TABLE・特性行列のモデル・実測表）か、係数を計算で返す
//! 利用者の膜（[`CoatingModel`]）。膜の計算は同梱の膜と同じ経路（[`crate::coating::Iface`]）を通る。
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
//! - 1 つの膜の名前は、層の並び・TABLE か、モデル・実測表か、利用者の膜（[`CoatingModel`]）のどれか 1 つだけに使う
//!   （層・TABLE があるとモデル・実測表は使われない）
//! - 層の材料は、組の [`CoatingSet::mate`] か系のガラスの表で引く。系の膜ファイルに同じ名前の違う材料があれば、
//!   組の材料を `<材料>@<一覧の名前（ファイルなら名前）>` に名前を替えて入れる（系の膜ファイルの材料は変えない）

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::coating::{CoatModel, CoatingFile, Layer, Measured, Table, C64};
use crate::{invalid, Result};

/// 利用者の膜の境界の通り方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoatPath {
    /// 透過（屈折面）。[`CoatCoefs::ts`]・[`CoatCoefs::tp`] を使う
    Transmit,
    /// 鏡。[`CoatCoefs::rs`]・[`CoatCoefs::rp`] を使う（[`CoatInput::n2`] は NaN）
    Mirror,
    /// 屈折面での反射（ゴーストの経路）。[`CoatCoefs::rs`]・[`CoatCoefs::rp`] を使う
    Reflect,
}

/// 利用者の膜に渡す境界の条件
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct CoatInput {
    /// 波長 [µm]
    pub wl: f64,
    /// 入射側の媒質の屈折率
    pub n1: f64,
    /// 反対側の媒質の屈折率（透過では後ろ、屈折面での反射では反対側。鏡では NaN）
    pub n2: f64,
    /// 入射角の cos（0〜1、入射側の媒質で）
    pub cos1: f64,
    pub path: CoatPath,
    /// 基板の側から入る（透過で後ろが空気、屈折面での反射で反対側が空気のとき。膜は空気の側に付いているとみなす。
    /// 鏡では false）
    pub from_substrate: bool,
}

impl CoatInput {
    /// 境界の条件（試験・膜の確かめ用）
    pub fn new(wl: f64, n1: f64, n2: f64, cos1: f64, path: CoatPath, from_substrate: bool) -> Self {
        CoatInput {
            wl,
            n1,
            n2,
            cos1,
            path,
            from_substrate,
        }
    }
}

/// 利用者の膜の複素振幅係数（s・p の反射と透過）
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CoatCoefs {
    pub rs: C64,
    pub rp: C64,
    pub ts: C64,
    pub tp: C64,
}

/// 利用者が係数を計算で返す膜。登録した膜の一覧（[`CoatingSet::add_user`]）か膜のファイル形式から、膜の名前で引く。
///
/// 約束（同梱の膜と同じ。膜なしのフレネルの係数はこの約束を満たす）:
/// - 係数はパワーで規格化した複素振幅（|t|² が強度透過率）。s・p の基底は各側で p = s × k
/// - 伝搬の位相は含めない。位相の基準は面の頂点（基板の面）。鏡・屈折面での反射の p は反射の後で反転する約束
///   （垂直入射で rs = rp）
/// - 吸収がなければ |rs|² + |ts|² = |rp|² + |tp|² = 1、吸収があれば 1 より小さい
/// - 状態を持たず、どのスレッドから呼んでもよい（光線ごとに呼ぶ）。求められない条件は誤りを返す
pub trait CoatingModel: Send + Sync + std::fmt::Debug {
    /// 係数（[`CoatInput::path`] の使わない側は 0 でよい）
    fn coefs(&self, x: &CoatInput) -> Result<CoatCoefs>;
    /// 1 行の説明（CLI の coatings の注記）
    fn doc(&self) -> String {
        String::new()
    }
    /// [`check_coating_model`] で確かめる波長 [µm]（既定は F・d・C 線）
    fn example_wavelengths(&self) -> Vec<f64> {
        vec![0.4861327, 0.5875618, 0.6562725]
    }
}

/// 利用者の膜が約束を守るかを確かめる（拡張の試験から呼ぶ）: [`CoatingModel::example_wavelengths`] の波長で、空気 → 硝材・
/// 硝材 → 空気の透過と屈折面での反射、鏡について、0〜89° の入射角（透過・反射は全反射の手前まで）で、係数が有限、
/// |r|² + |t|² ≤ 1（s・p ごと。鏡は |r|² ≤ 1）、垂直入射で s と p が同じ
pub fn check_coating_model(m: &dyn CoatingModel) -> Result<()> {
    const TOL: f64 = 1e-9;
    let wls = m.example_wavelengths();
    if wls.is_empty() || wls.iter().any(|w| !(*w > 0.0 && w.is_finite())) {
        invalid!("example wavelengths {wls:?} are not positive numbers");
    }
    let cases = [
        (1.0, 1.5, CoatPath::Transmit, false),
        (1.5, 1.0, CoatPath::Transmit, true),
        (1.0, 1.5, CoatPath::Reflect, false),
        (1.5, 1.0, CoatPath::Reflect, true),
        (1.0, f64::NAN, CoatPath::Mirror, false),
    ];
    for &wl in &wls {
        for &(n1, n2, path, from_substrate) in &cases {
            for deg in (0..90).step_by(5).chain([89]) {
                let (s1, c1) = (deg as f64).to_radians().sin_cos();
                if path != CoatPath::Mirror && n1 * s1 >= n2 {
                    break;
                }
                let x = CoatInput::new(
                    wl,
                    n1,
                    n2,
                    if deg == 0 { 1.0 } else { c1 },
                    path,
                    from_substrate,
                );
                let at = format!("{path:?} n1={n1} n2={n2} {deg}° {wl} µm");
                let c = m
                    .coefs(&x)
                    .map_err(|e| crate::Error::Invalid(format!("{at}: {e}")))?;
                let fin = |z: &C64| z.re.is_finite() && z.im.is_finite();
                let used = match path {
                    CoatPath::Transmit => [c.ts, c.tp],
                    _ => [c.rs, c.rp],
                };
                // 鏡は r だけ、透過・屈折面での反射は r と t の両方（エネルギーの検査に使う）
                let all = path == CoatPath::Mirror || [c.rs, c.rp, c.ts, c.tp].iter().all(fin);
                if !used.iter().all(fin) || !all {
                    invalid!("{at}: the coefficients are not finite ({c:?})");
                }
                let e = |r: C64, t: C64| {
                    if path == CoatPath::Mirror {
                        r.norm_sqr()
                    } else {
                        r.norm_sqr() + t.norm_sqr()
                    }
                };
                if e(c.rs, c.ts) > 1.0 + TOL || e(c.rp, c.tp) > 1.0 + TOL {
                    invalid!("{at}: |r|² + |t|² is more than 1 ({c:?})");
                }
                if deg == 0 && (used[0] - used[1]).norm() > 1e-9 * (1.0 + used[0].norm()) {
                    invalid!("{at}: s and p differ at normal incidence ({c:?})");
                }
            }
        }
    }
    Ok(())
}

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
    /// 膜の名前 → 係数を計算で返す利用者の膜
    pub user: HashMap<String, Arc<dyn CoatingModel>>,
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

    /// 係数を計算で返す利用者の膜を足す（同じ名前は差し替える）
    pub fn add_user(&mut self, name: &str, model: Arc<dyn CoatingModel>) -> &mut Self {
        self.user.insert(name.to_string(), model);
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

    /// 膜のファイルを読んで足し、足した膜の数を返す: 特性行列のモデル（`.lbcm`・`.lbcb`）・実測表（`.lbct`）は
    /// 1 行目で、それ以外は COATING.DAT の形式（`.dat`・`.lbcs` など。MATE・COAT・TABLE）として読む
    pub fn add_file(&mut self, path: &Path) -> Result<usize> {
        use crate::coating::{is_models_bin, parse_models, parse_models_bin, MODEL_HEADER};
        let at = |e: crate::Error| match e {
            crate::Error::Invalid(m) => crate::Error::Invalid(format!("{}: {m}", path.display())),
            e => e,
        };
        let b = crate::vfs::read(path)
            .map_err(|e| crate::Error::Io(format!("{}: {e}", path.display())))?;
        if is_models_bin(&b) {
            let v = parse_models_bin(&b).map_err(at)?;
            let n = v.len();
            self.models.extend(v);
            return Ok(n);
        }
        let txt = String::from_utf8_lossy(&b);
        let first = txt
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .unwrap_or("");
        if first == MODEL_HEADER {
            let v = parse_models(&txt).map_err(at)?;
            let n = v.len();
            self.models.extend(v);
            Ok(n)
        } else if first == crate::coating::MEASURED_HEADER {
            let v = crate::coating::parse_measured(&txt).map_err(at)?;
            let n = v.len();
            self.measured.extend(v);
            Ok(n)
        } else {
            let f = CoatingFile::parse(&txt).map_err(at)?;
            let n = f.coat.len() + f.table.len();
            self.mate.extend(f.mate);
            self.coat.extend(f.coat);
            self.table.extend(f.table);
            Ok(n)
        }
    }

    /// 膜の名前（足したときの綴り、名前の順、重なりなし）
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .coat
            .keys()
            .chain(self.table.keys())
            .chain(self.models.keys())
            .chain(self.user.keys())
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
            || self.user.keys().any(eq)
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
/// 層の厚さが有限で 0 以上、TABLE の角度の昇順、実測表の大きさと角度の昇順、モデルに透過か鏡のデータがあること、
/// 利用者の膜が約束を守ること（[`check_coating_model`]）
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
    for (c, m) in &set.user {
        let pre = format!("{}|", c.to_ascii_uppercase());
        let eq = |k: &String| k.eq_ignore_ascii_case(c);
        if set.coat.keys().any(eq)
            || set.table.keys().any(eq)
            || set.models.keys().any(eq)
            || set
                .measured
                .keys()
                .any(|k| k.to_ascii_uppercase().starts_with(&pre))
        {
            invalid!("coating {c} is a user coating and also has layers, a table, a model or a measured table; use one of them");
        }
        check_coating_model(m.as_ref()).map_err(|e| match e {
            crate::Error::Invalid(x) => crate::Error::Invalid(format!("user coating {c}: {x}")),
            e => e,
        })?;
    }
    Ok(())
}
