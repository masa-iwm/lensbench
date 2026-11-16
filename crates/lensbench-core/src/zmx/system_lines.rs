//! .zmx の系の行（鍵 4 文字の行）のうち、読まずに飛ばす行の注意と、瞳のアポダイゼーション（GFAC）

use super::*;

/// 系の行のうち、読む鍵（build・Raw のメソッドで使う）
const SYS_READ: &[&str] = &[
    "VERS", "MODE", "UNIT", "NAME", "ENPD", "FNUM", "OBNA", "FLOA", "ENVD", "GCAT", "RAIM", "FTYP",
    "ROPD", "XFLN", "YFLN", "FWGN", "VDXN", "VDYN", "VCXN", "VCYN", "VANN", "WAVM", "WAVL", "WWGT",
    "PWAV", "GLRS", "COFN", "AFOC", "MNUM", "HYPR",
];

/// 系の行のうち、読まなくても光線追跡・解析に効かない鍵（表示・作者・言語・公差・非順次の設定・最適化の設定・
/// エイミングの途中の値など）
const SYS_HARMLESS: &[&str] = &[
    "AUTH", "NOTE", "PFIL", "LANG", "IWDP", "TOL", "TOLE", "NSCD", "LUID", "EERA", "BLNK", "OMMA",
    "PUSH", "GSTD", "MAZH", "MCOM", "XFIE", "YFIE", "RSCE", "DMFS", "OPDX", "TELE",
];

/// 系の行のうち、値が既定（OpticStudio が普通に書く値）のときだけ害がない鍵と、その既定。違えば注意に残す
const SYS_DEFAULT_ONLY: &[(&str, &str)] = &[
    // 自動の半直径の余裕（mm・%）
    ("SDMA", "0 1 0"),
    // 偏光の状態（解析ごとに lensbench の設定を使う）
    ("POLS", "1 0 1 0 0 1 0"),
    // 近軸の量で座標ブレークを無視する
    ("PICB", "1"),
];

impl Raw {
    /// 読まずに飛ばした系の行の注意（行の鍵と値）。アポダイゼーション（GFAC）は系の設定として読むが、
    /// カタログの玉として読むとき（`catalog`）は使わず、一様でなければ注意に残す（玉の面の形・材料・径には
    /// 関係せず、差し込んだ先の系の瞳は使う側の系が決める）。系として読むときも、一様でなければ回折の解析にだけ
    /// 入ることを注意に残す
    pub(super) fn system_notes(&self, catalog: bool) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let mut keys: Vec<&String> = self.sysl.keys().collect();
        keys.sort();
        for k in keys {
            let k = k.as_str();
            if SYS_READ.contains(&k) || SYS_HARMLESS.contains(&k) {
                continue;
            }
            let vals = &self.sysl[k];
            let joined: Vec<String> = vals.iter().map(|v| v.join(" ")).collect();
            if k == "GFAC" {
                // GFAC 係数 種類。種類 0 は一様（係数は残っていても使わない）。カタログの玉でなければ apodization が読む
                if catalog
                    && vals
                        .first()
                        .and_then(|v| v.get(1))
                        .is_some_and(|x| !matches!(fv(x), Ok(v) if v == 0.0))
                {
                    out.push(format!(
                        ".zmx system line GFAC {} (pupil apodization) is not used because the system is read as catalog lenses",
                        joined.join(" / ")
                    ));
                } else if !catalog && !apodization(self, false)?.is_uniform() {
                    // 振幅は回折の解析（FFT PSF・Huygens・回折 MTF）にだけ入る（ANA-05）
                    out.push(format!(
                        ".zmx system line GFAC {} (pupil apodization) is used only by the diffraction analyses (FFT PSF, Huygens PSF/MTF, diffraction MTF); spot, RMS wavefront, Zernike and geometric MTF do not weight the pupil",
                        joined.join(" / ")
                    ));
                }
                continue;
            }
            if let Some((_, d)) = SYS_DEFAULT_ONLY.iter().find(|(n, _)| *n == k) {
                if joined.iter().all(|j| j == d) {
                    continue;
                }
                out.push(format!(
                    ".zmx system line {k} {} (differs from the default {d}, but lensbench doesn't read it)",
                    joined.join(" / ")
                ));
                continue;
            }
            out.push(format!(
                ".zmx system line {k} {} was skipped",
                joined.join(" / ")
            ));
        }
        Ok(out)
    }
}

/// 瞳のアポダイゼーション: 系の行 GFAC 係数 種類（[`crate::pupil::Apodization::from_zmx`]。OpticStudio 26.1 の実測で
/// `GFAC 1.5 1` = Gaussian 1.5、`GFAC 0 2` = Cosine Cubed、`GFAC 0 0` = 一様。種類が Gaussian でなくても係数は残る）。
/// APDT・APDF は系の行ではなく多重構成の操作数（構成で変わるものは誤り、変わらなければこの行と同じ）。
/// カタログの玉として読むときは使わない（[`Raw::system_notes`] が注意に残す）
pub(super) fn apodization(raw: &Raw, catalog: bool) -> Result<crate::pupil::Apodization> {
    if catalog {
        return Ok(crate::pupil::Apodization::Uniform);
    }
    let kind = sys_str(raw, "GFAC", 1).unwrap_or("0");
    crate::pupil::Apodization::from_zmx(kind, sys_f(raw, "GFAC", 0, 0.0)?)
}
