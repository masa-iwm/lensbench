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

use crate::huygens::{self, HuygensOpts};
use lensbench_core::analysis::{self, MtfMethod, ZernKind};
use lensbench_core::coating::Coatings;
use lensbench_core::fringe::{self, PupilOpts};
use lensbench_core::plugin::{
    check_wave, field_sel, Array, ArrayReader, Arrays, Config, ConfigReader, Context, Instance,
    Kind, OutputSpec, ParamSpec, ParamType, Plugin, Registry, Spec, Value,
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
        let (doc, outputs) = match self {
            Analysis::Spot => (
                "スポット（RMS・GEO 半径）",
                vec![
                    OutputSpec::from_doc("x", "光線の位置 [mm]"),
                    OutputSpec::from_doc("y", "光線の位置 [mm]"),
                    OutputSpec::from_doc("weight", "光線の重み"),
                    OutputSpec::from_doc("wave", "光線の波長の番号（1 始まり）"),
                    OutputSpec::from_doc("centroid", "(2,) [mm]"),
                    OutputSpec::from_doc("chief", "(2,) [mm]"),
                    OutputSpec::from_doc("rms_centroid", "[mm]"),
                    OutputSpec::from_doc("rms_chief", "[mm]"),
                    OutputSpec::from_doc("geo", "[mm]"),
                ],
            ),
            Analysis::Wavefront => (
                "波面収差（主光線基準の OPD [波]）",
                vec![
                    OutputSpec::from_doc("p", "格子の瞳座標 (n,)"),
                    OutputSpec::from_doc("w", "OPD (n, n) [波]、外は NaN"),
                    OutputSpec::from_doc("pv", "[波]"),
                    OutputSpec::from_doc("rms", "[波]"),
                ],
            ),
            Analysis::Zernike => (
                "Zernike 係数 [波]",
                vec![
                    OutputSpec::from_doc("coef", "係数 [波]"),
                    OutputSpec::from_doc("pv", "[波]"),
                    OutputSpec::from_doc("pv_centroid", "[波]"),
                    OutputSpec::from_doc("rms_fit", "[波]"),
                    OutputSpec::from_doc("max_fit", "[波]"),
                    OutputSpec::from_doc("rms_chief", "[波]"),
                    OutputSpec::from_doc("rms_centroid", "[波]"),
                    OutputSpec::from_doc("variance", "[波²]"),
                    OutputSpec::from_doc("strehl", "ストレール比"),
                ],
            ),
            Analysis::Mtf => (
                "回折 MTF",
                vec![
                    OutputSpec::from_doc("freqs", "[cycles/mm]"),
                    OutputSpec::from_doc("tan", "子午"),
                    OutputSpec::from_doc("sag", "球欠"),
                    OutputSpec::from_doc("cutoff", "(2,) 子午・球欠 [cycles/mm]"),
                ],
            ),
            Analysis::Psf => (
                "FFT PSF",
                vec![
                    OutputSpec::from_doc("x", "(image,) [mm]"),
                    OutputSpec::from_doc("y", "(image,) [mm]"),
                    OutputSpec::from_doc("i", "(image, image)、収差なしの中心を 1"),
                    OutputSpec::from_doc("center", "(2,) [mm]"),
                    OutputSpec::from_doc("delta", "(2,) [mm]"),
                    OutputSpec::from_doc("peak", "格子の最大値"),
                    OutputSpec::from_doc("pupil", "使った瞳の格子の一辺"),
                    OutputSpec::from_doc("phase_step", "瞳の隣り合う標本の位相差の最大 [rad]（π/2 を超えたら標本不足）"),
                    OutputSpec::from_doc("enclosed", "像の格子に入った光量の割合"),
                ],
            ),
            Analysis::Rms => (
                "RMS スポット半径・波面収差・ストレール比",
                vec![
                    OutputSpec::from_doc("spot", "[mm]"),
                    OutputSpec::from_doc("spot_x", "[mm]"),
                    OutputSpec::from_doc("spot_y", "[mm]"),
                    OutputSpec::from_doc("wfe", "[波]"),
                    OutputSpec::from_doc("strehl", "近似"),
                ],
            ),
            Analysis::GeoMtf => (
                "幾何 MTF",
                vec![
                    OutputSpec::from_doc("freqs", "[cycles/mm]"),
                    OutputSpec::from_doc("tan", "子午"),
                    OutputSpec::from_doc("sag", "球欠"),
                    OutputSpec::from_doc("cutoff", "(2,) 子午・球欠 [cycles/mm]"),
                ],
            ),
            Analysis::HuygensPsf => (
                "Huygens PSF（像面の上、参照球の各点からの 2 次波の和）",
                vec![
                    OutputSpec::from_doc("x", "(image,) [mm]、像面の局所 x の center からの差"),
                    OutputSpec::from_doc("y", "(image,) [mm]、像面の局所 y の center からの差"),
                    OutputSpec::from_doc("i", "(image, image)、収差なしの中心を 1"),
                    OutputSpec::from_doc("center", "(2,) [mm]"),
                    OutputSpec::from_doc("delta", "(2,) [mm]"),
                    OutputSpec::from_doc("peak", "格子の最大値"),
                    OutputSpec::from_doc("pupil", "使った瞳の格子の一辺"),
                    OutputSpec::from_doc("phase_step", "瞳の隣り合う標本の位相差の最大 [rad]（π/2 を超えたら標本不足）"),
                    OutputSpec::from_doc("enclosed", "像の格子に入った光量の割合"),
                ],
            ),
            Analysis::HuygensMtf => (
                "Huygens MTF（Huygens PSF のフーリエ変換）",
                vec![
                    OutputSpec::from_doc("freqs", "[cycles/mm]"),
                    OutputSpec::from_doc("tan", "子午（像面の y）"),
                    OutputSpec::from_doc("sag", "球欠（像面の x）"),
                    OutputSpec::from_doc("cutoff", "(2,) 子午・球欠 [cycles/mm]"),
                ],
            ),
            Analysis::Pop => (
                "簡易 POP（面の後の場を作り、距離か面まで伝搬）",
                vec![
                    OutputSpec::from_doc("u", "出力の場 complex (ny, nx)（パイロットを除く）"),
                    OutputSpec::from_doc("dx", "[mm]"),
                    OutputSpec::from_doc("dy", "[mm]"),
                    OutputSpec::from_doc("center", "(2,) [mm]"),
                    OutputSpec::from_doc("pilot", "(ax, ay, curv)"),
                    OutputSpec::from_doc("power", "出力の光量"),
                    OutputSpec::from_doc("peak", "出力の最大強度"),
                    OutputSpec::from_doc("centroid", "(2,) [mm]"),
                    OutputSpec::from_doc("rms_width", "[mm]"),
                    OutputSpec::from_doc("distance", "伝搬の距離 [mm]"),
                    OutputSpec::from_doc("wavelength", "[mm]"),
                    OutputSpec::from_doc("index", "媒質の屈折率"),
                    OutputSpec::from_doc("kernel_step", "核の位相の標本の間の差 [rad]"),
                    OutputSpec::from_doc("fresnel_error", "Fresnel 近似で落とす位相 [rad]"),
                    OutputSpec::from_doc("input_u", "入力の場 complex (ny, nx)"),
                    OutputSpec::from_doc("input_dx", "[mm]"),
                    OutputSpec::from_doc("input_dy", "[mm]"),
                    OutputSpec::from_doc("input_center", "(2,) [mm]"),
                ],
            ),
            Analysis::Ghost => (
                "2 回反射のゴーストの一覧（近軸の量と強度の比）",
                vec![
                    OutputSpec::from_doc("a", "1 回目の反射の面"),
                    OutputSpec::from_doc("b", "2 回目の反射の面"),
                    OutputSpec::from_doc("marginal_y", "[mm]"),
                    OutputSpec::from_doc("marginal_u", ""),
                    OutputSpec::from_doc("focus", "[mm]（無限遠は inf）"),
                    OutputSpec::from_doc("wfno", ""),
                    OutputSpec::from_doc("intensity", "強度の比（求まらなければ NaN）"),
                ],
            ),
            Analysis::Interferogram => (
                "単色の干渉縞（瞳上、OpticStudio の Interferogram に当たる）: Δ = scale·(W1 − W2) + (xtilt·px + ytilt·py)/2",
                vec![
                    OutputSpec::from_doc("p", "格子の瞳座標 (n,)"),
                    OutputSpec::from_doc("delta", "Δ (n, n) [波]、外は NaN"),
                    OutputSpec::from_doc("intensity", "(1 + cos 2πΔ)/2 (n, n)、外は NaN"),
                    OutputSpec::from_doc("pv", "scale·(W1 − W2) の P-V [波]（傾きを除く）"),
                    OutputSpec::from_doc("rms", "scale·(W1 − W2) の RMS [波]（傾きを除く）"),
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
            inputs,
            outputs,
            needs_system: true,
        }
    }

    fn params(&self) -> Vec<ParamSpec> {
        use ParamType::{Choice, Count, Flag, Num, Nums};
        let p = ParamSpec::new;
        let n = |x: f64| Some(Value::Num(x));
        let b = |x: bool| Some(Value::Bool(x));
        let st = |x: &str| Some(Value::Str(x.into()));
        let ch = |v: &[&str]| Choice(v.iter().map(|x| x.to_string()).collect());
        let wave = || {
            p(
                "wave",
                Count,
                None,
                "",
                "波長の番号（1 始まり、省けば多色・主波長）",
            )
        };
        let freqs = || {
            p(
                "freqs",
                Nums { len: None },
                None,
                "cycles/mm",
                "周波数（省けば遮断周波数の 0〜0.95 を 21 点）",
            )
        };
        let refp = || {
            p(
                "ref_primary",
                Flag,
                b(false),
                "",
                "主波長の主光線を基準（false: その波長の主光線。CLI・OpticStudio と同じ）",
            )
        };
        let pupil = || {
            p(
                "pupil",
                Count,
                None,
                "",
                "瞳の格子の一辺（省けば自動: 隣り合う標本の位相差が π/2 以下）",
            )
        };
        let image = || p("image", Count, n(128.0), "", "像の格子の一辺");
        let delta = || p("delta", Num, None, "mm", "像の格子の間隔（省けば自動）");
        match self {
            Analysis::Spot => vec![
                wave(),
                p("rings", Count, n(6.0), "", "ガウス求積のリング数"),
            ],
            Analysis::Wavefront => vec![wave(), p("n", Count, n(64.0), "", "格子の一辺"), refp()],
            Analysis::Zernike => vec![
                wave(),
                p(
                    "kind",
                    ch(&["standard", "fringe"]),
                    st("standard"),
                    "",
                    "並び",
                ),
                p("terms", Count, n(37.0), "", "項数"),
                p("n", Count, n(64.0), "", "格子の一辺"),
                refp(),
            ],
            Analysis::Mtf => vec![
                wave(),
                freqs(),
                p("n", Count, None, "", "瞳の格子の一辺（省けば自動）"),
                p(
                    "method",
                    ch(&["auto", "interp", "direct", "fft"]),
                    st("auto"),
                    "",
                    "方式",
                ),
            ],
            Analysis::Psf | Analysis::HuygensPsf => vec![wave(), pupil(), image(), delta()],
            Analysis::Rms => vec![
                wave(),
                p("rings", Count, n(6.0), "", "ガウス求積のリング数"),
                p(
                    "reference",
                    ch(&["centroid", "chief"]),
                    st("centroid"),
                    "",
                    "基準",
                ),
            ],
            Analysis::GeoMtf => vec![
                wave(),
                freqs(),
                p("n", Count, n(128.0), "", "求積の点の数"),
                p("diff_limit", Flag, b(false), "", "回折限界を掛ける"),
            ],
            Analysis::HuygensMtf => vec![wave(), freqs(), pupil(), image(), delta()],
            Analysis::Pop => vec![
                wave(),
                p("surface", Count, None, "", "場を作る面（必須）"),
                p("dist", Num, None, "mm", "伝搬の距離（to とどちらか）"),
                p("to", Count, None, "", "伝搬の先の面（省けば surface + 1）"),
                p(
                    "method",
                    ch(&["auto", "fresnel", "angular", "direct"]),
                    st("auto"),
                    "",
                    "方式",
                ),
                p("rays", Count, n(65.0), "", "瞳の格子の一辺の光線の数"),
                p("n", Count, n(256.0), "", "入力の格子の一辺"),
                p("half", Num, None, "mm", "入力の格子の半幅（省けば自動）"),
                p("aperture", Flag, b(true), "", "面の開口で切る"),
                p("out_n", Count, None, "", "出力の格子の一辺"),
                p("out_half", Num, None, "mm", "出力の格子の半幅"),
                p(
                    "out_center",
                    Nums { len: Some(2) },
                    None,
                    "mm",
                    "出力の格子の中心 (x, y)",
                ),
            ],
            Analysis::Ghost => vec![
                p("first", Count, n(1.0), "", "最初の面"),
                p("last", Count, None, "", "最後の面（省けば像面）"),
            ],
            Analysis::Interferogram => vec![
                wave(),
                p("n", Count, n(64.0), "", "格子の一辺"),
                p("beam1", Count, n(1.0), "", "光束 1 の構成"),
                p("beam2", Count, n(2.0), "", "光束 2 の構成（0 で参照球）"),
                p(
                    "scale",
                    Num,
                    n(1.0),
                    "",
                    "OPD の差にかける数（ダブルパスは 2）",
                ),
                p("xtilt", Num, n(0.0), "", "傾きの縞の数（瞳の直径あたり）"),
                p("ytilt", Num, n(0.0), "", "傾きの縞の数（瞳の直径あたり）"),
            ],
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
                    x => invalid!("setting method must be auto, interp, direct or fft ({x})"),
                },
            },
            Analysis::Psf => {
                let d = PsfOpts::default();
                let delta = r.opt_num("delta")?;
                Prepared::Psf {
                    wave,
                    opts: PsfOpts {
                        pupil: r.opt_count("pupil")?,
                        image: r.count("image", d.image)?,
                        delta,
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
                        x => invalid!("setting reference must be centroid or chief ({x})"),
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
                let delta = r.opt_num("delta")?;
                let opts = HuygensOpts {
                    pupil: r.opt_count("pupil")?,
                    image: r.count("image", d.image)?,
                    delta,
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
                    invalid!("setting surface (the surface that makes the field) is needed");
                };
                let dist = opt_num(&mut r, "dist")?;
                let to = r.opt_count("to")?;
                if dist.is_some() && to.is_some() {
                    invalid!("give only one of the settings dist and to");
                }
                let method = match r.str("method", "auto")?.as_str() {
                    "auto" => PopMethod::Auto,
                    "fresnel" => PopMethod::Fresnel,
                    "angular" => PopMethod::Angular,
                    "direct" => PopMethod::Direct,
                    x => invalid!("setting method must be auto, fresnel, angular or direct ({x})"),
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
                        Some(v) => {
                            invalid!("setting out_center needs 2 values ({} values)", v.len())
                        }
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
                    invalid!("wave can't be used with ghost (computed at the primary wavelength)");
                }
                Prepared::Ghost {
                    first: r.count("first", 1)?,
                    last: r.opt_count("last")?,
                }
            }
            Analysis::Interferogram => {
                let beam1 = r.count("beam1", 1)?;
                if beam1 == 0 {
                    invalid!("setting beam1 must be 1 or more (a configuration number)");
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
    r.opt_num(key)
}

fn zern_kind(s: &str) -> Result<ZernKind> {
    match s.to_ascii_lowercase().as_str() {
        "standard" => Ok(ZernKind::Standard),
        "fringe" => Ok(ZernKind::Fringe),
        x => invalid!("setting kind must be standard or fringe ({x})"),
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
                    ("cutoff", v1(vec![r.cutoff.tan, r.cutoff.sag])?),
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
                    ("cutoff", v1(vec![r.cutoff.tan, r.cutoff.sag])?),
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
                    ("cutoff", v1(vec![r.cutoff.tan, r.cutoff.sag])?),
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
            Prepared::Ghost { .. } => invalid!("ghost doesn't use fields"),
        })
    }

    /// ghost: first..=last の面の全部の組
    fn ghost(&self, s: &System, coat: Option<&Coatings>) -> Result<Arrays> {
        let Prepared::Ghost { first, last } = self else {
            invalid!("not a ghost");
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
            let c = analysis::mtf_cutoff(s, f, wave.unwrap_or(s.primary))?.min();
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
