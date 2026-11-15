# 壊れる変更

版を上げるときに読むための、使い方が変わる変更の一覧（新しい順）。細かい経緯は各 PR。

## 次の版（v0.4.0 の後）

### 名前・設定の書き方
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
  true・false か 0・1 を受ける（Python は True・False がそのまま渡る。前は bool を誤りにした）。数・整数の項目に NaN を書くと「省略」
  でなく誤り。
- 公差の設定（`tolerance --spec`）も型で読む（スキーマ `tolerance.schema.json`）。`wedge_face` は小文字の `back`・`front` だけ、
  `relative`・`catalog`・`samples` などの真偽に真偽でない値は誤り（前は無視）。誤りの文は欄の道つきの serde のもの
  （例「tolerance.lenses.L1: unknown field `tlit`, …」）。先頭の `$schema` は受ける。

### 終了コード
- CLI の終了コードを誤りの種類で分けた: 1 失敗（系・値の不正・計算・照合の不合格・一部の失敗）、2 使い方（引数・設定ファイル）、
  3 ファイルが無い・読めない・書けない、4 未対応の機能、130 中断（以前は引数の誤り 2 のほかは全部 1）。表は [使い方（CLI）](cli.md)・
  `lensbench --help` の末尾。誤りの文の次の行に `案内:` を出すことがある。

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

### 光学系
- 2 軸の折り曲げ鏡・ビームスプリッターの後の座標の軸の向きが変わる。

### Rust API
- `edit_ops::apply` は `&[EditOp]` を受ける（JSON は `edit_ops::parse` で読むか `apply_json`）。`Project::surface_ops` は
  `Vec<Value>` から `Vec<SurfaceOp>` に（型は `lensbench_project::edit_spec`）。
- 差し替え探索 `SwapSearch::eval`・`run`・`run_to` の返り値は `serde_json::Value` から `SwapResult` に（`to_value()` で前と同じ JSON）。
- 多色を受ける解析（spot・mtf・psf・huygens・sweep など）の波長の引数は `Option<usize>` から `Waves`（`All`・`One`）に。
  単色の解析（OPD・波面・Zernike・縞）は `Option<usize>` のまま（None は主波長）。
- 追跡: panic する `System::trace` を消した（`try_trace` を使う）。`try_trace_rec`・`trace_launch_rec` は
  `try_trace`・`trace_launch` にまとめた（記録の引数は `bool` か `RecSel`）。`trace_spec` のケラレ係数・止める面・記録・
  視野は `TraceOpts`（`TraceOpts::new().vig(..).upto(..).record()`）。
- `Mtf.cutoff` は `TanSag<f64>`、`huygens::psf_mtf` の戻りは `TanSag<Vec<f64>>`。遮断周波数は `analysis::mtf_cutoff`。

### GUI
- CAD の置き場の Secret は保存しない（環境変数 `LENSBENCH_CAD_STORE_SECRET`）。
