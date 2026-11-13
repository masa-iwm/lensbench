//! lensbench のコマンドライン。視野・波長の番号は OpticStudio と同じく 1 始まり。
//!
//! この crate はライブラリでもある（GUI が serve と同じ API を同じプロセスで呼ぶため。[`serve::call`]）。
//! 実行ファイルの入口は `src/bin/lensbench.rs` で、[`main`] を呼ぶだけ。

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use lensbench_core::analysis::{self, ZernKind};
use lensbench_core::catalog::LensCatalog;
use lensbench_core::coating::{self, Coatings};
use lensbench_core::{
    elements, glass, huygens, pop, psf, report, sweep, zmx, zmx_write, FieldSel, System,
};
use lensbench_export::{d2, mesh, scene, table};
use serde_json::{json, Value};
use std::f64::consts::PI;

pub mod arg_spec;
mod browse;
#[cfg(not(target_arch = "wasm32"))]
pub mod cad;
pub mod cad_detail;
#[cfg(not(target_arch = "wasm32"))]
pub mod coat_remote;
/// カタログの一覧を覚えておくフォルダ・前もって作った一覧（GUI・Web 版が起動のときに決める）
pub use browse::{
    set_cache_dir as set_lens_list_cache_dir, set_prepared as set_prepared_lens_list,
};
pub mod ext;
pub mod run;
pub mod serve;
mod serve_ipc;
mod watch;

mod fringe_cmd;
mod search_cmd;
use fringe_cmd::FringeArgs;

#[derive(Parser)]
#[command(name = "lensbench", version = lensbench_core::VERSION, about = "順次光線追跡と解析")]
struct Cli {
    /// 追跡・解析のスレッド数（1 で逐次。省くと LENSBENCH_THREADS、なければ全コアで RAYON_NUM_THREADS が効く）。結果はスレッド数によらず同じ
    #[arg(long, global = true)]
    threads: Option<usize>,
    #[command(subcommand)]
    cmd: Cmd,
}

/// 系を読む解析の共通の引数（拡張のサブコマンドも `#[command(flatten)]` で使う、[`ext`]）
#[derive(Args, Clone)]
pub struct Common {
    /// .zmx ファイル、またはブロックで組んだプロジェクトファイル（.json）
    pub file: PathBuf,
    /// ガラスカタログを探すフォルダ（複数可。LENSBENCH_GLASSCAT も使う）
    #[arg(long = "glass-dir")]
    pub glass_dir: Vec<PathBuf>,
    /// プロジェクトファイルのカタログレンズ（"vendor"・"part"）を探すレンズカタログ（.zmx のファイル、または .zmx を
    /// 入れたフォルダ。メーカーはフォルダの名前、型番はファイル名。複数可）
    #[arg(long = "lens-catalog")]
    pub lens_catalog: Vec<PathBuf>,
    /// マルチコンフィグの番号（1 始まり）
    #[arg(long, default_value_t = 1)]
    pub cfg: usize,
    /// プロジェクトファイル（.json）の状態の名前（多重構成に当たる）
    #[arg(long)]
    pub state: Option<String>,
    /// プロジェクトファイルのビームスプリッターの経路（BS を通る順の T・R、例 TR。省略でプロジェクトの path・各 BS の arm）
    #[arg(long)]
    pub path: Option<String>,
    /// JSON で出力する
    #[arg(long)]
    pub json: bool,
    /// 結果を書き出すファイル（拡張子で csv・json・npz）
    #[arg(long)]
    pub save: Option<PathBuf>,
}

/// 視野・波長の選択（1 始まり）
#[derive(Args, Clone)]
pub struct Sel {
    /// 視野の番号（1 始まり、省略で全視野）
    #[arg(long)]
    pub field: Option<usize>,
    /// 波長の番号（1 始まり、省略で多色）
    #[arg(long)]
    pub wave: Option<usize>,
}

/// 拡張のサブコマンドを解釈したもの（[`Cmd::Ext`]）
struct ExtCmd {
    args: Box<dyn ext::Args>,
}

#[derive(Subcommand)]
enum Cmd {
    /// システムデータ（EFL、F 値、瞳など）
    Info(Common),
    /// 系を書き出す: 拡張子 .json なら系ファイル（欠けのない lensbench の形式）、.zmx なら .zmx。
    /// 読むのは .zmx・プロジェクト・系ファイル（--state・--cfg を当てた系）
    Convert {
        #[command(flatten)]
        c: Common,
        /// 書き出すファイル（.json か .zmx）
        #[arg(long, short)]
        out: PathBuf,
    },
    /// スポット（RMS・GEO 半径）
    Spot {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// ガウス求積のリング数
        #[arg(long, default_value_t = 6)]
        rings: usize,
    },
    /// 波面収差（主光線基準の OPD、P-V・RMS [波]）
    Wavefront {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// 格子の一辺
        #[arg(long, default_value_t = 64)]
        n: usize,
        /// 波面の格子も出す（"p": 正規化瞳座標、"map": W[iy][ix]、瞳の外・けられは null。GUI の波面図）
        #[arg(long)]
        map: bool,
    },
    /// Zernike 係数 [波]
    Zernike {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// standard または fringe
        #[arg(long, default_value = "standard")]
        kind: String,
        /// 項数
        #[arg(long, default_value_t = 37)]
        terms: usize,
        /// 格子の一辺
        #[arg(long, default_value_t = 64)]
        n: usize,
    },
    /// 系を .zmx に書き出す（プロジェクトの展開など）。--paths・--states・--cfgs で、構成ごとに 1 構成の多重構成の .zmx に
    /// まとめる（BS の経路ごとなど。面の並びが違う分は IGNR で無視する）
    Zmx {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        sel: ConfigSel,
        /// 書き出す .zmx
        #[arg(long)]
        out: PathBuf,
    },
    /// 玉モード: 系を玉・鏡・座標ブレーク・その他の面と空気間隔に分けて示す。--reverse で玉を反転し、--out で .zmx に書き出す
    Elements {
        #[command(flatten)]
        c: Common,
        /// 反転する玉の番号（1 始まり、玉だけを数える。カンマ区切り）
        #[arg(long)]
        reverse: Option<String>,
        /// 組み直した系を書き出す .zmx
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// ゴースト（面 a で反射し、面 b で反射する 2 回反射、OpticStudio の Ghost Focus Generator の double bounce）の一覧:
    /// 近軸のゴーストの焦点の位置・像面でのぼけの半径と、軸上の主光線の強度の比（膜込み）。--out-dir で展開した系を .zmx に書く
    Ghost {
        #[command(flatten)]
        c: Common,
        /// 反射させる最初の面
        #[arg(long, default_value_t = 1)]
        first: usize,
        /// 反射させる最後の面（省略で像面。像面は膜があるときだけ反射させる）
        #[arg(long)]
        last: Option<usize>,
        /// 膜の実測表（LBCT）・モデル（LBCM）のファイル（複数可）
        #[arg(long)]
        coatings: Vec<PathBuf>,
        /// 展開したゴーストの系を書くフォルダ（ghost_a_b.zmx）
        #[arg(long = "out-dir")]
        out_dir: Option<PathBuf>,
    },
    /// 回折 MTF（変調度、子午・球欠）。--wave を省くと多色
    Mtf {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// 周波数 [cycles/mm、アフォーカル像空間では cycles/角度の単位]（カンマ区切り）。省くと 0 から上限（--max-freq、既定は遮断周波数の 0.95 倍）までの --freq-steps 等分
        #[arg(long)]
        freqs: Option<String>,
        /// 周波数を省いたときの上限 [cycles/mm、アフォーカル像空間では cycles/角度の単位]。省くと遮断周波数の 0.95 倍
        #[arg(long = "max-freq", conflicts_with = "freqs")]
        max_freq: Option<f64>,
        /// 周波数を省いたときの分割数（0 から上限までを等分。点は分割数 + 1 個）
        #[arg(long = "freq-steps", default_value_t = 20, conflicts_with = "freqs")]
        freq_steps: usize,
        /// 瞳の重なりの求積の点の数の目安（点は約 n² 個）。省くと OPD の傾きから自動で決める
        #[arg(long)]
        n: Option<usize>,
        /// 求積点の OPD をすべて直接追跡する（既定は瞳の格子を 1 度追跡して補間。遅い、確かめ用）
        #[arg(long, conflicts_with_all = ["fft", "interp"])]
        direct: bool,
        /// 瞳関数の自己相関を FFT で全周波数まとめて求める（周波数が多いとき・収差が大きいとき速い。
        /// 既定との差は 1e-4 程度）。既定でも --n を省いて求積の点数が 1024 を超え、周波数が 8 個以上なら FFT になる
        #[arg(long, conflicts_with = "interp")]
        fft: bool,
        /// 求積点の数が多い系でも FFT に切り替えず、補間の求積で求める
        #[arg(long)]
        interp: bool,
        /// 多色に加えて波長ごとの MTF も出す（"waves"。波長ごとの計算を使い回すので別々に求めるより速い）。
        /// --wave とは一緒に使えない
        #[arg(long = "each-wave")]
        each_wave: bool,
    },
    /// スルーフォーカス MTF（像面を --range の範囲で --steps 点ずらした回折 MTF）。--wave を省くと多色
    MtfFocus {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// 周波数 [cycles/mm、アフォーカル像空間では cycles/角度の単位]（カンマ区切り）
        #[arg(long, required = true)]
        freqs: String,
        /// 像面のずらしの範囲 ±range [レンズ単位]
        #[arg(long, default_value_t = 0.1)]
        range: f64,
        /// ずらしの点数（−range から +range まで）
        #[arg(long, default_value_t = 11)]
        steps: usize,
        /// 瞳の重なりの求積の点の数の目安（mtf と同じ。省くと自動）
        #[arg(long)]
        n: Option<usize>,
    },
    /// フィールド MTF（正規化視野 0〜1 を --points 点で刻んだ回折 MTF）。--wave を省くと多色
    MtfField {
        #[command(flatten)]
        c: Common,
        /// 波長の番号（1 始まり、省略で多色）
        #[arg(long)]
        wave: Option<usize>,
        /// 周波数 [cycles/mm、アフォーカル像空間では cycles/角度の単位]（カンマ区切り）
        #[arg(long, required = true)]
        freqs: String,
        /// 視野の点数（正規化視野 0 から 1 まで）
        #[arg(long, default_value_t = 11)]
        points: usize,
        /// 視野を振る向き（y か x）
        #[arg(long, default_value = "y")]
        axis: String,
        #[arg(long)]
        n: Option<usize>,
    },
    /// RMS（スポット半径・x・y、波面、ストレール比の近似）を視野・フォーカス・波長に対して
    Rms {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// 横軸: field（正規化視野 0〜1）、focus（像面のずらし ±range）、wavelength（系の波長の最小〜最大、単色）
        #[arg(long, default_value = "field")]
        vs: String,
        /// 横軸の点数
        #[arg(long, default_value_t = 11)]
        points: usize,
        /// focus のずらしの範囲 ±range [レンズ単位]
        #[arg(long, default_value_t = 0.1)]
        range: f64,
        /// field の視野を振る向き（y か x）
        #[arg(long, default_value = "y")]
        axis: String,
        /// 基準: centroid（重心）か chief（主光線）
        #[arg(long = "ref", default_value = "centroid")]
        reference: String,
        /// 瞳のガウス求積のリング数
        #[arg(long, default_value_t = 6)]
        rings: usize,
        /// 単色の field・wavelength の掃引で、実像高（と条件で定める視野）の視野を解析の波長の主光線で解く
        /// （OpticStudio の単色の RMS vs Field・vs Wavelength と同じ。省くと主波長で解く）
        #[arg(long)]
        field_wave: bool,
    },
    /// 減衰最小二乗（DLS）で変数を最適化する。評価関数は全視野（視野の重み）・多色の RMS スポットか RMS 波面に、
    /// --term・--terms の項を足したもの。--out で最適化した系を .zmx に書き出す
    Optimize {
        #[command(flatten)]
        c: Common,
        /// 変数（"7.t"・"lens2.tx"・"G1.shift" など。範囲は "7.t=0.5:3"、片側は "7.t=0.5:" "7.t=:3"）。複数可
        #[arg(long = "var", required = true)]
        vars: Vec<String>,
        /// 既定の評価関数: spot・wavefront・none（--term の項だけ）
        #[arg(long, default_value = "spot")]
        merit: String,
        /// 足す項（JSON、例 '{"type":"paraxial","q":"efl","target":50,"weight":10}'。種類とキーは
        /// docs の最適化の項を参照）。複数可
        #[arg(long = "term")]
        terms: Vec<String>,
        /// 足す項の JSON ファイル（項の配列）
        #[arg(long = "terms")]
        terms_file: Option<PathBuf>,
        /// 基準: centroid（重心）か chief（主光線）
        #[arg(long = "ref", default_value = "centroid")]
        reference: String,
        /// 瞳のガウス求積のリング数
        #[arg(long, default_value_t = 3)]
        rings: usize,
        /// 反復の上限
        #[arg(long, default_value_t = 50)]
        iters: usize,
        /// ヤコビアン: auto（解析的に求められる列が 16 個以上の項は解析的に、少ない項は中心差分）・fd（すべて中心差分）・forward（すべて前進差分）
        #[arg(long, default_value = "auto")]
        jac: String,
        /// 最適化した系を書き出す .zmx
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// モンテカルロ公差シミュレーション。玉ごとの公差の組（既定は一般の組、カタログの玉はメーカーの公差）と
    /// 個別の公差で系をずらし、補償子を合わせ直して評価関数（RMS 波面・スポット・MTF など）の分布と感度を出す。
    /// 設定は JSON（docs の公差の設定、--spec のファイルと --set の上書き）
    Tolerance {
        #[command(flatten)]
        c: Common,
        /// 設定の JSON ファイル
        #[arg(long)]
        spec: Option<PathBuf>,
        /// 設定の上書き（JSON のオブジェクト、例 '{"preset":"precision","compensators":["9.t"]}'）。複数可
        #[arg(long = "set")]
        set: Vec<String>,
        /// 試行の数（設定の trials より優先）
        #[arg(long)]
        trials: Option<usize>,
        /// 乱数の種（設定の seed より優先）
        #[arg(long)]
        seed: Option<u64>,
        /// 評価関数が最も悪かった試行の系（補償の後）を書き出すファイル。.json は系ファイル（屈折率の公差も書ける）、
        /// ほかは .zmx（屈折率・分散のずれは .zmx に書けない）
        #[arg(long = "save-worst")]
        save_worst: Option<PathBuf>,
        /// 試行ごとの補償子（と評価関数・傾きの x・y の組の大きさ [分]）を書き出す CSV ファイル
        #[arg(long = "comp-csv")]
        comp_csv: Option<PathBuf>,
    },
    /// 回折 PSF（瞳関数の DFT）。強度は収差のない同じ瞳の中心の値を 1 とする。--wave を省くと多色。
    /// --huygens で Huygens PSF（参照球の各点からの 2 次波を像面の上で足す。座標は像面の局所座標）
    Psf {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// Huygens PSF（像面の上。傾いた像面・収差の大きい系向け、FFT より遅い）
        #[arg(long)]
        huygens: bool,
        /// 瞳の格子の一辺
        #[arg(long, default_value_t = 64)]
        pupil: usize,
        /// 像の格子の一辺（間隔が既定なら瞳の格子の 2 倍まで）
        #[arg(long, default_value_t = 128)]
        image: usize,
        /// 像の格子の間隔 [レンズ単位、アフォーカル像空間では角度の単位]（省くと 1 / (2 × 遮断周波数)）
        #[arg(long)]
        delta: Option<f64>,
    },
    /// 幾何 MTF（光線の位置の分布から）。--wave を省くと多色
    GeoMtf {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// 周波数 [cycles/mm、アフォーカル像空間では cycles/角度の単位]（カンマ区切り）。省くと 0 から上限（--max-freq、既定は遮断周波数の 0.95 倍）までの --freq-steps 等分
        #[arg(long)]
        freqs: Option<String>,
        /// 周波数を省いたときの上限 [cycles/mm、アフォーカル像空間では cycles/角度の単位]。省くと遮断周波数の 0.95 倍
        #[arg(long = "max-freq", conflicts_with = "freqs")]
        max_freq: Option<f64>,
        /// 周波数を省いたときの分割数（0 から上限までを等分。点は分割数 + 1 個）
        #[arg(long = "freq-steps", default_value_t = 20, conflicts_with = "freqs")]
        freq_steps: usize,
        /// 瞳の求積の点の数の目安（点は約 n² 個）
        #[arg(long, default_value_t = 128)]
        n: usize,
        /// 回折限界の MTF をかける
        #[arg(long = "diff-limit")]
        diff_limit: bool,
    },
    /// Huygens MTF（Huygens PSF のフーリエ変換、像面の上の周波数）。--wave を省くと多色。出力は mtf と同じ形
    HuygensMtf {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        s: Sel,
        /// 周波数 [cycles/mm]（カンマ区切り）。省くと 0 から上限（--max-freq、既定は遮断周波数の 0.95 倍）までの --freq-steps 等分
        #[arg(long)]
        freqs: Option<String>,
        /// 周波数を省いたときの上限 [cycles/mm]。省くと遮断周波数の 0.95 倍
        #[arg(long = "max-freq", conflicts_with = "freqs")]
        max_freq: Option<f64>,
        /// 周波数を省いたときの分割数（0 から上限までを等分。点は分割数 + 1 個）
        #[arg(long = "freq-steps", default_value_t = 20, conflicts_with = "freqs")]
        freq_steps: usize,
        /// 瞳の格子の一辺
        #[arg(long, default_value_t = 64)]
        pupil: usize,
        /// 像の格子の一辺（間隔が既定なら瞳の格子の 2 倍まで）。PSF を覆いきれないと低周波の MTF が下がる
        #[arg(long, default_value_t = 128)]
        image: usize,
        /// 像の格子の間隔 [mm]（省くと 1 / (2 × 遮断周波数)）
        #[arg(long)]
        delta: Option<f64>,
    },
    /// ズーム軌跡: 状態（プロジェクトの states か .zmx の構成）ごとに群の位置（--var）を解く。
    /// 評価関数は像の RMS（全視野・多色）と、--efl・--pmag の目標。--steps で状態の間を継続法でたどる
    Zoom(Box<search_cmd::ZoomArgs>),
    /// カタログレンズの差し替え探索: --spec の所（玉のブロック）を候補で差し替えた組を並列に回し、
    /// 組ごとに変数（間隔など）を最適化して評価関数の小さい順に並べる
    Swap(Box<search_cmd::SwapArgs>),
    /// 単色の干渉縞: 2 つの構成（.zmx）か状態（プロジェクトファイル）の光を重ねる。--mode pupil は瞳上
    /// （OpticStudio の Interferogram に当たる: Δ = scale·(W1 − W2) + (xtilt·px + ytilt·py)/2、I = (1 + cos 2πΔ)/2）、
    /// detector は像面上（光線を画素へ補間して場を足す）。--out の拡張子で csv（格子、行 = +y）・json（要約と格子）・
    /// tif（16 bit、--float で 32 bit float）・png（16 bit）
    Fringe(Box<FringeArgs>),
    /// ユーザーカタログ（JSON）に玉かブロックを保存する（ファイルがなければ作る）
    CatalogAdd {
        /// ユーザーカタログのファイル
        catalog: PathBuf,
        /// 項目の名前
        #[arg(long)]
        name: String,
        /// 玉を取る .zmx かプロジェクトファイル（--lens と）、またはプロジェクトファイル（--block と）
        #[arg(long)]
        from: PathBuf,
        /// 玉の番号（1 始まり、玉だけを数える。.zmx・プロジェクトを玉モードで分けたもの）
        #[arg(long)]
        lens: Option<usize>,
        /// プロジェクトのブロックの id（2 つ渡すと最上位の並びのその間の範囲。面の表の変更 surface_edits は焼き込む）
        #[arg(long, num_args = 1)]
        block: Vec<String>,
        #[arg(long = "glass-dir")]
        glass_dir: Vec<PathBuf>,
        /// プロジェクトのカタログレンズを探すレンズカタログ（info などと同じ）
        #[arg(long = "lens-catalog")]
        lens_catalog: Vec<PathBuf>,
        /// .zmx のマルチコンフィグの番号
        #[arg(long, default_value_t = 1)]
        cfg: usize,
        #[arg(long, default_value = "")]
        description: String,
        /// 同じ名前の項目を置き換える
        #[arg(long)]
        replace: bool,
        #[arg(long)]
        json: bool,
    },
    /// カタログの品を探す: レンズカタログ（ZMF・.zmx・lensbench 形式の .json）の品を、種類・メーカー・語・仕様で
    /// 絞り込む。仕様は .json の品の `specs` と、同じメーカーの .json の `overlay`・`url_template` で足したもの
    /// （efl・diameter・thickness は一覧の値でも比べられる）
    CatalogSearch {
        /// レンズカタログ（ファイル・フォルダ、複数可）
        #[arg(required = true)]
        lens_catalog: Vec<PathBuf>,
        /// ガラスカタログを探すフォルダ（玉の焦点距離を求めるのに使う）
        #[arg(long = "glass-dir")]
        glass_dir: Vec<PathBuf>,
        /// 品の種類（lens・mirror・splitter・polarizer・waveplate・window・filter・prism・other、複数可）
        #[arg(long, num_args = 1)]
        kind: Vec<String>,
        /// メーカー（含む。大文字小文字・空白・記号は無視）
        #[arg(long, default_value = "")]
        vendor: String,
        /// 型番・説明・タグ・仕様に含む語（空白で区切るとすべて含む）
        #[arg(long, default_value = "")]
        text: String,
        /// 仕様の条件（複数可、すべて満たす）: `名前`（ある）、`名前=値`（数は一致、範囲の仕様は含む、文字列は含む）、
        /// `名前=a..b`、`名前>=値`・`<=`・`>`・`<`（範囲の仕様は重なりで）。例 `wavelength_nm=633`・`extinction_ratio>=1000`
        #[arg(long, num_args = 1)]
        spec: Vec<String>,
        /// 径 [mm] の素子が入る機構部品（チューブ・マウント・ケージ板）だけ: 保持径が径以上で 0.25 mm より太くない
        #[arg(long)]
        fits: Option<f64>,
        /// 名前の決まった仕様の一覧を出す
        #[arg(long)]
        list_specs: bool,
        #[arg(long)]
        json: bool,
    },
    /// 品の STEP（.step・.stp）を測る: 厚さ・径、枠付きの品はガラスと枠（`frame`）、ガラスの無い機構部品
    /// （チューブ・ケージ板・マウント）は全体を枠として（`mech`、`frame` に穴・長さ・外径・ねじ・ロッドの間隔・保持する素子）
    CatalogStep {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 品の詳しい形の STEP を公式の配布元（品の `cad.step`）から 1 件だけ取り、キャッシュに置く。2 回目からはキャッシュを使う。
    /// 取った STEP を catalog-step と同じ読み方で測り、カタログの簡易外形（`mech` の長さ・外径・外形の形）との食い違いを示す
    CadStep {
        /// ユーザーカタログ
        #[arg(long)]
        catalog: PathBuf,
        /// 品の名前
        #[arg(long)]
        item: String,
        /// キャッシュのフォルダ（省略で環境変数 LENSBENCH_CAD_CACHE、なければ OS の設定の場所の lensbench/cad）
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 手元の STEP を共有の置き場（環境変数 LENSBENCH_CAD_STORE。cad-step が配布元より先に見る所）に上げる。
    /// --cache はキャッシュのフォルダの STEP をそのまま（ファイル名が鍵）、--catalog はフォルダの STEP をファイル名で品に当てて
    /// （cad-step と同じ鍵）上げる。置き場にある鍵は上げない（--overwrite で上げ直す）
    CadUpload {
        /// キャッシュのフォルダの STEP を上げる（値を省くと cad-step の既定のキャッシュ）
        #[arg(long, num_args = 0..=1, conflicts_with = "catalog")]
        cache: Option<Option<PathBuf>>,
        /// ユーザーカタログ（--dir か --item・--file と）。フォルダなら中の .json・.json.gz を全部（下のフォルダも）
        #[arg(long)]
        catalog: Option<PathBuf>,
        /// STEP（.step・.stp・それを入れた .zip）のフォルダ。ファイル名に品の名前を含むものを当てる（下のフォルダも）
        #[arg(long, requires = "catalog")]
        dir: Option<PathBuf>,
        /// 1 品だけ上げる（--file と）
        #[arg(long, requires = "catalog", requires = "file", conflicts_with = "dir")]
        item: Option<String>,
        #[arg(long, requires = "item")]
        file: Option<PathBuf>,
        /// 置き場にあっても上げ直す
        #[arg(long)]
        overwrite: bool,
        /// 当て方だけ示して上げない
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// 膜ファイルのフォルダの膜のモデル（.lbcb）の目録 lbcb-remote.json を作り、共有の置き場（cad-step と同じ）に無いものを上げる。
    /// 目録を膜ファイルの隣に置くと、フォルダに無い膜のモデルを置き場から取って使う（配る版で膜のモデルを外すため）
    CoatUpload {
        /// 膜ファイル（COATING.DAT）と .lbcb のフォルダ
        dir: PathBuf,
        /// 目録の書き出し先（省略でフォルダの lbcb-remote.json）
        #[arg(long)]
        index: Option<PathBuf>,
        /// 目録だけ作って上げない
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// 膜ファイルのフォルダの目録 lbcb-remote.json の膜のモデルを置き場からすべて取り、フォルダに <膜>.lbcb として置く
    /// （Python など置き場を見ない使い方のため。ふつうは使う膜だけ自動で取る）
    CoatFetch {
        dir: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// ユーザーカタログの項目の一覧
    CatalogList {
        catalog: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// 面の膜: 全コンフィグの境界（膜|前の媒質|後の媒質）と扱い。実測表が要る境界（暗号化された膜など）も示す
    Coatings {
        /// .zmx ファイル
        file: PathBuf,
        /// ガラスカタログを探すフォルダ（複数可）
        #[arg(long = "glass-dir")]
        glass_dir: Vec<PathBuf>,
        /// 膜の実測表（LBCT 形式、複数可）
        #[arg(long)]
        table: Vec<PathBuf>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 実測表（LBCT）から、波長方向の折れ目（膜の材料の分散表の節）を探す。
    /// 波長の格子は膜の厚さから決めた格子の 4 倍くらい細かくして測っておく
    CoatKinks {
        /// 実測表（LBCT 形式、複数可）
        tables: Vec<PathBuf>,
        /// 調べる KEY（複数回指定できる。既定は空気から入る透過と鏡の KEY。硝材側は高い β の列で残差が大きく、検出が鈍い）
        #[arg(long)]
        key: Vec<String>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// レイアウト（断面図・立体）と光線を書き出す。形式は --out の拡張子で決める:
    /// 2D は svg・png・dxf、3D は gltf・glb・obj・stl、CAD 用は step（stp）
    Layout {
        #[command(flatten)]
        c: Common,
        #[command(flatten)]
        l: LayoutArgs,
        /// 書き出すファイル（複数可）
        #[arg(long, required = true)]
        out: Vec<PathBuf>,
        /// 2D の投影面（yz か xz）
        #[arg(long, default_value = "yz")]
        plane: String,
        /// PNG の横幅 [px]
        #[arg(long, default_value_t = 1600)]
        width: u32,
        /// 周方向の分割数（3D）・面の線の分割数（2D）
        #[arg(long, default_value_t = 64)]
        segments: usize,
        /// 半径方向の分割数（3D）
        #[arg(long, default_value_t = 16)]
        rings: usize,
        /// 光線を管にする半径 [mm]（3D。STL は省略時に場面の大きさの 0.1%）
        #[arg(long)]
        tube: Option<f64>,
        /// 機構部品を、取った品の STEP の詳しい形で描く（3D の glTF・OBJ・STL と STEP。キャッシュに STEP の無い部品は
        /// 簡易外形。STEP は lensbench cad-step で取る）
        #[arg(long)]
        mech_detail: bool,
    },
    /// 簡易 POP: 光線から面の接平面の上に場を作り（面の開口を掛けて）、一様な媒質の中を距離 --dist
    /// （省略で次の面の頂点面まで）伝えた照度。--save で格子（csv・npz）か全部（json）を書き出す
    Pop {
        #[command(flatten)]
        c: Common,
        /// 場を作る面（1 始まり。その面の後の媒質の中、接平面の上）。省略で、間に平面のダミー面だけを挟んで
        /// 像面まで伝えられる最後の面（--dist も --to もなければ像面まで伝える）。像面まで伝えられる面が無ければ、
        /// 次の面まで伝えられる最後の面から次の面まで
        #[arg(long)]
        surf: Option<usize>,
        /// 視野の番号（1 始まり、既定 1）
        #[arg(long, default_value_t = 1)]
        field: usize,
        /// 波長の番号（1 始まり、省略で主波長）
        #[arg(long)]
        wave: Option<usize>,
        /// 伝える距離 [mm]（伝搬の向きに測る）
        #[arg(long)]
        dist: Option<f64>,
        /// 伝える先の面（間は平面のダミー面だけ）。--dist も --to もなければ次の面
        #[arg(long)]
        to: Option<usize>,
        /// 方法: auto（既定。遠ければ fresnel、近ければ angular）、fresnel（Fresnel の積分、近軸）、
        /// angular（角スペクトル、FFT）、direct（Rayleigh-Sommerfeld の積分、近似なし・遅い）
        #[arg(long, default_value = "auto")]
        method: String,
        /// 瞳の格子の一辺の光線の数
        #[arg(long, default_value_t = 65)]
        rays: usize,
        /// 入力の格子の一辺の標本の数
        #[arg(long, default_value_t = 256)]
        n: usize,
        /// 入力の格子の半幅 [mm]（省略で光線と開口の範囲）
        #[arg(long)]
        half: Option<f64>,
        /// 面の開口を掛けない
        #[arg(long)]
        no_aperture: bool,
        /// 出力の格子の一辺の標本の数（省略で入力と同じ）
        #[arg(long)]
        out_n: Option<usize>,
        /// 出力の格子の半幅 [mm]（省略で自動）
        #[arg(long)]
        out_half: Option<f64>,
        /// 出力の格子の中心 "x,y" [mm]（省略で主光線の向きに進めた点）
        #[arg(long, allow_hyphen_values = true)]
        center: Option<String>,
        /// 結果の JSON に出力の照度の格子も入れる（"grid_irradiance"、番号 j·nx + i、j は +y の向き。GUI の図）
        #[arg(long)]
        grid: bool,
    },
    /// 光線の面ごとの交点・方向・光路長（1 行 = 1 光線 × 1 面）。--save で csv・json・npz に、
    /// 省略すると CSV を標準出力へ
    Rays {
        #[command(flatten)]
        c: Common,
        /// 視野の番号（1 始まり、カンマ区切り、省略で全視野）
        #[arg(long)]
        field: Option<String>,
        /// 波長の番号（1 始まり、カンマ区切り、all で全部、省略で主波長）
        #[arg(long)]
        wave: Option<String>,
        /// 光線の並び（y・x・cross・grid・ring）
        #[arg(long, default_value = "grid")]
        fan: String,
        /// 光線の本数（扇・十字は片方向の本数、grid は一辺、ring は周の本数で、主光線を足した n+1 本）
        #[arg(long, default_value_t = 5)]
        rays: usize,
        /// 瞳座標を直接（"px,py;px,py;…"）。--fan・--rays より優先
        #[arg(long)]
        pupil: Option<String>,
        /// 書く面（カンマ区切り、省略で全部）
        #[arg(long)]
        surface: Option<String>,
    },
    /// 機構部品（プロジェクトの mechanics）のケラレ: 瞳の格子の光線を追い、部品の穴（チューブの内径・
    /// ケージ板やマウントのクリア開口）の外に出た割合を部品ごと・視野と波長ごとに出す
    MechCheck {
        #[command(flatten)]
        c: Common,
        /// 視野の番号（1 始まり、カンマ区切り、省略で全視野）
        #[arg(long)]
        field: Option<String>,
        /// 波長の番号（1 始まり、カンマ区切り、all で全部、省略で全波長）
        #[arg(long)]
        wave: Option<String>,
        /// 瞳の格子の一辺の本数（単位円の中の点を使う）
        #[arg(long, default_value_t = 21)]
        grid: usize,
    },
    /// 暗号化された膜のモデル（LBCM 2）を、モデルガラスの組で測った実測表（LBCT）から作る
    CoatModel {
        /// 実測表（LBCT 形式、複数可。同じ KEY は後の表のものを使う）
        tables: Vec<PathBuf>,
        /// 書き出す LBCM ファイル
        #[arg(long)]
        out: PathBuf,
        /// 作る膜（カンマ区切り、既定は表にある全部）
        #[arg(long)]
        coat: Option<String>,
        /// 波長方向の折れ目 [µm]（カンマ区切り）。膜の材料の分散表の節。補間はこの点をまたがない
        #[arg(long)]
        kinks: Option<String>,
        /// BMAX を決める ψ の補間誤差の上限 [rad]
        #[arg(long, default_value_t = 1e-4)]
        tol_psi: f64,
        /// モデルに入れる注記（NOTE。`lensbench coatings` に出る。短い測定から作った近似のモデルの印など）
        #[arg(long)]
        note: Option<String>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 膜のモデル（LBCM）を小さくする: バイナリ（LBCM 3、値を 1e-9 刻み・鏡は 1e-12 刻みの整数にして縮めたもの）で書く。
    /// 読むときは LBCM 2（テキスト）と同じに扱える
    CoatCompact {
        /// 元のモデル（LBCM 2 か 3）
        model: PathBuf,
        /// 書き出すモデル（LBCM 3）
        #[arg(long)]
        out: PathBuf,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 分光の曲線（反射率・透過率）に合う多層膜を当てはめ、近似の膜（`.lbcs`、膜ファイルの形式）を書く。
    /// 曲線は CSV（列 wl_um か wl_nm、angle_deg、pol（s・p・avg）、quantity（R・T）、value か value_pct、weight。
    /// angle_deg・pol・quantity・weight は省けば 0・avg・R・1）。膜ファイルのフォルダに置くと膜の名前で使われる
    CoatDesign {
        /// 曲線の CSV（複数可）
        targets: Vec<PathBuf>,
        /// 膜の名前（書く COAT の名前）
        #[arg(long)]
        name: String,
        /// 材料（外側から繰り返す順、例 MGF2,TA2O5）。膜ファイルの MATE か、ガラスカタログのガラス
        #[arg(long, value_delimiter = ',')]
        materials: Vec<String>,
        /// 基板: 屈折率の数か、膜ファイルの MATE・ガラスの名前
        #[arg(long)]
        substrate: String,
        /// 材料を探す膜ファイル（COATING.DAT 形式）
        #[arg(long = "coating-file")]
        coating_file: Option<PathBuf>,
        /// ガラスカタログを探すフォルダ（複数可）
        #[arg(long = "glass-dir")]
        glass_dir: Vec<PathBuf>,
        /// 試す層の数（例 1..12）
        #[arg(long, default_value = "1..12")]
        layers: String,
        /// 層の数ごとの初期値の数
        #[arg(long, default_value_t = 20)]
        starts: usize,
        /// 層の厚さの上限 [µm]
        #[arg(long = "d-max", default_value_t = 0.6)]
        d_max: f64,
        /// いちばん合う RMS の (1 + slack) 倍以内で層のいちばん少ないものを選ぶ
        #[arg(long, default_value_t = 0.1)]
        slack: f64,
        /// 書き出す近似の膜（.lbcs）
        #[arg(long)]
        out: Option<PathBuf>,
        /// JSON で出力する
        #[arg(long)]
        json: bool,
    },
    /// 系を読み込んだまま待ち受け、外から JSON-RPC で系の変更・解析を繰り返す（既定は 127.0.0.1 だけ）。
    /// 手順と書式は docs/serve.md
    Serve(serve::ServeArgs),
    /// 系のファイルの更新を見張り、変わるたびに解析を実行して結果を書き出す。
    /// 例: lensbench watch -- spot sys.zmx --json --save out.json
    Watch(watch::WatchArgs),
    /// Python のスクリプトを画面なしで実行する（外の Python を起動し、ガラス・膜・スレッド数・出力先を渡す）。
    /// 例: lensbench run --threads 4 --out result sweep.py -- --n 100。終了コードはスクリプトのもの
    Run(Box<run::RunArgs>),
    /// 同梱のプラグイン（検出器・解析）の一覧と入出力の説明
    Plugins {
        /// JSON で出す
        #[arg(long)]
        json: bool,
    },
    /// プラグイン・解析部品を基準データと照合する。記述（*.verify.json）かそれを含むフォルダを与える。
    /// 通らない照合があれば終了コード 1
    Verify {
        /// 記述のファイルかフォルダ（フォルダは下まで *.verify.json を探す）
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// ci: false の照合（手元でだけ回す）を飛ばす
        #[arg(long)]
        ci: bool,
        /// JSON で出す
        #[arg(long)]
        json: bool,
    },
    /// 拡張のサブコマンド（[`ext`]。clap の定義は [`cli_command`] が足す）
    #[command(skip)]
    Ext(ExtCmd),
}

/// レイアウトの光線の選び方
#[derive(Args)]
struct LayoutArgs {
    /// 視野の番号（1 始まり、カンマ区切り、省略で全視野）
    #[arg(long)]
    field: Option<String>,
    /// 波長の番号（1 始まり、カンマ区切り、all で全部、省略で主波長）
    #[arg(long)]
    wave: Option<String>,
    /// 光線の並び（y・x・cross・grid・ring）。省略で 2D は y、3D・STEP は ring（主光線と瞳の縁）
    #[arg(long)]
    fan: Option<String>,
    /// 光線の本数（扇・十字は片方向の本数、grid は一辺、ring は周の本数で、主光線を足した n+1 本）。省略で y は 7、ring は 12
    #[arg(long)]
    rays: Option<usize>,
    /// 描き始める面（省略で、有限の物体が近ければ物体面、ほかは面 1）
    #[arg(long)]
    first: Option<usize>,
    /// 描き終える面（省略で像面）
    #[arg(long)]
    last: Option<usize>,
    /// 光線の色分け（field・wave・cfg。cfg は重ね描きの構成ごと）。省略で、重ね描きなら cfg、ほかは field
    #[arg(long)]
    color: Option<String>,
    /// 重ね描きする構成
    #[command(flatten)]
    sel: ConfigSel,
    /// 構成ごとにずらす量 dx,dy,dz [mm]（構成 k は k 倍。OpticStudio のレイアウトのオフセットと同じ。既定は重ねる）
    #[arg(long, allow_hyphen_values = true)]
    offset: Option<String>,
    /// 光路に沿って展開する（鏡・BS で折れた後の区間を、折れる前の光軸の延長に並べ直す。2D の書き出しだけ）
    #[arg(long)]
    unfold: bool,
}

/// 重ね描き・多重構成の書き出しで並べる構成
#[derive(Args, Clone, Default)]
pub(crate) struct ConfigSel {
    /// .zmx の多重構成（1 始まり、カンマ区切り、all で全部）
    #[arg(long)]
    cfgs: Option<String>,
    /// プロジェクトの状態（名前のカンマ区切り、all で全部）
    #[arg(long)]
    states: Option<String>,
    /// ビームスプリッターの経路（T・R の文字列のカンマ区切り、all でとれる経路の全部）
    #[arg(long)]
    paths: Option<String>,
}

impl ConfigSel {
    fn any(&self) -> bool {
        self.cfgs.is_some() || self.states.is_some() || self.paths.is_some()
    }
}

/// 重ね描きする構成の系と名前。重ね描きでなければ今の系 1 つ（名前は空）
pub(crate) fn overlay_systems(c: &Common, l: &ConfigSel) -> Result<Vec<(String, System)>, String> {
    if !l.any() {
        return Ok(vec![(String::new(), load(c)?)]);
    }
    let is_json = c
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    if let Some(v) = &l.cfgs {
        if is_json {
            return Err("--cfgs は .zmx だけ（プロジェクトは --states・--paths）".into());
        }
        let opt = zmx::LoadOptions {
            catalog_dirs: c.glass_dir.clone(),
            ..Default::default()
        };
        let txt = glass::read_text(&c.file).map_err(|x| format!("{}: {x}", c.file.display()))?;
        let n = zmx::parse_text(&txt)
            .map_err(|x| x.to_string())?
            .configurations();
        let ks: Vec<usize> = index_list(v, n, "構成")?
            .into_iter()
            .map(|k| k + 1)
            .collect();
        let ss = zmx::load_configs(&c.file, &opt, &ks).map_err(|x| x.to_string())?;
        return Ok(ks.iter().map(|k| format!("C{k}")).zip(ss).collect());
    }
    if !is_json {
        return Err(
            "--states・--paths はプロジェクトファイル（.json）だけ（.zmx は --cfgs）".into(),
        );
    }
    let txt = lensbench_core::vfs::read_to_string(&c.file)
        .map_err(|x| format!("{}: {x}", c.file.display()))?;
    let p = lensbench_project::Project::from_json(&txt)?;
    let states: Vec<Option<String>> = match &l.states {
        None => vec![c.state.clone()],
        Some(v) if v.eq_ignore_ascii_case("all") => {
            let n: Vec<Option<String>> = p
                .state_names()
                .into_iter()
                .map(|x| Some(x.to_string()))
                .collect();
            if n.is_empty() {
                return Err("プロジェクトに状態がない".into());
            }
            n
        }
        Some(v) => v.split(',').map(|x| Some(x.trim().to_string())).collect(),
    };
    let cat = LensCatalog::load_registered(&c.lens_catalog)?;
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let env = lensbench_project::Env {
        base: c.file.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
        glass_dirs: c.glass_dir.clone(),
        catalog: Some(&look),
    };
    let mut out = Vec::new();
    for st in &states {
        let ps = match &st {
            Some(n) => p.with_state(n)?,
            None => p.clone(),
        };
        let paths: Vec<Option<String>> = match &l.paths {
            None => vec![c.path.clone()],
            Some(v) if v.eq_ignore_ascii_case("all") => {
                let v = lensbench_project::paths(&ps, &env)?;
                if v.is_empty() {
                    return Err("プロジェクトにビームスプリッターがない".into());
                }
                v.into_iter().map(Some).collect()
            }
            Some(v) => v.split(',').map(|x| Some(x.trim().to_string())).collect(),
        };
        for pa in &paths {
            let (s, warn) =
                lensbench_project::expand_path(&ps, &env, pa.as_deref()).map_err(|x| match pa {
                    Some(q) => format!("経路 {q}: {x}"),
                    None => x,
                })?;
            for w in warn {
                eprintln!("注意: {w}");
            }
            let mut name: Vec<&str> = Vec::new();
            if l.states.is_some() {
                name.extend(st.as_deref());
            }
            if l.paths.is_some() {
                name.extend(pa.as_deref());
            }
            out.push((name.join("/"), s));
        }
    }
    Ok(out)
}

/// "1,3" のような 1 始まりの番号の並びを 0 始まりにする。"all" は 0..n
fn index_list(s: &str, n: usize, what: &str) -> Result<Vec<usize>, String> {
    if s.eq_ignore_ascii_case("all") {
        return Ok((0..n).collect());
    }
    s.split(',')
        .map(|t| {
            let k: usize = t.trim().parse().map_err(|_| format!("{what}の番号 {t}"))?;
            if k == 0 || k > n {
                return Err(format!("{what} {k} はありません（1〜{n}）"));
            }
            Ok(k - 1)
        })
        .collect()
}

/// 2D と 3D で既定の光線の並びが違う（2D は断面に乗る y 扇、3D は光束の形が分かる円周）
fn scene_options(s: &System, l: &LayoutArgs, is_2d: bool) -> Result<scene::Options, String> {
    let fan: scene::Fan = match &l.fan {
        Some(f) => f.parse()?,
        None if is_2d => scene::Fan::Y,
        None => scene::Fan::Ring,
    };
    let rays = l.rays.unwrap_or(match fan {
        scene::Fan::Ring => 12,
        _ => 7,
    });
    Ok(scene::Options {
        fields: match &l.field {
            None => None,
            Some(v) => Some(index_list(v, s.fields.len(), "視野")?),
        },
        waves: match &l.wave {
            None => None,
            Some(v) => Some(index_list(v, s.wavelengths.len(), "波長")?),
        },
        fan,
        rays,
        first: l.first,
        last: l.last,
        unfold: l.unfold,
    })
}

thread_local! {
    /// このスレッドで最後に読んだ系の hash（--save の JSON の provenance に刻む）
    static LOADED_HASH: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn load(c: &Common) -> Result<System, String> {
    // serve で系を読み込んであれば、ファイルを読まずにその系を使う
    let s = match serve::loaded() {
        Some(s) => s,
        None => {
            let s = load_file(c)?;
            for w in s
                .dispersion_warnings()
                .into_iter()
                .chain(s.macro_warnings())
            {
                eprintln!("注意: {w}");
            }
            s
        }
    };
    let h = lensbench_project::sysfile::system_hash(&s).ok();
    LOADED_HASH.with(|x| *x.borrow_mut() = h);
    Ok(s)
}

/// 結果に刻む版と系の hash
fn provenance() -> Value {
    let h = LOADED_HASH.with(|x| x.borrow().clone());
    json!({ "lensbench": lensbench_core::VERSION, "system_hash": h })
}

/// プロジェクトファイルのカタログの玉（ブロックの id → メーカー）と裏返して置いた玉の id。.zmx・系ファイルなら空
fn tolerance_vendors(
    c: &Common,
) -> (
    std::collections::HashMap<String, String>,
    std::collections::HashSet<String>,
) {
    if !c
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        || lensbench_project::sysfile::is_system_file(&c.file)
    {
        return Default::default();
    }
    lensbench_core::vfs::read_to_string(&c.file)
        .ok()
        .and_then(|t| lensbench_project::Project::from_json(&t).ok())
        .map(|p| {
            (
                lensbench_project::tolerance::catalog_vendors(&p),
                lensbench_project::tolerance::reversed_lenses(&p),
            )
        })
        .unwrap_or_default()
}

/// ファイルから系を読む
fn load_file(c: &Common) -> Result<System, String> {
    if c.cfg == 0 {
        return Err("--cfg は 1 以上".into());
    }
    if lensbench_project::sysfile::is_system_file(&c.file) {
        if c.cfg != 1 || c.state.is_some() {
            return Err("系ファイルでは --cfg・--state は使えない".into());
        }
        let (s, _, warn) = lensbench_project::sysfile::load(&c.file)?;
        for w in warn {
            eprintln!("注意: {w}");
        }
        return Ok(s);
    }
    if c.file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
    {
        if c.cfg != 1 {
            return Err("プロジェクトファイルでは --cfg ではなく --state で状態を選ぶ".into());
        }
        let cat = LensCatalog::load_registered(&c.lens_catalog)?;
        let look = |v: &str, p: &str| cat.zmx(v, p);
        let (s, warn) = lensbench_project::load_path_report(
            &c.file,
            &c.glass_dir,
            Some(&look),
            c.state.as_deref(),
            c.path.as_deref(),
        )?;
        for w in warn {
            eprintln!("注意: {w}");
        }
        return Ok(s);
    }
    if c.state.is_some() {
        return Err("--state はプロジェクトファイル（.json）だけ（.zmx は --cfg）".into());
    }
    if c.path.is_some() {
        return Err("--path はプロジェクトファイル（.json）だけ".into());
    }
    let opt = zmx::LoadOptions {
        cfg: c.cfg,
        catalog_dirs: c.glass_dir.clone(),
        ..Default::default()
    };
    zmx::load_with(&c.file, &opt).map_err(|e| e.to_string())
}

/// 単色の干渉縞の光束（.zmx は構成の番号、プロジェクトファイルは状態の名前）の系
pub(crate) fn load_beam(a: &FringeArgs, beam: &str) -> Result<System, String> {
    let project = a
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    let (cfg, state) = if project {
        (1, Some(beam.to_string()))
    } else {
        let k: usize = beam
            .trim()
            .parse()
            .map_err(|_| format!("光束 {beam}（.zmx では構成の番号、1 始まり）"))?;
        (k, None)
    };
    let c = Common {
        file: a.file.clone(),
        glass_dir: a.glass_dir.clone(),
        lens_catalog: a.lens_catalog.clone(),
        cfg,
        state,
        path: a.path.clone(),
        json: true,
        save: None,
    };
    let s = load_file(&c)?;
    for w in s
        .dispersion_warnings()
        .into_iter()
        .chain(s.macro_warnings())
    {
        eprintln!("注意: {w}");
    }
    Ok(s)
}

/// プロジェクトファイルの状態の名前（.zmx なら空）
/// プロジェクトファイルのビームスプリッターの経路の一覧（.zmx・BS なしなら空）
fn split_paths(c: &Common) -> Result<Vec<String>, String> {
    if !c
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        || lensbench_project::sysfile::is_system_file(&c.file)
    {
        return Ok(Vec::new());
    }
    let t = lensbench_core::vfs::read_to_string(&c.file)
        .map_err(|x| format!("{}: {x}", c.file.display()))?;
    let mut p = lensbench_project::Project::from_json(&t)?;
    if let Some(st) = &c.state {
        p = p.with_state(st)?;
    }
    let cat = LensCatalog::load_registered(&c.lens_catalog)?;
    let look = |v: &str, q: &str| cat.zmx(v, q);
    let env = lensbench_project::Env {
        base: c.file.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
        glass_dirs: c.glass_dir.clone(),
        catalog: Some(&look),
    };
    lensbench_project::paths(&p, &env)
}

fn state_names(c: &Common) -> Vec<String> {
    if !c
        .file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"))
    {
        return Vec::new();
    }
    lensbench_core::vfs::read_to_string(&c.file)
        .ok()
        .and_then(|t| lensbench_project::Project::from_json(&t).ok())
        .map(|p| p.states.iter().map(|s| s.name.clone()).collect())
        .unwrap_or_default()
}

fn fields(s: &System, sel: &Sel) -> Result<Vec<usize>, String> {
    match sel.field {
        None => Ok((0..s.fields.len()).collect()),
        Some(i) if i >= 1 && i <= s.fields.len() => Ok(vec![i - 1]),
        Some(i) => Err(format!("視野 {i} はありません（1〜{}）", s.fields.len())),
    }
}

fn wave(s: &System, sel: &Sel) -> Result<Option<usize>, String> {
    match sel.wave {
        None => Ok(None),
        Some(i) if i >= 1 && i <= s.wavelengths.len() => Ok(Some(i - 1)),
        Some(i) => Err(format!(
            "波長 {i} はありません（1〜{}）",
            s.wavelengths.len()
        )),
    }
}

/// 周波数を省いたときの並び: 0 から `top` までの `steps` 等分（点は steps + 1 個）
fn auto_freqs(top: f64, steps: usize) -> Result<Vec<f64>, String> {
    if !(top.is_finite() && top > 0.0) {
        return Err(format!("--max-freq は正の数（{top}）"));
    }
    if steps == 0 {
        return Err("--freq-steps は 1 以上".into());
    }
    Ok((0..=steps).map(|k| top * k as f64 / steps as f64).collect())
}
/// カンマ区切りの数
fn parse_list(s: &str, what: &str) -> Result<Vec<f64>, String> {
    let v: Vec<f64> = s
        .split(',')
        .filter(|x| !x.trim().is_empty())
        .map(|x| {
            x.trim()
                .parse::<f64>()
                .map_err(|_| format!("{what} の数が読めない（{x}）"))
        })
        .collect::<Result<_, _>>()?;
    if v.is_empty() {
        return Err(format!("{what} が空"));
    }
    Ok(v)
}

fn parse_axis(s: &str) -> Result<sweep::FieldAxis, String> {
    match s.to_ascii_lowercase().as_str() {
        "x" => Ok(sweep::FieldAxis::X),
        "y" => Ok(sweep::FieldAxis::Y),
        _ => Err(format!("--axis は x か y（{s}）")),
    }
}

/// 文字で出すときは長い表を省く（PSF の格子など。--json・--save では全体）
fn brief(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), brief(x))).collect()),
        Value::Array(a) if a.len() > 1000 => {
            json!(format!("（{} 行。全体は --json か --save で）", a.len()))
        }
        _ => v.clone(),
    }
}

/// "px,py;px,py" の瞳座標
fn parse_pupil(s: &str) -> Result<(Vec<f64>, Vec<f64>), String> {
    let mut px = Vec::new();
    let mut py = Vec::new();
    for t in s.split(';').filter(|t| !t.trim().is_empty()) {
        let (a, b) = t
            .split_once(',')
            .ok_or_else(|| format!("瞳座標 {t} は px,py"))?;
        px.push(a.trim().parse::<f64>().map_err(|_| format!("瞳座標 {t}"))?);
        py.push(b.trim().parse::<f64>().map_err(|_| format!("瞳座標 {t}"))?);
    }
    if px.is_empty() {
        return Err("瞳座標がない".into());
    }
    Ok((px, py))
}

/// --save: 解析の結果を書き出す（json はそのまま、csv・npz は表にして）
fn save_value(p: &std::path::Path, v: &Value) -> Result<(), String> {
    let json = p
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    if json {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.insert("provenance".into(), provenance());
        }
        let s = serde_json::to_string_pretty(&v).unwrap();
        lensbench_core::vfs::write(p, s + "\n").map_err(|x| format!("{}: {x}", p.display()))
    } else {
        table::from_json(v).save(p)
    }
}

impl Cmd {
    /// 系のファイルを読む解析の共通の引数
    fn common_mut(&mut self) -> Option<&mut Common> {
        match self {
            Cmd::Info(c)
            | Cmd::Spot { c, .. }
            | Cmd::Wavefront { c, .. }
            | Cmd::Zernike { c, .. }
            | Cmd::Mtf { c, .. }
            | Cmd::MtfFocus { c, .. }
            | Cmd::MtfField { c, .. }
            | Cmd::Rms { c, .. }
            | Cmd::Optimize { c, .. }
            | Cmd::Tolerance { c, .. }
            | Cmd::Psf { c, .. }
            | Cmd::GeoMtf { c, .. }
            | Cmd::HuygensMtf { c, .. }
            | Cmd::Elements { c, .. }
            | Cmd::Convert { c, .. }
            | Cmd::Ghost { c, .. }
            | Cmd::Layout { c, .. }
            | Cmd::Pop { c, .. }
            | Cmd::MechCheck { c, .. }
            | Cmd::Rays { c, .. } => Some(c),
            Cmd::Ext(e) => e.args.common_mut(),
            _ => None,
        }
    }

    /// 解析が読み書きするファイルのパスと、書き出しか（--save・--out・ghost の膜表など。serve で場所を確かめる）
    fn paths_mut(&mut self) -> Vec<(&mut PathBuf, bool)> {
        let (c, mut v): (Option<&mut Common>, Vec<(&mut PathBuf, bool)>) = match self {
            Cmd::Layout { c, out, .. } => (Some(c), out.iter_mut().map(|p| (p, true)).collect()),
            Cmd::Elements { c, out, .. } => (Some(c), out.iter_mut().map(|p| (p, true)).collect()),
            Cmd::Optimize {
                c, out, terms_file, ..
            } => (
                Some(c),
                out.iter_mut()
                    .map(|p| (p, true))
                    .chain(terms_file.iter_mut().map(|p| (p, false)))
                    .collect(),
            ),
            Cmd::Tolerance {
                c,
                spec,
                save_worst,
                comp_csv,
                ..
            } => {
                let mut v: Vec<_> = spec.iter_mut().map(|p| (p, false)).collect();
                v.extend(save_worst.iter_mut().map(|p| (p, true)));
                v.extend(comp_csv.iter_mut().map(|p| (p, true)));
                (Some(c), v)
            }
            Cmd::Convert { c, out } => (Some(c), vec![(out, true)]),
            Cmd::Ext(e) => {
                let v = e.args.paths_mut();
                // 拡張の paths_mut は --save も含む
                return v;
            }
            Cmd::Ghost {
                c,
                coatings,
                out_dir,
                ..
            } => {
                let mut v: Vec<_> = coatings.iter_mut().map(|p| (p, false)).collect();
                v.extend(out_dir.iter_mut().map(|p| (p, true)));
                (Some(c), v)
            }
            other => (other.common_mut(), Vec::new()),
        };
        if let Some(p) = c.and_then(|c| c.save.as_mut()) {
            v.push((p, true));
        }
        v
    }

    /// 結果を書き出すファイル（rays は自分で書くので除く）
    fn save(&self) -> Option<PathBuf> {
        match self {
            Cmd::Info(c)
            | Cmd::Spot { c, .. }
            | Cmd::Wavefront { c, .. }
            | Cmd::Zernike { c, .. }
            | Cmd::Mtf { c, .. }
            | Cmd::MtfFocus { c, .. }
            | Cmd::MtfField { c, .. }
            | Cmd::Rms { c, .. }
            | Cmd::Optimize { c, .. }
            | Cmd::Tolerance { c, .. }
            | Cmd::Psf { c, .. }
            | Cmd::GeoMtf { c, .. }
            | Cmd::HuygensMtf { c, .. }
            | Cmd::Elements { c, .. }
            | Cmd::MechCheck { c, .. }
            | Cmd::Layout { c, .. } => c.save.clone(),
            Cmd::Zoom(a) => a.save.clone(),
            Cmd::Swap(a) => a.save.clone(),
            Cmd::Ext(e) => e.args.common().and_then(|c| c.save.clone()),
            _ => None,
        }
    }
}

/// 同梱のプラグインを登録した表（exe に組み込む分）に、プロセスで共通の登録
/// （[`lensbench_core::plugin::global`]、この CLI を組み込んだ exe が足した分）を重ねたもの
pub fn builtin_plugins() -> lensbench_core::plugin::Registry {
    lensbench_project::plugins::builtin()
}

pub(crate) fn plugins_json() -> Value {
    let reg = builtin_plugins();
    let pairs = |v: &[(String, String)]| -> Value {
        v.iter()
            .map(|(k, d)| json!({ "name": k, "doc": d }))
            .collect()
    };
    let list: Vec<Value> = reg
        .list()
        .into_iter()
        .filter_map(|(name, kind)| {
            let pl = reg.get(&name).ok()?;
            let s = pl.spec();
            let params: Vec<Value> = pl.params().iter().map(arg_spec::param_json).collect();
            Some(json!({
                "name": name, "kind": kind.as_str(), "doc": s.doc, "needs_system": s.needs_system,
                "config": pairs(&s.config), "params": params, "inputs": pairs(&s.inputs), "outputs": pairs(&s.outputs),
            }))
        })
        .collect();
    json!({ "plugins": list })
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::Serve(a) => serve::serve(a, cli.threads),
        Cmd::Watch(a) => watch::watch(a),
        Cmd::Verify { paths, ci, json } => verify(&paths, ci, json),
        cmd => {
            let (json_out, out) = execute(cmd)?;
            print_out(json_out, &out);
            Ok(())
        }
    }
}

/// 照合: 記述を回して表か JSON を出し、通らなければ誤り
fn verify(paths: &[PathBuf], ci: bool, json_out: bool) -> Result<(), String> {
    let opt = lensbench_verify::Options { ci_only: ci };
    let reps = lensbench_verify::run_paths(paths, &builtin_plugins(), &opt)?;
    if json_out {
        let v: Vec<Value> = reps.iter().map(|r| r.to_json()).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "reports": v })).unwrap()
        );
    } else {
        for r in &reps {
            print!("{}", r.to_text());
        }
    }
    let bad = reps.iter().filter(|r| !r.passed()).count();
    if reps.is_empty() {
        return Err("照合の記述（*.verify.json）が見つからない".into());
    }
    if bad > 0 {
        return Err(format!("照合 {} 件のうち {bad} 件が通らない", reps.len()));
    }
    Ok(())
}

/// catalog-search の表（メーカー・型番・種類・焦点距離・径・仕様・リンク）
fn print_catalog_table(v: &Value) {
    let rows = v["lenses"].as_array().cloned().unwrap_or_default();
    let f = |x: &Value| {
        x.as_f64()
            .map(|x| format!("{x:.4}"))
            .unwrap_or_else(|| "-".into())
    };
    println!("{} 品", rows.len());
    for r in &rows {
        let specs = r["specs"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| match v {
                        Value::String(t) => format!("{k}={t}"),
                        o => format!("{k}={o}"),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        println!(
            "{:<14} {:<16} {:<9} efl={:<10} d={:<8} {}{}{}",
            r["vendor"].as_str().unwrap_or(""),
            r["part"].as_str().unwrap_or(""),
            lensbench_project::catalog_spec::row_category(r),
            f(&r["efl"]),
            f(&r["diameter"]),
            specs,
            r["url"]
                .as_str()
                .map(|u| format!(" {u}"))
                .unwrap_or_default(),
            r["error"]
                .as_str()
                .map(|e| format!(" エラー: {e}"))
                .unwrap_or_default(),
        );
    }
}

/// 結果を標準出力に出す
/// 機構のケラレ（[`Cmd::MechCheck`]）
fn mech_report(
    s: &lensbench_core::System,
    fields: &[usize],
    waves: &[usize],
    grid: usize,
) -> Result<Value, String> {
    if s.mechanics.is_empty() {
        return Err("機構部品がない（プロジェクトの mechanics に書く）".into());
    }
    if grid < 2 {
        return Err("--grid は 2 以上".into());
    }
    let (px, py) = lensbench_core::mech::pupil_grid(grid);
    let v = lensbench_core::mech::vignetting(s, &s.mechanics, fields, waves, &px, &py)
        .map_err(|e| e.to_string())?;
    let pct = |n: usize, d: usize| {
        if d == 0 {
            0.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };
    let parts: Vec<Value> = s
        .mechanics
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let worst = v.iter().map(|r| pct(r.clipped[k], r.launched)).fold(0.0, f64::max);
            let first = v.iter().map(|r| pct(r.first[k], r.launched)).fold(0.0, f64::max);
            json!({
                "name": p.name, "surface": p.surface,
                "clipped_max_pct": worst, "first_max_pct": first,
                "bores": p.bores.iter().map(|b| json!({"z0": b.z0, "z1": b.z1, "diameter": 2.0 * b.radius})).collect::<Vec<_>>(),
            })
        })
        .collect();
    let rows: Vec<Value> = v
        .iter()
        .map(|r| {
            json!({
                "field": r.field + 1, "wave": r.wave + 1,
                "rays": r.launched, "failed": r.failed,
                "optical_pct": pct(r.optical, r.launched),
                "passed_pct": pct(r.passed, r.launched),
                "parts": s.mechanics.iter().enumerate().map(|(k, p)| json!({
                    "name": p.name,
                    "clipped_pct": pct(r.clipped[k], r.launched),
                    "first_pct": pct(r.first[k], r.launched),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let fit: Vec<Value> = lensbench_core::mech::fit_check(s, &s.mechanics)
        .map_err(|e| e.to_string())?
        .iter()
        .map(|f| {
            json!({
                "kind": f.kind, "error": f.error,
                "part": s.mechanics[f.part].name,
                "other": f.other.map(|o| s.mechanics[o].name.clone()),
                "surface": f.surface, "value": f.value, "message": f.message,
            })
        })
        .collect();
    Ok(json!({ "pupil_rays": px.len(), "parts": parts, "results": rows, "fit": fit }))
}

fn print_out(json_out: bool, out: &Value) {
    if out.is_null() {
        return;
    }
    if json_out {
        println!("{}", serde_json::to_string_pretty(out).unwrap());
    } else {
        print_text(&brief(&tolerance_brief(out)), 0);
    }
}

/// 公差の結果（`merit_values` を持つ）を画面用に要点だけにする。--save・serve・GUI には全体を渡す
fn tolerance_brief(out: &Value) -> Value {
    let mut out = out.clone();
    let Some(o) = out.as_object_mut() else {
        return out;
    };
    if !o.contains_key("merit_values") {
        return out;
    }
    o.remove("cdf");
    o.remove("tolerances");
    o.remove("merit_values");
    for k in ["compensators", "tilt_magnitudes"] {
        if let Some(Value::Array(a)) = o.get_mut(k) {
            for t in a {
                if let Some(t) = t.as_object_mut() {
                    t.remove("values");
                }
            }
        }
    }
    for k in ["sensitivity", "correlation"] {
        if let Some(Value::Array(a)) = o.get_mut(k) {
            a.truncate(10);
        }
    }
    if let Some(Value::Array(a)) = o.get_mut("terms") {
        for t in a {
            if let Some(t) = t.as_object_mut() {
                t.remove("cdf");
            }
        }
    }
    out
}

/// 解析のコマンドを 1 つ実行して結果を返す（--save があれば書き出す）。serve・watch もこれを使う
fn execute(cmd: Cmd) -> Result<(bool, Value), String> {
    execute_in(cmd, &ext::Ctx::default())
}

/// [`execute`] を実行の場（serve の中か。拡張のサブコマンドに渡す）を与えて
fn execute_in(cmd: Cmd, ctx: &ext::Ctx) -> Result<(bool, Value), String> {
    let e = |e: lensbench_core::Error| e.to_string();
    let save = cmd.save();
    LOADED_HASH.with(|x| *x.borrow_mut() = None);
    let (json_out, out): (bool, Value) = match cmd {
        Cmd::Serve(_) | Cmd::Watch(_) | Cmd::Run(_) => {
            return Err("serve・watch・run は入れ子にできない".into())
        }
        // 記述の中のパス（基準データ・系）を serve の --root で確かめられないので、コマンドラインからだけ
        Cmd::Verify { .. } => return Err("verify はコマンドラインからだけ使える".into()),
        Cmd::Plugins { json } => (json, plugins_json()),
        Cmd::Ext(e) => e.args.run(ctx)?,
        Cmd::Zoom(a) => (a.json, search_cmd::zoom(&a)?),
        Cmd::Swap(a) => (a.json, search_cmd::swap(&a)?),
        Cmd::Fringe(a) => {
            let ld = |b: &str| load_beam(&a, b);
            (true, fringe_cmd::run(&a, &ld)?)
        }
        Cmd::Info(c) => {
            let s = load(&c)?;
            let d = report::system_data(&s);
            (
                c.json,
                json!({
                    "name": s.name, "wavelengths_um": s.wavelengths, "primary": s.primary + 1,
                    "fields": s.fields.iter().map(|f| [f.x, f.y]).collect::<Vec<_>>(),
                    "effl": d.effl, "epd": d.epdi, "enpp": d.enpp, "exp_diameter": d.expd,
                    "expp": d.expp, "wfno": d.wfno, "paraxial_wfno": d.pwfn, "image_fno": d.isfn,
                    "paraxial_image_height": d.pimh, "paraxial_mag": d.pmag,
                    "total_track": d.totr, "object_na": d.obna, "image_na": d.isna,
                    "states": state_names(&c), "state": c.state,
                    "paths": split_paths(&c)?, "path": c.path,
                    "lensbench": lensbench_core::VERSION,
                    "system_hash": lensbench_project::sysfile::system_hash(&s)?,
                }),
            )
        }
        Cmd::Convert { c, out } => {
            let s = load(&c)?;
            let zmx = out
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("zmx"));
            if zmx {
                zmx_write::save(&out, &s).map_err(e)?;
            } else {
                lensbench_project::sysfile::save(&out, &s, &[])?;
            }
            (
                c.json,
                json!({
                    "written": out.display().to_string(),
                    "lensbench": lensbench_core::VERSION,
                    "system_hash": lensbench_project::sysfile::system_hash(&s)?,
                }),
            )
        }
        Cmd::Zmx { c, sel, out } => {
            let systems = overlay_systems(&c, &sel)?;
            let v: Vec<System> = systems.iter().map(|x| x.1.clone()).collect();
            zmx_write::save_configs(&out, &v).map_err(e)?;
            (
                c.json,
                json!({
                    "file": out.display().to_string(),
                    "configs": systems.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
                }),
            )
        }
        Cmd::Elements { c, reverse, out } => {
            let s = load(&c)?;
            let mut els = elements::split(&s).map_err(e)?;
            let lenses: Vec<usize> = (0..els.len())
                .filter(|&i| els[i].kind == elements::Kind::Lens)
                .collect();
            if let Some(r) = reverse {
                for t in r.split(',') {
                    let k: usize = t.trim().parse().map_err(|_| format!("--reverse の {t}"))?;
                    let i = *lenses
                        .get(k.wrapping_sub(1))
                        .ok_or_else(|| format!("玉 {k} はありません（1〜{}）", lenses.len()))?;
                    els[i] = elements::reverse(&els[i]).map_err(e)?;
                }
            }
            let a = elements::assemble(&s, &els).map_err(e)?;
            if let Some(p) = &out {
                zmx_write::save(p, &a).map_err(e)?;
            }
            let w = s.primary;
            let mut nl = 0;
            let v: Vec<Value> = els
                .iter()
                .map(|x| {
                    let kind = match x.kind {
                        elements::Kind::Lens => "lens",
                        elements::Kind::Mirror => "mirror",
                        elements::Kind::CoordBreak => "coordinate_break",
                        elements::Kind::Surface => "surface",
                        elements::Kind::Air => "air",
                    };
                    let mut o = json!({ "kind": kind });
                    if x.kind == elements::Kind::Air {
                        o["thickness"] = json!(x.t);
                    } else {
                        o["surfaces"] = json!([x.from, x.from + x.surfaces.len() - 1]);
                    }
                    if x.kind == elements::Kind::Lens {
                        nl += 1;
                        o["lens"] = json!(nl);
                        o["materials"] = json!(x.materials());
                        o["length"] = json!(x.length());
                        o["efl"] = json!(x.efl(w));
                        o["reversed"] = json!(x.reversed);
                    }
                    o
                })
                .collect();
            (
                c.json,
                json!({ "elements": v, "efl": a.paraxial().efl, "written": out.map(|p| p.display().to_string()) }),
            )
        }
        Cmd::Spot { c, s: sel, rings } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let mut v = Vec::new();
            for i in fields(&s, &sel)? {
                let sp = analysis::spot(&s, FieldSel::Index(i), w, rings).map_err(e)?;
                v.push(json!({
                    "field": i + 1, "rms_centroid": sp.rms_centroid, "rms_chief": sp.rms_chief,
                    "geo": sp.geo, "centroid": [sp.centroid.0, sp.centroid.1],
                    "chief": [sp.chief.0, sp.chief.1],
                }));
            }
            (c.json, json!({ "unit": spot_unit(&s), "spot": v }))
        }
        Cmd::Wavefront { c, s: sel, n, map } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let mut v = Vec::new();
            for i in fields(&s, &sel)? {
                let wf = analysis::wavefront(&s, FieldSel::Index(i), w, n, false).map_err(e)?;
                let mut x = json!({ "field": i + 1, "pv": wf.pv, "rms": wf.rms });
                if map {
                    // NaN（瞳の外・けられ）は JSON の null になる
                    x["p"] = json!(wf.p);
                    x["map"] = json!(wf.w);
                }
                v.push(x);
            }
            (c.json, json!({ "unit": "waves", "wavefront": v }))
        }
        Cmd::Zernike {
            c,
            s: sel,
            kind,
            terms,
            n,
        } => {
            let k = match kind.to_ascii_lowercase().as_str() {
                "standard" => ZernKind::Standard,
                "fringe" => ZernKind::Fringe,
                _ => return Err(format!("--kind は standard か fringe（{kind}）")),
            };
            if k == ZernKind::Fringe && terms > 37 {
                return Err("Fringe は 37 項まで".into());
            }
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let mut v = Vec::new();
            for i in fields(&s, &sel)? {
                let z =
                    analysis::zernike(&s, FieldSel::Index(i), w, k, terms, n, false).map_err(e)?;
                v.push(json!({
                    "field": i + 1, "coef": z.coef, "pv": z.pv, "rms_chief": z.rms_chief,
                    "rms_centroid": z.rms_centroid, "rms_fit": z.rms_fit, "strehl": z.strehl,
                }));
            }
            (
                c.json,
                json!({ "unit": "waves", "kind": kind, "zernike": v }),
            )
        }
        Cmd::Layout {
            c,
            l,
            out,
            plane,
            width,
            segments,
            rings,
            tube,
            mech_detail,
        } => {
            let systems = overlay_systems(&c, &l.sel)?;
            let color: d2::ColorBy = match &l.color {
                Some(v) => v.parse()?,
                None if l.sel.any() => d2::ColorBy::Config,
                None => d2::ColorBy::Field,
            };
            let offset = match &l.offset {
                None => [0.0; 3],
                Some(v) => {
                    let x = parse_list(v, "--offset")?;
                    if x.len() != 3 {
                        return Err("--offset は dx,dy,dz の 3 つ".into());
                    }
                    [x[0], x[1], x[2]]
                }
            };
            let o2 = d2::Options2D {
                plane: plane.parse()?,
                color,
                width,
                segments,
            };
            let o3 = mesh::Options3D {
                segments,
                rings,
                tube,
                color,
                mech_detail,
            };
            // 拡張子を先に確かめ、2D 用と 3D 用の場面を要るものだけ作る
            let fmts = out
                .iter()
                .map(|p| {
                    lensbench_export::LayoutFormat::from_path(p).ok_or_else(|| {
                        format!(
                            "{}: 拡張子は svg・png・dxf・gltf・glb・obj・stl・step・stp のどれか",
                            p.display()
                        )
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            if l.unfold && fmts.iter().any(|f| !f.is_2d()) {
                return Err("--unfold は 2D（svg・png・dxf）だけ".into());
            }
            let mut scenes: [Option<scene::Scene>; 2] = [None, None];
            for f in &fmts {
                let k = usize::from(!f.is_2d());
                if scenes[k].is_none() {
                    let mut parts = Vec::new();
                    // 重ね描きでは描き始める面をそろえる（物体面から描けない構成があれば全部面 1 から）
                    let first0 = (l.first.is_none()
                        && systems.iter().any(|x| scene::auto_first(&x.1) != 0))
                    .then_some(1);
                    for (name, sy) in &systems {
                        let mut so = scene_options(sy, &l, f.is_2d())?;
                        so.first = so.first.or(first0);
                        let sc = scene::build(sy, &so).map_err(|x| {
                            if name.is_empty() {
                                e(x)
                            } else {
                                format!("{name}: {x}")
                            }
                        })?;
                        parts.push((name.clone(), sc));
                    }
                    scenes[k] = Some(scene::overlay(parts, offset).map_err(e)?);
                }
            }
            let mut written = Vec::new();
            for (p, f) in out.iter().zip(&fmts) {
                let sc = scenes[usize::from(!f.is_2d())].as_ref().unwrap();
                lensbench_export::write_layout(p, sc, &o2, &o3)?;
                written.push(json!({ "file": p.display().to_string(), "format": format!("{f:?}").to_lowercase() }));
            }
            let sc = scenes.iter().flatten().next().unwrap();
            let (lo, hi) = sc.bounds();
            (
                c.json,
                json!({
                    "written": written,
                    "configs": sc.configs,
                    "solids": sc.solids.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
                    "surfaces": sc.plates.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
                    "mechanics": sc.mechs.iter().map(|x| x.name.clone()).collect::<Vec<_>>(),
                    "rays_2d": scenes[0].as_ref().map(|x| x.rays.len()),
                    "rays_3d": scenes[1].as_ref().map(|x| x.rays.len()),
                    "bounds_mm": [lo, hi],
                }),
            )
        }
        Cmd::MechCheck {
            c,
            field,
            wave,
            grid,
        } => {
            let s = load(&c)?;
            let fields = match field.as_deref() {
                Some(v) => index_list(v, s.fields.len(), "視野")?,
                None => (0..s.fields.len()).collect(),
            };
            let waves = match wave.as_deref() {
                Some(v) => index_list(v, s.wavelengths.len(), "波長")?,
                None => (0..s.wavelengths.len()).collect(),
            };
            (c.json, mech_report(&s, &fields, &waves, grid)?)
        }
        Cmd::Rays {
            c,
            field,
            wave,
            fan,
            rays,
            pupil,
            surface,
        } => {
            let s = load(&c)?;
            let (px, py) = match &pupil {
                Some(p) => parse_pupil(p)?,
                None => scene::pupil(fan.parse()?, rays),
            };
            let img = s.image();
            let o = table::RayOptions {
                fields: field
                    .as_deref()
                    .map(|v| index_list(v, s.fields.len(), "視野"))
                    .transpose()?,
                waves: wave
                    .as_deref()
                    .map(|v| index_list(v, s.wavelengths.len(), "波長"))
                    .transpose()?,
                px,
                py,
                surfaces: surface
                    .as_deref()
                    .map(|v| {
                        v.split(',')
                            .map(|t| {
                                let j: usize =
                                    t.trim().parse().map_err(|_| format!("面の番号 {t}"))?;
                                if j == 0 || j > img {
                                    return Err(format!("面 {j} は書けない（1〜{img}）"));
                                }
                                Ok(j)
                            })
                            .collect::<Result<Vec<_>, String>>()
                    })
                    .transpose()?,
            };
            let t = table::rays(&s, &o).map_err(e)?;
            match &c.save {
                Some(p) => {
                    t.save(p)?;
                    (
                        c.json,
                        json!({ "written": p.display().to_string(), "rows": t.rows() }),
                    )
                }
                None if c.json => (true, t.to_json()),
                None => {
                    let mut o = std::io::stdout().lock();
                    t.write_csv(&mut o).map_err(|x| x.to_string())?;
                    // 表は書き出し済み（print_out は何も出さない）
                    return Ok((false, Value::Null));
                }
            }
        }
        Cmd::Ghost {
            c,
            first,
            last,
            coatings,
            out_dir,
        } => {
            let s = load(&c)?;
            let last = last.unwrap_or(s.image());
            let mut co = Coatings::for_system(&s).map_err(e)?;
            for p in &coatings {
                co.load_file(p).map_err(e)?;
            }
            let v = lensbench_core::ghost::survey(&s, first, last, &co).map_err(e)?;
            let mut written = Vec::new();
            if let Some(dir) = &out_dir {
                lensbench_core::vfs::create_dir_all(dir)
                    .map_err(|x| format!("{}: {x}", dir.display()))?;
                for g in &v {
                    let gs =
                        lensbench_core::ghost::ghost_system(&s, g.par.a, g.par.b).map_err(e)?;
                    let p = dir.join(format!("ghost_{}_{}.zmx", g.par.a, g.par.b));
                    zmx_write::save(&p, &gs.sys).map_err(e)?;
                    written.push(p.display().to_string());
                }
            }
            let rows: Vec<Value> = v
                .iter()
                .map(|g| {
                    let p = &g.par;
                    let mut o = json!({
                        "a": p.a, "b": p.b, "marginal_y": p.marginal_y, "marginal_u": p.marginal_u,
                        "focus": if p.focus.is_finite() { json!(p.focus) } else { Value::Null },
                        "wfno": p.wfno,
                    });
                    match &g.intensity {
                        Ok(x) => o["intensity"] = json!(x),
                        Err(m) => o["intensity_error"] = json!(m),
                    }
                    o
                })
                .collect();
            (
                c.json,
                json!({ "first": first, "last": last, "ghosts": rows, "written": written }),
            )
        }
        Cmd::Mtf {
            c,
            s: sel,
            freqs,
            max_freq,
            freq_steps,
            n,
            direct,
            fft,
            interp,
            each_wave,
        } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            if each_wave && w.is_some() {
                return Err("--each-wave は --wave と一緒に使えない".into());
            }
            let given: Option<Vec<f64>> = match freqs {
                Some(f) => Some(
                    f.split(',')
                        .map(|x| {
                            x.trim()
                                .parse::<f64>()
                                .map_err(|_| format!("--freqs の数が読めない（{x}）"))
                        })
                        .collect::<Result<_, _>>()?,
                ),
                None => None,
            };
            let mut v = Vec::new();
            for i in fields(&s, &sel)? {
                let fs = match &given {
                    Some(f) => f.clone(),
                    None => {
                        let top = match max_freq {
                            Some(x) => x,
                            None => {
                                let m = analysis::mtf(&s, FieldSel::Index(i), w, Some(4), &[])
                                    .map_err(e)?;
                                0.95 * m.cutoff.0.min(m.cutoff.1)
                            }
                        };
                        auto_freqs(top, freq_steps)?
                    }
                };
                let method = if direct {
                    analysis::MtfMethod::Direct
                } else if fft {
                    analysis::MtfMethod::Fft
                } else if interp {
                    analysis::MtfMethod::Interp
                } else {
                    analysis::MtfMethod::Auto
                };
                let f = FieldSel::Index(i);
                let (m, waves) = if each_wave {
                    let mw = analysis::mtf_waves(&s, f, n, &fs, method).map_err(e)?;
                    (mw.poly, Some(mw.waves))
                } else {
                    (
                        analysis::mtf_with(&s, f, w, n, &fs, method).map_err(e)?,
                        None,
                    )
                };
                let mut o = json!({
                    "field": i + 1, "freqs": m.freqs, "tangential": m.tan, "sagittal": m.sag,
                    "cutoff": [m.cutoff.0, m.cutoff.1],
                });
                if let Some(ws) = waves {
                    o["waves"] = ws
                        .iter()
                        .enumerate()
                        .map(|(k, m)| json!({ "wave": k + 1, "tangential": m.tan, "sagittal": m.sag }))
                        .collect();
                }
                v.push(o);
            }
            (
                c.json,
                json!({ "unit": format!("cycles/{}", s.image_unit()), "mtf": v }),
            )
        }
        Cmd::MtfFocus {
            c,
            s: sel,
            freqs,
            range,
            steps,
            n,
        } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let fs = parse_list(&freqs, "--freqs")?;
            let dz = sweep::steps_sym(range, steps);
            let mut v = Vec::new();
            let mut warn = Vec::new();
            for i in fields(&s, &sel)? {
                let m = sweep::through_focus_mtf(&s, FieldSel::Index(i), w, n, &fs, &dz);
                for (k, &f) in fs.iter().enumerate() {
                    for (j, &z) in dz.iter().enumerate() {
                        v.push(json!({
                            "field": i + 1, "freq": f, "dz": z,
                            "tangential": m.tan[k][j], "sagittal": m.sag[k][j],
                        }));
                    }
                }
                warn.extend(m.warnings.iter().map(|x| format!("視野 {}: {x}", i + 1)));
            }
            (
                c.json,
                json!({ "unit": format!("cycles/{}", s.image_unit()), "through_focus": v, "warnings": warn }),
            )
        }
        Cmd::MtfField {
            c,
            wave: wv,
            freqs,
            points,
            axis,
            n,
        } => {
            let s = load(&c)?;
            let w = wave(
                &s,
                &Sel {
                    field: None,
                    wave: wv,
                },
            )?;
            let fs = parse_list(&freqs, "--freqs")?;
            let ax = parse_axis(&axis)?;
            let h = sweep::steps01(points);
            let m = sweep::field_mtf(&s, ax, &h, w, n, &fs);
            let mut v = Vec::new();
            for (k, &f) in fs.iter().enumerate() {
                for (j, &hh) in h.iter().enumerate() {
                    v.push(json!({
                        "freq": f, "h": hh, "field_value": ax.value(&s, hh),
                        "tangential": m.tan[k][j], "sagittal": m.sag[k][j],
                    }));
                }
            }
            (
                c.json,
                json!({ "unit": format!("cycles/{}", s.image_unit()), "field_mtf": v, "warnings": m.warnings }),
            )
        }
        Cmd::Optimize {
            c,
            vars,
            merit,
            terms,
            terms_file,
            reference,
            rings,
            iters,
            jac,
            out,
        } => {
            use lensbench_core::optim;
            let s = load(&c)?;
            let mut p = optim::Problem::single(s.clone());
            for v in &vars {
                let (spec, range) = match v.split_once('=') {
                    Some((a, b)) => (a.trim(), Some(b)),
                    None => (v.trim(), None),
                };
                let param = s.param(spec).map_err(e)?;
                let mut var = optim::Var::new(param);
                if let Some(r) = range {
                    let (lo, hi) = r
                        .split_once(':')
                        .ok_or_else(|| format!("--var {v} の範囲は 下限:上限"))?;
                    let num = |t: &str, d: f64| -> Result<f64, String> {
                        if t.trim().is_empty() {
                            Ok(d)
                        } else {
                            t.trim()
                                .parse()
                                .map_err(|_| format!("--var {v} の範囲 {t}"))
                        }
                    };
                    var = var.bounds(num(lo, f64::NEG_INFINITY)?, num(hi, f64::INFINITY)?);
                }
                p.vars.push(var);
            }
            let kind = match merit.to_ascii_lowercase().as_str() {
                "spot" => Some(optim::DefaultMerit::Spot),
                "wavefront" | "wfe" => Some(optim::DefaultMerit::Wavefront),
                "none" => None,
                m => return Err(format!("--merit {m}（spot・wavefront・none）")),
            };
            let reference = match reference.to_ascii_lowercase().as_str() {
                "centroid" => sweep::RmsRef::Centroid,
                "chief" => sweep::RmsRef::Chief,
                r => return Err(format!("--ref {r}（centroid・chief）")),
            };
            p.jac = optim::JacMode::parse(&jac)
                .ok_or_else(|| format!("--jac {jac}（{}）", optim::JacMode::NAMES))?;
            if let Some(kind) = kind {
                p.add_default(kind, rings, reference);
            }
            let n_default = p.terms.len();
            let mut specs: Vec<Value> = Vec::new();
            for t in &terms {
                specs.push(serde_json::from_str(t).map_err(|x| format!("--term {t}: {x}"))?);
            }
            if let Some(f) = &terms_file {
                let txt = lensbench_core::vfs::read_to_string(f)
                    .map_err(|x| format!("{}: {x}", f.display()))?;
                let v: Value =
                    serde_json::from_str(&txt).map_err(|x| format!("{}: {x}", f.display()))?;
                match v {
                    Value::Array(a) => specs.extend(a),
                    v => specs.push(v),
                }
            }
            // serve の中（--root・--allow-remote）なら zplm の file・macro_dirs の場所を確かめる
            for v in specs.iter_mut() {
                serve::check_term_paths(v).map_err(|x| x.msg)?;
            }
            for (i, v) in specs.iter().enumerate() {
                let t = lensbench_project::merit::term_from_json(v, &p.configs)
                    .map_err(|x| format!("項 {}: {x}", i + 1))?;
                p.terms.push(t);
            }
            p.check().map_err(e)?;
            let o = optim::DlsOpts {
                max_iter: iters,
                ..Default::default()
            };
            let json = c.json;
            let r = optim::dls_with(&p, &o, |pr| {
                if !json {
                    eprintln!("反復 {:>3}: 評価関数 {:.9e}", pr.iter, pr.merit);
                }
                true
            })
            .map_err(e)?;
            if let Some(path) = &out {
                zmx_write::save(path, &r.systems[0]).map_err(e)?;
            }
            let stop = match r.stop {
                optim::Stop::Converged => "converged",
                optim::Stop::SmallStep => "small_step",
                optim::Stop::NoProgress => "no_progress",
                optim::Stop::MaxIter => "max_iter",
                optim::Stop::Cancelled => "cancelled",
                optim::Stop::NoVars => "no_vars",
            };
            let vs: Vec<Value> = p
                .vars
                .iter()
                .enumerate()
                .map(|(q, v)| json!({ "name": s.param_name(v.param), "x0": r.x0[q], "x": r.x[q] }))
                .collect();
            // 足した項の値（最適化の前と後）
            let tv0 = p.term_values(&r.x0).map_err(e)?;
            let tv = p.term_values(&r.x).map_err(e)?;
            let ts: Vec<Value> = specs
                .iter()
                .enumerate()
                .map(|(i, v)| json!({ "term": v, "value0": tv0[n_default + i], "value": tv[n_default + i] }))
                .collect();
            (
                c.json,
                json!({
                    "merit": merit, "merit0": r.merit0, "merit_final": r.merit, "iterations": r.iterations,
                    "evals": r.evals, "stop": stop, "vars": vs, "terms": ts, "history": r.history,
                }),
            )
        }
        Cmd::Tolerance {
            c,
            spec,
            set,
            trials,
            seed,
            save_worst,
            comp_csv,
        } => {
            let s = load(&c)?;
            let mut v = match &spec {
                Some(f) => {
                    let txt = lensbench_core::vfs::read_to_string(f)
                        .map_err(|x| format!("{}: {x}", f.display()))?;
                    serde_json::from_str::<Value>(&txt)
                        .map_err(|x| format!("{}: {x}", f.display()))?
                }
                None => json!({}),
            };
            for t in &set {
                let o: Value = serde_json::from_str(t).map_err(|x| format!("--set {t}: {x}"))?;
                let Value::Object(o) = o else {
                    return Err(format!("--set {t} はオブジェクト"));
                };
                let m = v.as_object_mut().ok_or("設定はオブジェクト")?;
                m.extend(o);
            }
            if let Some(n) = trials {
                v["trials"] = json!(n);
            }
            if let Some(n) = seed {
                v["seed"] = json!(n);
            }
            if let Some(t) = v.get_mut("terms") {
                serve::check_term_paths(t).map_err(|x| x.msg)?;
            }
            let (vendors, reversed) = tolerance_vendors(&c);
            let ms = lensbench_project::tolerance::spec_from_json_with(
                &v,
                vec![s],
                &vendors,
                &reversed,
            )?;
            let oo = lensbench_project::tolerance::out_opts(&v)?;
            let json_out = c.json;
            let step = (ms.trials + 2 * ms.tols.len()).div_ceil(20).max(1);
            let run = || {
                lensbench_core::tolerance::run_with(&ms, |k, n| {
                    if !json_out && (k % step == 0 || k == n) {
                        eprintln!("{k} / {n}");
                    }
                    true
                })
            };
            let r = match oo.threads {
                Some(t) => lensbench_core::with_threads(Some(t), run),
                None => run(),
            }
            .map_err(e)?;
            if let Some(path) = &save_worst {
                let worst = r
                    .trials
                    .iter()
                    .filter_map(|t| t.eval.as_ref().map(|x| (x.merit, t)))
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                let Some((_, t)) = worst else {
                    return Err("--save-worst: 追跡できた試行がない".into());
                };
                let cs = lensbench_core::tolerance::perturbed(&ms, &t.deltas).map_err(e)?;
                let mut p = ms.problem.clone();
                p.configs = cs;
                let x = &t.eval.as_ref().unwrap().comp;
                let sys = p.systems(x).map_err(e)?;
                // .json は系ファイル（屈折率のずれ dn・disp も書ける）、ほかは .zmx
                if path
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("json"))
                {
                    lensbench_project::sysfile::save(path, &sys[0], &[])?;
                } else {
                    zmx_write::save(path, &sys[0]).map_err(|x| {
                        format!("--save-worst: {x}（.json の系ファイルなら書ける）")
                    })?;
                }
            }
            let out = lensbench_project::tolerance::result_json(&ms, &r, &oo);
            if let Some(path) = &comp_csv {
                let csv = lensbench_project::tolerance::compensators_csv(&out)?;
                lensbench_core::vfs::write(path, csv)
                    .map_err(|x| format!("--comp-csv {}: {x}", path.display()))?;
            }
            (json_out, out)
        }
        Cmd::Rms {
            c,
            s: sel,
            vs,
            points,
            range,
            axis,
            reference,
            rings,
            field_wave,
        } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let o = sweep::RmsOpts {
                rings,
                field_wave,
                reference: match reference.to_ascii_lowercase().as_str() {
                    "centroid" => sweep::RmsRef::Centroid,
                    "chief" => sweep::RmsRef::Chief,
                    _ => return Err(format!("--ref は centroid か chief（{reference}）")),
                },
            };
            let row = |x: (&str, f64), f: Option<usize>, r: &sweep::Rms| {
                let mut o = json!({
                    "spot": r.spot, "spot_x": r.spot_x, "spot_y": r.spot_y,
                    "wfe": r.wfe, "strehl": r.strehl,
                });
                o[x.0] = json!(x.1);
                if let Some(f) = f {
                    o["field"] = json!(f + 1);
                }
                o
            };
            let mut v = Vec::new();
            let mut warn = Vec::new();
            match vs.to_ascii_lowercase().as_str() {
                "field" => {
                    if sel.field.is_some() {
                        return Err("--vs field では --field は使わない".into());
                    }
                    let ax = parse_axis(&axis)?;
                    let r = sweep::rms_vs_field(&s, ax, &sweep::steps01(points), w, o);
                    for (x, q) in r.x.iter().zip(&r.rms) {
                        let mut e = row(("h", *x), None, q);
                        e["field_value"] = json!(ax.value(&s, *x));
                        v.push(e);
                    }
                    warn = r.warnings;
                }
                "focus" => {
                    for i in fields(&s, &sel)? {
                        let r = sweep::rms_vs_focus(
                            &s,
                            FieldSel::Index(i),
                            &sweep::steps_sym(range, points),
                            w,
                            o,
                        );
                        for (x, q) in r.x.iter().zip(&r.rms) {
                            v.push(row(("dz", *x), Some(i), q));
                        }
                        warn.extend(r.warnings.iter().map(|x| format!("視野 {}: {x}", i + 1)));
                    }
                }
                "wavelength" => {
                    if sel.wave.is_some() {
                        return Err("--vs wavelength では --wave は使わない".into());
                    }
                    let lams = sweep::wavelength_steps(&s, points);
                    for i in fields(&s, &sel)? {
                        let r = sweep::rms_vs_wavelength(&s, FieldSel::Index(i), &lams, o);
                        for (x, q) in r.x.iter().zip(&r.rms) {
                            v.push(row(("wavelength", *x), Some(i), q));
                        }
                        warn.extend(r.warnings.iter().map(|x| format!("視野 {}: {x}", i + 1)));
                    }
                }
                _ => return Err(format!("--vs は field・focus・wavelength（{vs}）")),
            }
            (
                c.json,
                json!({ "unit": spot_unit(&s), "wfe_unit": "waves", "vs": vs, "reference": reference, "rms": v, "warnings": warn }),
            )
        }
        Cmd::Psf {
            c,
            s: sel,
            huygens,
            pupil,
            image,
            delta,
        } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let o = psf::PsfOpts {
                pupil,
                image,
                delta,
            };
            let ho = huygens::HuygensOpts {
                pupil,
                image,
                delta,
            };
            let (mut v, mut axes) = (Vec::new(), Vec::new());
            for i in fields(&s, &sel)? {
                let p = if huygens {
                    huygens::huygens_psf(&s, FieldSel::Index(i), w, ho)
                } else {
                    psf::psf(&s, FieldSel::Index(i), w, o)
                }
                .map_err(e)?;
                axes.push(json!({ "field": i + 1, "center": [p.center.0, p.center.1], "e1": p.axes[0], "e2": p.axes[1] }));
                for (iy, row) in p.i.iter().enumerate() {
                    for (ix, val) in row.iter().enumerate() {
                        v.push(json!({ "field": i + 1, "x": p.x[ix], "y": p.y[iy], "i": val }));
                    }
                }
                eprintln!(
                    "視野 {}: ピーク {:.6e}、中心 ({:.6e}, {:.6e})、間隔 ({:.6e}, {:.6e})",
                    i + 1,
                    p.peak,
                    p.center.0,
                    p.center.1,
                    p.delta.0,
                    p.delta.1
                );
            }
            // x・y は主光線に垂直な面の座標（e1・e2 の向き、OpticStudio の FFT PSF と同じ）。
            // Huygens では像面の局所座標（e1・e2 = x̂・ŷ）
            let method = if huygens { "huygens" } else { "fft" };
            (
                c.json,
                json!({ "unit": spot_unit(&s), "method": method, "psf": v, "axes": axes }),
            )
        }
        Cmd::GeoMtf {
            c,
            s: sel,
            freqs,
            max_freq,
            freq_steps,
            n,
            diff_limit,
        } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let given = freqs.map(|f| parse_list(&f, "--freqs")).transpose()?;
            let mut v = Vec::new();
            for i in fields(&s, &sel)? {
                let fs = match &given {
                    Some(f) => f.clone(),
                    None => {
                        let top = match max_freq {
                            Some(x) => x,
                            None => {
                                let m = analysis::mtf(&s, FieldSel::Index(i), w, Some(4), &[])
                                    .map_err(e)?;
                                0.95 * m.cutoff.0.min(m.cutoff.1)
                            }
                        };
                        auto_freqs(top, freq_steps)?
                    }
                };
                let m =
                    psf::geometric_mtf(&s, FieldSel::Index(i), w, n, &fs, diff_limit).map_err(e)?;
                v.push(json!({
                    "field": i + 1, "freqs": m.freqs, "tangential": m.tan, "sagittal": m.sag,
                    "cutoff": [m.cutoff.0, m.cutoff.1],
                }));
            }
            (
                c.json,
                json!({ "unit": format!("cycles/{}", s.image_unit()), "geometric_mtf": v }),
            )
        }
        Cmd::HuygensMtf {
            c,
            s: sel,
            freqs,
            max_freq,
            freq_steps,
            pupil,
            image,
            delta,
        } => {
            let s = load(&c)?;
            let w = wave(&s, &sel)?;
            let given = freqs.map(|f| parse_list(&f, "--freqs")).transpose()?;
            let o = huygens::HuygensOpts {
                pupil,
                image,
                delta,
            };
            let mut v = Vec::new();
            for i in fields(&s, &sel)? {
                let fs = match &given {
                    Some(f) => f.clone(),
                    None => {
                        let top = match max_freq {
                            Some(x) => x,
                            None => {
                                let m = analysis::mtf(&s, FieldSel::Index(i), w, Some(4), &[])
                                    .map_err(e)?;
                                0.95 * m.cutoff.0.min(m.cutoff.1)
                            }
                        };
                        auto_freqs(top, freq_steps)?
                    }
                };
                let m = huygens::huygens_mtf(&s, FieldSel::Index(i), w, o, &fs).map_err(e)?;
                v.push(json!({
                    "field": i + 1, "freqs": m.freqs, "tangential": m.tan, "sagittal": m.sag,
                    "cutoff": [m.cutoff.0, m.cutoff.1],
                }));
            }
            (
                c.json,
                json!({ "unit": format!("cycles/{}", s.image_unit()), "method": "huygens", "mtf": v }),
            )
        }
        Cmd::CatalogAdd {
            catalog,
            name,
            from,
            lens,
            block,
            glass_dir,
            lens_catalog,
            cfg,
            description,
            replace,
            json,
        } => {
            let mut cat = lensbench_project::UserCatalog::load_or_new(&catalog)?;
            if cat.items.contains_key(&name) && !replace {
                return Err(format!("項目 {name} はもうある（置き換えは --replace）"));
            }
            if block.len() > 2 {
                return Err("--block は 1 つか 2 つ（範囲の最初と最後）".into());
            }
            let item = match (lens, block.is_empty()) {
                (Some(k), true) => {
                    let c = Common {
                        file: from.clone(),
                        glass_dir: glass_dir.clone(),
                        lens_catalog: lens_catalog.clone(),
                        cfg,
                        state: None,
                        path: None,
                        json,
                        save: None,
                    };
                    lensbench_project::lens_item(&load(&c)?, k, &description)?
                }
                (None, false) => {
                    let txt = lensbench_core::vfs::read_to_string(&from)
                        .map_err(|x| format!("{}: {x}", from.display()))?;
                    let p = lensbench_project::Project::from_json(&txt)?;
                    let cat = LensCatalog::load_registered(&lens_catalog)?;
                    let look = |v: &str, p: &str| cat.zmx(v, p);
                    let env = lensbench_project::Env {
                        base: from.parent().map(|x| x.to_path_buf()).unwrap_or_default(),
                        glass_dirs: glass_dir.clone(),
                        catalog: Some(&look),
                    };
                    lensbench_project::blocks_item(&p, &env, &block, &description)?
                }
                _ => return Err("--lens か --block のどちらか 1 つ".into()),
            };
            cat.items.insert(name.clone(), item);
            cat.save(&catalog)?;
            (
                json,
                json!({ "catalog": catalog, "added": name, "items": cat.items.keys().collect::<Vec<_>>() }),
            )
        }
        Cmd::CatalogSearch {
            lens_catalog,
            glass_dir,
            kind,
            vendor,
            text,
            spec,
            fits,
            list_specs,
            json,
        } => {
            use lensbench_project::catalog_spec::{CatalogQuery, ItemKind, SpecFilter, SPEC_NAMES};
            if list_specs {
                let v: Vec<Value> = SPEC_NAMES
                    .iter()
                    .map(|(n, u, d)| json!({ "name": n, "unit": u, "doc": d }))
                    .collect();
                if !json {
                    for (n, u, d) in SPEC_NAMES {
                        println!("{n:<18} {u:<7} {d}");
                    }
                    return Ok((false, Value::Null));
                }
                return Ok((true, json!({ "specs": v })));
            }
            let q = CatalogQuery {
                kinds: kind
                    .iter()
                    .map(|k| ItemKind::parse(k).ok_or_else(|| format!("--kind: 知らない種類 {k}")))
                    .collect::<Result<_, _>>()?,
                vendor,
                text,
                specs: spec
                    .iter()
                    .map(|s| SpecFilter::parse(s))
                    .collect::<Result<_, _>>()?,
                fits,
            };
            let v = browse::search(&lens_catalog, &glass_dir, &q)?;
            if !json {
                print_catalog_table(&v);
                return Ok((false, Value::Null));
            }
            (json, v)
        }
        Cmd::CatalogStep { files, json } => {
            let v: Vec<Value> = files
                .iter()
                .map(|f| {
                    let r = lensbench_core::vfs::read(f)
                        .map_err(|e| e.to_string())
                        .and_then(|b| {
                            lensbench_project::catalog_fill::read_step(&String::from_utf8_lossy(&b))
                        });
                    match r {
                        Ok(s) => {
                            let mut v = serde_json::to_value(&s).unwrap_or(Value::Null);
                            v["file"] = json!(f.display().to_string());
                            v
                        }
                        Err(e) => json!({ "file": f.display().to_string(), "error": e }),
                    }
                })
                .collect();
            if !json {
                for x in &v {
                    println!("{}", x["file"].as_str().unwrap_or(""));
                    match x["error"].as_str() {
                        Some(e) => println!("  誤り: {e}"),
                        None => println!(
                            "  厚さ {} mm、径 {}{}\n  {}",
                            x["thickness"],
                            x["diameter"],
                            if x["frame"].is_null() {
                                ""
                            } else {
                                "、枠あり"
                            },
                            x["note"].as_str().unwrap_or("")
                        ),
                    }
                }
                return Ok((false, Value::Null));
            }
            (true, json!({ "files": v }))
        }
        Cmd::CadStep {
            catalog,
            item,
            cache,
            json,
        } => {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (catalog, item, cache, json);
                return Err("cad-step は Web 版では使えない".into());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let cat = lensbench_project::UserCatalog::load(&catalog)?;
                let it = cat
                    .items
                    .get(&item)
                    .ok_or_else(|| format!("{} に品 {item} がない", catalog.display()))?;
                let dir = cache.unwrap_or_else(cad::default_cache_dir);
                let r = match cad::fetch_item(&dir, &item, it) {
                    Ok(r) => r,
                    Err(e) if it.url.is_empty() => return Err(e),
                    Err(e) => return Err(format!("{e}\n品ページ: {}", it.url)),
                };
                if !json {
                    println!(
                        "{}{}",
                        r.path.display(),
                        match r.source {
                            cad::Source::Cache => "（キャッシュ）",
                            cad::Source::Store => "（置き場から）",
                            cad::Source::Vendor => "",
                            cad::Source::VendorUploaded => "（置き場にも上げた）",
                        }
                    );
                    for n in &r.store_notes {
                        println!("  置き場: {n}");
                    }
                    if let Some(m) = &r.measured {
                        println!("  {}", m.note);
                    }
                    if r.notes.is_empty() {
                        println!("  簡易外形と合う");
                    }
                    for n in &r.notes {
                        println!("  食い違い: {n}");
                    }
                    return Ok((false, Value::Null));
                }
                (true, r.to_json())
            }
        }
        Cmd::CadUpload {
            cache,
            catalog,
            dir,
            item,
            file,
            overwrite,
            dry_run,
            json,
        } => {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (cache, catalog, dir, item, file, overwrite, dry_run, json);
                return Err("cad-upload は Web 版では使えない".into());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let store = cad::store().ok_or(
                    "置き場が無い（環境変数 LENSBENCH_CAD_STORE に口の URL、Access で守った口なら LENSBENCH_CAD_STORE_ID・LENSBENCH_CAD_STORE_SECRET にサービストークン）",
                )?;
                // （鍵、ファイル、品）
                let mut jobs: Vec<(String, PathBuf, String)> = Vec::new();
                let mut unmatched: Vec<PathBuf> = Vec::new();
                if let Some(c) = cache {
                    let d = c.unwrap_or_else(cad::default_cache_dir);
                    for f in cad::step_files(&d)? {
                        let n = f.to_string_lossy();
                        if n.ends_with(".step") || n.ends_with(".step.gz") {
                            jobs.push((cad::store_key(&f), f, String::new()));
                        }
                    }
                } else if let Some(catalog) = catalog {
                    let cat = cad::load_catalogs(&catalog)?;
                    let key_of = |name: &str| -> Result<String, String> {
                        let it = cat
                            .items
                            .get(name)
                            .ok_or_else(|| format!("{} に品 {name} がない", catalog.display()))?;
                        Ok(cad::store_key(&cad::cache_path(
                            std::path::Path::new(""),
                            name,
                            &it.cad.step,
                        )))
                    };
                    if let (Some(name), Some(f)) = (item, file) {
                        jobs.push((key_of(&name)?, f, name));
                    } else if let Some(d) = dir {
                        // 機構部品（mech）の品だけに当てる
                        let names = cat
                            .items
                            .iter()
                            .filter(|(_, it)| it.mech.is_some())
                            .map(|(k, _)| k.as_str());
                        let (got, miss) = cad::match_files(names, &cad::step_sources(&d)?);
                        unmatched = miss;
                        for (name, f) in got {
                            jobs.push((key_of(&name)?, f, name));
                        }
                    } else {
                        return Err("--catalog には --dir か --item・--file".into());
                    }
                } else {
                    return Err("--cache か --catalog".into());
                }
                let mut rows = Vec::new();
                for (k, f, name) in &jobs {
                    let u = cad::upload_file(&store, k, f, name, overwrite, dry_run);
                    if !json {
                        let what = if u.item.is_empty() {
                            String::new()
                        } else {
                            format!("{} ← ", u.item)
                        };
                        let st = match u.status {
                            "uploaded" => "上げた",
                            "exists" => "置き場にある",
                            "dry_run" => "（上げない）",
                            _ => "誤り",
                        };
                        println!(
                            "{st}: {what}{} → {}{}",
                            f.display(),
                            u.key,
                            if u.error.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", u.error)
                            }
                        );
                    }
                    rows.push(u);
                }
                let count = |s: &str| rows.iter().filter(|u| u.status == s).count();
                if !json {
                    for f in &unmatched {
                        println!("品に当たらない: {}", f.display());
                    }
                    println!(
                        "上げた {}・置き場にある {}・誤り {}・品に当たらない {}",
                        count("uploaded"),
                        count("exists"),
                        count("error"),
                        unmatched.len()
                    );
                    return Ok((false, Value::Null));
                }
                (
                    true,
                    json!({
                        "store": store.url,
                        "files": rows.iter().map(|u| u.to_json()).collect::<Vec<_>>(),
                        "unmatched": unmatched.iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
                        "uploaded": count("uploaded"), "exists": count("exists"), "errors": count("error"),
                    }),
                )
            }
        }
        Cmd::CoatUpload {
            dir,
            index,
            dry_run,
            json,
        } => {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (dir, index, dry_run, json);
                return Err("coat-upload は Web 版では使えない".into());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let store = cad::store();
                let rows =
                    coat_remote::upload_dir(&dir, index.as_deref(), store.as_ref(), dry_run)?;
                let n = |s: &str| rows.iter().filter(|r| r.2 == s).count();
                if !json {
                    for (name, key, st, e) in &rows {
                        if *st != "exists" {
                            println!(
                                "{st}: {name} → {key}{}",
                                if e.is_empty() {
                                    String::new()
                                } else {
                                    format!(": {e}")
                                }
                            );
                        }
                    }
                    println!(
                        "上げた {}・置き場にある {}・誤り {}（{} 膜）",
                        n("uploaded"),
                        n("exists"),
                        n("error"),
                        rows.len()
                    );
                    if n("error") > 0 {
                        return Err("上げられない膜がある".into());
                    }
                    return Ok((false, Value::Null));
                }
                (
                    true,
                    json!({
                        "files": rows.iter().map(|(name, key, st, e)| json!({"name": name, "key": key, "status": st, "error": e})).collect::<Vec<_>>(),
                        "uploaded": n("uploaded"), "exists": n("exists"), "errors": n("error"),
                    }),
                )
            }
        }
        Cmd::CoatFetch { dir, json } => {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (dir, json);
                return Err("coat-fetch は Web 版では使えない".into());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let rows = coat_remote::fetch_dir(&dir, cad::store().as_ref())?;
                let n = |s: &str| rows.iter().filter(|r| r.1 == s).count();
                if !json {
                    for (name, st, e) in &rows {
                        if *st == "error" {
                            println!("誤り: {name}: {e}");
                        }
                    }
                    println!(
                        "取った {}・あった {}・誤り {}",
                        n("fetched"),
                        n("exists"),
                        n("error")
                    );
                    return Ok((false, Value::Null));
                }
                (
                    true,
                    json!({
                        "files": rows.iter().map(|(name, st, e)| json!({"name": name, "status": st, "error": e})).collect::<Vec<_>>(),
                        "fetched": n("fetched"), "exists": n("exists"), "errors": n("error"),
                    }),
                )
            }
        }
        Cmd::CatalogList { catalog, json } => {
            let cat = lensbench_project::UserCatalog::load(&catalog)?;
            let v: Vec<Value> = cat
                .items
                .iter()
                .map(|(k, it)| json!({ "name": k, "description": it.description, "glass_catalogs": it.glass_catalogs, "blocks": it.blocks.len() }))
                .collect();
            (json, json!({ "items": v }))
        }
        Cmd::Coatings {
            file,
            glass_dir,
            table,
            json,
        } => (json, coatings(&file, &glass_dir, &table)?),
        Cmd::CoatKinks { tables, key, json } => (json, coat_kinks(&tables, &key)?),
        Cmd::CoatModel {
            tables,
            out,
            coat,
            kinks,
            tol_psi,
            note,
            json,
        } => (
            json,
            coat_model(
                &tables,
                &out,
                coat.as_deref(),
                kinks.as_deref(),
                tol_psi,
                note.as_deref(),
            )?,
        ),
        Cmd::CoatCompact { model, out, json } => (json, coat_compact(&model, &out)?),
        Cmd::CoatDesign {
            targets,
            name,
            materials,
            substrate,
            coating_file,
            glass_dir,
            layers,
            starts,
            d_max,
            slack,
            out,
            json,
        } => (
            json,
            coat_design(&CoatDesignArgs {
                targets: &targets,
                name: &name,
                materials: &materials,
                substrate: &substrate,
                coating_file: coating_file.as_deref(),
                glass_dir: &glass_dir,
                layers: &layers,
                starts,
                d_max,
                slack,
                out: out.as_deref(),
            })?,
        ),
        Cmd::Pop {
            c,
            surf,
            field,
            wave,
            dist,
            to,
            method,
            rays,
            n,
            half,
            no_aperture,
            out_n,
            out_half,
            center,
            grid,
        } => {
            let s = load(&c)?;
            // --surf を省くと、像面まで伝えられる最後の面（間は平面のダミー面だけで、距離が 0 でない面）から像面まで
            let (surf, to) = match surf {
                Some(j) => (j, to),
                None => {
                    let img = s.image();
                    let ok = |j: usize, t: usize| {
                        [1.0, -1.0]
                            .iter()
                            .any(|&dir| pop::distance_to(&s, j, t, dir).is_ok())
                    };
                    // 像面まで伝えられなければ（像面の手前に曲面など）、次の面まで伝えられる最後の面から次の面まで
                    let (j, t) = match (1..img).rev().find(|&j| ok(j, img)) {
                        Some(j) => (j, img),
                        None => (1..img)
                            .rev()
                            .find(|&j| ok(j, j + 1))
                            .map(|j| (j, j + 1))
                            .ok_or("--surf を指定する（次の面まで伝えられる面がない）")?,
                    };
                    (j, to.or((dist.is_none()).then_some(t)))
                }
            };
            let sel = Sel {
                field: Some(field),
                wave,
            };
            let fi = fields(&s, &sel)?[0];
            let w = self::wave(&s, &sel)?;
            let m = match method.as_str() {
                "auto" => pop::PopMethod::Auto,
                "fresnel" => pop::PopMethod::Fresnel,
                "angular" => pop::PopMethod::Angular,
                "direct" => pop::PopMethod::Direct,
                x => return Err(format!("--method {x}（auto・fresnel・angular・direct）")),
            };
            let center = center
                .as_deref()
                .map(|v| -> Result<(f64, f64), String> {
                    let p: Vec<f64> = v
                        .split(',')
                        .map(|t| t.trim().parse::<f64>().map_err(|_| format!("--center {v}")))
                        .collect::<Result<_, _>>()?;
                    match p[..] {
                        [x, y] => Ok((x, y)),
                        _ => Err(format!("--center は x,y（{v}）")),
                    }
                })
                .transpose()?;
            let o = pop::RayBeamOpts {
                rays,
                n,
                half,
                aperture: !no_aperture,
                overfill: None,
            };
            let rb = pop::beam_from_rays(&s, FieldSel::Index(fi), w, surf, &o).map_err(e)?;
            let b = &rb.beam;
            let d = match (dist, to) {
                (Some(d), None) => d,
                (None, t) => {
                    let t = t.unwrap_or(surf + 1);
                    pop::distance_to(&s, surf, t, b.dir).map_err(e)?
                }
                _ => return Err("--dist と --to はどちらか".into()),
            };
            let out = pop::OutGrid {
                n: out_n.map(|v| (v, v)),
                half: out_half.map(|v| (v, v)),
                center,
            };
            let p = pop::propagate(b, d, m, out).map_err(e)?;
            let pb = &p.beam;
            if p.kernel_step > PI / 2.0 {
                eprintln!(
                    "注意: 核の位相の標本の間の差が {:.2} rad（π/2 超え）。--n を増やすか --half・--out-half を狭める",
                    p.kernel_step
                );
            }
            if p.fresnel_error > 0.01 && p.method == pop::PopMethod::Fresnel {
                eprintln!(
                    "注意: Fresnel 近似で落とす位相が {:.2} rad。--method direct を使う",
                    p.fresnel_error
                );
            }
            let summary = |b: &pop::Beam| {
                let (cx, cy) = b.centroid();
                json!({
                    "n": [b.nx, b.ny], "dx": b.dx, "dy": b.dy, "center": [b.x0, b.y0],
                    "pilot": { "ax": b.pilot.ax, "ay": b.pilot.ay, "curv": b.pilot.curv },
                    "power": b.power(), "peak": b.peak(), "centroid": [cx, cy],
                    "rms_width": b.rms_width(), "phase_step": b.phase_step(),
                })
            };
            let row = |b: &pop::Beam, j: usize| -> Vec<f64> {
                (0..b.nx).map(|i| b.u[j * b.nx + i].norm_sqr()).collect()
            };
            let colm = |b: &pop::Beam, i: usize| -> Vec<f64> {
                (0..b.ny).map(|j| b.u[j * b.nx + i].norm_sqr()).collect()
            };
            let mut v = json!({
                "unit": "mm, W/mm² (power 1 in the pupil)",
                "surface": surf, "field": field, "wavelength_um": b.wl * 1e3, "index": b.index,
                "distance": d, "method": format!("{:?}", p.method).to_lowercase(), "rays_used": rb.used, "rays_traced": rb.traced,
                "aperture": rb.aperture.map(|a| format!("{a:?}")),
                "input": summary(b),
                "output": summary(pb),
                "kernel_step": p.kernel_step, "fresnel_error": p.fresnel_error,
                "x": (0..pb.nx).map(|i| pb.x(i)).collect::<Vec<_>>(),
                "irradiance_x": row(pb, pb.ny / 2),
                "y": (0..pb.ny).map(|j| pb.y(j)).collect::<Vec<_>>(),
                "irradiance_y": colm(pb, pb.nx / 2),
            });
            if grid {
                v["grid_irradiance"] = json!(pb.irradiance());
            }
            if let Some(path) = &c.save {
                let json_file = path
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("json"));
                if json_file {
                    v["grid"] = json!({
                        "irradiance": pb.irradiance(),
                        "phase": pb.u.iter().map(|c| c.arg()).collect::<Vec<_>>(),
                    });
                    save_value(path, &v)?;
                } else {
                    let mut t = table::Table::default();
                    let (mut xs, mut ys, mut ir, mut ph) = (vec![], vec![], vec![], vec![]);
                    for j in 0..pb.ny {
                        for i in 0..pb.nx {
                            xs.push(pb.x(i));
                            ys.push(pb.y(j));
                            ir.push(pb.u[j * pb.nx + i].norm_sqr());
                            ph.push(pb.u[j * pb.nx + i].arg());
                        }
                    }
                    t.cols.push(("x".into(), table::Col::F64(xs)));
                    t.cols.push(("y".into(), table::Col::F64(ys)));
                    t.cols.push(("irradiance".into(), table::Col::F64(ir)));
                    t.cols.push(("phase".into(), table::Col::F64(ph)));
                    t.meta.insert(
                        "note".into(),
                        json!("phase はパイロット（output.pilot）を除いた位相 [rad]"),
                    );
                    t.meta.insert("output".into(), v["output"].clone());
                    t.save(path)?;
                }
                v["written"] = json!(path.display().to_string());
            }
            (c.json, v)
        }
    };
    if let Some(p) = &save {
        save_value(p, &out)?;
    }
    Ok((json_out, out))
}

/// 膜の名前で自動で読んだ膜（膜, パス, 注記）
type AutoCoat = (String, String, String);

/// 系の膜の境界（[`coat_interfaces`]）
pub(crate) struct CoatInterfaces {
    /// 境界の並び（surface・key・kind・note）
    pub list: Vec<Value>,
    /// 実測表が要る境界の KEY
    pub needs: Vec<String>,
    /// 膜の名前で自動で読んだ膜
    pub auto: Vec<AutoCoat>,
}

/// 系 s の膜の境界（[`coatings`] の 1 構成分）
pub(crate) fn coat_interfaces(s: &System, tables: &[PathBuf]) -> Result<CoatInterfaces, String> {
    let e = |e: lensbench_core::Error| e.to_string();
    let mut c = Coatings::for_system(s).map_err(e)?;
    for t in tables {
        c.load_file(t).map_err(e)?;
    }
    let mut auto = Vec::new();
    for (coat, p) in &c.auto {
        let p = p.display().to_string();
        // 注記: モデルの NOTE、近似の多層膜（.lbcs）なら近似の膜であること
        let note = match c.models.get(coat) {
            Some(m) => m.note.clone(),
            None if p.to_ascii_lowercase().ends_with(".lbcs") => {
                "近似の膜（分光の曲線に合わせた多層膜）".to_string()
            }
            None => String::new(),
        };
        auto.push((coat.clone(), p, note));
    }
    let (keys, _) = coating::media_keys(s);
    let mut v = Vec::new();
    let mut need = Vec::new();
    for (j, k) in keys.iter().enumerate() {
        let Some(k) = k else { continue };
        let kind = c.kind(k);
        let mut it = json!({ "surface": j, "key": k, "kind": kind.name() });
        if kind == coating::CoatKind::Model {
            let coat = k.split('|').next().unwrap_or("");
            if let Some(m) = c.models.get(coat).filter(|m| !m.note.is_empty()) {
                it["note"] = json!(m.note);
            }
        }
        v.push(it);
        if kind.needs_table() && !need.contains(k) {
            need.push(k.clone());
        }
    }
    Ok(CoatInterfaces {
        list: v,
        needs: need,
        auto,
    })
}

fn coatings(
    file: &std::path::Path,
    glass_dir: &[PathBuf],
    tables: &[PathBuf],
) -> Result<Value, String> {
    let e = |e: lensbench_core::Error| e.to_string();
    let txt = glass::read_text(file).map_err(|x| format!("{}: {x}", file.display()))?;
    let raw = zmx::parse_text(&txt).map_err(e)?;
    let mut cfgs = Vec::new();
    // 実測表が要る境界 → (コンフィグ, 波長)
    let mut need: Vec<(String, Vec<usize>, Vec<f64>)> = Vec::new();
    let mut found = None;
    // 膜ファイルのフォルダから膜の名前で自動で読んだもの（膜, パス）
    let mut auto: Vec<AutoCoat> = Vec::new();
    for cfg in 1..=raw.configurations() {
        let opt = zmx::LoadOptions {
            cfg,
            catalog_dirs: glass_dir.to_vec(),
            ..Default::default()
        };
        let s = zmx::load_with(file, &opt).map_err(|x| format!("構成 {cfg}: {x}"))?;
        found = s.coating_file.clone();
        let CoatInterfaces {
            list: v,
            needs: keys,
            auto: au,
        } = coat_interfaces(&s, tables)?;
        for a in au {
            if !auto.iter().any(|b| b.0 == a.0 && b.1 == a.1) {
                auto.push(a);
            }
        }
        for k in keys {
            let i = match need.iter().position(|n| n.0 == k) {
                Some(i) => i,
                None => {
                    need.push((k, Vec::new(), Vec::new()));
                    need.len() - 1
                }
            };
            let n = &mut need[i];
            if !n.1.contains(&cfg) {
                n.1.push(cfg);
            }
            for &w in &s.wavelengths {
                if !n.2.contains(&w) {
                    n.2.push(w);
                }
            }
        }
        cfgs.push(json!({ "cfg": cfg, "wavelengths_um": s.wavelengths, "interfaces": v }));
    }
    Ok(json!({
        "coating_file": raw.coating_file_name(),
        "coating_file_path": found.map(|p| p.display().to_string()),
        "catalogs": raw.catalogs(),
        "auto_coatings": auto.iter().map(|(c, p, n)| json!({ "coat": c, "path": p, "note": n })).collect::<Vec<_>>(),
        "configurations": cfgs,
        "needs_table": need.iter().map(|(k, c, w)| json!({ "key": k, "cfgs": c, "wavelengths_um": w })).collect::<Vec<_>>(),
    }))
}

fn coat_kinks(tables: &[PathBuf], only: &[String]) -> Result<Value, String> {
    use lensbench_core::coat_fit::detect_kinks;
    let mut keys = Vec::new();
    // 膜ごとに、KEY で見つかった折れ目をまとめる（5e-4 µm 以内は同じ）
    let mut per: Vec<(String, Vec<f64>)> = Vec::new();
    for t in tables {
        let txt =
            lensbench_core::vfs::read_to_string(t).map_err(|e| format!("{}: {e}", t.display()))?;
        for (k, m) in coating::parse_measured(&txt).map_err(|e| format!("{}: {e}", t.display()))? {
            let air = k.split('|').nth(1) == Some("AIR");
            if (only.is_empty() && !air) || (!only.is_empty() && !only.contains(&k)) {
                continue;
            }
            let ks = detect_kinks(&m);
            let coat = k.split('|').next().unwrap_or("").to_string();
            let i = match per.iter().position(|(c, _)| *c == coat) {
                Some(i) => i,
                None => {
                    per.push((coat.clone(), Vec::new()));
                    per.len() - 1
                }
            };
            for kk in &ks {
                if !per[i].1.iter().any(|&w| (w - kk.wl).abs() < 5e-4) {
                    per[i].1.push(kk.wl);
                }
            }
            keys.push(json!({
                "key": k,
                "wavelengths": m.wavelengths.len(),
                "kinks": ks.iter().map(|k| json!({"wl_um": k.wl, "smooth": k.smooth, "kinked": k.kinked})).collect::<Vec<_>>(),
            }));
        }
    }
    let coats: Vec<Value> = per
        .into_iter()
        .map(|(c, mut w)| {
            w.sort_by(f64::total_cmp);
            // --kinks にそのまま渡せる形（0.1 nm に丸める）
            let arg: Vec<String> = w.iter().map(|x| format!("{:.4}", x)).collect();
            json!({ "coat": c, "kinks_um": w, "kinks_arg": arg.join(",") })
        })
        .collect();
    Ok(json!({ "coats": coats, "keys": keys }))
}

fn coat_model(
    tables: &[PathBuf],
    out: &std::path::Path,
    only: Option<&str>,
    kinks: Option<&str>,
    tol_psi: f64,
    note: Option<&str>,
) -> Result<Value, String> {
    let mut kinks: Vec<f64> = match kinks {
        None => Vec::new(),
        Some(k) => k
            .split(',')
            .map(|x| x.trim().parse::<f64>().map_err(|_| format!("--kinks: {x}")))
            .collect::<Result<_, _>>()?,
    };
    kinks.sort_by(f64::total_cmp);
    use lensbench_core::coat_fit::{fit_model, format_models, FitOptions};
    let mut all = Vec::new();
    for t in tables {
        let txt =
            lensbench_core::vfs::read_to_string(t).map_err(|e| format!("{}: {e}", t.display()))?;
        for (k, m) in coating::parse_measured(&txt).map_err(|e| format!("{}: {e}", t.display()))? {
            // 同じ KEY は後の表で置き換える（渡す順で結果が変わらないように、混ぜない）
            if let Some(i) = all.iter().position(|(a, _): &(String, _)| *a == k) {
                eprintln!("warning: {k}: {} の表で置き換えた", t.display());
                all.remove(i);
            }
            all.push((k, m));
        }
    }
    let mut coats: Vec<String> = Vec::new();
    for (k, _) in &all {
        let c = k.split('|').next().unwrap_or("").to_string();
        if !coats.contains(&c) {
            coats.push(c);
        }
    }
    if let Some(o) = only {
        let want: Vec<&str> = o.split(',').collect();
        coats.retain(|c| want.contains(&c.as_str()));
    }
    let opt = FitOptions {
        tol_psi,
        ..Default::default()
    };
    let mut models = Vec::new();
    let mut rep = Vec::new();
    for c in &coats {
        let t: Vec<(&str, &coating::Measured)> = all
            .iter()
            .filter(|(k, _)| k.split('|').next() == Some(c.as_str()))
            .map(|(k, m)| (k.as_str(), m))
            .collect();
        // モデルガラスの組がない膜などは飛ばして、ほかの膜は作る
        let r = match fit_model(&t, &opt) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("warning: {c}: 飛ばした（{e}）");
                rep.push(json!({ "coat": c, "skipped": e.to_string() }));
                continue;
            }
        };
        let mut r = r;
        r.model.kinks = kinks.clone();
        r.model.note = note.unwrap_or("").trim().to_string();
        // 折れ目の波長で測っていないと、そこをはさむ区間の端は外挿になる
        let unmeasured: Vec<f64> = kinks
            .iter()
            .copied()
            .filter(|&k| {
                let w = &r.model.wavelengths;
                !w.is_empty()
                    && (w[0] < k && k < w[w.len() - 1])
                    && !w.iter().any(|&x| (x - k).abs() < 1e-9)
            })
            .collect();
        if !unmeasured.is_empty() {
            eprintln!("warning: {c}: 折れ目 {unmeasured:?} um の波長で測っていない（その付近は外挿になる）");
        }
        let coarse = r.wave_gap > r.wave_gap_needed;
        if coarse {
            eprintln!(
                "warning: {c}: 波長の格子が粗い（波数の間隔 {:.4} > 目安 {:.4} [1/um]）。波長の間で係数がずれる",
                r.wave_gap, r.wave_gap_needed
            );
        }
        rep.push(json!({
            "coat": c,
            "d_um": r.model.d,
            "bmax": r.model.bmax,
            "mirror_d_um": r.model.mirror.as_ref().map(|m| m.d),
            "wavelengths": r.model.wavelengths.len(),
            "betas": r.model.betas.len(),
            "points": r.points,
            "residual_max": r.residual_max,
            "wave_gap": r.wave_gap,
            "wave_gap_needed": r.wave_gap_needed,
            "coarse": coarse,
            "kinks_unmeasured": unmeasured,
        }));
        models.push((c.clone(), r.model));
    }
    if models.is_empty() {
        return Err("作れた膜のモデルがない".into());
    }
    let e: Vec<(&str, &coating::CoatModel)> = models.iter().map(|(c, m)| (c.as_str(), m)).collect();
    lensbench_core::vfs::write(out, format_models(&e))
        .map_err(|x| format!("{}: {x}", out.display()))?;
    Ok(json!({ "out": out.display().to_string(), "models": rep }))
}

fn coat_compact(model: &std::path::Path, out: &std::path::Path) -> Result<Value, String> {
    use lensbench_core::coat_fit::format_models_bin;
    let b = lensbench_core::vfs::read(model).map_err(|e| format!("{}: {e}", model.display()))?;
    let models = if b.starts_with(coating::MODEL_BIN_HEADER) {
        coating::parse_models_bin(&b)
    } else {
        coating::parse_models(&String::from_utf8_lossy(&b))
    }
    .map_err(|e| format!("{}: {e}", model.display()))?;
    let rep: Vec<Value> = models
        .iter()
        .map(|(name, m)| {
            json!({
                "coat": name,
                "wavelengths": m.wavelengths.len(),
                "betas": m.betas.len(),
                "mirror_wavelengths": m.mirror.as_ref().map_or(0, |x| x.wavelengths.len()),
                "mirror_betas": m.mirror.as_ref().map_or(0, |x| x.betas.len()),
            })
        })
        .collect();
    let e: Vec<(&str, &coating::CoatModel)> = models.iter().map(|(c, m)| (c.as_str(), m)).collect();
    let bytes = format_models_bin(&e).map_err(|x| x.to_string())?;
    lensbench_core::vfs::write(out, &bytes).map_err(|x| format!("{}: {x}", out.display()))?;
    Ok(
        json!({ "out": out.display().to_string(), "bytes_in": b.len(), "bytes": bytes.len(), "models": rep }),
    )
}

struct CoatDesignArgs<'a> {
    targets: &'a [PathBuf],
    name: &'a str,
    materials: &'a [String],
    substrate: &'a str,
    coating_file: Option<&'a std::path::Path>,
    glass_dir: &'a [PathBuf],
    layers: &'a str,
    starts: usize,
    d_max: f64,
    slack: f64,
    out: Option<&'a std::path::Path>,
}

/// 曲線の CSV を読む（[`Cmd::CoatDesign`] の列）
fn read_targets(p: &std::path::Path) -> Result<Vec<lensbench_core::coat_design::Target>, String> {
    use lensbench_core::coat_design::{Pol, Quantity, Target};
    let txt =
        lensbench_core::vfs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut lines = txt
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let head: Vec<String> = lines
        .next()
        .ok_or_else(|| format!("{}: 空", p.display()))?
        .split(',')
        .map(|x| x.trim().to_ascii_lowercase())
        .collect();
    let col = |n: &str| head.iter().position(|h| h == n);
    let (wl, scale_wl) = match (col("wl_um"), col("wl_nm")) {
        (Some(i), _) => (i, 1.0),
        (None, Some(i)) => (i, 1e-3),
        _ => return Err(format!("{}: 列 wl_um か wl_nm がない", p.display())),
    };
    let (val, scale_v) = match (col("value"), col("value_pct")) {
        (Some(i), _) => (i, 1.0),
        (None, Some(i)) => (i, 0.01),
        _ => return Err(format!("{}: 列 value か value_pct がない", p.display())),
    };
    let (ang, pol, qty, w) = (col("angle_deg"), col("pol"), col("quantity"), col("weight"));
    let mut v = Vec::new();
    for (ln, l) in lines.enumerate() {
        let c: Vec<&str> = l.split(',').map(str::trim).collect();
        let num = |i: usize| -> Result<f64, String> {
            c.get(i)
                .and_then(|x| x.parse::<f64>().ok())
                .ok_or_else(|| format!("{}: {} 行目の数値", p.display(), ln + 2))
        };
        let pol = match pol.and_then(|i| c.get(i)).map(|x| x.to_ascii_lowercase()) {
            None => Pol::Avg,
            Some(x) if x == "avg" || x.is_empty() => Pol::Avg,
            Some(x) if x == "s" => Pol::S,
            Some(x) if x == "p" => Pol::P,
            Some(x) => return Err(format!("{}: 偏光 {x}（s・p・avg）", p.display())),
        };
        let q = match qty.and_then(|i| c.get(i)).map(|x| x.to_ascii_uppercase()) {
            None => Quantity::R,
            Some(x) if x == "R" || x.is_empty() => Quantity::R,
            Some(x) if x == "T" => Quantity::T,
            Some(x) => return Err(format!("{}: 量 {x}（R・T）", p.display())),
        };
        v.push(Target {
            wl: num(wl)? * scale_wl,
            angle_deg: match ang {
                Some(i) => num(i)?,
                None => 0.0,
            },
            pol,
            q,
            value: num(val)? * scale_v,
            weight: match w {
                Some(i) => num(i)?,
                None => 1.0,
            },
        });
    }
    Ok(v)
}

fn coat_design(a: &CoatDesignArgs) -> Result<Value, String> {
    use lensbench_core::coat_design::{fit_all, format_lbcs, pick, Material, Options};
    let mut ts = Vec::new();
    for p in a.targets {
        ts.extend(read_targets(p)?);
    }
    let file = match a.coating_file {
        Some(p) => coating::CoatingFile::load(p).map_err(|e| e.to_string())?,
        None => coating::CoatingFile::default(),
    };
    // ガラスカタログは必要になったときに全部読む
    let mut glasses: Option<std::collections::HashMap<String, lensbench_core::glass::Glass>> = None;
    let mut glass = |name: &str| -> Option<lensbench_core::glass::Glass> {
        let g = glasses.get_or_insert_with(|| {
            let mut m = std::collections::HashMap::new();
            for d in lensbench_core::glass::catalog_dirs(a.glass_dir) {
                for f in lensbench_core::vfs::read_dir(&d).unwrap_or_default() {
                    if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("agf")) {
                        if let Ok(x) = lensbench_core::glass::load_agf(&f) {
                            for (k, v) in x {
                                m.entry(k).or_insert(v);
                            }
                        }
                    }
                }
            }
            m
        });
        g.get(name).cloned()
    };
    // 材料: 膜ファイルの MATE、なければガラス（0.2〜2.5 µm を 5 nm 刻みの表にして .lbcs に MATE も書く）
    let mut mats = Vec::new();
    let mut with_mate = Vec::new();
    let mut lookup = |name: &str| -> Result<(Material, bool), String> {
        if let Some(r) = file.mate.get(name) {
            let mut rows = r.clone();
            rows.sort_by(|x, y| x[0].total_cmp(&y[0]));
            return Ok((
                Material {
                    name: name.into(),
                    rows,
                },
                false,
            ));
        }
        let g = glass(name)
            .ok_or_else(|| format!("材料 {name} が膜ファイルにもガラスカタログにもない"))?;
        let rows = (0..=460)
            .map(|i| {
                let wl = 0.2 + 0.005 * i as f64;
                [wl, g.index(wl, 20.0, 1.0, false), 0.0]
            })
            .collect();
        Ok((
            Material {
                name: name.into(),
                rows,
            },
            true,
        ))
    };
    for m in a.materials {
        let (mat, from_glass) = lookup(m)?;
        if from_glass {
            with_mate.push(mats.len());
        }
        mats.push(mat);
    }
    let sub: Box<dyn Fn(f64) -> coating::C64> = match a.substrate.parse::<f64>() {
        Ok(n) => Box::new(move |_| coating::C64::new(n, 0.0)),
        Err(_) => {
            let (m, _) = lookup(a.substrate)?;
            Box::new(move |wl| m.index(wl))
        }
    };
    let (lo, hi) = match a.layers.split_once("..") {
        Some((x, y)) => (
            x.trim()
                .parse::<usize>()
                .map_err(|_| format!("--layers {}", a.layers))?,
            y.trim()
                .parse::<usize>()
                .map_err(|_| format!("--layers {}", a.layers))?,
        ),
        None => {
            let n = a
                .layers
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("--layers {}", a.layers))?;
            (n, n)
        }
    };
    let o = Options {
        materials: mats.clone(),
        layers: (lo, hi),
        starts: a.starts,
        d_max: a.d_max,
        ..Default::default()
    };
    let all = fit_all(&ts, &*sub, &o).map_err(|e| e.to_string())?;
    let d = pick(&all, a.slack).ok_or("当てはめられない")?;
    let lay = |d: &lensbench_core::coat_design::Design| -> Vec<Value> {
        d.layers
            .iter()
            .map(|&(k, t)| json!({ "material": mats[k].name, "d_um": t }))
            .collect()
    };
    let mut written = None;
    if let Some(out) = a.out {
        let note = format!(
            "近似の膜: lensbench coat-design で分光の曲線 {} 点に合わせた（RMS {:.3e}、最大 {:.3e}）。\n\
             曲線の外（位相・別の角度・硝材の側から）は元の膜と違いうる",
            ts.len(),
            d.rms,
            d.max_err
        );
        let txt = format_lbcs(a.name, d, &mats, &with_mate, &note);
        lensbench_core::vfs::write(out, txt.as_bytes())
            .map_err(|e| format!("{}: {e}", out.display()))?;
        written = Some(out.display().to_string());
    }
    Ok(json!({
        "name": a.name,
        "points": ts.len(),
        "layers": lay(d),
        "rms": d.rms,
        "max_err": d.max_err,
        "by_layers": all.iter().map(|x| json!({ "layers": x.layers.len(), "rms": x.rms, "max_err": x.max_err })).collect::<Vec<_>>(),
        "out": written,
    }))
}

/// JSON をそのまま読める形で出す（GUI ができるまでの簡易表示）
fn print_text(v: &Value, ind: usize) {
    let pad = "  ".repeat(ind);
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match x {
                    Value::Object(_) | Value::Array(_) if !is_flat(x) => {
                        println!("{pad}{k}:");
                        print_text(x, ind + 1);
                    }
                    _ => println!("{pad}{k}: {}", fmt(x)),
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                println!("{pad}-");
                print_text(x, ind + 1);
            }
        }
        _ => println!("{pad}{}", fmt(v)),
    }
}

fn is_flat(v: &Value) -> bool {
    match v {
        Value::Array(a) => a.iter().all(|x| x.is_number() || is_flat_pair(x)),
        _ => false,
    }
}

fn is_flat_pair(v: &Value) -> bool {
    matches!(v, Value::Array(a) if a.iter().all(Value::is_number))
}

fn fmt(v: &Value) -> String {
    match v {
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                format!("{f:.6e}")
            }
        }
        Value::Array(a) => format!("[{}]", a.iter().map(fmt).collect::<Vec<_>>().join(", ")),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}

/// スポット・PSF の位置の単位: レンズの単位（mm）、アフォーカル像空間では角度の単位
fn spot_unit(s: &System) -> &'static str {
    if s.afocal {
        s.image_unit()
    } else {
        "lens"
    }
}

/// コマンドラインの入口（`lensbench` の実行ファイル）
/// clap の定義に拡張のサブコマンド（[`ext::list`]）を足したもの
fn cli_command() -> clap::Command {
    use clap::CommandFactory;
    let mut c = Cli::command();
    for e in ext::list() {
        c = c.subcommand((e.command)().name(e.name));
    }
    negative_numbers(c)
}

/// `--dist -5`・`--pmag -0.5` のような負の数を値として受ける（入れ子のサブコマンドも）
fn negative_numbers(c: clap::Command) -> clap::Command {
    c.allow_negative_numbers(true)
        .mut_subcommands(negative_numbers)
}

/// 同梱のサブコマンドの名前
fn builtin_command_names() -> Vec<String> {
    use clap::CommandFactory;
    Cli::command()
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect()
}

/// 引数を解釈する（拡張のサブコマンドも）
fn parse_cli<I, T>(argv: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    use clap::FromArgMatches;
    let m = cli_command().try_get_matches_from(argv)?;
    if let Some((name, sub)) = m.subcommand() {
        if let Some(e) = ext::get(name) {
            let threads = sub
                .try_get_one::<usize>("threads")
                .ok()
                .flatten()
                .or_else(|| m.try_get_one::<usize>("threads").ok().flatten())
                .copied();
            let args = (e.parse)(sub)
                .map_err(|x| cli_command().error(clap::error::ErrorKind::ValueValidation, x))?;
            return Ok(Cli {
                threads,
                cmd: Cmd::Ext(ExtCmd { args }),
            });
        }
    }
    Cli::from_arg_matches(&m)
}

pub fn main() -> ExitCode {
    let cli = parse_cli(std::env::args_os()).unwrap_or_else(|e| e.exit());
    // 機構部品の詳しい形（取った STEP）を描けるように
    #[cfg(not(target_arch = "wasm32"))]
    cad_detail::install_detail_loader();
    #[cfg(not(target_arch = "wasm32"))]
    coat_remote::install();
    // run はスレッド数を自分では使わず、起動する Python に渡す
    if let Cmd::Run(a) = &cli.cmd {
        return match run::run(a, cli.threads) {
            // 0〜255 の外（Windows の異常終了の NTSTATUS は負など）は 1 にする（0 に丸めると成功に見える）
            Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            Err(m) => {
                eprintln!("lensbench run: {m}");
                ExitCode::from(2)
            }
        };
    }
    match lensbench_core::with_threads(cli.threads, || run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(m) => {
            eprintln!("lensbench: {m}");
            ExitCode::FAILURE
        }
    }
}
