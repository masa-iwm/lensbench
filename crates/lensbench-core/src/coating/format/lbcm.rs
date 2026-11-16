//! 特性行列モデル LBCM 2（テキスト）の読み書き

use super::super::*;

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
    pub(in crate::coating) fn branches(&self, b2: &[f64]) -> &[Option<MirrorBranch>] {
        self.branches
            .get_or_init(|| self.r.iter().map(|rw| mirror_branch(b2, rw)).collect())
    }
}

impl CoatModel {
    pub(in crate::coating) fn has_transmit(&self) -> bool {
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
            invalid!("coating model {name}: the β values differ between wavelengths");
        }
        if b.len() < 4 || w.iter().any(|x| x.1.is_empty()) {
            invalid!("coating model {name}: too few β points (4 or more)");
        }
        if w.windows(2).any(|x| !(x[0].0 < x[1].0)) {
            invalid!("coating model {name}: the wavelengths are not ascending");
        }
        Ok(b)
    }
    fn finish(cur: Option<Cur>, out: &mut Vec<(String, CoatModel)>) -> Result<()> {
        let Some(mut c) = cur else { return Ok(()) };
        let name = c.name.clone();
        if c.t.is_empty() && c.r.is_empty() {
            invalid!("coating model {name} has no data");
        }
        if !c.t.is_empty() {
            let (Some(d), Some(bmax)) = (c.d, c.bmax) else {
                invalid!("coating model {name}: transmission needs D and BMAX");
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
                invalid!("coating model {name}: MIRROR has no data");
            }
            m.betas = same_betas(&name, &c.r)?;
            m.wavelengths = c.r.iter().map(|w| w.0).collect();
            m.r =
                c.r.iter()
                    .map(|w| w.1.iter().map(|r| r.1).collect())
                    .collect();
        }
        if out.iter().any(|(k, _)| *k == name) {
            invalid!("coating model {name} appears twice");
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
                invalid!("line 1 of the coating model is not {MODEL_HEADER}");
            }
            header = true;
            continue;
        }
        let n = |s: &str| -> Result<f64> {
            s.parse::<f64>()
                .map_err(|_| Error::Invalid(format!("coating model line {} number {s}", ln + 1)))
        };
        let t: Vec<&str> = line.split_whitespace().collect();
        if t[0] == "COAT" {
            finish(cur.take(), &mut out)?;
            if t.len() != 2 {
                invalid!("coating model line {}: COAT name", ln + 1);
            }
            cur = Some(Cur {
                name: t[1].to_string(),
                ..Default::default()
            });
            continue;
        }
        let Some(c) = cur.as_mut() else {
            invalid!("coating model line {}: before COAT", ln + 1);
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
                    invalid!("coating model line {}: data before WAVE", ln + 1);
                };
                if prev.is_some_and(|p| !(b > p)) {
                    invalid!("coating model line {}: β is not ascending", ln + 1);
                }
                let z = |i: usize| C64::new(v[i], v[i + 1]);
                if c.in_mirror {
                    if v.len() != 5 {
                        let line = ln + 1;
                        invalid!("coating model line {line}: a mirror has β and 4 coefficients");
                    }
                    c.r.last_mut()
                        .expect("block started above")
                        .1
                        .push((b, [z(1), z(3)]));
                } else {
                    if v.len() != 19 {
                        let line = ln + 1;
                        invalid!("coating model line {line}: transmission has β and 18 columns");
                    }
                    let m = [[z(1), z(3), z(5), z(7)], [z(10), z(12), z(14), z(16)]];
                    c.t.last_mut()
                        .expect("block started above")
                        .1
                        .push((b, (m, [v[9], v[18]])));
                }
            }
        }
    }
    if !header {
        invalid!("the coating model is empty");
    }
    finish(cur, &mut out)?;
    Ok(out)
}

/// 膜のモデルをファイルの形式（[`parse_models`]）にする
pub fn format_models(entries: &[(&str, &CoatModel)]) -> String {
    use std::fmt::Write;
    let mut s = format!("{}\n", MODEL_HEADER);
    for (name, m) in entries {
        writeln!(s, "COAT {name}").expect("write to String");
        if !m.note.is_empty() {
            writeln!(s, "NOTE {}", m.note.replace(['\n', '\r'], " ")).expect("write to String");
        }
        if !m.kinks.is_empty() {
            let k: Vec<String> = m.kinks.iter().map(|k| format!("{k:?}")).collect();
            writeln!(s, "KINKS {}", k.join(" ")).expect("write to String");
        }
        if !m.wavelengths.is_empty() {
            writeln!(s, "D {:?}\nBMAX {:?}", m.d, m.bmax).expect("write to String");
            for (iw, w) in m.wavelengths.iter().enumerate() {
                writeln!(s, "WAVE {w:?}").expect("write to String");
                for (j, b) in m.betas.iter().enumerate() {
                    write!(s, "{b:?}").expect("write to String");
                    for pol in 0..2 {
                        for z in m.m[iw][j][pol] {
                            write!(s, " {:?} {:?}", z.re, z.im).expect("write to String");
                        }
                        write!(s, " {:?}", m.psi[iw][j][pol]).expect("write to String");
                    }
                    s.push('\n');
                }
            }
        }
        if let Some(r) = &m.mirror {
            writeln!(s, "MIRROR {:?} {:?}", r.n_ref, r.d).expect("write to String");
            for (iw, w) in r.wavelengths.iter().enumerate() {
                writeln!(s, "WAVE {w:?}").expect("write to String");
                for (j, b) in r.betas.iter().enumerate() {
                    let v = r.r[iw][j];
                    writeln!(
                        s,
                        "{b:?} {:?} {:?} {:?} {:?}",
                        v[0].re, v[0].im, v[1].re, v[1].im
                    )
                    .expect("write to String");
                }
            }
        }
    }
    s
}
