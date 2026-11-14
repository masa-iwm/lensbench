//! 系を追跡する解析を、解析のプラグイン（`lensbench_core::plugin`）として登録する同梱の crate。
//!
//! 計算は core（`lensbench_core::analysis`・`lensbench_core::psf`）のまま、名前・設定・配列の約束で包む。
//! 結果は core の関数を直接呼んだ値とビット一致する。どれも入力は `field`（数 1 つなら視野の番号（1 始まり）、
//! 2 個なら正規化視野 (hx, hy)）で、系は `Context` で受け取る。`wave` は波長の番号（1 始まり、省けば全波長の
//! 重み付き。wavefront・zernike は省けば主波長）。
//!
//! | 名前 | 設定 | 出力 |
//! |---|---|---|
//! | `spot` | `wave`・`rings`（6） | `x`・`y`・`weight`・`wave`・`centroid`・`chief`・`rms_centroid`・`rms_chief`・`geo` |
//! | `wavefront` | `wave`・`n`（64）・`ref_primary`（0） | `p`・`w`（(n, n)、外は NaN）・`pv`・`rms` |
//! | `zernike` | `wave`・`kind`（standard）・`terms`（37）・`n`（64）・`ref_primary`（0） | `coef`・`pv`・`pv_centroid`・`rms_fit`・`max_fit`・`rms_chief`・`rms_centroid`・`variance`・`strehl` |
//! | `mtf` | `wave`・`freqs`（省けば遮断周波数の 0〜0.95 を 21 点）・`n`・`method`（auto・interp・direct・fft） | `freqs`・`tan`・`sag`・`cutoff` |
//! | `psf` | `wave`・`pupil`（自動）・`image`（128）・`delta` | `x`・`y`・`i`（(image, image)）・`center`・`delta`・`peak`・`pupil`・`phase_step`・`enclosed` |
//! | `rms` | `wave`・`rings`（6）・`reference`（centroid・chief） | `spot`・`spot_x`・`spot_y`・`wfe`・`strehl` |
//! | `huygens_psf` | `wave`・`pupil`（自動）・`image`（128）・`delta` | psf と同じ（x・y は像面の局所座標の中心からの差） |
//! | `huygens_mtf` | `wave`・`freqs`（省けば mtf と同じ既定）・`pupil`（自動）・`image`（128）・`delta` | `freqs`・`tan`・`sag`・`cutoff` |
//! | `geo_mtf` | `wave`・`freqs`（省けば mtf と同じ既定）・`n`（128）・`diff_limit`（0） | `freqs`・`tan`・`sag`・`cutoff` |
//! | `pop` | `surface`（必須）・`wave`・`dist` か `to`・`method`（auto・fresnel・angular・direct）・`rays`（65）・`n`（256）・`half`・`aperture`（1）・`out_n`・`out_half`・`out_center`（2 個） | 出力の場 `u`（complex (ny, nx)、パイロットを除く）・`dx`・`dy`・`center`・`pilot`（ax・ay・curv）・`power`・`peak`・`centroid`・`rms_width`・`distance`・`wavelength`・`index`・`kernel_step`・`fresnel_error`、入力の場 `input_u`・`input_dx`・`input_dy`・`input_center` |
//! | `interferogram` | `wave`・`n`（64）・`beam1`（1）・`beam2`（2、0 で参照球）・`scale`（1）・`xtilt`・`ytilt`（0） | `p`・`delta`・`intensity`（(n, n)、外は NaN）・`pv`・`rms` |
//! | `ghost` | `first`（1）・`last`（省けば像面） | 組ごとの `a`・`b`・`marginal_y`・`marginal_u`・`focus`（無限遠は inf）・`wfno`・`intensity`（求まらなければ NaN） |
//!
//! `interferogram`（単色の干渉縞）の光束は系の多重構成の構成（`System::config`、構成が 1 つなら 1 だけ）。
//! `pop` は系の視野で入力の場を作り、`ghost` は視野を使わない（入力なし）。`ghost` の膜は `Context` の膜（なければ
//! 系の膜ファイル）を使う。

use lensbench_core::Waves;
use std::sync::Arc;

use lensbench_core::analysis::{self, MtfMethod, ZernKind};
use lensbench_core::coating::Coatings;
use lensbench_core::fringe::{self, PupilOpts};
use lensbench_core::huygens::{self, HuygensOpts};
use lensbench_core::plugin::{
    check_wave, field_sel, Array, ArrayReader, Arrays, Config, ConfigReader, Context, Instance,
    Kind, Plugin, Registry, Spec,
};
use lensbench_core::pop::{self, OutGrid, PopMethod, RayBeamOpts};
use lensbench_core::psf::{self, PsfOpts};
use lensbench_core::sweep::{self, RmsOpts, RmsRef};
use lensbench_core::{ghost, invalid, FieldSel, Result, System};

/// 同梱の解析を `reg` に登録する
pub fn register(reg: &mut Registry) {
    for p in [
        Analysis::Spot,
        Analysis::Wavefront,
        Analysis::Zernike,
        Analysis::Mtf,
        Analysis::Psf,
        Analysis::Rms,
        Analysis::GeoMtf,
        Analysis::HuygensPsf,
        Analysis::HuygensMtf,
        Analysis::Pop,
        Analysis::Ghost,
        Analysis::Interferogram,
    ] {
        reg.register(Arc::new(p));
    }
}

/// 同梱の解析の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Analysis {
    Spot,
    Wavefront,
    Zernike,
    Mtf,
    Psf,
    Rms,
    GeoMtf,
    HuygensPsf,
    HuygensMtf,
    Pop,
    Ghost,
    Interferogram,
}

/// 設定の決まった解析
#[derive(Debug, Clone)]
pub enum Prepared {
    Spot {
        wave: Option<usize>,
        rings: usize,
    },
    Wavefront {
        wave: Option<usize>,
        n: usize,
        ref_primary: bool,
    },
    Zernike {
        wave: Option<usize>,
        kind: ZernKind,
        terms: usize,
        n: usize,
        ref_primary: bool,
    },
    Mtf {
        wave: Option<usize>,
        freqs: Option<Vec<f64>>,
        n: Option<usize>,
        method: MtfMethod,
    },
    Psf {
        wave: Option<usize>,
        opts: PsfOpts,
    },
    Rms {
        wave: Option<usize>,
        opts: RmsOpts,
    },
    GeoMtf {
        wave: Option<usize>,
        freqs: Option<Vec<f64>>,
        n: usize,
        diff_limit: bool,
    },
    HuygensPsf {
        wave: Option<usize>,
        opts: HuygensOpts,
    },
    HuygensMtf {
        wave: Option<usize>,
        freqs: Option<Vec<f64>>,
        opts: HuygensOpts,
    },
    Pop {
        wave: Option<usize>,
        surface: usize,
        dist: Option<f64>,
        to: Option<usize>,
        method: PopMethod,
        opts: RayBeamOpts,
        out: OutGrid,
    },
    Ghost {
        first: usize,
        last: Option<usize>,
    },
    Interferogram {
        wave: Option<usize>,
        beam1: usize,
        beam2: usize,
        opts: PupilOpts,
    },
}

fn s2(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

impl Plugin for Analysis {
    fn name(&self) -> &str {
        match self {
            Analysis::Spot => "spot",
            Analysis::Wavefront => "wavefront",
            Analysis::Zernike => "zernike",
            Analysis::Mtf => "mtf",
            Analysis::Psf => "psf",
            Analysis::Rms => "rms",
            Analysis::GeoMtf => "geo_mtf",
            Analysis::HuygensPsf => "huygens_psf",
            Analysis::HuygensMtf => "huygens_mtf",
            Analysis::Pop => "pop",
            Analysis::Ghost => "ghost",
            Analysis::Interferogram => "interferogram",
        }
    }

    fn kind(&self) -> Kind {
        Kind::Analysis
    }

    fn spec(&self) -> Spec {
        let wave = s2("wave", "波長の番号（1 始まり）");
        let (doc, config, outputs) = match self {
            Analysis::Spot => (
                "スポット（RMS・GEO 半径）",
                vec![wave, s2("rings", "ガウス求積のリング数（6）")],
                vec![
                    s2("x", "光線の位置 [mm]"),
                    s2("y", "光線の位置 [mm]"),
                    s2("weight", "光線の重み"),
                    s2("wave", "光線の波長の番号（1 始まり）"),
                    s2("centroid", "(2,) [mm]"),
                    s2("chief", "(2,) [mm]"),
                    s2("rms_centroid", "[mm]"),
                    s2("rms_chief", "[mm]"),
                    s2("geo", "[mm]"),
                ],
            ),
            Analysis::Wavefront => (
                "波面収差（主光線基準の OPD [波]）",
                vec![
                    wave,
                    s2("n", "格子の一辺（64）"),
                    s2("ref_primary", "1 なら主波長の主光線を基準（0: その波長の主光線。CLI・OpticStudio と同じ）"),
                ],
                vec![
                    s2("p", "格子の瞳座標 (n,)"),
                    s2("w", "OPD (n, n) [波]、外は NaN"),
                    s2("pv", "[波]"),
                    s2("rms", "[波]"),
                ],
            ),
            Analysis::Zernike => (
                "Zernike 係数 [波]",
                vec![
                    wave,
                    s2("kind", "standard か fringe"),
                    s2("terms", "項数（37）"),
                    s2("n", "格子の一辺（64）"),
                    s2("ref_primary", "1 なら主波長の主光線を基準（0: その波長の主光線。CLI・OpticStudio と同じ）"),
                ],
                vec![
                    s2("coef", "係数 [波]"),
                    s2("pv", "[波]"),
                    s2("pv_centroid", "[波]"),
                    s2("rms_fit", "[波]"),
                    s2("max_fit", "[波]"),
                    s2("rms_chief", "[波]"),
                    s2("rms_centroid", "[波]"),
                    s2("variance", "[波²]"),
                    s2("strehl", "ストレール比"),
                ],
            ),
            Analysis::Mtf => (
                "回折 MTF",
                vec![
                    wave,
                    s2(
                        "freqs",
                        "周波数 [cycles/mm]（省けば遮断周波数の 0〜0.95 を 21 点）",
                    ),
                    s2("n", "瞳の格子の一辺（省けば自動）"),
                    s2("method", "auto・interp・direct・fft"),
                ],
                vec![
                    s2("freqs", "[cycles/mm]"),
                    s2("tan", "子午"),
                    s2("sag", "球欠"),
                    s2("cutoff", "(2,) [cycles/mm]"),
                ],
            ),
            Analysis::Psf => (
                "FFT PSF",
                vec![
                    wave,
                    s2("pupil", "瞳の格子の一辺（省けば自動: 隣り合う標本の位相差が π/2 以下）"),
                    s2("image", "像の格子の一辺（128）"),
                    s2("delta", "像の格子の間隔 [mm]（省けば自動）"),
                ],
                vec![
                    s2("x", "(image,) [mm]"),
                    s2("y", "(image,) [mm]"),
                    s2("i", "(image, image)、収差なしの中心を 1"),
                    s2("center", "(2,) [mm]"),
                    s2("delta", "(2,) [mm]"),
                    s2("peak", "格子の最大値"),
                    s2("pupil", "使った瞳の格子の一辺"),
                    s2("phase_step", "瞳の隣り合う標本の位相差の最大 [rad]（π/2 を超えたら標本不足）"),
                    s2("enclosed", "像の格子に入った光量の割合"),
                ],
            ),
            Analysis::Rms => (
                "RMS スポット半径・波面収差・ストレール比",
                vec![
                    wave,
                    s2("rings", "ガウス求積のリング数（6）"),
                    s2("reference", "centroid か chief"),
                ],
                vec![
                    s2("spot", "[mm]"),
                    s2("spot_x", "[mm]"),
                    s2("spot_y", "[mm]"),
                    s2("wfe", "[波]"),
                    s2("strehl", "近似"),
                ],
            ),
            Analysis::GeoMtf => (
                "幾何 MTF",
                vec![
                    wave,
                    s2(
                        "freqs",
                        "周波数 [cycles/mm]（省けば遮断周波数の 0〜0.95 を 21 点）",
                    ),
                    s2("n", "求積の点の数（128）"),
                    s2("diff_limit", "1 なら回折限界を掛ける（0）"),
                ],
                vec![
                    s2("freqs", "[cycles/mm]"),
                    s2("tan", "子午"),
                    s2("sag", "球欠"),
                    s2("cutoff", "(2,) [cycles/mm]"),
                ],
            ),
            Analysis::HuygensPsf => (
                "Huygens PSF（像面の上、参照球の各点からの 2 次波の和）",
                vec![
                    wave,
                    s2("pupil", "瞳の格子の一辺（省けば自動: 隣り合う標本の位相差が π/2 以下）"),
                    s2("image", "像の格子の一辺（128）"),
                    s2("delta", "像の格子の間隔 [mm]（省けば自動）"),
                ],
                vec![
                    s2("x", "(image,) [mm]、像面の局所 x の center からの差"),
                    s2("y", "(image,) [mm]、像面の局所 y の center からの差"),
                    s2("i", "(image, image)、収差なしの中心を 1"),
                    s2("center", "(2,) [mm]"),
                    s2("delta", "(2,) [mm]"),
                    s2("peak", "格子の最大値"),
                    s2("pupil", "使った瞳の格子の一辺"),
                    s2("phase_step", "瞳の隣り合う標本の位相差の最大 [rad]（π/2 を超えたら標本不足）"),
                    s2("enclosed", "像の格子に入った光量の割合"),
                ],
            ),
            Analysis::HuygensMtf => (
                "Huygens MTF（Huygens PSF のフーリエ変換）",
                vec![
                    wave,
                    s2(
                        "freqs",
                        "周波数 [cycles/mm]（省けば遮断周波数の 0〜0.95 を 21 点）",
                    ),
                    s2("pupil", "瞳の格子の一辺（省けば自動: 隣り合う標本の位相差が π/2 以下）"),
                    s2("image", "像の格子の一辺（128）"),
                    s2("delta", "像の格子の間隔 [mm]（省けば自動）"),
                ],
                vec![
                    s2("freqs", "[cycles/mm]"),
                    s2("tan", "子午（像面の y）"),
                    s2("sag", "球欠（像面の x）"),
                    s2("cutoff", "(2,) [cycles/mm]"),
                ],
            ),
            Analysis::Pop => (
                "簡易 POP（面の後の場を作り、距離か面まで伝搬）",
                vec![
                    wave,
                    s2("surface", "場を作る面（必須）"),
                    s2("dist", "伝搬の距離 [mm]（to と一方）"),
                    s2("to", "伝搬の先の面（省けば surface + 1）"),
                    s2("method", "auto・fresnel・angular・direct"),
                    s2("rays", "瞳の格子の一辺の光線の数（65）"),
                    s2("n", "入力の格子の一辺（256）"),
                    s2("half", "入力の格子の半幅 [mm]"),
                    s2("aperture", "1 なら面の開口で切る（1）"),
                    s2("out_n", "出力の格子の一辺"),
                    s2("out_half", "出力の格子の半幅 [mm]"),
                    s2("out_center", "出力の格子の中心 (2,) [mm]"),
                ],
                vec![
                    s2("u", "出力の場 complex (ny, nx)（パイロットを除く）"),
                    s2("dx", "[mm]"),
                    s2("dy", "[mm]"),
                    s2("center", "(2,) [mm]"),
                    s2("pilot", "(ax, ay, curv)"),
                    s2("power", "出力の光量"),
                    s2("peak", "出力の最大強度"),
                    s2("centroid", "(2,) [mm]"),
                    s2("rms_width", "[mm]"),
                    s2("distance", "伝搬の距離 [mm]"),
                    s2("wavelength", "[mm]"),
                    s2("index", "媒質の屈折率"),
                    s2("kernel_step", "核の位相の標本の間の差 [rad]"),
                    s2("fresnel_error", "Fresnel 近似で落とす位相 [rad]"),
                    s2("input_u", "入力の場 complex (ny, nx)"),
                    s2("input_dx", "[mm]"),
                    s2("input_dy", "[mm]"),
                    s2("input_center", "(2,) [mm]"),
                ],
            ),
            Analysis::Ghost => (
                "2 回反射のゴーストの一覧（近軸の量と強度の比）",
                vec![
                    s2("first", "最初の面（1）"),
                    s2("last", "最後の面（省けば像面）"),
                ],
                vec![
                    s2("a", "1 回目の反射の面"),
                    s2("b", "2 回目の反射の面"),
                    s2("marginal_y", "[mm]"),
                    s2("marginal_u", ""),
                    s2("focus", "[mm]（無限遠は inf）"),
                    s2("wfno", ""),
                    s2("intensity", "強度の比（求まらなければ NaN）"),
                ],
            ),
            Analysis::Interferogram => (
                "単色の干渉縞（瞳上、OpticStudio の Interferogram に当たる）: Δ = scale·(W1 − W2) + (xtilt·px + ytilt·py)/2",
                vec![
                    wave,
                    s2("n", "格子の一辺（64）"),
                    s2("beam1", "光束 1 の構成（1）"),
                    s2("beam2", "光束 2 の構成（2、0 で参照球）"),
                    s2("scale", "OPD の差にかける数（1、ダブルパスは 2）"),
                    s2("xtilt", "傾きの縞の数（瞳の直径あたり、0）"),
                    s2("ytilt", "傾きの縞の数（瞳の直径あたり、0）"),
                ],
                vec![
                    s2("p", "格子の瞳座標 (n,)"),
                    s2("delta", "Δ (n, n) [波]、外は NaN"),
                    s2("intensity", "(1 + cos 2πΔ)/2 (n, n)、外は NaN"),
                    s2("pv", "scale·(W1 − W2) の P-V [波]（傾きを除く）"),
                    s2("rms", "scale·(W1 − W2) の RMS [波]（傾きを除く）"),
                ],
            ),
        };
        let inputs = if *self == Analysis::Ghost {
            Vec::new()
        } else {
            vec![s2("field", "視野の番号（1 始まり）か正規化視野 (2,)")]
        };
        Spec {
            doc: doc.into(),
            config,
            inputs,
            outputs,
            needs_system: true,
        }
    }

    fn prepare(&self, config: &Config) -> Result<Box<dyn Instance>> {
        let mut r = ConfigReader::new(config);
        let wave = r.wave("wave")?;
        let p = match self {
            Analysis::Spot => Prepared::Spot {
                wave,
                rings: r.count("rings", 6)?,
            },
            Analysis::Wavefront => Prepared::Wavefront {
                wave,
                n: r.count("n", 64)?,
                ref_primary: r.flag("ref_primary", false)?,
            },
            Analysis::Zernike => Prepared::Zernike {
                wave,
                kind: zern_kind(&r.str("kind", "standard")?)?,
                terms: r.count("terms", 37)?,
                n: r.count("n", 64)?,
                ref_primary: r.flag("ref_primary", false)?,
            },
            Analysis::Mtf => Prepared::Mtf {
                wave,
                freqs: r.nums("freqs")?,
                n: r.opt_count("n")?,
                method: match r.str("method", "auto")?.as_str() {
                    "auto" => MtfMethod::Auto,
                    "interp" => MtfMethod::Interp,
                    "direct" => MtfMethod::Direct,
                    "fft" => MtfMethod::Fft,
                    x => invalid!("設定 method は auto・interp・direct・fft に（{x}）"),
                },
            },
            Analysis::Psf => {
                let d = PsfOpts::default();
                let delta = r.num("delta", f64::NAN)?;
                Prepared::Psf {
                    wave,
                    opts: PsfOpts {
                        pupil: r.opt_count("pupil")?,
                        image: r.count("image", d.image)?,
                        delta: (!delta.is_nan()).then_some(delta),
                    },
                }
            }
            Analysis::Rms => Prepared::Rms {
                wave,
                opts: RmsOpts {
                    rings: r.count("rings", RmsOpts::default().rings)?,
                    reference: match r
                        .str("reference", "centroid")?
                        .to_ascii_lowercase()
                        .as_str()
                    {
                        "centroid" => RmsRef::Centroid,
                        "chief" => RmsRef::Chief,
                        x => invalid!("設定 reference は centroid か chief に（{x}）"),
                    },
                    ..Default::default()
                },
            },
            Analysis::GeoMtf => Prepared::GeoMtf {
                wave,
                freqs: r.nums("freqs")?,
                n: r.count("n", 128)?,
                diff_limit: r.flag("diff_limit", false)?,
            },
            Analysis::HuygensPsf | Analysis::HuygensMtf => {
                let d = HuygensOpts::default();
                let delta = r.num("delta", f64::NAN)?;
                let opts = HuygensOpts {
                    pupil: r.opt_count("pupil")?,
                    image: r.count("image", d.image)?,
                    delta: (!delta.is_nan()).then_some(delta),
                };
                if *self == Analysis::HuygensPsf {
                    Prepared::HuygensPsf { wave, opts }
                } else {
                    Prepared::HuygensMtf {
                        wave,
                        freqs: r.nums("freqs")?,
                        opts,
                    }
                }
            }
            Analysis::Pop => {
                let Some(surface) = r.opt_count("surface")? else {
                    invalid!("設定 surface（場を作る面）が要る");
                };
                let dist = opt_num(&mut r, "dist")?;
                let to = r.opt_count("to")?;
                if dist.is_some() && to.is_some() {
                    invalid!("設定 dist と to はどちらか");
                }
                let method = match r.str("method", "auto")?.as_str() {
                    "auto" => PopMethod::Auto,
                    "fresnel" => PopMethod::Fresnel,
                    "angular" => PopMethod::Angular,
                    "direct" => PopMethod::Direct,
                    x => invalid!("設定 method は auto・fresnel・angular・direct に（{x}）"),
                };
                let d = RayBeamOpts::default();
                let opts = RayBeamOpts {
                    rays: r.count("rays", d.rays)?,
                    n: r.count("n", d.n)?,
                    half: opt_num(&mut r, "half")?,
                    aperture: r.flag("aperture", true)?,
                    overfill: None,
                };
                let out = OutGrid {
                    n: r.opt_count("out_n")?.map(|v| (v, v)),
                    half: opt_num(&mut r, "out_half")?.map(|v| (v, v)),
                    center: match r.nums("out_center")? {
                        None => None,
                        Some(v) if v.len() == 2 => Some((v[0], v[1])),
                        Some(v) => invalid!("設定 out_center は 2 個に（{} 個）", v.len()),
                    },
                };
                Prepared::Pop {
                    wave,
                    surface,
                    dist,
                    to,
                    method,
                    opts,
                    out,
                }
            }
            Analysis::Ghost => {
                if wave.is_some() {
                    invalid!("ghost に wave は使えない（主波長で求める）");
                }
                Prepared::Ghost {
                    first: r.count("first", 1)?,
                    last: r.opt_count("last")?,
                }
            }
            Analysis::Interferogram => {
                let beam1 = r.count("beam1", 1)?;
                if beam1 == 0 {
                    invalid!("設定 beam1 は 1 以上（構成の番号）");
                }
                Prepared::Interferogram {
                    wave,
                    beam1,
                    beam2: r.count("beam2", 2)?,
                    opts: PupilOpts {
                        n: r.count("n", 64)?,
                        scale: r.num("scale", 1.0)?,
                        xtilt: r.num("xtilt", 0.0)?,
                        ytilt: r.num("ytilt", 0.0)?,
                    },
                }
            }
        };
        r.finish()?;
        Ok(Box::new(p))
    }
}

fn opt_num(r: &mut ConfigReader<'_>, key: &str) -> Result<Option<f64>> {
    let x = r.num(key, f64::NAN)?;
    Ok((!x.is_nan()).then_some(x))
}

fn zern_kind(s: &str) -> Result<ZernKind> {
    match s.to_ascii_lowercase().as_str() {
        "standard" => Ok(ZernKind::Standard),
        "fringe" => Ok(ZernKind::Fringe),
        x => invalid!("設定 kind は standard か fringe に（{x}）"),
    }
}

fn v1(v: Vec<f64>) -> Result<Array> {
    Array::f64(vec![v.len()], v)
}

fn rows(v: &[Vec<f64>]) -> Result<Array> {
    let n = v.first().map_or(0, Vec::len);
    Array::f64(vec![v.len(), n], v.concat())
}

fn out(items: Vec<(&str, Array)>) -> Arrays {
    items.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

impl Prepared {
    fn wave(&self) -> Option<usize> {
        match self {
            Prepared::Spot { wave, .. }
            | Prepared::Wavefront { wave, .. }
            | Prepared::Zernike { wave, .. }
            | Prepared::Mtf { wave, .. }
            | Prepared::Psf { wave, .. }
            | Prepared::Rms { wave, .. }
            | Prepared::GeoMtf { wave, .. }
            | Prepared::HuygensPsf { wave, .. }
            | Prepared::HuygensMtf { wave, .. }
            | Prepared::Pop { wave, .. }
            | Prepared::Interferogram { wave, .. } => *wave,
            Prepared::Ghost { .. } => None,
        }
    }

    /// 系と視野で解析する
    pub fn evaluate(&self, s: &System, f: FieldSel) -> Result<Arrays> {
        check_wave(s, self.wave())?;
        Ok(match self {
            Prepared::Spot { wave, rings } => {
                let r = analysis::spot(s, f, Waves::all_or(*wave), *rings)?;
                out(vec![
                    ("x", v1(r.x)?),
                    ("y", v1(r.y)?),
                    ("weight", v1(r.weight)?),
                    (
                        "wave",
                        v1(r.wave.iter().map(|&w| (w + 1) as f64).collect())?,
                    ),
                    ("centroid", v1(vec![r.centroid.0, r.centroid.1])?),
                    ("chief", v1(vec![r.chief.0, r.chief.1])?),
                    ("rms_centroid", Array::scalar(r.rms_centroid)),
                    ("rms_chief", Array::scalar(r.rms_chief)),
                    ("geo", Array::scalar(r.geo)),
                ])
            }
            Prepared::Wavefront {
                wave,
                n,
                ref_primary,
            } => {
                let r = analysis::wavefront(s, f, *wave, *n, *ref_primary)?;
                out(vec![
                    ("p", v1(r.p)?),
                    ("w", rows(&r.w)?),
                    ("pv", Array::scalar(r.pv)),
                    ("rms", Array::scalar(r.rms)),
                ])
            }
            Prepared::Zernike {
                wave,
                kind,
                terms,
                n,
                ref_primary,
            } => {
                let r = analysis::zernike(s, f, *wave, *kind, *terms, *n, *ref_primary)?;
                out(vec![
                    ("coef", v1(r.coef)?),
                    ("pv", Array::scalar(r.pv)),
                    ("pv_centroid", Array::scalar(r.pv_centroid)),
                    ("rms_fit", Array::scalar(r.rms_fit)),
                    ("max_fit", Array::scalar(r.max_fit)),
                    ("rms_chief", Array::scalar(r.rms_chief)),
                    ("rms_centroid", Array::scalar(r.rms_centroid)),
                    ("variance", Array::scalar(r.variance)),
                    ("strehl", Array::scalar(r.strehl)),
                ])
            }
            Prepared::Mtf {
                wave,
                freqs,
                n,
                method,
            } => {
                let fs = default_freqs(s, f, *wave, freqs)?;
                let r = analysis::mtf_with(s, f, Waves::all_or(*wave), *n, &fs, *method)?;
                out(vec![
                    ("freqs", v1(r.freqs)?),
                    ("tan", v1(r.tan)?),
                    ("sag", v1(r.sag)?),
                    ("cutoff", v1(vec![r.cutoff.0, r.cutoff.1])?),
                ])
            }
            Prepared::Psf { wave, opts } => {
                let r = psf::psf(s, f, Waves::all_or(*wave), *opts)?;
                out(vec![
                    ("x", v1(r.x)?),
                    ("y", v1(r.y)?),
                    ("i", rows(&r.i)?),
                    ("center", v1(vec![r.center.0, r.center.1])?),
                    ("delta", v1(vec![r.delta.0, r.delta.1])?),
                    ("peak", Array::scalar(r.peak)),
                    ("pupil", Array::scalar(r.pupil as f64)),
                    ("phase_step", Array::scalar(r.phase_step)),
                    ("enclosed", Array::scalar(r.enclosed)),
                ])
            }
            Prepared::Rms { wave, opts } => {
                let r = sweep::rms(s, f, Waves::all_or(*wave), *opts)?;
                out(vec![
                    ("spot", Array::scalar(r.spot)),
                    ("spot_x", Array::scalar(r.spot_x)),
                    ("spot_y", Array::scalar(r.spot_y)),
                    ("wfe", Array::scalar(r.wfe)),
                    ("strehl", Array::scalar(r.strehl)),
                ])
            }
            Prepared::GeoMtf {
                wave,
                freqs,
                n,
                diff_limit,
            } => {
                let fs = default_freqs(s, f, *wave, freqs)?;
                let r = psf::geometric_mtf(s, f, Waves::all_or(*wave), *n, &fs, *diff_limit)?;
                out(vec![
                    ("freqs", v1(r.freqs)?),
                    ("tan", v1(r.tan)?),
                    ("sag", v1(r.sag)?),
                    ("cutoff", v1(vec![r.cutoff.0, r.cutoff.1])?),
                ])
            }
            Prepared::HuygensPsf { wave, opts } => {
                let r = huygens::huygens_psf(s, f, Waves::all_or(*wave), *opts)?;
                out(vec![
                    ("x", v1(r.x)?),
                    ("y", v1(r.y)?),
                    ("i", rows(&r.i)?),
                    ("center", v1(vec![r.center.0, r.center.1])?),
                    ("delta", v1(vec![r.delta.0, r.delta.1])?),
                    ("peak", Array::scalar(r.peak)),
                    ("pupil", Array::scalar(r.pupil as f64)),
                    ("phase_step", Array::scalar(r.phase_step)),
                    ("enclosed", Array::scalar(r.enclosed)),
                ])
            }
            Prepared::HuygensMtf { wave, freqs, opts } => {
                let fs = default_freqs(s, f, *wave, freqs)?;
                let r = huygens::huygens_mtf(s, f, Waves::all_or(*wave), *opts, &fs)?;
                out(vec![
                    ("freqs", v1(r.freqs)?),
                    ("tan", v1(r.tan)?),
                    ("sag", v1(r.sag)?),
                    ("cutoff", v1(vec![r.cutoff.0, r.cutoff.1])?),
                ])
            }
            Prepared::Pop {
                wave,
                surface,
                dist,
                to,
                method,
                opts,
                out: grid,
            } => {
                let rb = pop::beam_from_rays(s, f, *wave, *surface, opts)?;
                let b = &rb.beam;
                let d = match dist {
                    Some(d) => *d,
                    None => pop::distance_to(s, *surface, to.unwrap_or(surface + 1), b.dir)?,
                };
                let p = pop::propagate(b, d, *method, *grid)?;
                let o = &p.beam;
                let (cx, cy) = o.centroid();
                out(vec![
                    ("u", field(o)?),
                    ("dx", Array::scalar(o.dx)),
                    ("dy", Array::scalar(o.dy)),
                    ("center", v1(vec![o.x0, o.y0])?),
                    ("pilot", v1(vec![o.pilot.ax, o.pilot.ay, o.pilot.curv])?),
                    ("power", Array::scalar(o.power())),
                    ("peak", Array::scalar(o.peak())),
                    ("centroid", v1(vec![cx, cy])?),
                    ("rms_width", Array::scalar(o.rms_width())),
                    ("distance", Array::scalar(d)),
                    ("wavelength", Array::scalar(b.wl)),
                    ("index", Array::scalar(b.index)),
                    ("kernel_step", Array::scalar(p.kernel_step)),
                    ("fresnel_error", Array::scalar(p.fresnel_error)),
                    ("input_u", field(b)?),
                    ("input_dx", Array::scalar(b.dx)),
                    ("input_dy", Array::scalar(b.dy)),
                    ("input_center", v1(vec![b.x0, b.y0])?),
                ])
            }
            Prepared::Interferogram {
                wave,
                beam1,
                beam2,
                opts,
            } => {
                let s1 = s.config(*beam1)?;
                let s2 = if *beam2 == 0 {
                    None
                } else {
                    Some(s.config(*beam2)?)
                };
                let r = fringe::interferogram(&s1, s2.as_ref(), f, *wave, opts)?;
                out(vec![
                    ("p", v1(r.p)?),
                    ("delta", rows(&r.delta)?),
                    ("intensity", rows(&r.intensity)?),
                    ("pv", Array::scalar(r.pv)),
                    ("rms", Array::scalar(r.rms)),
                ])
            }
            Prepared::Ghost { .. } => invalid!("ghost は視野を使わない"),
        })
    }

    /// ghost: first..=last の面の全部の組
    fn ghost(&self, s: &System, coat: Option<&Coatings>) -> Result<Arrays> {
        let Prepared::Ghost { first, last } = self else {
            invalid!("ghost ではない");
        };
        let last = last.unwrap_or(s.image());
        let own;
        let co = match coat {
            Some(c) => c,
            None => {
                own = Coatings::for_system(s)?;
                &own
            }
        };
        let v = ghost::survey(s, *first, last, co)?;
        let col = |f: &dyn Fn(&ghost::GhostInfo) -> f64| v1(v.iter().map(f).collect());
        Ok(out(vec![
            ("a", col(&|g| g.par.a as f64)?),
            ("b", col(&|g| g.par.b as f64)?),
            ("marginal_y", col(&|g| g.par.marginal_y)?),
            ("marginal_u", col(&|g| g.par.marginal_u)?),
            ("focus", col(&|g| g.par.focus)?),
            ("wfno", col(&|g| g.par.wfno)?),
            (
                "intensity",
                col(&|g| g.intensity.as_ref().copied().unwrap_or(f64::NAN))?,
            ),
        ]))
    }
}

/// 周波数（省けば CLI の mtf と同じ既定: 遮断周波数の 0〜0.95 を 21 点）
fn default_freqs(
    s: &System,
    f: FieldSel,
    wave: Option<usize>,
    freqs: &Option<Vec<f64>>,
) -> Result<Vec<f64>> {
    Ok(match freqs {
        Some(v) => v.clone(),
        None => {
            let m = analysis::mtf(s, f, Waves::all_or(wave), Some(4), &[])?;
            let c = m.cutoff.0.min(m.cutoff.1);
            (0..=20).map(|k| 0.95 * c * k as f64 / 20.0).collect()
        }
    })
}

/// 場 (ny, nx) の複素数の配列
fn field(b: &pop::Beam) -> Result<Array> {
    Array::c64(vec![b.ny, b.nx], b.u.clone())
}

impl Instance for Prepared {
    fn run(&self, ctx: &Context<'_>, inputs: &Arrays) -> Result<Arrays> {
        let s = ctx.system()?;
        let mut r = ArrayReader::new(inputs);
        if matches!(self, Prepared::Ghost { .. }) {
            r.finish()?;
            return self.ghost(s, ctx.coatings);
        }
        let f = field_sel(s, r.req("field")?)?;
        r.finish()?;
        self.evaluate(s, f)
    }
}
