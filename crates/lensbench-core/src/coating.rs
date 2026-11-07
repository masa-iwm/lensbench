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
//! 境界の鍵は `膜|前の媒質|後の媒質（鏡は MIRROR）`（[`media_keys`]）。

use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::Path;

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
                _ => match &mut cur {
                    Cur::None => {}
                    Cur::Mate(name) => {
                        if t.len() < 2 {
                            invalid!("膜ファイル: 材料 {name} の行 {line}");
                        }
                        let k = if t.len() > 2 { num(t[2], name)? } else { 0.0 };
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
                },
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
}

impl Coatings {
    /// 系の膜ファイル（zmx の COFN）。なければ空（膜なし・理想膜・実測表だけ）
    pub fn for_system(sys: &System) -> Result<Coatings> {
        let file = match &sys.coating_file {
            Some(p) => CoatingFile::load(p)?,
            None => CoatingFile::default(),
        };
        Ok(Coatings {
            file,
            measured: HashMap::new(),
            models: HashMap::new(),
            phase: PhaseConvention::default(),
            table_normal: TableNormal::default(),
        })
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
        } else if self.measured.contains_key(key) {
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
        let txt = std::fs::read_to_string(path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let v = parse_models(&txt).map_err(|e| match e {
            Error::Invalid(m) => Error::Invalid(format!("{}: {m}", path.display())),
            e => e,
        })?;
        let n = v.len();
        self.models.extend(v);
        Ok(n)
    }

    /// 実測表（LBCT）か膜のモデル（LBCM）のファイルを、1 行目で見分けて読む
    pub fn load_file(&mut self, path: &Path) -> Result<usize> {
        let txt = std::fs::read_to_string(path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
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
        let txt = std::fs::read_to_string(path)
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let v = parse_measured(&txt).map_err(|e| match e {
            Error::Invalid(m) => Error::Invalid(format!("{}: {m}", path.display())),
            e => e,
        })?;
        let n = v.len();
        for (k, m) in v {
            self.add_measured(&k, m)?;
        }
        Ok(n)
    }

    /// 材料の複素屈折率（MATE の表、なければ系のガラス）
    fn mat_n(&self, sys: &System, name: &str, wl: f64) -> Result<C64> {
        if let Some(m) = self.file.mate.get(name) {
            if m.is_empty() {
                invalid!("膜の材料 {name} にデータがない");
            }
            let xs: Vec<f64> = m.iter().map(|r| r[0]).collect();
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
    pub fn prepare(&self, sys: &System, key: &str, wl: f64, n1: f64, n2: f64) -> Result<Iface> {
        let mut it = key.splitn(3, '|');
        let (coat, _m1, m2) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        let mirror = m2 == "MIRROR";
        let kind = if let Some(tab) = self.file.table.get(coat) {
            let angs: Vec<f64> = tab.iter().map(|a| a.0).collect();
            let mut vals: [Vec<f64>; 8] = Default::default();
            for (a, rows) in tab {
                if rows.is_empty() {
                    invalid!("膜の表 {coat} の角度 {a} に行がない");
                }
                let xs: Vec<f64> = rows.iter().map(|r| r[0]).collect();
                for c in 0..8 {
                    let ys: Vec<f64> = rows.iter().map(|r| r[c + 1]).collect();
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
            Kind::Table {
                name: coat.to_string(),
                step,
                xc: angs.iter().map(|a| -a.to_radians().cos()).collect(),
                angs,
                vals: Box::new(vals),
            }
        } else if coat == "-" {
            Kind::Fresnel {
                ns: if mirror { MIRROR_N } else { C64::new(n2, 0.0) },
            }
        } else if coat.starts_with("I.") {
            // I.xx: 透過率 .xx
            let t: f64 = coat[1..]
                .parse()
                .map_err(|_| Error::Invalid(format!("理想膜 {coat}")))?;
            let v = if mirror { -(1.0 - t).sqrt() } else { t.sqrt() };
            Kind::Ideal(C64::new(v, 0.0))
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
            if mirror {
                let ns = match layers.pop() {
                    Some((n, _)) => n,
                    None => MIRROR_N,
                };
                Kind::Stack {
                    layers,
                    ns,
                    n_out: n1,
                    layer_phase: true,
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
                }
            }
        } else {
            // 実測表・モデル: 暗号化された膜
            let Some(m) = self.measured.get(key) else {
                return match self.models.get(coat) {
                    Some(md) => Ok(Iface {
                        kind: prepare_model(coat, md, mirror, m2 == "AIR", wl, n2, self.phase)?,
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
    let s = r * th.sin();
    -(1.0 - s * s).max(0.0).sqrt()
}

/// 線形補間（xs は昇順。範囲外は端の値、numpy の interp と同じ）
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
        let n = self.x.len();
        let i = if v.is_nan() {
            return f64::NAN;
        } else {
            self.x[1..n - 1].partition_point(|&a| a <= v)
        };
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

#[derive(Debug, Clone)]
enum Kind {
    Fresnel {
        ns: C64,
    },
    Ideal(C64),
    Stack {
        layers: Vec<(C64, f64)>,
        ns: C64,
        /// 膜の外側（基板と反対側）の媒質の屈折率（位相の約束に使う）
        n_out: f64,
        /// 透過で層ごとの位相を含める（[`PhaseConvention::OpticStudio`]）
        layer_phase: bool,
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
    },
    Measured {
        key: String,
        r: f64,
        max_deg: f64,
        sp: Box<[Spline; 4]>,
    },
    /// モデルの透過: β² のスプライン（偏光ごとに M̃ の実部・虚部 8 本と ψ）
    Model {
        name: String,
        /// 層ごとの位相（ψ）を含める（[`PhaseConvention::OpticStudio`]）
        layer_phase: bool,
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
    },
}

/// 膜のモデルを波長 wl で用意する
fn prepare_model(
    name: &str,
    md: &CoatModel,
    mirror: bool,
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
        let mut r: [Vec<C64>; 2] = [Vec::new(), Vec::new()];
        for (pol, rp) in r.iter_mut().enumerate() {
            *rp = (0..m.betas.len())
                .map(|j| rational_at(&xs, 0, xs.len(), 1.0 / wl, &|k| m.r[e0 - 1 - k][j][pol]))
                .collect();
        }
        return Ok(Kind::ModelMirror {
            name: name.to_string(),
            n_ref: m.n_ref,
            d: m.d,
            b2,
            r: Box::new(r),
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
    /// 入射角 theta [rad] での (s, p): 透過、鏡では反射
    pub fn eval(&self, theta: f64) -> Result<[C64; 2]> {
        let mirror = self.mirror;
        Ok(match &self.kind {
            Kind::Ideal(v) => [*v, *v],
            Kind::Fresnel { ns } => {
                let (rs, rp, ts, tp) = stack(self.n1, &[], *ns, self.wl, theta);
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
            } => {
                let (rs, rp, ts, tp) = stack(self.n1, layers, *ns, self.wl, theta);
                let k = 2.0 * PI / self.wl;
                let d: f64 = layers.iter().map(|l| l.1).sum();
                if mirror {
                    let ph = C64::new(0.0, 2.0 * k * self.n1 * theta.cos() * d).exp();
                    [rs * ph, rp * ph]
                } else {
                    let b = self.n1 * theta.sin();
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
            } => {
                let deg = theta.to_degrees();
                let (lo, hi) = (angs[0], *angs.last().unwrap());
                if deg.is_finite() && (deg > hi + 1e-9 || deg < lo - 1e-9) {
                    notimpl!("膜の表 {name}: 入射角 {deg:.4} deg が表の範囲 {lo}〜{hi} の外");
                }
                // 角度ではなく cosθ について線形（OpticStudio と同じ。偏光追跡で照合）
                let x = -theta.cos();
                let v = |k: usize| interp(x, xc, &vals[k]);
                let (a, b) = if mirror { (0, 4) } else { (2, 6) };
                let p = C64::from_polar(v(a + 1).sqrt(), v(b + 1).to_radians());
                // 垂直入射では s・p の区別がないので p の値を使う。OpticStudio と同じく、ごく小さい角度
                // （1 − cosθ < TABLE_NORMAL）だけを垂直入射とみなす（その外は表の s に段差で戻る）
                if *step && 1.0 - theta.cos() < TABLE_NORMAL {
                    return Ok([p, p]);
                }
                [C64::from_polar(v(a).sqrt(), v(b).to_radians()), p]
            }
            Kind::Measured {
                key,
                r,
                max_deg,
                sp,
            } => {
                let deg = theta.to_degrees();
                if deg.is_finite() && deg > max_deg + 1e-12_f64.to_degrees() {
                    notimpl!(
                        "膜 {key}: 入射角 {deg:.4} deg が実測表の範囲（0〜{max_deg} deg）の外"
                    );
                }
                let x = xcos(*r, theta);
                [
                    C64::new(sp[0].eval(x), sp[1].eval(x)),
                    C64::new(sp[2].eval(x), sp[3].eval(x)),
                ]
            }
            Kind::Model {
                name,
                layer_phase,
                reverse,
                n2,
                d,
                bmax,
                b2,
                sp,
            } => {
                let b = self.n1 * theta.sin();
                let x = b * b;
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
                for (pol, o) in out.iter_mut().enumerate() {
                    let s = &sp[pol];
                    let g = if *layer_phase {
                        C64::new(0.0, k * s[8].eval(x)).exp()
                    } else {
                        C64::new(1.0, 0.0)
                    };
                    let m: Vec<C64> = (0..4)
                        .map(|c| g * C64::new(s[2 * c].eval(x), s[2 * c + 1].eval(x)))
                        .collect();
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
            } => {
                let b = self.n1 * theta.sin();
                let x = b * b;
                let (lo, hi) = (b2[0], b2[b2.len() - 1]);
                if x < lo - 1e-12 || x > hi + 1e-12 {
                    notimpl!(
                        "膜のモデル {name}（鏡）: β = {b:.4} がモデルの範囲（β ≤ {:.4}）の外",
                        hi.sqrt()
                    );
                }
                let k = 2.0 * PI / self.wl;
                let ph = C64::new(0.0, 2.0 * k * self.n1 * theta.cos() * d).exp();
                let mut out = [C64::new(0.0, 0.0); 2];
                for (pol, o) in out.iter_mut().enumerate() {
                    let r = rational_at(b2, 0, b2.len(), x, &|j| rr[pol][j]);
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
        // 名前の違う媒質は、ある波長で屈折率が同じでも境界にする（他の波長でフレネルが抜けないように）
        if k > 0
            && (!d.coat.is_empty() || d.mirror || after != prev || (d.n[0] - nprev).abs() > 1e-12)
        {
            let c = if d.coat.is_empty() { "-" } else { &d.coat };
            let a = if d.mirror { "MIRROR" } else { &after };
            keys[k] = Some(format!("{c}|{prev}|{a}"));
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
    }
}
