//! 像の質の項の値: MTF の種類（回折・矩形波・幾何・Huygens）と向き（[`TermKind::Mtf`]）

use super::*;
use lensbench_analysis::analysis::mtf_cutoff;
use lensbench_analysis::huygens::{huygens_mtf, HuygensOpts};
use lensbench_analysis::psf::geometric_mtf;

/// 幾何 MTF の瞳の求積の点の数の目安の既定（照合の系で 32 と 64 の差が 1e-6 以下）
const GEO_N: usize = 32;

/// 周波数 freqs の (子午, 球欠) の MTF（kind の求め方、only は片方の向きだけ求めるとき 1 = 子午・0 = 球欠）
fn mtf_at(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    n: Option<usize>,
    freqs: &[f64],
    kind: MtfKind,
    only: Option<usize>,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let w = Waves::all_or(wave);
    let m = match kind {
        MtfKind::Diffraction | MtfKind::SquareWave => {
            mtf_only(s, field, w, n, freqs, MtfMethod::default(), only)?
        }
        MtfKind::Geometric { diff_limit } => {
            geometric_mtf(s, field, w, n.unwrap_or(GEO_N), freqs, diff_limit)?
        }
        MtfKind::Huygens => {
            let o = HuygensOpts {
                pupil: n,
                ..HuygensOpts::default()
            };
            huygens_mtf(s, field, w, o, freqs)?
        }
    };
    Ok((m.tan, m.sag))
}

/// 矩形波の応答の級数の周波数（freq の奇数倍で、使う波長の回折の遮断周波数の最大まで）
fn square_freqs(s: &System, field: FieldSel, wave: Option<usize>, freq: f64) -> Result<Vec<f64>> {
    if !(freq > 0.0) {
        invalid!("the square wave MTF needs a positive frequency ({freq})");
    }
    let ws: Vec<usize> = match wave {
        Some(w) => vec![w],
        None => (0..s.wavelengths.len()).collect(),
    };
    let mut top = 0.0f64;
    for w in ws {
        let c = mtf_cutoff(s, field, w)?;
        top = top.max(c.tan).max(c.sag);
    }
    let mut f = vec![freq];
    let mut k = 3.0;
    while k * freq <= top {
        f.push(k * freq);
        k += 2.0;
    }
    Ok(f)
}

/// 矩形波の応答 (4/π) Σ (−1)^k M((2k+1)ν)/(2k+1)（Coltman）。m は ν の奇数倍の MTF
fn square_sum(m: &[f64]) -> f64 {
    let mut t = 0.0;
    for (k, v) in m.iter().enumerate() {
        let sg = if k % 2 == 0 { 1.0 } else { -1.0 };
        t += sg * v / (2 * k + 1) as f64;
    }
    4.0 / std::f64::consts::PI * t
}

/// MTF の項の残差（向き dir の数だけ）
#[allow(clippy::too_many_arguments)]
pub(super) fn mtf_term(
    s: &System,
    field: FieldSel,
    wave: Option<usize>,
    n: Option<usize>,
    freq: f64,
    dir: MtfDir,
    kind: MtfKind,
    target: f64,
) -> Result<Vec<f64>> {
    // 片方の向きだけの項はその向きだけ求める（値は両方求めたときと同じ）。幾何・Huygens はいつも両方
    let only = match dir {
        MtfDir::Tan => Some(1),
        MtfDir::Sag => Some(0),
        MtfDir::Both | MtfDir::Avg => None,
    };
    let (t, g) = if kind == MtfKind::SquareWave {
        let f = square_freqs(s, field, wave, freq)?;
        let (t, g) = mtf_at(s, field, wave, n, &f, kind, only)?;
        // 求めなかった向きは NaN（使わない）
        let sum = |v: &[f64]| {
            if v.len() == f.len() {
                square_sum(v)
            } else {
                f64::NAN
            }
        };
        (sum(&t), sum(&g))
    } else {
        let (t, g) = mtf_at(s, field, wave, n, &[freq], kind, only)?;
        (
            t.first().copied().unwrap_or(f64::NAN),
            g.first().copied().unwrap_or(f64::NAN),
        )
    };
    let r = match dir {
        MtfDir::Tan => vec![t - target],
        MtfDir::Sag => vec![g - target],
        MtfDir::Both => vec![t - target, g - target],
        MtfDir::Avg => vec![(t + g) / 2.0 - target],
    };
    if r.iter().any(|v| !v.is_finite()) {
        invalid!("can't compute the MTF");
    }
    Ok(r)
}

/// 包囲エネルギーの幾何の瞳の格子の既定（照合の系で 256 と 512 の 80 % の半径の差が 1e-3 倍以下）
const ENERGY_N: usize = 256;
/// 照度の瞳の縁の点の数の既定
const ILLUM_N: usize = lensbench_analysis::illum::RIM_POINTS;

/// 包囲エネルギー・ストレール比・照度の項の確かめ（i は項の番号）
pub(super) fn check(i: usize, k: &TermKind, s: &System) -> Result<()> {
    let (field, wave, n) = match k {
        TermKind::Energy {
            field,
            wave,
            n,
            method,
            shape,
            q,
            target,
            ..
        } => {
            match q {
                EnergyQ::Size(f) if !(*f > 0.0 && *f < 1.0) => {
                    invalid!("term {i}: energy fraction {f} (between 0 and 1)")
                }
                EnergyQ::Fraction(r) if !(*r > 0.0 && r.is_finite()) => {
                    invalid!("term {i}: energy size {r}")
                }
                _ => {}
            }
            if matches!(method, EnergyMethod::Geometric { airy: true })
                && *shape != lensbench_analysis::energy::EnergyShape::Circle
            {
                invalid!("term {i}: the Airy factor goes only with the circle");
            }
            if !target.is_finite() {
                invalid!("term {i}: target {target}");
            }
            (*field, *wave, *n)
        }
        TermKind::Strehl {
            field,
            wave,
            n,
            target,
        } => {
            if !target.is_finite() {
                invalid!("term {i}: target {target}");
            }
            (*field, *wave, *n)
        }
        TermKind::Illum {
            field,
            wave,
            n,
            target,
            ..
        } => {
            if !target.is_finite() {
                invalid!("term {i}: target {target}");
            }
            if s.afocal {
                invalid!("term {i}: the illumination of an afocal image space is not supported");
            }
            (*field, *wave, *n)
        }
        _ => unreachable!(),
    };
    if let FieldSel::Index(f) = field {
        if f >= s.fields.len() {
            invalid!("term {i}: field {f} (of {})", s.fields.len());
        }
    }
    if let Some(w) = wave {
        if w >= s.wavelengths.len() {
            invalid!("term {i}: wavelength {w} (of {})", s.wavelengths.len());
        }
    } else if waves(s, None).is_empty() {
        invalid!("term {i}: no weighted wavelength");
    }
    if n == Some(0) {
        invalid!("term {i}: sampling 0");
    }
    Ok(())
}

/// 包囲エネルギー・ストレール比・照度の項の値 − 目標
pub(super) fn term(s: &System, k: &TermKind) -> Result<f64> {
    use lensbench_analysis::energy::{AiryEnergy, Spots};
    let v = match k {
        TermKind::Energy {
            field,
            wave,
            n,
            method,
            shape,
            center,
            q,
            target,
        } => {
            let w = Waves::all_or(*wave);
            if let EnergyMethod::Geometric { airy: true } = method {
                let a = AiryEnergy::new(s, *field, w, n.unwrap_or(ENERGY_N), *center)?;
                match q {
                    EnergyQ::Size(f) => a.size(*f)? - target,
                    EnergyQ::Fraction(r) => a.fraction(*r) - target,
                }
            } else {
                let sp = match method {
                    EnergyMethod::Geometric { .. } => {
                        Spots::geometric(s, *field, w, n.unwrap_or(ENERGY_N))?
                    }
                    EnergyMethod::Fft => {
                        let o = lensbench_analysis::psf::PsfOpts {
                            pupil: *n,
                            ..Default::default()
                        };
                        Spots::psf(&lensbench_analysis::psf::psf(s, *field, w, o)?)?
                    }
                    EnergyMethod::Huygens => {
                        let o = HuygensOpts {
                            pupil: *n,
                            ..HuygensOpts::default()
                        };
                        Spots::psf(&lensbench_analysis::huygens::huygens_psf(s, *field, w, o)?)?
                    }
                };
                let at = sp.center(*center);
                match q {
                    EnergyQ::Size(f) => sp.size(at, *shape, *f)? - target,
                    EnergyQ::Fraction(r) => sp.fraction(at, *shape, *r) - target,
                }
            }
        }
        TermKind::Strehl {
            field,
            wave,
            n,
            target,
        } => lensbench_analysis::energy::strehl(s, *field, Waves::all_or(*wave), *n)? - target,
        TermKind::Illum {
            field,
            wave,
            n,
            relative,
            target,
        } => {
            let (w, n) = (wave.unwrap_or(s.primary), n.unwrap_or(ILLUM_N));
            let v = if *relative {
                lensbench_analysis::illum::relative_illumination(s, *field, w, n)?
            } else {
                lensbench_analysis::illum::effective_fno(s, *field, w, n)?
            };
            v - target
        }
        _ => unreachable!(),
    };
    if !v.is_finite() {
        invalid!("can't compute the term");
    }
    Ok(v)
}
