# 壊れる変更

版を上げるときに読むための、使い方が変わる変更の一覧（新しい順）。細かい経緯は各 PR。

## 次の版（v0.4.0 の後）

### 名前・設定の書き方
- Rust のモジュールの置き場所を変えた（XCT-04）: `lensbench_core::zmx_write` は新しいクレートの `lensbench_formats::zmx_write`、
  `lensbench_core::{ghost, interference, polgrid}` は `lensbench_analysis::{ghost, interference, polgrid}`。中身・Python・CLI は同じ。
- 単色の干渉縞の解析 `fringe` を `interference` に改名した（CLI のサブコマンド・serve の `analyze` の `cmd`・GUI の解析パネル・
  `lensbench_client` の `interference()`）。Python の `fringe_detector` は `interference_detector`、Rust の `lensbench_core::fringe` は
  `lensbench_core::interference`。旧名は残さない（GUI の設定に残った `fringe` のパネルは開き直す）。Zernike の並び `kind = "fringe"` は変えない。
- 引数の名前を CLI・serve・Python で 1 つにそろえ、旧名は消した（旧名はエラー。serve は「X は Y に名前を変えた」と出す）。
  対応表は [Python API の約束](python.md) の「引数の名前」。Python API は 2.0。
- プロジェクトの知らないキーはエラー（GUI だけの設定は `gui` 欄）。`environment` 欄（既定 20°C・1 気圧）。
- 差し替え探索の設定の `merit`・`reference`・`jac` は決まった綴りだけ（大文字小文字を区別する）。
- serve の `analyze` は CLI と同じ `--flag=値` の並びで受ける。
- `tolerance --save-worst` は `--out` に（最も悪い試行の系の書き出し先。系を書き出す旗はどれも `--out`）。`--out` はどのコマンドでも `-o` でも渡せる。
- 評価関数の項（`optimize --term`・`terms` 欄）は種類ごとの型で読む（スキーマ `terms.schema.json`）。`dir`・`reference`・`kind`・
  centroid の `q` は決まった綴りだけ（小文字。`type` は前どおり大文字も可）。型の合わない値・知らないキーの誤りの文は serde のもの
  （例「ray の項: unknown field `taget`, expected one of …」）。
- serve の `edit` の操作とプロジェクトの `surface_ops` は操作ごとの型で読む（スキーマ `edit-ops.schema.json`、`surface_ops` は
  `project.schema.json` の中）。知らないキー（以前は黙って捨てた）・型の合わない値（以前は `scale`・`value` などは既定値にした）は誤り。
  `edit` の読みの誤りは invalid params（-32602、以前は -32000）で、どの操作も当てない。`surface_ops` は `set_stop`・`add_solve`・
  `add_pickup` だけで、ほかの操作は展開のときでなく読むときに誤り。書き直したプロジェクトの `surface_ops` の数は実数で書く（`-1` → `-1.0`）。
- プラグインの約束（ANA-19）: 設定の説明は `Plugin::params()` だけ（`Spec::config` を消した）。同梱の解析も型・既定・単位つきの
  項目を持つ。Python の `plugin_spec(name)["config"]` は (名前, 説明) の組でなく項目の dict（name・type・default・unit・doc・choices・len）
  の並び、serve の `plugins` の各プラグインから `config` を消した（`params` だけ）。設定の値に真偽（true・false）を足し、真偽の項目は
  true・false か 0・1 を受ける（Python は True・False がそのまま渡る。前は bool を誤りにした）。省略できる数・整数の項目に NaN を書くと
  「省略」でなく誤り。出力の説明も名前・型（float64・complex128・bool）・形・単位つき（`Spec::outputs` は `OutputSpec` の並び）で、
  Python の `plugin_spec(name)["outputs"]` と serve の `plugins` の `outputs` は dict の並び。
- 公差の設定（`tolerance --spec`）も型で読む（スキーマ `tolerance.schema.json`）。`wedge_face` は小文字の `back`・`front` だけ、
  `relative`・`catalog`・`samples` などの真偽に真偽でない値は誤り（前は無視）。誤りの文は欄の道つきの serde のもの
  （例「tolerance.lenses.L1: unknown field `tlit`, …」）。先頭の `$schema` は受ける。

### 終了コード
- CLI の終了コードを誤りの種類で分けた: 1 失敗（系・値の不正・計算・照合の不合格・一部の失敗）、2 使い方（引数・設定ファイル）、
  3 ファイルが無い・読めない・書けない、4 未対応の機能、130 中断（以前は引数の誤り 2 のほかは全部 1）。表は [使い方（CLI）](cli.md)・
  `lensbench --help` の末尾。誤りの文の次の行に `案内:` を出すことがある。
- 誤りの文の言語: 英語が元で、日本語の表示（`LENSBENCH_LANG=ja`、無ければ OS の言語が日本語のとき）では訳す。種類の前置きは
  英語の表示で `invalid:`・`read:`・`not supported:`・`cancelled`（日本語の表示は前どおり `不正:` など）、案内の行は `hint:`（日本語は `案内:`）。
  引数の誤りは英語の表示で clap の文のまま、日本語の表示では訳す（`lensbench: 知らない引数 --ring（--rings のこと？）`）。
  個々の誤りの文は順に英語へ移す（それまでは日本語だけの文が残る）。

### 結果の形
- 回折 MTF（`mtf`・`geo-mtf`・`huygens-mtf`）の `cutoff` は `[x, y]` から `{"tangential", "sagittal"}` に（JSON・Python）。
  プラグインの出力 `cutoff` は (2,) のまま、並びを [子午, 球欠] にした。
- 遮断周波数は計算した波長の値（以前は単色でも主波長）。多色は主波長のまま。単色の MTF で周波数を省いたときの範囲も
  その波長の遮断周波数の 0〜0.95 になる。

- Huygens MTF（`huygens-mtf`）は、像の格子の間隔 Δ のナイキスト 1/(2Δ) を超える周波数を `null`（NaN）にする（以前は折り返した値）。
  多色の MTF（`mtf-focus`・`mtf-field`・Python の `through_focus_mtf`・`field_mtf` も）で重みのある波長が無いときは、全部 NaN でなく誤り。
- スポット・PSF・RMS の結果の `unit` は `"lens"` から `"mm"` に（意味は同じ。アフォーカル像空間は角度の単位のまま）。
- serve: 読むだけの要求は並行に、`optimize`・`tolerance`・`swap`・`zoom` は始めたときの系で回る。`optimize` の `apply` は
  回している間に系が変わっていれば当てない（`applied: false` と `notes`）。
- 公差（`tolerance`・Python の `tolerance`）の結果の `label`（`tolerances`・`sensitivity`・`correlation`）は英語の名前に
  （例: `L1 偏心x` → `L1 decenter x`、`L1 厚さ1` → `L1 thickness 1`、`G1 群の偏心x` → `G1 group decenter x`、`焦点距離` → `focal length`、
  `後の間隔` → `space after`）。名前で項目を探すスクリプトは書き換えが要る。CLI の文字の出力と GUI は表示の言語で出す。

### 光学系
- 共軸の面の間（前の面の後と次の面の前に傾き・座標ブレークがない区間）の座標の変換は、大域の座標の差でなく面の厚さそのものを
  使う。追跡の結果は丸めの分だけ変わる（公開のテスト系 38 本のうち 29 本。光線の座標 1e-12 前後、RMS・波面は相対 5e-10 まで、
  傾きのある C1 で 3e-9 まで、MTF は値が小さい周波数で相対 1e-5 まで）。自動の半直径・実光線のソルブの値も丸めの分変わるので `system_hash` も変わる。
- 2 軸の折り曲げ鏡・ビームスプリッターの後の座標の軸の向きが変わる。

### Rust API
- `edit_ops::apply` は `&[EditOp]` を受ける（JSON は `edit_ops::parse` で読むか `apply_json`）。`Project::surface_ops` は
  `Vec<Value>` から `Vec<SurfaceOp>` に（型は `lensbench_project::edit_spec`）。
- 差し替え探索 `SwapSearch::eval`・`run`・`run_to` の返り値は `serde_json::Value` から `SwapResult` に（`to_value()` で前と同じ JSON）。
- 粗いふるいがあれば差し替え探索 `SwapSearch::run_to` は一段目の値を `screen_path(out)`（`r.pre.jsonl`）に書く。
  ふるいを止められる形は `SwapSearch::screen(&RunOpts)`。
- 長い計算の進み具合・中断の口を計算の文脈（`lensbench_core::par::Ctx`）に 1 つにした: 進み具合は `par::with_progress(Some(受け手), ..)`
  の受け手に `par::Event`（最適化は `Event::Iter`、公差・runs・差し替え探索は `Event::Count`。差し替え探索の段は `Count::stage` の
  `"screen"`・`"main"`）で届き、中断は `par::with_cancel` の印で頼む。`optim::dls_with`・`montecarlo::run_with` を消し（`dls`・`run` が
  文脈を見る）、`RunOpts` の `progress`・`cancel` を消して `stage` を足した。`SwapSearch::run_to`・`beam_run` は進み具合・中断の引数を取らない。
  ほかの計算の一部として回す最適化は `optim::dls_inner`（進み具合を知らせない）。`CancelToken::child` は親が倒れても倒れる印。
  serve の `cancel`（要求の中断）でも走っている最適化・公差・差し替え探索はそこまでの結果を返す（`stop` と同じ）。
- 多色を受ける解析（spot・mtf・psf・huygens・sweep など）の波長の引数は `Option<usize>` から `Waves`（`All`・`One`）に。
  単色の解析（OPD・波面・Zernike・縞）は `Option<usize>` のまま（None は主波長）。
- 追跡: panic する `System::trace` を消した（`try_trace` を使う）。`try_trace_rec`・`trace_launch_rec` は
  `try_trace`・`trace_launch` にまとめた（記録の引数は `bool` か `RecSel`）。`trace_spec` のケラレ係数・止める面・記録・
  視野は `TraceOpts`（`TraceOpts::new().vig(..).upto(..).record()`）。
- `Mtf.cutoff` は `TanSag<f64>`、`huygens::psf_mtf` の戻りは `TanSag<Vec<f64>>`。遮断周波数は `analysis::mtf_cutoff`。
- core のクレートを分けた（XCT-04）: `lensbench_core::runs` → `lensbench_runs`（一括実行・乱数・検出器の雑音）、
  `lensbench_core::{optim, zoom, zplm}` → `lensbench_optim`（`lensbench_optim::dls` など）・`lensbench_optim::{zoom, zplm}`、
  `lensbench_core::huygens` → `lensbench_analysis::huygens`。`lensbench_core::{tolerance, coat_fit, coat_design}` は前の版で
  `lensbench_design::{montecarlo, coat_fit, coat_design}` に移した。
- 拡張の表を値にした（XCT-07・IOO-29）: `catalog::register_source`・`catalog::sources`・`usersurf::register`・`unregister`・`get`・
  `models`・`plugin::global` を消し、`lensbench_core::ext::Ext`（カタログの形式・面のモデル・プラグイン）にまとめた。計算ごとに
  `ext::with(Arc<Ext>, || ..)` で渡すか、入口で `ext::set_default`・`update_default` でプロセスの既定にする（今の表は `ext::current()`）。
  `lensbench_engine::Extensions::install` は既定に足す（`core_ext()` で値も作れる）。`LensCatalogSource::load`・`LensCatalog::load`・
  `load_registered` の誤りは `String` から core の `Error` に（読めないファイルは `Error::Io`）。
- core は環境変数 LENSBENCH_GLASSCAT・LENSBENCH_MACROS を読まない（XCT-08）。CLI・GUI・Python は前どおり起動時に読む。
  Rust から使うときは `lensbench_core::init_from_env()` を呼ぶか、`glass::set_default_dirs`・`zpl_solve::set_default_dirs` で決める。
- 2 次元の結果は `lensbench_core::grid::Grid2`（座標の並び x・y と行優先の値、`g[iy][ix]`・`rows()`・`data()`）（ANA-18）:
  `Psf.x`・`y`・`i` は `Psf.i: Grid2`（座標は `i.x()`・`i.y()`）、`Wavefront.p`・`w` は `Wavefront.w: Grid2`、`Interferogram.p`・
  `delta`・`intensity` は `delta`・`intensity: Grid2`。CLI・serve・Python・プラグインの出力の形は変えない。
- 外から使われていない core の pub の項目（約 50: 追跡の中の関数 `trace_ray`・`trace_lanes`・`isect`・`paraxial_curv`、
  ファイルの読みの部品 `decode_text_enc`・`parse_grd`・`parse_sgrin`・`parse_grad9`、定数 `AUTO_FFT_N`・`LBCAT_VERSION` など）を
  クレートの中だけに（XCT-22）。`trace_ray_pol` は使われていないので消した。よく使う型は `lensbench_core::prelude`
  （`use lensbench_core::prelude::*;`）。CLI のクレートの `error`・`run`・`serve` のモジュールは外に出さない。

### GUI
- 組合せの図のキー: ロックは L から Ctrl+L に。R・F は Ctrl などを押していないときだけ効く。要素を外すのは Delete だけ（Backspace では外さない。文字を消すつもりの誤操作を防ぐ）。
- 既定の組合せの名前は画面の言語で付く（英語の画面では Catalog lens combination）。
- CAD の置き場の Secret は保存しない（環境変数 `LENSBENCH_CAD_STORE_SECRET`）。
