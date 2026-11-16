//! 実測表 LBCT（テキスト）の読み書き

use super::super::*;

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

/// 読込中の KEY: (キー, [(波長, [(角度, (s, p))])])
pub(in crate::coating) type Pending = (String, Vec<(f64, Vec<(f64, [C64; 2])>)>);

/// 実測表のファイル形式の 1 行目
pub(crate) const MEASURED_HEADER: &str = "LBCT 1";

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
            invalid!("measured table {key} has no data");
        }
        let angles: Vec<f64> = waves[0].1.iter().map(|r| r.0).collect();
        if waves
            .iter()
            .any(|w| w.1.iter().map(|r| r.0).ne(angles.iter().copied()))
        {
            invalid!("measured table {key}: the angles differ between wavelengths");
        }
        if out.iter().any(|(k, _)| *k == key) {
            invalid!("measured table {key} appears twice");
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
                invalid!("line 1 of the measured table is not {MEASURED_HEADER}");
            }
            header = true;
            continue;
        }
        let n = |s: &str| -> Result<f64> {
            s.parse::<f64>()
                .map_err(|_| Error::Invalid(format!("measured table line {} number {s}", ln + 1)))
        };
        if let Some(k) = line.strip_prefix("KEY ") {
            finish(cur.take(), &mut out)?;
            cur = Some((k.trim().to_string(), Vec::new()));
        } else if let Some(w) = line.strip_prefix("WAVE ") {
            let Some(c) = cur.as_mut() else {
                invalid!("measured table line {}: WAVE before KEY", ln + 1);
            };
            c.1.push((n(w.trim())?, Vec::new()));
        } else {
            let t: Vec<&str> = line.split_whitespace().collect();
            let Some(w) = cur.as_mut().and_then(|c| c.1.last_mut()) else {
                invalid!("measured table line {}: data before WAVE", ln + 1);
            };
            if t.len() != 5 {
                let line = ln + 1;
                invalid!(
                    "measured table line {line}: an angle and 4 coefficients (Re s Im s Re p Im p)"
                );
            }
            let a = n(t[0])?;
            if let Some(&(b, _)) = w.1.last() {
                if !(a > b) {
                    let line = ln + 1;
                    invalid!("measured table line {line}: the angles are not ascending");
                }
            }
            let v = [C64::new(n(t[1])?, n(t[2])?), C64::new(n(t[3])?, n(t[4])?)];
            w.1.push((a, v));
        }
    }
    if !header {
        invalid!("the measured table is empty");
    }
    finish(cur, &mut out)?;
    Ok(out)
}

/// 実測表をファイルの形式（[`parse_measured`]）にする
pub fn format_measured(entries: &[(&str, &Measured)]) -> String {
    use std::fmt::Write;
    let mut s = format!("{MEASURED_HEADER}\n");
    for (k, m) in entries {
        writeln!(s, "KEY {k}").expect("write to String");
        for (w, row) in m.wavelengths.iter().zip(&m.data) {
            writeln!(s, "WAVE {w:?}").expect("write to String");
            for (a, v) in m.angles_deg.iter().zip(row) {
                writeln!(
                    s,
                    "{a:?} {:?} {:?} {:?} {:?}",
                    v[0].re, v[0].im, v[1].re, v[1].im
                )
                .expect("write to String");
            }
        }
    }
    s
}
