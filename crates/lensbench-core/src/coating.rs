//! 面の膜と境界の偏光係数。
//!
//! OpticStudio の偏光追跡の約束に合わせる: s・p の複素振幅係数で、パワーで規格化（|t|² = T）、
//! 各側で p = s × k、伝搬の位相は含まない。
//!
//! - 膜なしの誘電体境界: フレネル
//! - 膜なしの鏡: OpticStudio の金属基板 N = 0.7 − 7i
//! - `I.xx`: 理想膜（強度透過率 xx、反射率 1 − xx、位相なし。鏡では s・p とも −√(1 − xx)）
//! - `COAT`: 薄膜（特性行列、傾いた入射のアドミタンス）。材料は `MATE` の表（n, k。k < 0 で吸収）か
//!   ガラスカタログ。厚さは基準波長での光学的な波長単位、またはフラグ 1 で絶対値 [µm]。
//!   係数の位相は OpticStudio の約束に合わせる（[`Coatings::prepare`] の注記）
//! - `TABLE`: 角度・波長ごとの Rs Rp Ts Tp と位相、線形補間（角度は cosθ について。垂直入射は [`TableNormal`]）
//! - 暗号化された膜（.ZEC）: 層は読めないので（復号もしない）、OpticStudio の偏光追跡で測った係数の表
//!   （[`Measured`]）か、膜ごとの特性行列モデル（[`CoatModel`]、任意の硝材・向き・波長）を使う。
//!   どちらもなければ追跡を止める
//!
//! 境界の鍵は `膜|前の媒質|後の媒質（鏡は MIRROR）`（[`media_keys`]）。屈折面での反射（ゴーストの経路、
//! [`crate::ghost`]）は `膜|入射側の媒質|反対側の媒質|R` で、係数は境界の反射（[`Coatings::prepare`] の注記）。

use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};

pub use num_complex::Complex64 as C64;

use crate::glass;
use crate::surface::SurfType;
use crate::system::System;
use crate::{invalid, notimpl, Error, Result};

/// TABLE 膜で垂直入射とみなす範囲（1 − cosθ。OpticStudio 26.1 で境界が 0.008102°〜0.008104° の間、
/// 1 − cosθ = 1e-8 の 0.0081029° と合う）
pub const TABLE_NORMAL: f64 = 1e-8;

/// 膜なしの鏡の基板
pub const MIRROR_N: C64 = C64::new(0.7, -7.0);

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub mat: String,
    pub d: f64,
    /// true: d は絶対値 [µm]、false: 基準波長での光学的な波長単位
    pub absolute: bool,
}

/// 膜の表: 角度 [deg] の昇順に、波長ごとの (波長, Rs, Rp, Ts, Tp, 位相 rs rp ts tp [deg])
pub type Table = Vec<(f64, Vec<[f64; 9]>)>;

/// 膜ファイル（COATING.DAT 形式）の内容
#[derive(Debug, Clone, Default)]
pub struct CoatingFile {
    /// 材料: 波長 [µm]、n、k
    pub mate: HashMap<String, Vec<[f64; 3]>>,
    pub coat: HashMap<String, Vec<Layer>>,
    pub table: HashMap<String, Table>,
}

fn num(s: &str, what: &str) -> Result<f64> {
    s.parse::<f64>()
        .map_err(|_| Error::Invalid(format!("膜ファイル: {what} の数値 {s}")))
}

impl CoatingFile {
    pub fn load(path: &Path) -> Result<CoatingFile> {
        let txt =
            glass::read_text(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        Self::parse(&txt)
    }

    pub fn parse(txt: &str) -> Result<CoatingFile> {
        enum Cur {
            None,
            Mate(String),
            Coat(String),
            Table(String, Option<f64>),
        }
        let mut f = CoatingFile::default();
        let mut tables: HashMap<String, Table> = HashMap::new();
        let mut cur = Cur::None;
        for line in txt.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            if t.is_empty() || t[0].starts_with('!') {
                continue;
            }
            match t[0] {
                // 理想膜・テーパ・暗号化の宣言は使わない（I.xx は組込み、暗号化は実測表）
                "IDEAL" | "IDEAL2" | "ENCRYPTED" | "TAPR" => cur = Cur::None,
                "MATE" | "COAT" | "TABLE" if t.len() >= 2 => {
                    let name = t[1].to_string();
                    cur = match t[0] {
                        "MATE" => {
                            f.mate.insert(name.clone(), Vec::new());
                            Cur::Mate(name)
                        }
                        "COAT" => {
                            f.coat.insert(name.clone(), Vec::new());
                            Cur::Coat(name)
                        }
                        _ => {
                            tables.insert(name.clone(), Vec::new());
                            Cur::Table(name, None)
                        }
                    };
                }
                _ => {
                    match &mut cur {
                        Cur::None => {}
                        Cur::Mate(name) => {
                            if t.len() < 2 {
                                invalid!("膜ファイル: 材料 {name} の行 {line}");
                            }
                            let k = if t.len() > 2 { num(t[2], name)? } else { 0.0 };
                            if k > 0.0 {
                                // N = n + ik の約束で吸収は k < 0（文献の n − ik の k は符号を変えて書く）。k > 0 は増幅する媒質になる
                                invalid!("膜ファイル: 材料 {name} の k が正（{line}）。吸収は k < 0 で書く");
                            }
                            let row = [num(t[0], name)?, num(t[1], name)?, k];
                            f.mate.get_mut(name.as_str()).unwrap().push(row);
                        }
                        Cur::Coat(name) => {
                            if t.len() < 2 {
                                invalid!("膜ファイル: 膜 {name} の行 {line}");
                            }
                            let flag = if t.len() > 2 {
                                num(t[2], name)? as i64
                            } else {
                                0
                            };
                            let l = Layer {
                                mat: t[0].to_string(),
                                d: num(t[1], name)?,
                                absolute: flag == 1,
                            };
                            f.coat.get_mut(name.as_str()).unwrap().push(l);
                        }
                        Cur::Table(name, ang) => match t[0] {
                            "ANGL" if t.len() >= 2 => {
                                let a = num(t[1], name)?;
                                let v = tables.get_mut(name.as_str()).unwrap();
                                v.retain(|(b, _)| *b != a);
                                v.push((a, Vec::new()));
                                *ang = Some(a);
                            }
                            "WAVE" => {
                                let Some(a) = *ang else {
                                    invalid!("膜ファイル: 表 {name} の最初の ANGL より前に WAVE");
                                };
                                // 波長 Rs Rp Ts Tp に位相 4 つが続く（位相は省略すると 0）
                                if t.len() < 6 {
                                    invalid!("膜ファイル: 表 {name} の行 {line}");
                                }
                                let mut row = [0.0; 9];
                                for i in 0..(t.len() - 1).min(9) {
                                    row[i] = num(t[i + 1], name)?;
                                }
                                let v = tables.get_mut(name.as_str()).unwrap();
                                v.iter_mut().find(|(b, _)| *b == a).unwrap().1.push(row);
                            }
                            _ => {}
                        },
                    }
                }
            }
        }
        for (k, mut v) in tables {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            f.table.insert(k, v);
        }
        Ok(f)
    }
}

/// 暗号化された膜の実測の係数: 波長ごと・入射角ごとの (s, p)（透過、鏡では反射）。
/// 全反射より先の角度は NaN にしておく（補間に使わない）。
#[derive(Debug, Clone)]
pub struct Measured {
    pub wavelengths: Vec<f64>,
    /// 入射角 [deg]、昇順
    pub angles_deg: Vec<f64>,
    /// [波長][角度] → (s, p)
    pub data: Vec<Vec<[C64; 2]>>,
}

/// 境界の膜の扱い
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoatKind {
    /// 膜なし（フレネル、鏡は金属基板）
    Bare,
    /// 理想膜 I.xx
    Ideal,
    /// 膜ファイルの薄膜（透過・鏡）
    Stack,
    /// 膜ファイルの表
    Table,
    /// 実測表
    Measured,
    /// 特性行列モデル（[`CoatModel`]）
    Model,
    /// 膜ファイルになく、実測表もない（暗号化された膜など）
    Missing,
}

impl CoatKind {
    pub fn name(self) -> &'static str {
        match self {
            CoatKind::Bare => "bare",
            CoatKind::Ideal => "ideal",
            CoatKind::Stack => "stack",
            CoatKind::Table => "table",
            CoatKind::Measured => "measured",
            CoatKind::Model => "model",
            CoatKind::Missing => "missing",
        }
    }

    /// 実測表が要る（なければ偏光追跡が止まる）
    pub fn needs_table(self) -> bool {
        matches!(self, CoatKind::Missing)
    }
}

/// 読込中の KEY: (キー, [(波長, [(角度, (s, p))])])
type Pending = (String, Vec<(f64, Vec<(f64, [C64; 2])>)>);

/// 実測表のファイル形式の 1 行目
pub const MEASURED_HEADER: &str = "LBCT 1";

/// 実測表のファイル（テキスト、UTF-8）を読む。
///
/// ```text
/// LBCT 1
/// # コメント
/// KEY 膜|前の媒質|後の媒質（鏡は MIRROR）
/// WAVE 波長 [µm]
/// 入射角 [deg]  Re s  Im s  Re p  Im p      （角度の昇順。全反射より先は nan）
/// ...
/// WAVE ...
/// KEY ...
/// ```
///
/// 係数は OpticStudio の偏光追跡の約束（透過、鏡では反射。パワー規格化、p = s × k、伝搬の位相なし）。
/// 1 つの KEY の中では、どの波長も同じ角度の並びにする。
pub fn parse_measured(txt: &str) -> Result<Vec<(String, Measured)>> {
    let mut out: Vec<(String, Measured)> = Vec::new();
    let mut header = false;
    // 今の KEY の (波長, [(角度, s, p)])
    let mut cur: Option<Pending> = None;
    fn finish(cur: Option<Pending>, out: &mut Vec<(String, Measured)>) -> Result<()> {
        let Some((key, waves)) = cur else {
            return Ok(());
        };
        if waves.is_empty() || waves.iter().any(|w| w.1.is_empty()) {
            invalid!("実測表 {key} にデータがない");
        }
        let angles: Vec<f64> = waves[0].1.iter().map(|r| r.0).collect();
        if waves
            .iter()
            .any(|w| w.1.iter().map(|r| r.0).ne(angles.iter().copied()))
        {
            invalid!("実測表 {key}: 波長ごとに角度の並びが違う");
        }
        if out.iter().any(|(k, _)| *k == key) {
            invalid!("実測表 {key} が 2 回ある");
        }
        let m = Measured {
            wavelengths: waves.iter().map(|w| w.0).collect(),
            angles_deg: angles,
            data: waves
                .iter()
                .map(|w| w.1.iter().map(|r| r.1).collect())
                .collect(),
        };
        out.push((key, m));
        Ok(())
    }
    for (ln, line) in txt.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !header {
            if line != MEASURED_HEADER {
                invalid!("実測表の 1 行目が {MEASURED_HEADER} でない");
            }
            header = true;
            continue;
        }
        let n = |s: &str| -> Result<f64> {
            s.parse::<f64>()
                .map_err(|_| Error::Invalid(format!("実測表の {} 行目の数値 {s}", ln + 1)))
        };
        if let Some(k) = line.strip_prefix("KEY ") {
            finish(cur.take(), &mut out)?;
            cur = Some((k.trim().to_string(), Vec::new()));
        } else if let Some(w) = line.strip_prefix("WAVE ") {
            let Some(c) = cur.as_mut() else {
                invalid!("実測表の {} 行目: KEY より前に WAVE", ln + 1);
            };
            c.1.push((n(w.trim())?, Vec::new()));
        } else {
            let t: Vec<&str> = line.split_whitespace().collect();
            let Some(w) = cur.as_mut().and_then(|c| c.1.last_mut()) else {
                invalid!("実測表の {} 行目: WAVE より前にデータ", ln + 1);
            };
            if t.len() != 5 {
                invalid!(
                    "実測表の {} 行目: 角度と係数 4 つ（Re s Im s Re p Im p）",
                    ln + 1
                );
            }
            let a = n(t[0])?;
            if let Some(&(b, _)) = w.1.last() {
                if !(a > b) {
                    invalid!("実測表の {} 行目: 角度が昇順でない", ln + 1);
                }
            }
            let v = [C64::new(n(t[1])?, n(t[2])?), C64::new(n(t[3])?, n(t[4])?)];
            w.1.push((a, v));
        }
    }
    if !header {
        invalid!("実測表が空");
    }
    finish(cur, &mut out)?;
    Ok(out)
}

/// 実測表をファイルの形式（[`parse_measured`]）にする
pub fn format_measured(entries: &[(&str, &Measured)]) -> String {
    use std::fmt::Write;
    let mut s = format!("{MEASURED_HEADER}\n");
    for (k, m) in entries {
        writeln!(s, "KEY {k}").unwrap();
        for (w, row) in m.wavelengths.iter().zip(&m.data) {
            writeln!(s, "WAVE {w:?}").unwrap();
            for (a, v) in m.angles_deg.iter().zip(row) {
                writeln!(
                    s,
                    "{a:?} {:?} {:?} {:?} {:?}",
                    v[0].re, v[0].im, v[1].re, v[1].im
                )
                .unwrap();
            }
        }
    }
    s
}

/// 膜のモデルのファイル形式の 1 行目
pub const MODEL_HEADER: &str = "LBCM 2";

/// 膜のモデルのバイナリ形式（[`parse_models_bin`]）の先頭
pub const MODEL_BIN_HEADER: &[u8] = b"LBCM 3\n";
/// LBCM 3 の末尾の注記の印（注記のある膜があるときだけ書く）
pub const MODEL_BIN_NOTES: &[u8] = b"NOTES\n";

/// 暗号化された膜の特性行列モデル（層を読まずに、任意の硝材・向き・波長で使う）。
///
/// 透過は膜ごと・波長ごと・β = n sinθ ごと・偏光ごとの特性行列 M（OpticStudio の位相を含む）を
/// M = exp(i k ψ) · M̃ と分けて持つ。M̃ は det M̃ = 1 の滑らかな行列、ψ [µm] は OpticStudio が掛ける
/// s・p 共通の位相 −Σ d_j Re(β²/q_j) 由来のスカラー（層の屈折率 Re(n_j) で発散するので、β ≤ `bmax` だけ使う）。
/// 入射側のアドミタンス e1、基板側 e2（s: n cos、p: n / cos）で
///   t = 2 e1 / (e1 M00 + M10 + e1 e2 M01 + e2 M11) · sqrt(e2 / e1) · exp(i k D n_o / cos_o)
/// （n_o は膜の外側の媒質）。基板側から入る（後ろの媒質が空気）ときは M の対角を入れ替える。
/// 鏡は基準の媒質 `n_ref` から測った r（位相 exp(2 i k n cos D_m) を外したもの）を持ち、
/// アドミタンス Y = e_ref (1 − r) / (1 + r) を通して入射側の媒質へ換算する。
/// 補間は β² で 3 次スプライン、波長は近い 4 点で 1/λ の 3 次。
#[derive(Debug, Clone, Default)]
pub struct CoatModel {
    /// 透過の位相の厚さ D [µm]
    pub d: f64,
    /// 透過で使える β の上限
    pub bmax: f64,
    pub wavelengths: Vec<f64>,
    /// β（昇順、全波長で共通）
    pub betas: Vec<f64>,
    /// [波長][β][偏光 s, p] → (M̃00, M̃10, M̃01, M̃11)
    pub m: Vec<Vec<[[C64; 4]; 2]>>,
    /// [波長][β][偏光] → ψ [µm]
    pub psi: Vec<Vec<[f64; 2]>>,
    pub mirror: Option<MirrorModel>,
    /// 波長方向の折れ目 [µm]（昇順）。膜の材料の分散表の節で、係数の波長微分がここで跳ぶ。
    /// 波長の補間はこの点をまたがない（透過・鏡とも。折れ目の波長でも測っておく）
    pub kinks: Vec<f64>,
    /// 注記（空でなければ利用者に見せる。短い測定から作った近似のモデルの印など）。LBCM 2 の `NOTE` 行、LBCM 3 の末尾の `NOTES`
    pub note: String,
}

/// 鏡のモデル（[`CoatModel`]）
#[derive(Debug, Clone, Default)]
pub struct MirrorModel {
    /// 基準の媒質の屈折率
    pub n_ref: f64,
    /// 鏡の位相の厚さ D_m [µm]
    pub d: f64,
    pub wavelengths: Vec<f64>,
    pub betas: Vec<f64>,
    /// [波長][β] → (r_s, r_p)
    pub r: Vec<Vec<[C64; 2]>>,
    /// 波長ごとの β² 方向の分岐点（[`mirror_branch`]、初めて使うときに求める）
    pub(crate) branches: std::sync::OnceLock<Vec<Option<MirrorBranch>>>,
}

impl MirrorModel {
    fn branches(&self, b2: &[f64]) -> &[Option<MirrorBranch>] {
        self.branches
            .get_or_init(|| self.r.iter().map(|rw| mirror_branch(b2, rw)).collect())
    }
}

impl CoatModel {
    fn has_transmit(&self) -> bool {
        !self.wavelengths.is_empty()
    }
}

/// 膜のモデルのファイル（テキスト、UTF-8）を読む。
///
/// ```text
/// LBCM 2
/// COAT 膜の名前
/// D 透過の位相の厚さ [µm]
/// BMAX 透過で使える β の上限
/// KINKS 折れ目の波長 [µm] ...      （なくてもよい。波長の補間はこの点をまたがない）
/// NOTE 注記（なくてもよい。近似のモデルの印など、利用者に見せる 1 行）
/// WAVE 波長 [µm]
/// β  (Re Im)×(M̃00 M̃10 M̃01 M̃11) ψ（s）  (Re Im)×(M̃00 M̃10 M̃01 M̃11) ψ（p）     （19 列、β の昇順）
/// ...
/// MIRROR 基準の屈折率 鏡の位相の厚さ [µm]
/// WAVE 波長
/// β  Re r_s  Im r_s  Re r_p  Im r_p
/// ...
/// COAT ...
/// ```
///
/// 透過と鏡はどちらか片方だけでもよい。どの波長も同じ β の並びにする。
pub fn parse_models(txt: &str) -> Result<Vec<(String, CoatModel)>> {
    /// (波長, [(β, 値)])
    type Waves<T> = (f64, Vec<(f64, T)>);
    /// 透過の 1 点: 偏光ごとの M̃ と ψ
    type TRow = ([[C64; 4]; 2], [f64; 2]);
    #[derive(Default)]
    struct Cur {
        name: String,
        model: CoatModel,
        d: Option<f64>,
        bmax: Option<f64>,
        // 透過の [(波長, [(β, 値)])]、鏡の同じもの
        t: Vec<Waves<TRow>>,
        r: Vec<Waves<[C64; 2]>>,
        in_mirror: bool,
    }
    fn same_betas<T>(name: &str, w: &[(f64, Vec<(f64, T)>)]) -> Result<Vec<f64>>
    where
        T: Clone,
    {
        let b: Vec<f64> = w[0].1.iter().map(|r| r.0).collect();
        if w.iter()
            .any(|x| x.1.iter().map(|r| r.0).ne(b.iter().copied()))
        {
            invalid!("膜のモデル {name}: 波長ごとに β の並びが違う");
        }
        if b.len() < 4 || w.iter().any(|x| x.1.is_empty()) {
            invalid!("膜のモデル {name}: β の点が足りない（4 点以上）");
        }
        if w.windows(2).any(|x| !(x[0].0 < x[1].0)) {
            invalid!("膜のモデル {name}: 波長が昇順でない");
        }
        Ok(b)
    }
    fn finish(cur: Option<Cur>, out: &mut Vec<(String, CoatModel)>) -> Result<()> {
        let Some(mut c) = cur else { return Ok(()) };
        let name = c.name.clone();
        if c.t.is_empty() && c.r.is_empty() {
            invalid!("膜のモデル {name} にデータがない");
        }
        if !c.t.is_empty() {
            let (Some(d), Some(bmax)) = (c.d, c.bmax) else {
                invalid!("膜のモデル {name}: 透過には D と BMAX が要る");
            };
            c.model.d = d;
            c.model.bmax = bmax;
            c.model.betas = same_betas(&name, &c.t)?;
            c.model.wavelengths = c.t.iter().map(|w| w.0).collect();
            c.model.m =
                c.t.iter()
                    .map(|w| w.1.iter().map(|r| r.1 .0).collect())
                    .collect();
            c.model.psi =
                c.t.iter()
                    .map(|w| w.1.iter().map(|r| r.1 .1).collect())
                    .collect();
        }
        if let Some(m) = c.model.mirror.as_mut() {
            if c.r.is_empty() {
                invalid!("膜のモデル {name}: MIRROR にデータがない");
            }
            m.betas = same_betas(&name, &c.r)?;
            m.wavelengths = c.r.iter().map(|w| w.0).collect();
            m.r =
                c.r.iter()
                    .map(|w| w.1.iter().map(|r| r.1).collect())
                    .collect();
        }
        if out.iter().any(|(k, _)| *k == name) {
            invalid!("膜のモデル {name} が 2 回ある");
        }
        out.push((name, c.model));
        Ok(())
    }
    let mut out = Vec::new();
    let mut header = false;
    let mut cur: Option<Cur> = None;
    for (ln, line) in txt.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !header {
            if line != MODEL_HEADER {
                invalid!("膜のモデルの 1 行目が {MODEL_HEADER} でない");
            }
            header = true;
            continue;
        }
        let n = |s: &str| -> Result<f64> {
            s.parse::<f64>()
                .map_err(|_| Error::Invalid(format!("膜のモデルの {} 行目の数値 {s}", ln + 1)))
        };
        let t: Vec<&str> = line.split_whitespace().collect();
        if t[0] == "COAT" {
            finish(cur.take(), &mut out)?;
            if t.len() != 2 {
                invalid!("膜のモデルの {} 行目: COAT 名前", ln + 1);
            }
            cur = Some(Cur {
                name: t[1].to_string(),
                ..Default::default()
            });
            continue;
        }
        let Some(c) = cur.as_mut() else {
            invalid!("膜のモデルの {} 行目: COAT より前", ln + 1);
        };
        match t[0] {
            "D" | "BMAX" if t.len() == 2 && !c.in_mirror => {
                let v = Some(n(t[1])?);
                if t[0] == "D" {
                    c.d = v
                } else {
                    c.bmax = v
                }
            }
            "NOTE" => {
                c.model.note = line[4..].trim().to_string();
            }
            "KINKS" => {
                let mut k = t[1..].iter().map(|x| n(x)).collect::<Result<Vec<f64>>>()?;
                k.sort_by(f64::total_cmp);
                c.model.kinks = k;
            }
            "MIRROR" if t.len() == 3 && !c.in_mirror => {
                c.in_mirror = true;
                c.model.mirror = Some(MirrorModel {
                    n_ref: n(t[1])?,
                    d: n(t[2])?,
                    ..Default::default()
                });
            }
            "WAVE" if t.len() == 2 => {
                let w = n(t[1])?;
                if c.in_mirror {
                    c.r.push((w, Vec::new()))
                } else {
                    c.t.push((w, Vec::new()))
                }
            }
            _ => {
                let v = t.iter().map(|x| n(x)).collect::<Result<Vec<f64>>>()?;
                let b = v[0];
                let prev = if c.in_mirror {
                    c.r.last().map(|w| w.1.last().map(|r| r.0))
                } else {
                    c.t.last().map(|w| w.1.last().map(|r| r.0))
                };
                let Some(prev) = prev else {
                    invalid!("膜のモデルの {} 行目: WAVE より前にデータ", ln + 1);
                };
                if prev.is_some_and(|p| !(b > p)) {
                    invalid!("膜のモデルの {} 行目: β が昇順でない", ln + 1);
                }
                let z = |i: usize| C64::new(v[i], v[i + 1]);
                if c.in_mirror {
                    if v.len() != 5 {
                        invalid!("膜のモデルの {} 行目: 鏡は β と係数 4 つ", ln + 1);
                    }
                    c.r.last_mut().unwrap().1.push((b, [z(1), z(3)]));
                } else {
                    if v.len() != 19 {
                        invalid!("膜のモデルの {} 行目: 透過は β と 18 列", ln + 1);
                    }
                    let m = [[z(1), z(3), z(5), z(7)], [z(10), z(12), z(14), z(16)]];
                    c.t.last_mut().unwrap().1.push((b, (m, [v[9], v[18]])));
                }
            }
        }
    }
    if !header {
        invalid!("膜のモデルが空");
    }
    finish(cur, &mut out)?;
    Ok(out)
}

/// LBCM 3 の透過の値の刻み（この刻みの整数にして持つ。元の値との差は刻みの半分以下）
pub const MODEL_BIN_QUANTUM: f64 = 1e-9;
/// LBCM 3 の鏡の r の刻み。鏡の補間は |r| = 1 からのずれ [`MIRROR_TIR_TOL`] 未満を全反射とみて分岐点を探す（`mirror_branch`）ので、
/// それより十分細かくする
pub const MODEL_BIN_QUANTUM_MIRROR: f64 = 1e-12;

/// 値の表 v（[nw][nb][k] の順に並べたもの）を LBCM 3 の値の塊にする:
/// 刻み q の整数にし、β の方向に 3 階、波長の方向に 2 階の差分を取った残り（端は低い階の差分）を zigzag で u64 にし、
/// [k][nw][nb] の順に並べてバイトの位ごとに分け（8 面）、deflate で縮める。
/// 値は β にも波長にも滑らかなので残りが小さく、上の位のバイトがほとんど 0 になって縮む
pub(crate) fn encode_values(v: &[f64], nw: usize, nb: usize, k: usize, q: f64) -> Result<Vec<u8>> {
    debug_assert_eq!(v.len(), nw * nb * k);
    let lim = (1u64 << 60) as f64;
    // [k][nw][nb] の整数
    let mut x = vec![0i64; v.len()];
    for w in 0..nw {
        for j in 0..nb {
            for f in 0..k {
                let y = v[(w * nb + j) * k + f] / q;
                if !(y.abs() < lim) {
                    invalid!(
                        "膜のモデルの値 {} が LBCM 3 で持てない（有限で、絶対値が刻み {q} の 2^60 倍より小さいこと）",
                        y * q
                    );
                }
                x[(f * nw + w) * nb + j] = y.round() as i64;
            }
        }
    }
    let lines_b: Vec<(usize, usize, usize)> = (0..k * nw).map(|r| (r * nb, 1, nb)).collect();
    let lines_w: Vec<(usize, usize, usize)> = (0..k)
        .flat_map(|f| (0..nb).map(move |j| (f * nw * nb + j, nb, nw)))
        .collect();
    diff_lines(&mut x, &lines_b, 2);
    diff_lines(&mut x, &lines_b, 1);
    diff_lines(&mut x, &lines_w, 2);
    let n = x.len();
    let mut planes = vec![0u8; 8 * n];
    for (t, d) in x.iter().enumerate() {
        let z = ((d << 1) ^ (d >> 63)) as u64;
        for (bi, byte) in z.to_le_bytes().iter().enumerate() {
            planes[bi * n + t] = *byte;
        }
    }
    Ok(miniz_oxide::deflate::compress_to_vec(&planes, 9))
}

/// 線（先頭, 間隔, 長さ）ごとに差分を取る。order 1: x[i] − x[i−1]（i ≥ 1）、
/// order 2: 1 階の差分 d のうえで d[i] − d[i−1]（i ≥ 2。先頭 2 つは x[0] と d[1]）
fn diff_lines(x: &mut [i64], lines: &[(usize, usize, usize)], order: usize) {
    for &(s, st, len) in lines {
        for i in (1..len).rev() {
            x[s + i * st] = x[s + i * st].wrapping_sub(x[s + (i - 1) * st]);
        }
        if order == 2 {
            for i in (2..len).rev() {
                x[s + i * st] = x[s + i * st].wrapping_sub(x[s + (i - 1) * st]);
            }
        }
    }
}

/// [`diff_lines`] の逆
fn undiff_lines(x: &mut [i64], lines: &[(usize, usize, usize)], order: usize) {
    for &(s, st, len) in lines {
        if order == 2 {
            for i in 2..len {
                x[s + i * st] = x[s + i * st].wrapping_add(x[s + (i - 1) * st]);
            }
        }
        for i in 1..len {
            x[s + i * st] = x[s + i * st].wrapping_add(x[s + (i - 1) * st]);
        }
    }
}

/// [`encode_values`] の逆（[nw][nb][k] の順の値）
fn decode_values(c: &[u8], nw: usize, nb: usize, k: usize, q: f64) -> Result<Vec<f64>> {
    let n = match nw.checked_mul(nb).and_then(|x| x.checked_mul(k)) {
        Some(n) if n.checked_mul(8).is_some() => n,
        _ => invalid!("膜のモデル（LBCM 3）の波長・β の数が大きすぎる"),
    };
    let planes = miniz_oxide::inflate::decompress_to_vec_with_limit(c, 8 * n)
        .map_err(|_| Error::Invalid("膜のモデル（LBCM 3）の値の塊を展開できない".into()))?;
    if planes.len() != 8 * n {
        invalid!("膜のモデル（LBCM 3）の値の数が合わない");
    }
    let mut x: Vec<i64> = (0..n)
        .map(|t| {
            let mut b = [0u8; 8];
            for (bi, byte) in b.iter_mut().enumerate() {
                *byte = planes[bi * n + t];
            }
            let u = u64::from_le_bytes(b);
            ((u >> 1) as i64) ^ -((u & 1) as i64)
        })
        .collect();
    let lines_b: Vec<(usize, usize, usize)> = (0..k * nw).map(|r| (r * nb, 1, nb)).collect();
    let lines_w: Vec<(usize, usize, usize)> = (0..k)
        .flat_map(|f| (0..nb).map(move |j| (f * nw * nb + j, nb, nw)))
        .collect();
    undiff_lines(&mut x, &lines_w, 2);
    undiff_lines(&mut x, &lines_b, 1);
    undiff_lines(&mut x, &lines_b, 2);
    let mut v = vec![0f64; n];
    for w in 0..nw {
        for j in 0..nb {
            for f in 0..k {
                v[(w * nb + j) * k + f] = x[(f * nw + w) * nb + j] as f64 * q;
            }
        }
    }
    Ok(v)
}

/// 膜のモデルのバイナリ形式（LBCM 3）を読む。中身は [`parse_models`] と同じで、値は透過を刻み
/// [`MODEL_BIN_QUANTUM`]（1e-9）、鏡を [`MODEL_BIN_QUANTUM_MIRROR`]（1e-12）の整数にして縮めて持つ（`encode_values`）。
/// f32 にしないのは、M̃ の成分が大きい（20 を超える）高い β で丸めだけで係数が 4e-6 ずれ、鏡は r から求める分岐点が丸めで動いて
/// 1e-5 ほどずれるため。
///
/// リトルエンディアン。先頭 `LBCM 3\n`、u32 膜の数、膜ごとに:
/// u16 名前の長さ・名前（UTF-8）、f64 D、f64 BMAX、u32 折れ目の数・f64 折れ目、
/// u32 波長の数 nw・u32 β の数 nb・f64 波長・f64 β・値の塊 [nw][nb][偏光 s, p][(Re Im)×(M̃00 M̃10 M̃01 M̃11), ψ]（k = 18。透過がなければ nw = 0 で塊もない）、
/// u8 鏡の有無、鏡があれば f64 基準の屈折率・f64 D_m・u32 nw・u32 nb・f64 波長・f64 β・値の塊 [nw][nb][Re r_s, Im r_s, Re r_p, Im r_p]（k = 4）。
/// 値の塊は f64 刻み q・u32 バイト数・deflate したバイト列（中身は `encode_values`）。
pub fn parse_models_bin(b: &[u8]) -> Result<Vec<(String, CoatModel)>> {
    struct Rd<'a> {
        b: &'a [u8],
        p: usize,
    }
    impl Rd<'_> {
        fn take(&mut self, n: usize) -> Result<&[u8]> {
            if self.p + n > self.b.len() {
                invalid!("膜のモデル（LBCM 3）が途中で切れている");
            }
            self.p += n;
            Ok(&self.b[self.p - n..self.p])
        }
        fn u8(&mut self) -> Result<u8> {
            Ok(self.take(1)?[0])
        }
        fn u16(&mut self) -> Result<u16> {
            Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
        }
        fn u32(&mut self) -> Result<usize> {
            Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize)
        }
        fn f64s(&mut self, n: usize) -> Result<Vec<f64>> {
            Ok(self
                .take(8 * n)?
                .chunks_exact(8)
                .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                .collect())
        }
        fn values(&mut self, nw: usize, nb: usize, k: usize) -> Result<Vec<f64>> {
            let q = self.f64s(1)?[0];
            if !(q > 0.0 && q.is_finite()) {
                invalid!("膜のモデル（LBCM 3）の値の刻みが正でない");
            }
            let len = self.u32()?;
            decode_values(self.take(len)?, nw, nb, k, q)
        }
    }
    fn grid(name: &str, w: &[f64], b: &[f64]) -> Result<()> {
        if b.len() < 4
            || w.windows(2).any(|x| !(x[0] < x[1]))
            || b.windows(2).any(|x| !(x[0] < x[1]))
        {
            invalid!("膜のモデル {name}: 波長か β が昇順でない、または β が 4 点より少ない");
        }
        Ok(())
    }
    if !b.starts_with(MODEL_BIN_HEADER) {
        invalid!("膜のモデルの先頭が LBCM 3 でない");
    }
    let mut r = Rd {
        b,
        p: MODEL_BIN_HEADER.len(),
    };
    let nc = r.u32()?;
    // 膜の数はファイルの値なので、それで先に確保しない（壊れたファイルで巨大な確保にならないように）
    let mut out: Vec<(String, CoatModel)> = Vec::new();
    for _ in 0..nc {
        let ln = r.u16()? as usize;
        let name = String::from_utf8(r.take(ln)?.to_vec())
            .map_err(|_| Error::Invalid("膜のモデル（LBCM 3）の名前が UTF-8 でない".into()))?;
        let mut m = CoatModel {
            d: r.f64s(1)?[0],
            bmax: r.f64s(1)?[0],
            ..Default::default()
        };
        if !(m.d.is_finite() && m.bmax.is_finite()) {
            invalid!("膜のモデル {name}: D か BMAX が有限でない");
        }
        let nk = r.u32()?;
        m.kinks = r.f64s(nk)?;
        let (nw, nb) = (r.u32()?, r.u32()?);
        if nw > 0 {
            m.wavelengths = r.f64s(nw)?;
            m.betas = r.f64s(nb)?;
            grid(&name, &m.wavelengths, &m.betas)?;
            let v = r.values(nw, nb, 18)?;
            let z = |i: usize| C64::new(v[i], v[i + 1]);
            m.m = (0..nw)
                .map(|w| {
                    (0..nb)
                        .map(|j| {
                            let o = (w * nb + j) * 18;
                            [0, 9].map(|q| [z(o + q), z(o + q + 2), z(o + q + 4), z(o + q + 6)])
                        })
                        .collect()
                })
                .collect();
            m.psi = (0..nw)
                .map(|w| {
                    (0..nb)
                        .map(|j| {
                            let o = (w * nb + j) * 18;
                            [v[o + 8], v[o + 17]]
                        })
                        .collect()
                })
                .collect();
        }
        let has_mirror = match r.u8()? {
            0 => false,
            1 => true,
            x => invalid!("膜のモデル {name}: 鏡の有無の値 {x} が 0 か 1 でない"),
        };
        if has_mirror {
            let h = r.f64s(2)?;
            if !(h[0].is_finite() && h[1].is_finite()) {
                invalid!("膜のモデル {name}: 鏡の基準の屈折率か D_m が有限でない");
            }
            let (nw, nb) = (r.u32()?, r.u32()?);
            let wavelengths = r.f64s(nw)?;
            let betas = r.f64s(nb)?;
            grid(&name, &wavelengths, &betas)?;
            let v = r.values(nw, nb, 4)?;
            let rr = (0..nw)
                .map(|w| {
                    (0..nb)
                        .map(|j| {
                            let o = (w * nb + j) * 4;
                            [C64::new(v[o], v[o + 1]), C64::new(v[o + 2], v[o + 3])]
                        })
                        .collect()
                })
                .collect();
            m.mirror = Some(MirrorModel {
                n_ref: h[0],
                d: h[1],
                wavelengths,
                betas,
                r: rr,
                ..Default::default()
            });
        } else if nw == 0 {
            invalid!("膜のモデル {name} にデータがない");
        }
        if out.iter().any(|(k, _)| *k == name) {
            invalid!("膜のモデル {name} が 2 回ある");
        }
        out.push((name, m));
    }
    // 末尾の注記（なくてもよい）: NOTES\n、数、（名前の長さ u16、名前、注記の長さ u32、注記）× 数
    if b.len() > r.p && b[r.p..].starts_with(MODEL_BIN_NOTES) {
        r.p += MODEL_BIN_NOTES.len();
        let nn = r.u32()?;
        for _ in 0..nn {
            let ln = r.u16()? as usize;
            let name = String::from_utf8_lossy(r.take(ln)?).to_string();
            let ln = r.u32()?;
            let note = String::from_utf8(r.take(ln)?.to_vec())
                .map_err(|_| Error::Invalid("膜のモデル（LBCM 3）の注記が UTF-8 でない".into()))?;
            match out.iter_mut().find(|(k, _)| *k == name) {
                Some((_, m)) => m.note = note,
                None => invalid!("膜のモデル（LBCM 3）の注記の膜 {name} がない"),
            }
        }
    }
    if r.p != b.len() {
        invalid!("膜のモデル（LBCM 3）の後ろに余分なデータがある");
    }
    Ok(out)
}

/// wl をはさむ折れ目の間の点の範囲 [s, e)（範囲外や点が足りなければ None）
fn wave_segment(wls: &[f64], wl: f64, kinks: &[f64]) -> Option<(usize, usize)> {
    const EPS: f64 = 1e-9;
    let n = wls.len();
    if n == 0 || !(wls[0] - EPS <= wl && wl <= wls[n - 1] + EPS) {
        return None;
    }
    // wl をはさむ折れ目の間 [lo, hi] の点だけを使う（折れ目の上の点は両側で使う）
    let lo = kinks
        .iter()
        .copied()
        .filter(|&k| k <= wl + EPS)
        .fold(f64::NEG_INFINITY, f64::max);
    let hi = kinks
        .iter()
        .copied()
        .filter(|&k| k > wl + EPS)
        .fold(f64::INFINITY, f64::min);
    let s = wls.partition_point(|&w| w < lo - EPS);
    let e = wls.partition_point(|&w| w <= hi + EPS);
    let n = e - s;
    // 折れ目の間に点が 1 つ以下だと補間できない（呼ぶ側が範囲の外と区別して知らせる）
    if n == 0 || (n < 2 && n < wls.len()) {
        return None;
    }
    Some((s, e))
}

/// 波長 wl での値: 近い 4 点（少なければ全点）を 1/λ の多項式で補間する（範囲外は None）
fn wave_weights(wls: &[f64], wl: f64, kinks: &[f64]) -> Option<(usize, Vec<f64>)> {
    let (s, e) = wave_segment(wls, wl, kinks)?;
    let seg = &wls[s..e];
    let n = seg.len();
    let m = n.min(4);
    let i = seg
        .partition_point(|&w| w < wl)
        .saturating_sub(2)
        .min(n - m);
    let x: Vec<f64> = seg[i..i + m].iter().map(|w| 1.0 / w).collect();
    let xv = 1.0 / wl;
    let w = (0..m)
        .map(|a| {
            (0..m)
                .filter(|&b| b != a)
                .map(|b| (xv - x[b]) / (x[a] - x[b]))
                .product()
        })
        .collect();
    Some((s + i, w))
}

/// 鏡の 1 つの波長の分岐点（[`mirror_branch`]）
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MirrorBranch {
    /// 分岐点の β²
    xb: f64,
    /// 全反射が最後まで続く最初の点。分岐点は点 k − 1 と k の間にある
    k: usize,
    /// 分岐点が測った点の上にある（[`MIRROR_BRANCH_SNAP`] で寄せた）
    on_grid: bool,
}

/// 鏡の分岐点として扱う、全反射の手前の 2 点での |r| の 1 からのずれの下限。これより小さい sqrt の項は、
/// 分岐点をまたいで補間しても誤差がその数分の 1 に収まる（|r| が 1 の 2e-6 手前で全反射に移る膜では、
/// 分岐点として扱うと推定の位置が 1e-4 ずれて係数が 1.2e-3 ずれ、またいで補間すると 3e-7）
const MIRROR_BRANCH_MIN_DEV: f64 = 1e-5;

/// 鏡の分岐点を探すとき、|r| = 1 からのずれがこれ未満の点を全反射とみる。値の 1e-9 ほどの違い（丸め・測定の桁）で
/// 全反射の判定が外れないよう、それより十分大きくする（1e-10 だと r を 1e-9 に丸めただけで分岐点が消え、
/// 分岐点をまたいで補間して係数が 2e-2 変わった）
pub const MIRROR_TIR_TOL: f64 = 1e-8;

/// 鏡の分岐点の推定が、隣り合う β² の点の間隔のこの割合より格子の点に近ければ、その点に寄せる。
/// 分岐点が測った点の上にある（層の屈折率がちょうど格子の β）とき、推定は値の 1e-9 の違いで 4e-10 ほど揺れ、
/// 分岐点のそばでは r が sqrt(|β² − xb|) で変わるので、揺れが係数で 5e-5 に広がる。格子の点に寄せれば推定は揺れない
const MIRROR_BRANCH_SNAP: f64 = 1e-6;

/// 鏡の補間に使う有理関数の点の数
const RATIONAL_N: usize = 8;

/// 昇順の点 xs[s..e] のうち q に近い m 点（[`RATIONAL_N`] から始めて、解けなければ 2 点ずつ減らす）を通る
/// 有理関数（分子 m/2 − 1 次・分母 m/2 次）の q での値。4 点でも解けなければ近い 4 点の多項式にする。
/// 連立方程式が退化する（データがもっと低い次数の有理関数で表せる）ときは、ピボットが相対的に小さくなるので次の m にする
fn rational_at(xs: &[f64], s: usize, e: usize, q: f64, f: &dyn Fn(usize) -> C64) -> C64 {
    let n = e - s;
    let start = |m: usize| {
        s + xs[s..e]
            .partition_point(|&x| x < q)
            .saturating_sub(m / 2)
            .min(n - m)
    };
    let mut m = RATIONAL_N.min(n - n % 2);
    while m >= 4 {
        if let Some(v) = rational_solve(xs, start(m), m, q, f) {
            return v;
        }
        m -= 2;
    }
    let m = n.min(4);
    let i = start(m);
    (i..i + m)
        .map(|a| {
            let w: f64 = (i..i + m)
                .filter(|&b| b != a)
                .map(|b| (q - xs[b]) / (xs[a] - xs[b]))
                .product();
            f(a) * w
        })
        .sum::<C64>()
}

/// xs[i..i+m] を通る有理関数 P/Q（Q(q) = 1）の q での値。退化していれば None
fn rational_solve(xs: &[f64], i: usize, m: usize, q: f64, f: &dyn Fn(usize) -> C64) -> Option<C64> {
    let t: Vec<f64> = (i..i + m).map(|j| xs[j] - q).collect();
    let h = t.iter().fold(0.0f64, |a, b| a.max(b.abs())).max(1e-300);
    // P(t) − f Q(t) = 0、Q(0) = 1（t は h で割って条件をよくする）
    let (np, nq) = (m / 2, m - m / 2);
    let mut a = vec![vec![C64::new(0.0, 0.0); m + 1]; m];
    for (r, row) in a.iter_mut().enumerate() {
        let u = t[r] / h;
        let fv = f(i + r);
        for (k, v) in row.iter_mut().enumerate().take(np) {
            *v = C64::new(u.powi(k as i32), 0.0);
        }
        for k in 1..=nq {
            row[np + k - 1] = -fv * u.powi(k as i32);
        }
        row[m] = fv;
    }
    let scale = a
        .iter()
        .flat_map(|r| r[..m].iter().map(|v| v.norm()))
        .fold(0.0, f64::max);
    for c in 0..m {
        let p = (c..m).max_by(|&x, &y| a[x][c].norm().total_cmp(&a[y][c].norm()))?;
        if a[p][c].norm() <= 1e-11 * scale {
            return None;
        }
        a.swap(c, p);
        for r in 0..m {
            if r != c {
                let g = a[r][c] / a[c][c];
                for k in c..=m {
                    let v = a[c][k];
                    a[r][k] -= g * v;
                }
            }
        }
    }
    let v = a[0][m] / a[0][0];
    (v.re.is_finite() && v.im.is_finite()).then_some(v)
}

/// 鏡の 1 つの波長の表（β² の昇順 b2、点ごとに偏光 s・p の r）から、β² 方向の分岐点を探す。
/// 膜の外の媒質（屈折率 n）へ光が抜ける鏡では、β > n で全反射になって |r| = 1 に張り付き、
/// β = n で r が sqrt(n² − β²) の形で変わる。有理関数でも多項式でも表せないので、補間の点がまたがないようにし、
/// 両側それぞれ t = sqrt(|β² − xb|) で補間する（r は t の滑らかな関数）。
/// |r| = 1（[`MIRROR_TIR_TOL`] 以内）が最後まで続く最初の点を k として、位置 xb は k とその手前の点の間で、両側から t = 0 へ外挿した r が一致する点。
/// 推定が k か k − 1 の点にごく近ければ（[`MIRROR_BRANCH_SNAP`]）、その点にする。
/// 層が吸収して |r| < 1 のままなら None（分岐点は扱わない）
fn mirror_branch(b2: &[f64], r: &[[C64; 2]]) -> Option<MirrorBranch> {
    let dev = |j: usize| {
        r[j].iter()
            .map(|v| (v.norm() - 1.0).abs())
            .fold(0.0, f64::max)
    };
    let n = b2.len();
    let k = (0..n).rev().find(|&j| !(dev(j) < MIRROR_TIR_TOL))? + 1;
    // 両側に 4 点以上。sqrt の形なら手前で |r| は 1 からはっきり離れている（全反射しにくい高反射膜で
    // |r| が β とともに少しずつ 1 に近づくだけのときは分岐点ではない）
    if k < 4 || n - k < 4 || dev(k - 1).max(dev(k - 2)) < MIRROR_BRANCH_MIN_DEV {
        return None;
    }
    let m = RATIONAL_N.min(k).min(n - k);
    // 両側から t = 0 へ外挿した r の食い違い
    let gap = |xb: f64| -> f64 {
        let tl: Vec<f64> = (0..m)
            .map(|i| (xb - b2[k - 1 - i]).max(0.0).sqrt())
            .collect();
        let th: Vec<f64> = (0..m).map(|i| (b2[k + i] - xb).max(0.0).sqrt()).collect();
        (0..2)
            .map(|pol| {
                let lo = rational_at(&tl, 0, m, 0.0, &|i| r[k - 1 - i][pol]);
                let hi = rational_at(&th, 0, m, 0.0, &|i| r[k + i][pol]);
                (lo - hi).norm()
            })
            .sum::<f64>()
    };
    let (a0, c0) = (b2[k - 1], b2[k]);
    const N: usize = 200;
    let grid = |i: usize| a0 + (c0 - a0) * i as f64 / N as f64;
    let j = (0..=N)
        .min_by(|&p, &q| gap(grid(p)).total_cmp(&gap(grid(q))))
        .unwrap_or(0);
    let (mut a, mut c) = (grid(j.saturating_sub(1)), grid((j + 1).min(N)));
    // 黄金分割で詰める
    for _ in 0..60 {
        let (p, q) = (a + (c - a) * 0.381_966, a + (c - a) * 0.618_034);
        if gap(p) < gap(q) {
            c = q;
        } else {
            a = p;
        }
    }
    let xb = 0.5 * (a + c);
    // 格子の点より悪ければ格子の点
    let xb = if gap(xb) <= gap(grid(j)) { xb } else { grid(j) };
    // 測った点のすぐそばなら、その点に寄せる
    let tol = MIRROR_BRANCH_SNAP * (c0 - a0);
    let (xb, on_grid) = if xb - a0 <= tol {
        (a0, true)
    } else if c0 - xb <= tol {
        (c0, true)
    } else {
        (xb, false)
    };
    Some(MirrorBranch { xb, k, on_grid })
}

/// [`wave_weights`] が補間できない理由
fn out_of_waves(wls: &[f64], wl: f64) -> String {
    match (wls.first(), wls.last()) {
        (Some(&a), Some(&b)) if a - 1e-9 <= wl && wl <= b + 1e-9 => {
            "をはさむ折れ目（KINKS）の間に、測った波長が 2 つ以上ない".to_string()
        }
        (Some(&a), Some(&b)) => format!("が範囲（{a}〜{b} µm）の外"),
        _ => "の波長のデータがない".to_string(),
    }
}

/// 実数の値の列を β² の not-a-knot スプラインにする
fn beta_spline(name: &str, b2: &[f64], ys: &[f64]) -> Result<Spline> {
    match Spline::not_a_knot(b2, ys) {
        Some(s) => Ok(s),
        None => invalid!("膜のモデル {name}: β の補間ができない"),
    }
}

/// 媒質 n（実数）の β でのアドミタンス（s: n cos、p: n / cos。cos は実数の枝）
fn admittance(n: f64, b: f64, pol: usize) -> C64 {
    let q = C64::new(n * n - b * b, 0.0).sqrt();
    if pol == 0 {
        q
    } else {
        C64::new(n * n, 0.0) / q
    }
}

/// 媒質 n（実数）の β でのアドミタンス。n < β（全反射の先）では減衰する枝（Im(n cos) ≤ 0、[`stack`] と同じ）
fn admittance_ev(n: f64, b: f64, pol: usize) -> C64 {
    let q2 = n * n - b * b;
    let q = if q2 >= 0.0 {
        C64::new(q2.sqrt(), 0.0)
    } else {
        C64::new(0.0, -(-q2).sqrt())
    };
    if pol == 0 {
        q
    } else {
        C64::new(n * n, 0.0) / q
    }
}

/// 膜の透過の位相の約束
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhaseConvention {
    /// OpticStudio と同じ: 層ごとの項 −Σ d_j Re(β²/q_j)（光線の層の中での横ずれの位相）を含む。
    /// β が層の屈折率に近づくと発散するので、膜のモデルは BMAX を超える β で止める
    #[default]
    OpticStudio,
    /// 層ごとの項を外す（波として解いた係数のまま。どの β でも有限で連続）。OpticStudio とは
    /// s・p 共通の位相だけ違う。層のわかる膜（膜ファイルの COAT）とモデルにだけ効き、実測表・TABLE・
    /// 理想膜・膜なしには効かない（層の項がない、または測った値そのもの）
    Physical,
}

/// TABLE 膜の垂直入射の扱い（表の 0° で s と p が違うとき）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableNormal {
    /// OpticStudio と同じ: 垂直入射（1 − cosθ < [`TABLE_NORMAL`]）だけ s に p の値を使い、
    /// その外は表の s を補間する（0° のすぐ外で s に段差が出る）
    #[default]
    OpticStudio,
    /// 段差なし: 0° の行の s を p で置き換えて補間する（連続。小さい角度で OpticStudio と違う）
    Continuous,
}

/// フォルダに無い膜のモデルを取る口（膜ファイルのフォルダ、膜の名前）→ 取ったファイル（`.lbcb` など）
pub type RemoteLoader = fn(&Path, &str) -> Option<PathBuf>;

static REMOTE: std::sync::OnceLock<RemoteLoader> = std::sync::OnceLock::new();

/// [`Coatings::load_dir_for`] がフォルダで膜のモデル・実測表を見つけられなかったときに呼ぶ口を決める
/// （CLI・GUI の起動時。共有の置き場から取る、lensbench_cli::coat_remote）。1 回だけ
pub fn set_remote_loader(f: RemoteLoader) {
    let _ = REMOTE.set(f);
}

/// 系の膜: 膜ファイルと実測表
#[derive(Debug, Clone, Default)]
pub struct Coatings {
    pub file: CoatingFile,
    /// 境界の鍵 → 実測表
    pub measured: HashMap<String, Measured>,
    /// 膜の名前 → 特性行列モデル
    pub models: HashMap<String, CoatModel>,
    /// 透過の位相の約束
    pub phase: PhaseConvention,
    /// TABLE 膜の垂直入射の扱い
    pub table_normal: TableNormal,
    /// 膜ファイルのフォルダから膜の名前で自動で読んだモデル・実測表（系の膜の名前 → ファイル）。
    /// [`Coatings::for_system`] が埋める
    pub auto: Vec<(String, PathBuf)>,
    /// 自動で読んだ実測表（`.lbct`）の境界の鍵（明示したモデルのある膜から外すため）
    pub auto_measured: Vec<String>,
}

impl Coatings {
    /// 系の膜ファイル（zmx の COFN）。なければ空（膜なし・理想膜・実測表だけ）
    pub fn for_system(sys: &System) -> Result<Coatings> {
        let mut file = match &sys.coating_file {
            Some(p) => CoatingFile::load(p)?,
            None => CoatingFile::default(),
        };
        // 系に書いた表（フィルタ）は膜ファイルより優先
        for (k, t) in &sys.coat_tables {
            file.table.insert(k.clone(), t.clone());
        }
        let mut c = Coatings {
            file,
            measured: HashMap::new(),
            models: HashMap::new(),
            phase: PhaseConvention::default(),
            table_normal: TableNormal::default(),
            auto: Vec::new(),
            auto_measured: Vec::new(),
        };
        if let Some(dir) = sys.coating_file.as_ref().and_then(|p| p.parent()) {
            c.load_dir_for(sys, dir)?;
        }
        Ok(c)
    }

    /// フォルダ dir から、系 sys が使う膜のうち膜ファイルで決まらないもの（暗号化された膜など）の
    /// モデル・実測表を膜の名前で探して読む。ファイル名は `<膜の名前>.lbcb`・`.lbcm`・`.lbct`、
    /// 層のわかる近似の膜なら `.lbcs`（膜ファイルの形式で COAT とその材料の MATE）
    /// （大文字小文字は区別しない。同じ膜で複数あれば lbcb・lbcm・lbct・lbcs の順に 1 つ）。
    /// 使う膜だけ読む。読んだものは [`Coatings::auto`] に記録し、読んだ数を返す。
    /// 後から [`Coatings::load_file`] で読んだもの（明示の指定）が勝つ
    pub fn load_dir_for(&mut self, sys: &System, dir: &Path) -> Result<usize> {
        let (keys, _) = media_keys(sys);
        let mut want: Vec<&str> = Vec::new();
        for k in keys.iter().flatten() {
            let coat = k.split('|').next().unwrap_or("");
            if coat.is_empty()
                || coat == "-"
                || coat.starts_with("I.")
                || self.file.table.contains_key(coat)
                || self.file.coat.contains_key(coat)
                || want.contains(&coat)
            {
                continue;
            }
            want.push(coat);
        }
        if want.is_empty() || !crate::vfs::is_dir(dir) {
            return Ok(0);
        }
        let files =
            crate::vfs::read_dir(dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
        // 膜の名前（大文字）→ (拡張子の順位, パス)
        let mut found: HashMap<String, (usize, PathBuf)> = HashMap::new();
        for p in files {
            let (Some(stem), Some(ext)) = (p.file_stem(), p.extension()) else {
                continue;
            };
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            let Some(rank) = ["lbcb", "lbcm", "lbct", "lbcs"]
                .iter()
                .position(|e| *e == ext)
            else {
                continue;
            };
            let name = stem.to_string_lossy().to_uppercase();
            if found.get(&name).map_or(true, |(r, _)| rank < *r) {
                found.insert(name, (rank, p));
            }
        }
        let mut n = 0;
        for coat in want {
            // フォルダに無ければ、共有の置き場から取る（[`set_remote_loader`]、配る版で膜のモデルを外したとき）
            let remote;
            let p = match found.get(&coat.to_uppercase()) {
                Some((_, p)) => p,
                None => match REMOTE.get().and_then(|f| f(dir, coat)) {
                    Some(r) => {
                        remote = r;
                        &remote
                    }
                    None => continue,
                },
            };
            let lbcs = p
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lbcs"));
            if lbcs {
                // 層のわかる近似の膜（膜ファイルの形式）: 膜を系の名前で入れ、足りない材料も足す
                let f = CoatingFile::load(p)?;
                let Some(ls) = f
                    .coat
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(coat))
                    .map(|(_, l)| l.clone())
                else {
                    invalid!(
                        "{}: 膜 {coat} の COAT がない（ファイル名と中の膜の名前が違う）",
                        p.display()
                    );
                };
                for (m, rows) in f.mate {
                    self.file.mate.entry(m).or_insert(rows);
                }
                self.file.coat.insert(coat.to_string(), ls);
                self.auto.push((coat.to_string(), p.clone()));
                n += 1;
                continue;
            }
            let mut one = Coatings::default();
            one.load_file(p)?;
            // ファイルの中の膜の名前の大文字小文字が系と違っても、系の名前で引けるようにする
            let mut hit = false;
            for (name, m) in one.models {
                if name.eq_ignore_ascii_case(coat) {
                    self.models.insert(coat.to_string(), m);
                    hit = true;
                }
            }
            for (key, m) in one.measured {
                let mut it = key.splitn(2, '|');
                let (c0, rest) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
                if c0.eq_ignore_ascii_case(coat) {
                    let k = format!("{coat}|{rest}");
                    self.measured.insert(k.clone(), m);
                    self.auto_measured.push(k);
                    hit = true;
                }
            }
            if !hit {
                invalid!(
                    "{}: 膜 {coat} のモデル・実測表がない（ファイル名と中の膜の名前が違う）",
                    p.display()
                );
            }
            self.auto.push((coat.to_string(), p.clone()));
            n += 1;
        }
        Ok(n)
    }

    pub fn add_measured(&mut self, key: &str, m: Measured) -> Result<()> {
        if m.data.len() != m.wavelengths.len()
            || m.data.iter().any(|r| r.len() != m.angles_deg.len())
        {
            invalid!("実測表 {key} の大きさ");
        }
        if m.angles_deg.windows(2).any(|w| !(w[0] < w[1])) {
            invalid!("実測表 {key} の角度が昇順でない");
        }
        self.measured.insert(key.to_string(), m);
        Ok(())
    }

    /// 境界 key の膜の扱い（prepare と同じ順で判定する）
    pub fn kind(&self, key: &str) -> CoatKind {
        let mut it = key.splitn(3, '|');
        let coat = it.next().unwrap_or("");
        if self.file.table.contains_key(coat) {
            CoatKind::Table
        } else if coat == "-" {
            CoatKind::Bare
        } else if coat.starts_with("I.") {
            CoatKind::Ideal
        } else if self.file.coat.contains_key(coat) {
            CoatKind::Stack
        } else if self.measured.contains_key(key) && !key.ends_with("|R") {
            CoatKind::Measured
        } else if self.models.get(coat).is_some_and(|m| {
            if key.ends_with("|MIRROR") {
                m.mirror.is_some()
            } else {
                m.has_transmit()
            }
        }) {
            CoatKind::Model
        } else {
            CoatKind::Missing
        }
    }

    /// 膜のモデルのファイル（[`parse_models`] の形式）を読み、モデルの数を返す
    pub fn load_models(&mut self, path: &Path) -> Result<usize> {
        let b =
            crate::vfs::read(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let v = if b.starts_with(MODEL_BIN_HEADER) {
            parse_models_bin(&b)
        } else {
            std::str::from_utf8(&b)
                .map_err(|_| Error::Invalid("UTF-8 でない".into()))
                .and_then(parse_models)
        }
        .map_err(|e| match e {
            Error::Invalid(m) => Error::Invalid(format!("{}: {m}", path.display())),
            e => e,
        })?;
        let n = v.len();
        // 自動で読んだ実測表は、明示したモデルのある膜から外す（実測表はモデルより先に引かれる）
        let names: Vec<String> = v.iter().map(|(c, _)| c.clone()).collect();
        self.models.extend(v);
        let auto = std::mem::take(&mut self.auto_measured);
        for k in auto {
            let coat = k.split('|').next().unwrap_or("");
            if names.iter().any(|c| c == coat) {
                self.measured.remove(&k);
            } else {
                self.auto_measured.push(k);
            }
        }
        self.drop_auto_stacks();
        Ok(n)
    }

    /// 自動で読んだ近似の膜（`.lbcs`）のうち、後から明示して読んだモデル・実測表のある膜を膜ファイルから外す
    /// （膜ファイルの COAT はモデル・実測表より先に引かれるので、外さないと明示の指定が負ける）
    fn drop_auto_stacks(&mut self) {
        for (coat, p) in &self.auto {
            if !p
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lbcs"))
            {
                continue;
            }
            let pre = format!("{coat}|");
            if self.models.contains_key(coat) || self.measured.keys().any(|k| k.starts_with(&pre)) {
                self.file.coat.remove(coat);
            }
        }
    }

    /// 実測表（LBCT）か膜のモデル（LBCM）のファイルを、1 行目で見分けて読む
    pub fn load_file(&mut self, path: &Path) -> Result<usize> {
        let b =
            crate::vfs::read(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        if b.starts_with(MODEL_BIN_HEADER) {
            return self.load_models(path);
        }
        let txt = String::from_utf8_lossy(&b);
        let first = txt
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .unwrap_or("");
        if first == MODEL_HEADER {
            self.load_models(path)
        } else {
            self.load_measured(path)
        }
    }

    /// 実測表のファイル（[`parse_measured`] の形式）を読み、表の数を返す
    pub fn load_measured(&mut self, path: &Path) -> Result<usize> {
        let txt = crate::vfs::read_to_string(path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let v = parse_measured(&txt).map_err(|e| match e {
            Error::Invalid(m) => Error::Invalid(format!("{}: {m}", path.display())),
            e => e,
        })?;
        let n = v.len();
        for (k, m) in v {
            self.auto_measured.retain(|a| *a != k);
            self.add_measured(&k, m)?;
        }
        self.drop_auto_stacks();
        Ok(n)
    }

    /// 材料の複素屈折率（MATE の表、なければ系のガラス）
    fn mat_n(&self, sys: &System, name: &str, wl: f64) -> Result<C64> {
        if let Some(m) = self.file.mate.get(name) {
            if m.is_empty() {
                invalid!("膜の材料 {name} にデータがない");
            }
            let xs: Vec<f64> = m.iter().map(|r| r[0]).collect();
            // 1 行だけの材料は分散なし（どの波長も同じ値）。表なら範囲外を端の値で延ばさない
            let (lo, hi) = (xs[0], xs[xs.len() - 1]);
            if xs.len() > 1 && !(lo - 1e-9..=hi + 1e-9).contains(&wl) {
                invalid!("膜の材料 {name} の表は波長 {lo}〜{hi} µm で、{wl} µm がない（範囲外は延ばさない）");
            }
            let n: Vec<f64> = m.iter().map(|r| r[1]).collect();
            let k: Vec<f64> = m.iter().map(|r| r[2]).collect();
            return Ok(C64::new(interp(wl, &xs, &n), interp(wl, &xs, &k)));
        }
        match sys.glasses.get(name) {
            Some(g) => Ok(C64::new(
                g.index(wl, sys.temperature, sys.pressure, sys.adjust_index),
                0.0,
            )),
            None => invalid!("膜の材料 {name} が膜ファイルにもガラスカタログにもない"),
        }
    }

    /// 境界 key の波長 wl [µm] での係数を用意する（n1・n2 は前後の屈折率、鏡では n2 は使わない）
    ///
    /// 屈折面での反射（鍵 `膜|入射側|反対側|R`、ゴーストの経路）は鏡と同じ約束（p = s × k を反射の後で反転）の r を返す。
    /// n2 は反対側の媒質の屈折率。膜の向き（基板の側）は透過と同じく、反対側が空気でなければ反対側が基板、空気なら
    /// 入射側が基板。位相は基板の面を面の頂点に置いた値（膜の側から入るときは鏡と同じく exp(2 i k n1 cosθ1 Σ d_j)
    /// を掛け、基板の側から入るときは掛けない）。暗号化された膜は特性行列モデルの M から r を求める（透過の実測表
    /// だけでは求められない）
    pub fn prepare(&self, sys: &System, key: &str, wl: f64, n1: f64, n2: f64) -> Result<Iface> {
        let mut it = key.splitn(4, '|');
        let (coat, _m1, m2) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        let reflect = it.next() == Some("R");
        let mirror = m2 == "MIRROR" || reflect;
        let kind = if let Some(tab) = self.file.table.get(coat) {
            let angs: Vec<f64> = tab.iter().map(|a| a.0).collect();
            let mut vals: [Vec<f64>; 8] = Default::default();
            for (a, rows) in tab {
                if rows.is_empty() {
                    invalid!("膜の表 {coat} の角度 {a} に行がない");
                }
                let xs: Vec<f64> = rows.iter().map(|r| r[0]).collect();
                for c in 0..8 {
                    let mut ys: Vec<f64> = rows.iter().map(|r| r[c + 1]).collect();
                    if c >= 4 {
                        unwrap_deg(&mut ys);
                    }
                    vals[c].push(interp(wl, &xs, &ys));
                }
            }
            let step = self.table_normal == TableNormal::OpticStudio;
            if !step && angs[0] == 0.0 {
                // 段差なし: 0° の行の s を p で置き換えて補間する
                for c in [0, 2, 4, 6] {
                    vals[c][0] = vals[c + 1][0];
                }
            }
            // 位相（度）は ±180° の折り返しをほどいてから補間する（+179° と −179° の間を 0° にしない）
            for v in &mut vals[4..] {
                unwrap_deg(v);
            }
            Kind::Table {
                name: coat.to_string(),
                step,
                xc: angs.iter().map(|a| -a.to_radians().cos()).collect(),
                angs,
                vals: Box::new(vals),
                out: (!mirror && n1 > n2 && n2 > 0.0).then_some(n1 / n2),
            }
        } else if coat == "-" {
            Kind::Fresnel {
                ns: if m2 == "MIRROR" {
                    MIRROR_N
                } else {
                    C64::new(n2, 0.0)
                },
            }
        } else if coat.starts_with("I.") {
            // I.xx: 透過率 .xx。I.s=Ts,p=Tp: s・p で別の透過率（理想の偏光ビームスプリッター）。
            // 反射（鏡・屈折面での反射）は −√(1 − T)
            let [ts, tp] = ideal_t(coat)?;
            let v = |t: f64| C64::new(if mirror { -(1.0 - t).sqrt() } else { t.sqrt() }, 0.0);
            Kind::Ideal([v(ts), v(tp)])
        } else if let Some(ls) = self.file.coat.get(coat) {
            let wl_ref = sys.wavelengths[sys.primary];
            let mut layers = Vec::with_capacity(ls.len());
            for l in ls {
                let n = self.mat_n(sys, &l.mat, wl)?;
                let d = if l.absolute {
                    l.d
                } else {
                    l.d * wl_ref / self.mat_n(sys, &l.mat, wl_ref)?.re
                };
                layers.push((n, d));
            }
            // OpticStudio の約束（COATING.DAT の膜で、空気・硝材どうし・両向き・鏡を偏光追跡と照合）:
            // - 層は膜の外側（基板と反対側）から順。透過で後ろの媒質が空気なら、基板は前の媒質（層を逆に通る）
            // - 鏡の基板は最後の層の材料（半無限）。最後の層より上だけが薄膜
            // - 係数に位相 exp(i k Σ d_j (n_o / cos_o − β² / q_j)) が掛かる（n_o・cos_o は膜の外側の媒質、
            //   β = n1 sinθ1、q_j = sqrt(Re(n_j)² − β²)）。鏡では exp(2 i k n1 cosθ1 Σ d_j)
            if m2 == "MIRROR" {
                let ns = match layers.pop() {
                    Some((n, _)) => n,
                    None => MIRROR_N,
                };
                Kind::Stack {
                    layers,
                    ns,
                    n_out: n1,
                    layer_phase: true,
                    front: true,
                }
            } else if reflect {
                // 反対側が空気なら基板は入射側（層を逆に、基板の側から入る）
                let front = m2 != "AIR";
                if !front {
                    layers.reverse();
                }
                Kind::Stack {
                    layers,
                    ns: C64::new(n2, 0.0),
                    n_out: n1,
                    layer_phase: false,
                    front,
                }
            } else {
                if m2 == "AIR" {
                    layers.reverse();
                }
                Kind::Stack {
                    layers,
                    ns: C64::new(n2, 0.0),
                    n_out: if m2 == "AIR" { n2 } else { n1 },
                    layer_phase: self.phase == PhaseConvention::OpticStudio,
                    front: false,
                }
            }
        } else {
            // 実測表・モデル: 暗号化された膜
            if reflect && !self.models.contains_key(coat) {
                notimpl!("膜 {coat}（境界 {key}）: 反射の係数には特性行列モデルが要る（実測表は透過だけ）");
            }
            let Some(m) = self.measured.get(key).filter(|_| !reflect) else {
                return match self.models.get(coat) {
                    Some(md) => Ok(Iface {
                        kind: prepare_model(
                            coat,
                            md,
                            m2 == "MIRROR",
                            reflect,
                            m2 == "AIR",
                            wl,
                            n2,
                            self.phase,
                        )?,
                        mirror,
                        n1,
                        wl,
                    }),
                    None => notimpl!(
                        "膜 {coat}（境界 {key}）: 膜ファイルになく、実測表もモデルもない（暗号化された膜？）"
                    ),
                };
            };
            let Some(w) = m.wavelengths.iter().position(|&v| (v - wl).abs() <= 1e-9) else {
                notimpl!(
                    "膜の実測表 {key} に波長 {wl} µm がない（表は {:?}）",
                    m.wavelengths
                );
            };
            // 屈折角の余弦のスプライン（0 付近で角度に偶、全反射の平方根の振舞いもこれに線形）。鏡では入射角の余弦
            let r = if mirror { 1.0 } else { n1 / n2 };
            let mut xs = Vec::new();
            let mut ys: [Vec<f64>; 4] = Default::default();
            for (a, v) in m.angles_deg.iter().zip(&m.data[w]) {
                if !(v[0].re.is_finite() && v[0].im.is_finite()) {
                    continue;
                }
                xs.push(xcos(r, a.to_radians()));
                ys[0].push(v[0].re);
                ys[1].push(v[0].im);
                ys[2].push(v[1].re);
                ys[3].push(v[1].im);
            }
            let sp = [0, 1, 2, 3].map(|c| Spline::not_a_knot(&xs, &ys[c]));
            let sp = match sp {
                [Some(a), Some(b), Some(c), Some(d)] => [a, b, c, d],
                _ => invalid!("膜の実測表 {key}: 補間できる点が足りない"),
            };
            Kind::Measured {
                key: key.to_string(),
                r,
                max_deg: *m.angles_deg.last().unwrap(),
                cmax: m.angles_deg.last().unwrap().to_radians().cos(),
                sp: Box::new(sp),
            }
        };
        Ok(Iface {
            kind,
            mirror,
            n1,
            wl,
        })
    }
}

/// 実測表の横軸: −cos(屈折角)（増加）
fn xcos(r: f64, th: f64) -> f64 {
    xcos_s(r, th.sin())
}

/// [`xcos`] を入射角の sin で
fn xcos_s(r: f64, s0: f64) -> f64 {
    let s = r * s0;
    -(1.0 - s * s).max(0.0).sqrt()
}

/// 線形補間（xs は昇順。範囲外は端の値、numpy の interp と同じ）
/// 度の位相の列の ±180° の飛びをほどく（隣との差が 180° を超えれば 360° ずらす）
fn unwrap_deg(v: &mut [f64]) {
    for i in 1..v.len() {
        let d = v[i] - v[i - 1];
        v[i] -= 360.0 * (d / 360.0).round();
    }
}

pub fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    let n = xs.len();
    if x.is_nan() {
        return f64::NAN;
    }
    if n == 1 || x <= xs[0] {
        return ys[0];
    }
    if x >= xs[n - 1] {
        return ys[n - 1];
    }
    let i = xs.partition_point(|&v| v <= x) - 1;
    let t = (x - xs[i]) / (xs[i + 1] - xs[i]);
    ys[i] + t * (ys[i + 1] - ys[i])
}

/// 3 次スプライン（両端 not-a-knot、範囲外は端の多項式で外挿。scipy の CubicSpline と同じ）
#[derive(Debug, Clone)]
pub struct Spline {
    x: Vec<f64>,
    /// 区間ごとの係数（t = x − x_i の 3, 2, 1, 0 次）
    c: Vec<[f64; 4]>,
}

impl Spline {
    pub fn not_a_knot(x: &[f64], y: &[f64]) -> Option<Spline> {
        let n = x.len();
        if n < 2 || y.len() != n || x.windows(2).any(|w| !(w[0] < w[1])) {
            return None;
        }
        let dx: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
        let sl: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / dx[i]).collect();
        // 各点の傾き s
        let s: Vec<f64> = if n == 2 {
            vec![sl[0], sl[0]]
        } else if n == 3 {
            // 放物線
            let a = (sl[1] - sl[0]) / (x[2] - x[0]);
            (0..3)
                .map(|i| sl[0] + a * (2.0 * x[i] - x[0] - x[1]))
                .collect()
        } else {
            // 三重対角の連立（行 0 と n−1 が not-a-knot）
            let mut sub = vec![0.0; n];
            let mut dia = vec![0.0; n];
            let mut sup = vec![0.0; n];
            let mut b = vec![0.0; n];
            for i in 1..n - 1 {
                sub[i] = dx[i];
                dia[i] = 2.0 * (dx[i - 1] + dx[i]);
                sup[i] = dx[i - 1];
                b[i] = 3.0 * (dx[i] * sl[i - 1] + dx[i - 1] * sl[i]);
            }
            let d0 = x[2] - x[0];
            dia[0] = dx[1];
            sup[0] = d0;
            b[0] = ((dx[0] + 2.0 * d0) * dx[1] * sl[0] + dx[0] * dx[0] * sl[1]) / d0;
            let d1 = x[n - 1] - x[n - 3];
            dia[n - 1] = dx[n - 3];
            sub[n - 1] = d1;
            b[n - 1] = (dx[n - 2] * dx[n - 2] * sl[n - 3]
                + (2.0 * d1 + dx[n - 2]) * dx[n - 3] * sl[n - 2])
                / d1;
            solve_tridiag(&sub, &dia, &sup, &b)?
        };
        let c = (0..n - 1)
            .map(|i| {
                let t = (s[i] + s[i + 1] - 2.0 * sl[i]) / (dx[i] * dx[i]);
                let q = (3.0 * sl[i] - 2.0 * s[i] - s[i + 1]) / dx[i];
                [t, q, s[i], y[i]]
            })
            .collect();
        Some(Spline { x: x.to_vec(), c })
    }

    pub fn eval(&self, v: f64) -> f64 {
        if v.is_nan() {
            return f64::NAN;
        }
        self.eval_at(self.locate(v), v)
    }

    /// v の区間の番号（[`Self::eval_at`] に渡す）
    fn locate(&self, v: f64) -> usize {
        let n = self.x.len();
        self.x[1..n - 1].partition_point(|&a| a <= v)
    }

    /// 区間 i（節が同じか、このスプラインの節がその前の部分である別のスプラインで [`Self::locate`] したもの）での値。
    /// 節が前の部分のときは最後の区間に丸める（区間を探し直したのと同じ）
    fn eval_at(&self, i: usize, v: f64) -> f64 {
        let i = i.min(self.c.len() - 1);
        let t = v - self.x[i];
        let [a, b, c, d] = self.c[i];
        ((a * t + b) * t + c) * t + d
    }
}

/// 三重対角の連立方程式（部分ピボット付きの消去。行 0 と n−1 の対角が小さくてもよいように）
fn solve_tridiag(sub: &[f64], dia: &[f64], sup: &[f64], b: &[f64]) -> Option<Vec<f64>> {
    // 一般の帯行列として小さな密行列で解く（点数は 100 程度まで）
    let n = dia.len();
    let mut a = vec![vec![0.0; n + 1]; n];
    for i in 0..n {
        a[i][i] = dia[i];
        if i > 0 {
            a[i][i - 1] = sub[i];
        }
        if i + 1 < n {
            a[i][i + 1] = sup[i];
        }
        a[i][n] = b[i];
    }
    // 行 0 は (dia0, sup0)、行 n−1 は (sub, dia) の 2 要素。ピボット付きガウス消去
    for col in 0..n {
        let lim = (col + 3).min(n);
        let p = (col..lim).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[p][col] == 0.0 {
            return None;
        }
        a.swap(col, p);
        for r in col + 1..lim {
            let f = a[r][col] / a[col][col];
            if f != 0.0 {
                for c in col..=n {
                    a[r][c] -= f * a[col][c];
                }
            }
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut v = a[i][n];
        for c in i + 1..n {
            v -= a[i][c] * x[c];
        }
        x[i] = v / a[i][i];
    }
    Some(x)
}

/// 理想膜の名前から (s, p) の強度透過率: `I.xx`（s・p 共通、`I.98` は 0.98）、`I.s=Ts,p=Tp`（s・p で別）
pub fn ideal_t(coat: &str) -> Result<[f64; 2]> {
    let bad = || Error::Invalid(format!("理想膜 {coat}（I.xx か I.s=Ts,p=Tp）"));
    let rest = coat.strip_prefix("I.").ok_or_else(bad)?;
    let v = if rest.contains('=') {
        let (mut ts, mut tp) = (None, None);
        for part in rest.split(',') {
            let (k, x) = part.split_once('=').ok_or_else(bad)?;
            let x: f64 = x.trim().parse().map_err(|_| bad())?;
            match k.trim() {
                "s" => ts = Some(x),
                "p" => tp = Some(x),
                _ => return Err(bad()),
            }
        }
        [ts.ok_or_else(bad)?, tp.ok_or_else(bad)?]
    } else {
        let t: f64 = coat[1..].parse().map_err(|_| bad())?;
        [t, t]
    };
    if v.iter().any(|t| !(0.0..=1.0).contains(t)) {
        return Err(bad());
    }
    Ok(v)
}

#[derive(Debug, Clone)]
enum Kind {
    Fresnel {
        ns: C64,
    },
    /// 理想膜の (s, p) の係数
    Ideal([C64; 2]),
    Stack {
        layers: Vec<(C64, f64)>,
        ns: C64,
        /// 膜の外側（基板と反対側）の媒質の屈折率（位相の約束に使う）
        n_out: f64,
        /// 透過で層ごとの位相を含める（[`PhaseConvention::OpticStudio`]）
        layer_phase: bool,
        /// 反射で、膜の側から入る（位相 exp(2 i k n1 cosθ1 Σ d_j) を掛ける。基板が頂点にある）
        front: bool,
    },
    Table {
        name: String,
        /// 垂直入射（1 − cosθ < TABLE_NORMAL）だけ s に p を使う（[`TableNormal::OpticStudio`]）
        step: bool,
        angs: Vec<f64>,
        /// −cos(角度)（補間の横軸、増加）
        xc: Vec<f64>,
        /// 量ごと（Rs Rp Ts Tp と位相）の角度に対する値
        vals: Box<[Vec<f64>; 8]>,
        /// 透過で屈折率の高い側から入るとき n1 / n2（表の角度は屈折率の低い側の角度で引く。OpticStudio と同じ）
        out: Option<f64>,
    },
    Measured {
        key: String,
        r: f64,
        max_deg: f64,
        /// cos(max_deg)
        cmax: f64,
        sp: Box<[Spline; 4]>,
    },
    /// モデルの透過: β² のスプライン（偏光ごとに M̃ の実部・虚部 8 本と ψ）
    Model {
        name: String,
        /// 層ごとの位相（ψ）を含める（[`PhaseConvention::OpticStudio`]）
        layer_phase: bool,
        /// 屈折面での反射 r を返す（M から。ψ は打ち消し合うので使わない）
        reflect: bool,
        reverse: bool,
        n2: f64,
        d: f64,
        bmax: f64,
        b2: (f64, f64),
        sp: Box<[[Spline; 9]; 2]>,
    },
    /// モデルの鏡: 基準の媒質での r（この波長、β² の点ごと、偏光ごと）。β² の有理関数で補間する
    /// （高い β では導波の共鳴で、β 方向にも鋭く変わる）
    ModelMirror {
        name: String,
        n_ref: f64,
        d: f64,
        b2: Vec<f64>,
        r: Box<[Vec<C64>; 2]>,
        /// β² 方向の分岐点（膜の外の媒質で全反射が始まる β²）。補間の点はこれをまたがない
        branch: Option<Box<Branch>>,
        /// 値が決まらない β² の区間（両端を含む）。ここでは notimpl
        band: Option<(f64, f64)>,
    },
}

/// 鏡の β² 方向の分岐点 xb と、その両側の点の座標 t = sqrt(|β² − xb|)（どちらも t の昇順）。
/// r は分岐点の両側それぞれで t の滑らかな関数なので、t で補間する
#[derive(Debug, Clone)]
struct Branch {
    xb: f64,
    /// 下側: 点 lo_end − 1 − i の t
    lo_end: usize,
    t_lo: Vec<f64>,
    /// 上側: 点 hi_start + i の t
    hi_start: usize,
    t_hi: Vec<f64>,
}

impl Branch {
    fn new(b2: &[f64], xb: f64) -> Branch {
        // 分岐点ちょうどの点は両側で使う
        let lo_end = b2.partition_point(|&v| v <= xb + 1e-12);
        let hi_start = b2.partition_point(|&v| v < xb - 1e-12);
        Branch {
            xb,
            lo_end,
            t_lo: (0..lo_end)
                .rev()
                .map(|j| (xb - b2[j]).max(0.0).sqrt())
                .collect(),
            hi_start,
            t_hi: (hi_start..b2.len())
                .map(|j| (b2[j] - xb).max(0.0).sqrt())
                .collect(),
        }
    }

    /// β² = x での値（f(j) は点 j の値）
    fn eval(&self, x: f64, f: &dyn Fn(usize) -> C64) -> C64 {
        if x <= self.xb {
            let q = (self.xb - x).sqrt();
            rational_at(&self.t_lo, 0, self.t_lo.len(), q, &|i| {
                f(self.lo_end - 1 - i)
            })
        } else {
            let q = (x - self.xb).sqrt();
            rational_at(&self.t_hi, 0, self.t_hi.len(), q, &|i| f(self.hi_start + i))
        }
    }
}

/// 膜のモデルを波長 wl で用意する
#[allow(clippy::too_many_arguments)]
fn prepare_model(
    name: &str,
    md: &CoatModel,
    mirror: bool,
    reflect: bool,
    reverse: bool,
    wl: f64,
    n2: f64,
    phase: PhaseConvention,
) -> Result<Kind> {
    let range = |b: &[f64]| (b[0] * b[0], b[b.len() - 1] * b[b.len() - 1]);
    if mirror {
        let Some(m) = &md.mirror else {
            notimpl!("膜のモデル {name} に鏡のデータがない");
        };
        let Some((s0, e0)) = wave_segment(&m.wavelengths, wl, &md.kinks) else {
            notimpl!(
                "膜のモデル {name}（鏡）: 波長 {wl} µm {}",
                out_of_waves(&m.wavelengths, wl)
            );
        };
        // 波数の昇順に並べた点で補間する（波長は昇順なので逆順）
        let xs: Vec<f64> = (s0..e0).rev().map(|i| 1.0 / m.wavelengths[i]).collect();
        let b2: Vec<f64> = m.betas.iter().map(|b| b * b).collect();
        let brb = m.branches(&b2);
        let brs: Vec<Option<f64>> = brb.iter().map(|v| v.map(|v| v.xb)).collect();
        // 分岐点の位置は波長の滑らかな関数（基板の屈折率の 2 乗）なので、近い 4 点の 1/λ の 3 次式で補間する
        // （区間のどの波長にもあるときだけ使う）
        let branch = if brs[s0..e0].iter().all(Option::is_some) {
            let bx: Vec<f64> = (s0..e0).rev().map(|i| brs[i].unwrap_or(0.0)).collect();
            // どの波長でも同じ（分散のない媒質。格子の点に寄せたときなど）なら、補間の丸めで動かさない
            if bx.iter().all(|&v| v == bx[0]) {
                Some(bx[0])
            } else {
                let n = xs.len().min(4);
                let i = xs
                    .partition_point(|&x| x < 1.0 / wl)
                    .saturating_sub(n / 2)
                    .min(xs.len() - n);
                let q = 1.0 / wl;
                Some(
                    (i..i + n)
                        .map(|a| {
                            let w: f64 = (i..i + n)
                                .filter(|&c| c != a)
                                .map(|c| (q - xs[c]) / (xs[a] - xs[c]))
                                .product();
                            w * bx[a]
                        })
                        .sum::<f64>(),
                )
            }
        } else {
            None
        };
        let branch = branch.map(|xb| Box::new(Branch::new(&b2, xb)));
        // 補間に使いそうな波長（近い RATIONAL_N 点と、その両側に 2 点ずつ）で分岐点が点 j の β² の両側にあるなら、
        // β² を固定すると波長の補間が分岐点をまたぐ。その点は、各波長の表を分岐点からの β² の差 u = β² − xb で
        // 揃えてから波長で補間する。ほかの点は β² を固定して補間する（測った点で値が厳密に合う）
        // rational_at が選ぶ波長の点（端では内側へ寄せる）と、その両側 2 点
        let (w0, w1) = {
            let n = xs.len();
            let m = RATIONAL_N.min(n - n % 2).max(n.min(4));
            let st = xs
                .partition_point(|&x| x < 1.0 / wl)
                .saturating_sub(m / 2)
                .min(n - m);
            // xs の k 番目は波長の e0 − 1 − k 番目
            (
                (e0 - (st + m)).saturating_sub(2).max(s0),
                (e0 - st + 2).min(e0),
            )
        };
        // 分岐点が測った点の間にある波長では、そのそば（全反射の手前の最後の点 k − 1 から、全反射の 2 点目 k + 1 まで）は
        // r が急に変わるのに測った点が足りず、値が r のわずかな違いで大きく変わり、OpticStudio とも合わない
        // （細かく測った OpticStudio の値と 2.6e-2 違った。この範囲を外すと 8e-6 以下）。
        // 補間に使う波長のその範囲を合わせた区間では notimpl にする（測った点の上にある分岐点は除く）
        let band = if branch.is_some() {
            (w0..w1)
                .filter_map(|w| brb[w].filter(|v| !v.on_grid))
                .map(|v| (b2[v.k - 1], b2[(v.k + 1).min(b2.len() - 1)]))
                .reduce(|a, c| (a.0.min(c.0), a.1.max(c.1)))
        } else {
            None
        };
        let near = |j: usize| {
            branch.is_some() && {
                let above = |w: usize| brs[w].is_some_and(|xb| xb > b2[j]);
                let a = above(w0);
                (w0..w1).any(|w| above(w) != a)
            }
        };
        let side: Vec<Option<Branch>> = (s0..e0)
            .map(|i| {
                brs[i]
                    .filter(|_| branch.is_some())
                    .map(|xb| Branch::new(&b2, xb))
            })
            .collect();
        let mut r: [Vec<C64>; 2] = [Vec::new(), Vec::new()];
        for (pol, rp) in r.iter_mut().enumerate() {
            *rp = (0..m.betas.len())
                .map(|j| {
                    if near(j) {
                        let u = b2[j] - branch.as_ref().map_or(0.0, |bb| bb.xb);
                        rational_at(&xs, 0, xs.len(), 1.0 / wl, &|k| {
                            let w = e0 - 1 - k;
                            let bk = side[w - s0].as_ref().expect("分岐点");
                            bk.eval(bk.xb + u, &|jj| m.r[w][jj][pol])
                        })
                    } else {
                        rational_at(&xs, 0, xs.len(), 1.0 / wl, &|k| m.r[e0 - 1 - k][j][pol])
                    }
                })
                .collect();
        }
        return Ok(Kind::ModelMirror {
            name: name.to_string(),
            n_ref: m.n_ref,
            d: m.d,
            b2,
            r: Box::new(r),
            branch,
            band,
        });
    }
    if !md.has_transmit() {
        notimpl!("膜のモデル {name} に透過のデータがない");
    }
    let Some((i, w)) = wave_weights(&md.wavelengths, wl, &md.kinks) else {
        notimpl!(
            "膜のモデル {name}: 波長 {wl} µm {}",
            out_of_waves(&md.wavelengths, wl)
        );
    };
    let b2: Vec<f64> = md.betas.iter().map(|b| b * b).collect();
    let mut sp: Vec<[Spline; 9]> = Vec::new();
    for pol in 0..2 {
        let mut v: Vec<Spline> = Vec::new();
        for c in 0..9 {
            let ys: Vec<f64> = (0..md.betas.len())
                .map(|j| {
                    w.iter()
                        .enumerate()
                        .map(|(a, wa)| {
                            let k = i + a;
                            wa * if c == 8 {
                                md.psi[k][j][pol]
                            } else {
                                let z = md.m[k][j][pol][c / 2];
                                if c % 2 == 0 {
                                    z.re
                                } else {
                                    z.im
                                }
                            }
                        })
                        .sum()
                })
                .collect();
            if c == 8 {
                // ψ は BMAX までの点だけで（その先は層の屈折率で発散する）
                let m = md.betas.partition_point(|&b| b <= md.bmax + 1e-12);
                if m < 4 {
                    invalid!("膜のモデル {name}: BMAX までの β の点が足りない（4 点以上）");
                }
                v.push(beta_spline(name, &b2[..m], &ys[..m])?);
            } else {
                v.push(beta_spline(name, &b2, &ys)?);
            }
        }
        sp.push(v.try_into().unwrap());
    }
    let [s0, s1]: [[Spline; 9]; 2] = sp.try_into().unwrap();
    Ok(Kind::Model {
        name: name.to_string(),
        layer_phase: phase == PhaseConvention::OpticStudio,
        reflect,
        reverse,
        n2,
        d: md.d,
        bmax: md.bmax,
        b2: range(&md.betas),
        sp: Box::new([s0, s1]),
    })
}

/// ある波長で用意した境界の係数
#[derive(Debug, Clone)]
pub struct Iface {
    kind: Kind,
    mirror: bool,
    n1: f64,
    wl: f64,
}

impl Iface {
    /// 入射角の cos（0〜1）での (s, p)。屈折率が実数の境界（膜なし）は角度を経由せず実数で計算する
    /// （偏光追跡の面ごとの処理で、acos・複素数の計算を省く）。ほかは [`Self::eval`] と同じ
    pub fn eval_cos(&self, c0: f64) -> Result<[C64; 2]> {
        if let Kind::Fresnel { ns } = &self.kind {
            if ns.im == 0.0 && self.n1 > 0.0 {
                let (n0, ns) = (self.n1, ns.re);
                let s0 = ((1.0 - c0) * (1.0 + c0)).max(0.0).sqrt();
                let q = n0 * s0 / ns;
                let cc = 1.0 - q * q;
                if cc >= 0.0 && c0 > 0.0 {
                    let cs = cc.sqrt();
                    // s: η = n cosθ、p: η = n / cosθ（stack と同じ約束、パワー規格化）
                    let rt = |e0: f64, es: f64| {
                        let r = (e0 - es) / (e0 + es);
                        let t = 2.0 * e0 / (e0 + es) * (es / e0).sqrt();
                        (r, t)
                    };
                    let (rs, ts) = rt(n0 * c0, ns * cs);
                    let (rp, tp) = rt(n0 / c0, ns / cs);
                    return Ok(if self.mirror {
                        [C64::new(rs, 0.0), C64::new(rp, 0.0)]
                    } else {
                        [C64::new(ts, 0.0), C64::new(tp, 0.0)]
                    });
                }
            }
        }
        let c0 = c0.clamp(0.0, 1.0);
        self.eval_sc(((1.0 - c0) * (1.0 + c0)).sqrt(), c0)
    }

    /// 複屈折材の板の境界で、板の側の屈折率を `n` に置き換えた [`Self::eval_cos`]（公開 #52: 異常光の成分の係数）。
    /// `entering` = true は板へ入る面（後ろの媒質が板）、false は板から出る面（前の媒質が板）。
    /// 屈折率で決まる係数（膜なし・膜ファイルの層）だけ置き換え、理想膜・実測表・表・モデル・鏡は [`Self::eval_cos`] と同じ
    pub fn eval_cos_crystal(&self, c0: f64, entering: bool, n: f64) -> Result<[C64; 2]> {
        if entering {
            self.eval_cos_media(c0, None, Some(n))
        } else {
            self.eval_cos_media(c0, Some(n), None)
        }
    }

    /// 前の媒質の屈折率を `n1`、後ろの媒質の屈折率を `n2` に置き換えた [`Self::eval_cos`]（None は置き換えない）。
    /// 複屈折材の板の境界（公開 #52）と、板どうしの貼り合わせの境界（前の板の o・e → 次の板の o・e、公開 #73）に使う。
    /// 置き換えるのは [`Self::eval_cos_crystal`] と同じく屈折率で決まる係数だけ
    pub fn eval_cos_media(&self, c0: f64, n1: Option<f64>, n2: Option<f64>) -> Result<[C64; 2]> {
        if self.mirror {
            return self.eval_cos(c0);
        }
        let mut f = self.clone();
        match &mut f.kind {
            Kind::Fresnel { ns } => {
                if let Some(n) = n2 {
                    *ns = C64::new(n, 0.0);
                }
                if let Some(n) = n1 {
                    f.n1 = n;
                }
            }
            Kind::Stack { ns, n_out, .. } => {
                // n_out（位相の約束の外側の媒質）が板なら、それも置き換える（判定は置き換える前の値で）
                let (out_is_2, out_is_1) = (ns.im == 0.0 && *n_out == ns.re, *n_out == f.n1);
                if let Some(n) = n2 {
                    if out_is_2 {
                        *n_out = n;
                    }
                    *ns = C64::new(n, 0.0);
                }
                if let Some(n) = n1 {
                    if out_is_1 {
                        *n_out = n;
                    }
                    f.n1 = n;
                }
            }
            _ => return self.eval_cos(c0),
        }
        f.eval_cos(c0)
    }

    /// 入射角 theta [rad] での (s, p): 透過、鏡では反射
    pub fn eval(&self, theta: f64) -> Result<[C64; 2]> {
        let (s0, c0) = theta.sin_cos();
        self.eval_sc(s0, c0)
    }

    /// 入射角の sin・cos での (s, p)（角度を経由しない）
    fn eval_sc(&self, s0: f64, c0: f64) -> Result<[C64; 2]> {
        let mirror = self.mirror;
        Ok(match &self.kind {
            Kind::Ideal(v) => *v,
            Kind::Fresnel { ns } => {
                let (rs, rp, ts, tp) = stack_sc(self.n1, &[], *ns, self.wl, s0, c0);
                if mirror {
                    [rs, rp]
                } else {
                    [ts, tp]
                }
            }
            Kind::Stack {
                layers,
                ns,
                n_out,
                layer_phase,
                front,
            } => {
                let (rs, rp, ts, tp) = stack_sc(self.n1, layers, *ns, self.wl, s0, c0);
                let k = 2.0 * PI / self.wl;
                let d: f64 = layers.iter().map(|l| l.1).sum();
                if mirror {
                    let ph = if *front {
                        C64::new(0.0, 2.0 * k * self.n1 * c0 * d).exp()
                    } else {
                        C64::new(1.0, 0.0)
                    };
                    [rs * ph, rp * ph]
                } else {
                    let b = self.n1 * s0;
                    let co = (1.0 - (b / n_out) * (b / n_out)).max(0.0).sqrt();
                    if co == 0.0 {
                        // 膜の外側で全反射（光線の追跡ではその前に止まる）
                        notimpl!("膜の透過: 外側の媒質 n = {n_out} で全反射（β = {b}）");
                    }
                    let mut ph = d * n_out / co;
                    // q_j は層の屈折率の実部で（吸収のある層も OpticStudio はそうしている。金属薄膜の透過で照合）
                    for &(n, dj) in layers.iter().filter(|_| *layer_phase) {
                        let q = C64::new(n.re * n.re - b * b, 0.0).sqrt();
                        ph -= dj * (C64::new(b * b, 0.0) / q).re;
                    }
                    let ph = C64::new(0.0, k * ph).exp();
                    [ts * ph, tp * ph]
                }
            }
            Kind::Table {
                name,
                step,
                angs,
                xc,
                vals,
                out,
            } => {
                // 屈折率の高い側から入る透過（硝材 → 空気など）は、出る側（低い側）の屈折角で表を引く
                // （OpticStudio と同じ。tests/filter_os の裏面の膜で照合）
                let c0 = match out {
                    Some(r) => {
                        let so = r * s0;
                        ((1.0 - so) * (1.0 + so)).max(0.0).sqrt()
                    }
                    None => c0,
                };
                // 角度ではなく cosθ について線形（OpticStudio と同じ。偏光追跡で照合）
                let x = -c0;
                // 範囲の検査: 横軸 −cosθ が表の範囲の中なら角度も中（外れそうなときだけ角度にして調べる）
                if !(x >= xc[0] && x <= xc[xc.len() - 1]) {
                    let deg = c0.clamp(-1.0, 1.0).acos().to_degrees();
                    let (lo, hi) = (angs[0], *angs.last().unwrap());
                    if deg.is_finite() && (deg > hi + 1e-9 || deg < lo - 1e-9) {
                        let what = if out.is_some() {
                            "出る側の屈折角"
                        } else {
                            "入射角"
                        };
                        notimpl!("膜の表 {name}: {what} {deg:.4} deg が表の範囲 {lo}〜{hi} の外");
                    }
                }
                let v = |k: usize| interp(x, xc, &vals[k]);
                let (a, b) = if mirror { (0, 4) } else { (2, 6) };
                let p = C64::from_polar(v(a + 1).sqrt(), v(b + 1).to_radians());
                // 垂直入射では s・p の区別がないので p の値を使う。OpticStudio と同じく、ごく小さい角度
                // （1 − cosθ < TABLE_NORMAL）だけを垂直入射とみなす（その外は表の s に段差で戻る）
                if *step && 1.0 - c0 < TABLE_NORMAL {
                    return Ok([p, p]);
                }
                [C64::from_polar(v(a).sqrt(), v(b).to_radians()), p]
            }
            Kind::Measured {
                key,
                r,
                max_deg,
                cmax,
                sp,
            } => {
                // 範囲の検査: cosθ が表の最大角の cos より十分大きければ範囲内（境目の近くだけ角度にして調べる）
                if !(c0 > cmax + 1e-12) {
                    let deg = c0.clamp(-1.0, 1.0).acos().to_degrees();
                    if deg.is_finite() && deg > max_deg + 1e-12_f64.to_degrees() {
                        notimpl!(
                            "膜 {key}: 入射角 {deg:.4} deg が実測表の範囲（0〜{max_deg} deg）の外"
                        );
                    }
                }
                let x = xcos_s(*r, s0);
                // 4 本のスプラインは同じ節なので、区間は 1 回だけ探す
                let i = sp[0].locate(x);
                [
                    C64::new(sp[0].eval_at(i, x), sp[1].eval_at(i, x)),
                    C64::new(sp[2].eval_at(i, x), sp[3].eval_at(i, x)),
                ]
            }
            Kind::Model {
                name,
                layer_phase,
                reflect,
                reverse,
                n2,
                d,
                bmax,
                b2,
                sp,
            } => {
                let b = self.n1 * s0;
                let x = b * b;
                if *reflect {
                    if x < b2.0 - 1e-12 || x > b2.1 + 1e-12 {
                        notimpl!(
                            "膜のモデル {name}（反射）: β = {b:.4} がモデルの範囲（β ≤ {:.4}）の外",
                            b2.1.sqrt()
                        );
                    }
                    let k = 2.0 * PI / self.wl;
                    let ph = if *reverse {
                        C64::new(1.0, 0.0)
                    } else {
                        C64::new(0.0, 2.0 * k * self.n1 * c0 * d).exp()
                    };
                    let mut out = [C64::new(0.0, 0.0); 2];
                    for (pol, o) in out.iter_mut().enumerate() {
                        let s = &sp[pol];
                        let m: Vec<C64> = (0..4)
                            .map(|c| C64::new(s[2 * c].eval(x), s[2 * c + 1].eval(x)))
                            .collect();
                        let (m00, m10, m01, m11) = if *reverse {
                            (m[3], m[1], m[2], m[0])
                        } else {
                            (m[0], m[1], m[2], m[3])
                        };
                        let e1 = admittance(self.n1, b, pol);
                        let e2 = admittance_ev(*n2, b, pol);
                        let (bb, cc) = (m00 + m01 * e2, m10 + m11 * e2);
                        *o = (e1 * bb - cc) / (e1 * bb + cc) * ph;
                    }
                    return Ok(out);
                }
                if (*layer_phase && b > bmax + 1e-12) || x < b2.0 - 1e-12 || x > b2.1 + 1e-12 {
                    notimpl!(
                        "膜のモデル {name}: β = {b:.4} がモデルの範囲（β ≤ {:.4}、BMAX {bmax}）の外",
                        b2.1.sqrt()
                    );
                }
                if n2 * n2 <= x {
                    // 後ろの媒質で全反射（光線の追跡ではその前に止まる）
                    return Ok([C64::new(0.0, 0.0); 2]);
                }
                let n_out = if *reverse { *n2 } else { self.n1 };
                let co = (1.0 - x / (n_out * n_out)).max(0.0).sqrt();
                if co == 0.0 {
                    notimpl!("膜の透過: 外側の媒質 n = {n_out} で全反射（β = {b}）");
                }
                let k = 2.0 * PI / self.wl;
                let ph = C64::new(0.0, k * d * n_out / co).exp();
                let mut out = [C64::new(0.0, 0.0); 2];
                // 18 本のスプラインは同じ β² の節（ψ はその前の部分）なので、区間は 1 回だけ探す
                let i = sp[0][0].locate(x);
                for (pol, o) in out.iter_mut().enumerate() {
                    let s = &sp[pol];
                    let g = if *layer_phase {
                        C64::new(0.0, k * s[8].eval_at(i, x)).exp()
                    } else {
                        C64::new(1.0, 0.0)
                    };
                    let m: [C64; 4] = std::array::from_fn(|c| {
                        g * C64::new(s[2 * c].eval_at(i, x), s[2 * c + 1].eval_at(i, x))
                    });
                    let (m00, m10, m01, m11) = if *reverse {
                        (m[3], m[1], m[2], m[0])
                    } else {
                        (m[0], m[1], m[2], m[3])
                    };
                    let e1 = admittance(self.n1, b, pol);
                    let e2 = admittance(*n2, b, pol);
                    let tp = e1 * 2.0 / (e1 * m00 + m10 + e1 * e2 * m01 + e2 * m11);
                    *o = tp * (e2 / e1).sqrt() * ph;
                }
                out
            }
            Kind::ModelMirror {
                name,
                n_ref,
                d,
                b2,
                r: rr,
                branch,
                band,
            } => {
                let b = self.n1 * s0;
                let x = b * b;
                let (lo, hi) = (b2[0], b2[b2.len() - 1]);
                if x < lo - 1e-12 || x > hi + 1e-12 {
                    notimpl!(
                        "膜のモデル {name}（鏡）: β = {b:.4} がモデルの範囲（β ≤ {:.4}）の外",
                        hi.sqrt()
                    );
                }
                if let Some((a, c)) = band {
                    if *a <= x && x <= *c {
                        notimpl!(
                            "膜のモデル {name}（鏡）: β = {b:.5} は膜の外の媒質で全反射に変わる点（β {:.5}〜{:.5}）のそばで、測った点が足りず値が決まらない",
                            a.sqrt(),
                            c.sqrt()
                        );
                    }
                }
                let k = 2.0 * PI / self.wl;
                let ph = C64::new(0.0, 2.0 * k * self.n1 * c0 * d).exp();
                let mut out = [C64::new(0.0, 0.0); 2];
                for (pol, o) in out.iter_mut().enumerate() {
                    let r = match branch.as_deref() {
                        Some(bb) => bb.eval(x, &|j| rr[pol][j]),
                        None => rational_at(b2, 0, b2.len(), x, &|j| rr[pol][j]),
                    };
                    let one = C64::new(1.0, 0.0);
                    let y = admittance(*n_ref, b, pol) * (one - r) / (one + r);
                    let e0 = admittance(self.n1, b, pol);
                    *o = (e0 - y) / (e0 + y) * ph;
                }
                out
            }
        })
    }
}

/// 複素屈折率 n の媒質での角の余弦（n0 sinθ0 = n sinθ、Im(n cos) ≤ 0 の枝）
fn ccos(n0: f64, s0: f64, n: C64) -> C64 {
    let q = C64::new(n0 * s0, 0.0) / n;
    let c = (C64::new(1.0, 0.0) - q * q).sqrt();
    if (n * c).im > 0.0 {
        -c
    } else {
        c
    }
}

/// 媒質 n0（実数）から層 [(N, d [µm])] を経て基板 ns へ、n0 での入射角 theta の薄膜
/// → (r_s, r_p, t_s, t_p)、パワー規格化、OpticStudio の約束
pub fn stack(n0: f64, layers: &[(C64, f64)], ns: C64, wl: f64, theta: f64) -> (C64, C64, C64, C64) {
    let (s0, c0) = theta.sin_cos();
    stack_sc(n0, layers, ns, wl, s0, c0)
}

/// [`stack`] を入射角の sin・cos で
fn stack_sc(
    n0: f64,
    layers: &[(C64, f64)],
    ns: C64,
    wl: f64,
    s0: f64,
    c0: f64,
) -> (C64, C64, C64, C64) {
    let cs = ccos(n0, s0, ns);
    let one = C64::new(1.0, 0.0);
    let i = C64::new(0.0, 1.0);
    let mut out = [(one, one); 2];
    for (pi, o) in out.iter_mut().enumerate() {
        let eta = |n: C64, c: C64| if pi == 0 { n * c } else { n / c };
        let e0 = eta(C64::new(n0, 0.0), C64::new(c0, 0.0));
        let es = eta(ns, cs);
        let mut m = [[one, C64::new(0.0, 0.0)], [C64::new(0.0, 0.0), one]];
        for &(n, d) in layers {
            let c = ccos(n0, s0, n);
            let e = eta(n, c);
            let dl = n * c * (2.0 * PI * d / wl);
            let (cd, sd) = (dl.cos(), dl.sin());
            let l = [[cd, i * sd / e], [i * e * sd, cd]];
            m = [
                [
                    m[0][0] * l[0][0] + m[0][1] * l[1][0],
                    m[0][0] * l[0][1] + m[0][1] * l[1][1],
                ],
                [
                    m[1][0] * l[0][0] + m[1][1] * l[1][0],
                    m[1][0] * l[0][1] + m[1][1] * l[1][1],
                ],
            ];
        }
        let b = m[0][0] + m[0][1] * es;
        let c = m[1][0] + m[1][1] * es;
        let r = (e0 * b - c) / (e0 * b + c);
        let t = e0 * 2.0 / (e0 * b + c) * C64::new(es.re / e0.re, 0.0).sqrt();
        *o = (r, t);
    }
    let [(rs, ts), (rp, tp)] = out;
    (rs, rp, ts, tp)
}

/// 面ごとの境界の鍵（`膜|前の媒質|後の媒質（鏡は MIRROR）`、境界でなければ None）と、各面の前の媒質の名前
pub fn media_keys(sys: &System) -> (Vec<Option<String>>, Vec<String>) {
    let sf = &sys.surfaces;
    let mut keys = vec![None; sf.len()];
    let mut before = vec!["AIR".to_string(); sf.len()];
    let mut prev = "AIR".to_string();
    let mut nprev = sf[0].n[0];
    for (k, d) in sf.iter().enumerate() {
        before[k] = prev.clone();
        if d.ignore || d.typ == SurfType::CoordBrk {
            continue;
        }
        let after = if d.mirror {
            prev.clone()
        } else if d.mat.is_empty() {
            "AIR".to_string()
        } else {
            d.mat.clone()
        };
        // 名前の違う媒質は、ある波長で屈折率が同じでも境界にする（他の波長でフレネルが抜けないように）。
        // 複屈折材の板の入口・出口も境界にする（同じ材料の常光どうしでも、異常光の成分に Fresnel がある。公開 #52）
        if k > 0
            && (!d.coat.is_empty()
                || d.mirror
                || after != prev
                || (d.n[0] - nprev).abs() > 1e-12
                || d.bire.is_some())
        {
            let c = if d.coat.is_empty() { "-" } else { &d.coat };
            keys[k] = Some(match &d.reflect {
                // 屈折面での反射（ゴースト）: `膜|入射側|反対側|R`
                Some(r) if d.mirror => format!("{c}|{prev}|{}|R", r.mat),
                _ => {
                    let a = if d.mirror { "MIRROR" } else { &after };
                    format!("{c}|{prev}|{a}")
                }
            });
        }
        if !d.mirror {
            prev = after;
        }
        nprev = d.n[0];
    }
    (keys, before)
}

/// 媒質の内部透過率（厚さ 1 mm あたりの強度。IT データがない・波長が範囲外なら None）
pub fn internal_t(g: &glass::Glass, wl: f64) -> Option<f64> {
    if g.it.is_empty() {
        return None;
    }
    let mut it = g.it.clone();
    it.sort_by(|a, b| a[0].total_cmp(&b[0]));
    if !(it[0][0] <= wl && wl <= it[it.len() - 1][0]) {
        return None;
    }
    let xs: Vec<f64> = it.iter().map(|r| r[0]).collect();
    let t: Vec<f64> = it.iter().map(|r| r[1]).collect();
    let th: Vec<f64> = it.iter().map(|r| r[2]).collect();
    Some(interp(wl, &xs, &t).powf(1.0 / interp(wl, &xs, &th)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_cos_matches_eval_for_bare_interfaces() {
        // 実数の屈折率の境界（透過・鏡、n1 > ns の全反射の手前まで）で、cos からの直接計算が角度経由と一致する
        for &(n1, ns, mirror) in &[
            (1.0, 1.5, false),
            (1.5, 1.0, false),
            (1.0, 1.5, true),
            (1.7, 1.2, true),
        ] {
            let f = Iface {
                kind: Kind::Fresnel {
                    ns: C64::new(ns, 0.0),
                },
                mirror,
                n1,
                wl: 0.55,
            };
            for i in 0..=200 {
                let th = i as f64 / 200.0 * (std::f64::consts::FRAC_PI_2 - 1e-4);
                if n1 * th.sin() >= ns * (1.0 - 1e-9) {
                    continue;
                }
                let (a, b) = (f.eval(th).unwrap(), f.eval_cos(th.cos()).unwrap());
                for k in 0..2 {
                    assert!(
                        (a[k] - b[k]).norm() < 1e-13,
                        "{n1} {ns} {mirror} {th}: {:?} {:?}",
                        a,
                        b
                    );
                }
            }
        }
    }

    #[test]
    fn eval_cos_matches_eval_for_coated_interfaces() {
        // 膜（層・実測表）の境界でも、cos からの計算（acos を省く）が角度経由と丸めの差で一致する
        let layers = vec![(C64::new(1.38, 0.0), 0.1), (C64::new(2.1, -0.01), 0.07)];
        let mut cases = Vec::new();
        for mirror in [false, true] {
            cases.push(Iface {
                kind: Kind::Stack {
                    layers: layers.clone(),
                    ns: C64::new(1.52, 0.0),
                    n_out: 1.0,
                    layer_phase: true,
                    front: true,
                },
                mirror,
                n1: 1.0,
                wl: 0.55,
            });
        }
        let xs: Vec<f64> = (0..=12)
            .map(|i| xcos(1.0 / 1.5, (i as f64 * 7.0).to_radians()))
            .collect();
        let sp = [0.0, 1.0, 2.0, 3.0].map(|c| {
            let ys: Vec<f64> = xs.iter().map(|x| (c + 1.0) * x * x - c * x).collect();
            Spline::not_a_knot(&xs, &ys).unwrap()
        });
        cases.push(Iface {
            kind: Kind::Measured {
                key: "m".into(),
                r: 1.0 / 1.5,
                max_deg: 84.0,
                cmax: 84f64.to_radians().cos(),
                sp: Box::new(sp),
            },
            mirror: false,
            n1: 1.0,
            wl: 0.55,
        });
        for f in &cases {
            for i in 0..=200 {
                let th = i as f64 / 200.0 * 84f64.to_radians();
                let (a, b) = (f.eval(th).unwrap(), f.eval_cos(th.cos()).unwrap());
                for k in 0..2 {
                    assert!((a[k] - b[k]).norm() < 1e-12, "{th}: {a:?} {b:?}");
                }
            }
            // 範囲の外は eval と同じく止める
            if matches!(f.kind, Kind::Measured { .. }) {
                assert!(f.eval_cos(85f64.to_radians().cos()).is_err());
                assert!(f.eval(85f64.to_radians()).is_err());
            }
        }
    }

    #[test]
    fn rational_follows_a_sharp_resonance() {
        // 実軸のすぐ近くに極がある関数（導波の共鳴の形）。多項式では追えないが有理関数なら追える
        let f = |x: f64| C64::new(1.0, 0.0) / C64::new(x - 1.2345, 2e-3) + C64::new(0.3 * x, -0.1);
        let xs: Vec<f64> = (0..60).map(|i| 1.0 + 0.01 * i as f64).collect();
        let mut worst = 0.0f64;
        for k in 0..500 {
            let q = 1.02 + 0.55 * k as f64 / 499.0;
            let v = rational_at(&xs, 0, xs.len(), q, &|j| f(xs[j]));
            worst = worst.max((v - f(q)).norm());
        }
        assert!(worst < 1e-8, "{worst:e}");
    }

    #[test]
    fn mirror_branch_is_not_found_without_one() {
        let b2: Vec<f64> = (0..=800).map(|i| (0.0025 * i as f64).powi(2)).collect();
        let tab = |f: &dyn Fn(f64) -> f64| -> Vec<[C64; 2]> {
            b2.iter()
                .map(|&x| {
                    let a = f(x);
                    [
                        C64::from_polar(a, 0.3 + x),
                        C64::from_polar(a, -0.2 + 0.5 * x),
                    ]
                })
                .collect()
        };
        // 全反射しにくい高反射膜: |r| が β とともに少しずつ（指数的に）1 に近づくだけ
        let slow = tab(&|x| 1.0 - 1e-6 * (-8.0 * x).exp());
        assert!(slow.iter().any(|v| (v[0].norm() - 1.0).abs() < 1e-10));
        assert_eq!(mirror_branch(&b2, &slow), None);
        // 吸収のある膜: |r| は 1 に届かない
        assert_eq!(mirror_branch(&b2, &tab(&|x| 0.9 + 0.05 * x / 4.0)), None);
        // 分岐点（β = 1.4）の上で |r| = 1 に張り付く表では見つかる
        let xb = 1.96;
        let br = tab(&|x| {
            if x < xb {
                1.0 - 0.3 * (xb - x).sqrt()
            } else {
                1.0
            }
        });
        let got = mirror_branch(&b2, &br).unwrap().xb;
        assert!((got - xb).abs() < 1e-3, "{got}");
    }

    #[test]
    fn mirror_branch_is_stable_against_tiny_noise() {
        // 分岐点が測った β の点の上にある鏡（外の媒質 1.38、分散なし。β の格子 0.00125 刻みに 1.38 がある）。
        // r に 1e-9 の雑音を足しても、全反射の判定と分岐点の位置が変わらず、係数がほとんど変わらないこと
        let layers = [
            (C64::new(2.10, 0.0), 0.12),
            (C64::new(2.30, 0.0), 0.08),
            (C64::new(1.60, 0.0), 0.15),
        ];
        let exact = |wl: f64, b: f64| {
            let (rs, rp, _, _) = stack(2.05, &layers, C64::new(1.38, 0.0), wl, (b / 2.05).asin());
            [rs, rp]
        };
        let wavelengths: Vec<f64> = (0..41)
            .rev()
            .map(|i| 1.0 / (1.0 / 0.7 + i as f64 * (2.0 - 1.0 / 0.7) / 40.0))
            .collect();
        let betas: Vec<f64> = (0..=1600).map(|i| 0.00125 * i as f64).collect();
        let clean: Vec<Vec<[C64; 2]>> = wavelengths
            .iter()
            .map(|&w| betas.iter().map(|&b| exact(w, b)).collect())
            .collect();
        // 決まった擬似乱数で ±5e-10（1e-9 に丸めたのと同じ大きさ）
        let mut seed = 12345u64;
        let mut noise = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 1e-9
        };
        let noisy: Vec<Vec<[C64; 2]>> = clean
            .iter()
            .map(|w| {
                w.iter()
                    .map(|v| v.map(|z| z + C64::new(noise(), noise())))
                    .collect()
            })
            .collect();
        let model = |r: Vec<Vec<[C64; 2]>>| CoatModel {
            mirror: Some(MirrorModel {
                n_ref: 2.05,
                d: 0.0,
                wavelengths: wavelengths.clone(),
                betas: betas.clone(),
                r,
                ..Default::default()
            }),
            ..Default::default()
        };
        let (m0, m1) = (model(clean), model(noisy));
        let b2: Vec<f64> = betas.iter().map(|b| b * b).collect();
        let xb = b2[1104];
        for md in [&m0, &m1] {
            let brs = md.mirror.as_ref().unwrap().branches(&b2);
            assert!(
                brs.iter()
                    .all(|v| v.is_some_and(|v| v.xb == xb && v.on_grid)),
                "{brs:?}"
            );
        }
        // 測った波長と中点、β の点と中点で、硝材側の鏡の係数を比べる
        let mut wls = wavelengths.clone();
        wls.extend(wavelengths.windows(2).map(|w| 0.5 * (w[0] + w[1])));
        let mut worst = 0.0f64;
        for &wl in &wls {
            let prep = |md: &CoatModel| {
                prepare_model(
                    "T",
                    md,
                    true,
                    false,
                    false,
                    wl,
                    0.0,
                    PhaseConvention::OpticStudio,
                )
                .unwrap()
            };
            let (k0, k1) = (prep(&m0), prep(&m1));
            for n1 in [1.45, 1.65, 2.05] {
                let f = |k: &Kind| Iface {
                    kind: k.clone(),
                    mirror: true,
                    n1,
                    wl,
                };
                let (f0, f1) = (f(&k0), f(&k1));
                for i in 800..1400 {
                    let b = 0.000625 * i as f64;
                    if b >= n1 {
                        break;
                    }
                    let th = (b / n1).asin();
                    let (a, c) = (f0.eval(th).unwrap(), f1.eval(th).unwrap());
                    worst = worst.max((a[0] - c[0]).norm().max((a[1] - c[1]).norm()));
                }
            }
        }
        assert!(worst < 1e-6, "{worst:e}");
    }

    #[test]
    fn mirror_model_does_not_cross_the_branch() {
        // 2.05 の媒質から 3 層を経て、分散のある基板（n ≈ 1.41）へ光が抜ける鏡。β > n_s で |r| = 1 になり、
        // β = n_s で r が sqrt の形で変わる。分岐点は波長で動く
        let ns = |wl: f64| 1.4003 + 0.004 / (wl * wl);
        let layers = [
            (C64::new(2.10, 0.0), 0.12),
            (C64::new(2.30, 0.0), 0.08),
            (C64::new(2.15, 0.0), 0.15),
        ];
        let exact = |wl: f64, b: f64| {
            let (rs, rp, _, _) = stack(2.05, &layers, C64::new(ns(wl), 0.0), wl, (b / 2.05).asin());
            [rs, rp]
        };
        let wavelengths: Vec<f64> = (0..41)
            .rev()
            .map(|i| 1.0 / (1.0 / 0.7 + i as f64 * (2.0 - 1.0 / 0.7) / 40.0))
            .collect();
        let betas: Vec<f64> = (0..=1600).map(|i| 0.00125 * i as f64).collect();
        let r = wavelengths
            .iter()
            .map(|&w| betas.iter().map(|&b| exact(w, b)).collect())
            .collect();
        let md = CoatModel {
            mirror: Some(MirrorModel {
                n_ref: 2.05,
                d: 0.0,
                wavelengths,
                betas,
                r,
                ..Default::default()
            }),
            ..Default::default()
        };
        let (mut worst, mut over) = (0.0f64, 0.0f64);
        for wl in [0.5123, 0.5871, 0.6402, 0.6957] {
            let f = Iface {
                kind: prepare_model(
                    "T",
                    &md,
                    true,
                    false,
                    false,
                    wl,
                    0.0,
                    PhaseConvention::OpticStudio,
                )
                .unwrap(),
                mirror: true,
                n1: 2.05,
                wl,
            };
            let nb = ns(wl);
            let mut skipped = 0;
            for k in 0..2000 {
                let b = nb - 0.02 + 0.04 * k as f64 / 1999.0;
                // 分岐点のそば（β で 2〜3 刻み）は notimpl
                let Ok(v) = f.eval((b / 2.05).asin()) else {
                    assert!((b - nb).abs() < 0.004, "{wl} {b}");
                    skipped += 1;
                    continue;
                };
                let e = exact(wl, b);
                let d = (v[0] - e[0]).norm().max((v[1] - e[1]).norm());
                worst = worst.max(d);
                over = over.max(v[0].norm().max(v[1].norm()) - 1.0);
            }
            assert!(skipped < 300, "{skipped}");
        }
        assert!(worst < 1e-6, "{worst:e}");
        assert!(over < 1e-7, "{over:e}");
    }

    #[test]
    fn wave_weights_do_not_cross_kinks() {
        // 波数 x = 1/λ について、折れ目 0.5 µm の両側で別の 3 次式（値は連続、傾きが跳ぶ）
        let xk = 2.0;
        let f = |wl: f64| {
            let x = 1.0 / wl;
            let t = x - xk;
            1.0 + 0.3 * t - 0.2 * t * t + 0.05 * t * t * t + if t > 0.0 { 0.7 * t } else { 0.0 }
        };
        let wls: Vec<f64> = (0..15).map(|i| 0.4 + 0.025 * i as f64).collect();
        assert!(wls.iter().any(|&w| (w - 0.5).abs() < 1e-12));
        let ev = |wl: f64, k: &[f64]| {
            let (i, w) = wave_weights(&wls, wl, k).unwrap();
            w.iter()
                .enumerate()
                .map(|(a, wa)| wa * f(wls[i + a]))
                .sum::<f64>()
        };
        for wl in [0.41, 0.46, 0.49, 0.499, 0.5, 0.501, 0.51, 0.55, 0.74] {
            assert!((ev(wl, &[0.5]) - f(wl)).abs() < 1e-12, "{wl}");
        }
        // 折れ目を知らないと、またぐ区間でずれる
        assert!((ev(0.49, &[]) - f(0.49)).abs() > 1e-5);
    }

    #[test]
    fn fresnel_normal_incidence() {
        let (rs, rp, ts, tp) = stack(1.0, &[], C64::new(1.5, 0.0), 0.55, 0.0);
        assert!((rs.re + 0.2).abs() < 1e-15 && (rp.re + 0.2).abs() < 1e-15);
        assert!((ts.norm_sqr() + rs.norm_sqr() - 1.0).abs() < 1e-14);
        assert!((tp.norm_sqr() + rp.norm_sqr() - 1.0).abs() < 1e-14);
    }

    #[test]
    fn energy_is_conserved_off_axis() {
        for deg in [10.0f64, 30.0, 55.0] {
            let th = deg.to_radians();
            let (rs, rp, ts, tp) = stack(1.0, &[], C64::new(1.7, 0.0), 0.55, th);
            assert!((ts.norm_sqr() + rs.norm_sqr() - 1.0).abs() < 1e-13);
            assert!((tp.norm_sqr() + rp.norm_sqr() - 1.0).abs() < 1e-13);
        }
        // ブルースター角で p の反射が消える
        let th = 1.7f64.atan();
        let (_, rp, _, _) = stack(1.0, &[], C64::new(1.7, 0.0), 0.55, th);
        assert!(rp.norm() < 1e-14);
    }

    #[test]
    fn quarter_wave_ar() {
        // n = √1.5 の 1/4 波長膜は垂直入射で反射ゼロ
        let n = 1.5f64.sqrt();
        let d = 0.55 / 4.0 / n;
        let (rs, _, ts, _) = stack(1.0, &[(C64::new(n, 0.0), d)], C64::new(1.5, 0.0), 0.55, 0.0);
        assert!(rs.norm() < 1e-14);
        assert!((ts.norm_sqr() - 1.0).abs() < 1e-14);
    }

    #[test]
    fn spline_matches_cubic_exactly() {
        // not-a-knot は 3 次多項式を正確に再現する
        let f = |x: f64| 0.3 * x * x * x - x * x + 2.0 * x - 0.5;
        let x: Vec<f64> = (0..9)
            .map(|i| -1.0 + 0.13 * i as f64 + 0.01 * (i * i) as f64)
            .collect();
        let y: Vec<f64> = x.iter().map(|&v| f(v)).collect();
        let s = Spline::not_a_knot(&x, &y).unwrap();
        for v in [-1.2, -0.95, -0.3, 0.1, 0.7, 1.3] {
            assert!((s.eval(v) - f(v)).abs() < 1e-12, "{v}");
        }
    }

    #[test]
    fn measured_file_round_trip() {
        let m = Measured {
            wavelengths: vec![0.84, 0.85],
            angles_deg: vec![0.0, 10.0, 20.0],
            data: vec![
                vec![[C64::new(0.9, -0.1), C64::new(0.8, 0.05)]; 3],
                vec![
                    [C64::new(0.1, 0.2), C64::new(0.3, 0.4)],
                    [C64::new(1.0 / 3.0, 0.0), C64::new(0.0, -1e-17)],
                    [C64::new(f64::NAN, f64::NAN), C64::new(f64::NAN, f64::NAN)],
                ],
            ],
        };
        let txt = format_measured(&[("X|AIR|N-BK7", &m), ("Y|AIR|MIRROR", &m)]);
        let v = parse_measured(&txt).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].0, "Y|AIR|MIRROR");
        let r = &v[0].1;
        assert_eq!(r.wavelengths, m.wavelengths);
        assert_eq!(r.angles_deg, m.angles_deg);
        assert_eq!(r.data[1][1], m.data[1][1]);
        assert!(r.data[1][2][0].re.is_nan());
        // 誤り
        for bad in [
            "",
            "LBCT 2\n",
            "LBCT 1\nWAVE 0.5\n",
            "LBCT 1\nKEY A|AIR|B\n0 1 0 1 0\n",
            "LBCT 1\nKEY A|AIR|B\nWAVE 0.5\n0 1 0 1\n",
            "LBCT 1\nKEY A|AIR|B\nWAVE 0.5\n1 1 0 1 0\n0 1 0 1 0\n",
            "LBCT 1\nKEY A|AIR|B\nWAVE 0.5\n0 1 0 1 0\nWAVE 0.6\n1 1 0 1 0\n",
            "LBCT 1\nKEY A|AIR|B\n",
            "LBCT 1\nKEY A|AIR|B\nWAVE 0.5\n0 1 0 1 0\nKEY A|AIR|B\nWAVE 0.5\n0 1 0 1 0\n",
        ] {
            assert!(parse_measured(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn kinds() {
        let mut c = Coatings {
            file: CoatingFile::parse(
                "COAT AR\nN15 0.25\nTABLE T\nANGL 0\nWAVE 0.5 0 0 1 1\nMATE N15\n0.5 1.5 0\n",
            )
            .unwrap(),
            measured: HashMap::new(),
            models: HashMap::new(),
            phase: PhaseConvention::default(),
            table_normal: TableNormal::default(),
            auto: Vec::new(),
            auto_measured: Vec::new(),
        };
        assert_eq!(c.kind("-|AIR|N-BK7"), CoatKind::Bare);
        assert_eq!(c.kind("I.98|AIR|N-BK7"), CoatKind::Ideal);
        assert_eq!(c.kind("AR|AIR|MIRROR"), CoatKind::Stack);
        assert_eq!(c.kind("AR|AIR|N-BK7"), CoatKind::Stack);
        assert_eq!(c.kind("T|AIR|N-BK7"), CoatKind::Table);
        assert_eq!(c.kind("ZEC|AIR|N-BK7"), CoatKind::Missing);
        let m = Measured {
            wavelengths: vec![0.5],
            angles_deg: vec![0.0, 1.0],
            data: vec![vec![[C64::new(1.0, 0.0); 2]; 2]],
        };
        c.add_measured("AR|AIR|N-BK7", m.clone()).unwrap();
        c.add_measured("ZEC|AIR|N-BK7", m).unwrap();
        assert_eq!(c.kind("AR|AIR|N-BK7"), CoatKind::Stack);
        assert_eq!(c.kind("ZEC|AIR|N-BK7"), CoatKind::Measured);
        assert!(CoatKind::Missing.needs_table() && !CoatKind::Measured.needs_table());
    }

    #[test]
    fn parse_file() {
        let txt = "! c\nMATE N15\n0.55 1.5 0\n\nCOAT AR\nN15 0.25\nN15 0.1 1\n\nTABLE T1\nANGL 0\n\
                   WAVE 0.5 0.1 0.1 0.9 0.9 0 0 0 0\nANGL 45\nWAVE 0.5 0.2 0.3 0.8 0.7 0 0 10 20\n\
                   ENCRYPTED X\n";
        let f = CoatingFile::parse(txt).unwrap();
        assert_eq!(f.mate["N15"], vec![[0.55, 1.5, 0.0]]);
        assert_eq!(f.coat["AR"].len(), 2);
        assert!(f.coat["AR"][1].absolute && !f.coat["AR"][0].absolute);
        assert_eq!(f.table["T1"].len(), 2);
        assert!(CoatingFile::parse("TABLE T\nWAVE 1 2 3 4 5 6 7 8 9\n").is_err());
        assert!(CoatingFile::parse("TABLE T\nANGL 0\nWAVE 1 2 3 4\n").is_err());
        let f = CoatingFile::parse("TABLE T\nANGL 0\nWAVE 1 0.1 0.1 0.9 0.9\n").unwrap();
        assert_eq!(
            f.table["T"][0].1[0],
            [1.0, 0.1, 0.1, 0.9, 0.9, 0.0, 0.0, 0.0, 0.0]
        );
        // 吸収は k < 0。k > 0（増幅）は誤り
        assert!(CoatingFile::parse("MATE M\n0.5 0.5 -3\n").is_ok());
        assert!(CoatingFile::parse("MATE M\n0.5 0.5 3\n").is_err());
    }

    /// 表の位相は ±180° の折り返しをほどいて補間する
    #[test]
    fn table_phase_unwrap() {
        let mut v = vec![170.0, 179.0, -179.0, -170.0, 175.0];
        unwrap_deg(&mut v);
        assert_eq!(v, vec![170.0, 179.0, 181.0, 190.0, 175.0]);
        let x = interp(0.55, &[0.5, 0.6], &[179.0, 181.0]);
        assert!((x - 180.0).abs() < 1e-12);
    }
}
