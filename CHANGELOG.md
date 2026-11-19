# 変更の記録

版ごとの、利用者に見える変更のまとめ（新しい順）。版の付け方は [版の付け方](docs/versioning.md)。
互換が切れる変更の細かい書き方・移し方は [壊れる変更](docs/changes.md) にある。

## 0.6.0（開発中）

0.5.0 からの変更。

### 互換が切れる所

- **結果の型を外で組み立てない**: Rust の解析・追跡の結果の型（`Mtf`・`Psf`・`Spot`・`Zernike`・`Wavefront`・`Opd`・`Paraxial`・
  `RayOut` など）に `#[non_exhaustive]` を付けた。フィールドは読めるが、構造体の式では作れない（後でフィールドを足しても互換が切れない）。
  `raygrid::PupilGrid` は `PupilGrid::new(g)` で作り、`g()` で読む。設定の型（`PsfOpts`・`RmsOpts` など）は変わらない。
- **解析を lensbench-analysis へ**: Rust の `lensbench_core::{analysis, psf, pop, sweep, fiber, raygrid}` は `lensbench_analysis::…` になった。
  OPD と瞳の標本（`opd`・`gauss_legendre` など）は core の `lensbench_core::opd` に残り、`lensbench_analysis::analysis` からも引ける。
  `lensbench_analysis::prelude` は .zmx の読込と解析まで含む。CLI・GUI・Python は変わらない。
- **.zmx の読込を lensbench-formats へ**: Rust の `lensbench_core::zmx` は `lensbench_formats::zmx` になった。
  `lensbench_formats::prelude` は core の prelude に `zmx`・`LoadOptions` を足したもの。CLI・GUI・Python は変わらない。
- **過去の形式の読込をやめた**: システムファイルの版 1、列挙の旧名（`"Epd"`・`"Real"`・`"Milliradians"` など大文字始まりの名前）、
  GUI の `--convert-combo`、`glass_dirs` の無い古い pickle は読まない。0.5 で開いて保存し直すと今の形になる。
- **条件で定める視野の条件の JSON**: core のシステムの形（serve の `system_data` など）でも、システムファイルと同じ
  `{"cond": "position", "surface": 5}` の形にした。
- **Rust API**: 解析の結果の型（`Analysis::Output`）は `Send + Sync + 'static` であること（同じプロセスの呼び手へ
  結果を型のまま渡す `DynAnalysis::run_typed`・`Typed`）。`Answer::into_json` は `Result` を返す
- **ユーザーカタログの品の型**: Rust の `lensbench_project::catalog::UserItem` の中身を、レンズ・機構部品・未完成で分けた
  `body: ItemBody`（`Optic`・`Mech`・`Incomplete`）にした。前の欄は `blocks()`・`frame()`・`mech()`・`missing()` で読む。JSON の形は同じで、
  意味のない組み合わせ（面と機構部品の寸法の両方など）は読むときに `items.<名前>` 付きの誤りになる。カタログの絞り込みは一度だけ読んだ
  行（`lensbench_catalog::spec::CatalogRow`）に対して行う

### 結果が変わる所

- **Huygens PSF・Huygens MTF の値が変わる**: 既定を速い計算（下の「速さ」）にした。強度の差は収差のない中心の値の 1e-7 倍程度。
  前と同じ値は設定 `exact` で
- **Zernike の項の値が変わる**: 最適化の `zernike` の項を既定で 37 項以上で当てはめた係数にした（前は求める項までで当てはめて
  いた）。当てはめる項の数は `fit` で変えられ、.zmx から読んだ ZERN の行は OpticStudio と同じ数にする

### 変えたもの

- **Web 版の解析の別の窓**: 開いた後の編集は差分だけを送る（窓が写しを取り違えたら全体を送り直す）。前の写しの計算の誤りは数えない。
- **非順次の物体を含む .zmx**: 非順次の物体の行（NSOH）を含む系（非順次モード・混合モード）は、行の名前だけの誤りでなく
  非順次の物体には対応していないと分かる文で断る

### 足したもの

- **多重構成の表に系の値**: 波長 `wave1`〜、波長のウェイト `wwt1`〜、主波長 `primary`、開口の値 `aper`、視野 `fx1`〜・`fy1`〜・
  ウェイト `fwt1`〜 を多重構成の表の行にできる。.zmx の WAVE・WLWT・PRWV・APER・XFIE・YFIE・FLWT を表に読み、書き出す
  （前は選んだ構成だけを読んだ。WLWT・PRWV・FLWT が構成で変わる .zmx は読めなかった）。ガラス（GLSS）が構成で変わる .zmx は、
  前は黙って構成 1 のガラスにしていたが、選んだ構成だけを読んだと知らせる
- **プロジェクトの ZPL マクロ**: プロジェクトに `macros`（マクロの名前と .ZPL の中身）を書き、ブロックの `surface_ops` の
  `add_solve`（`macro_curv`・`macro_thick`）と新しい `add_macro_solve`（円錐係数・半直径・面タイプのパラメータ）でマクロを
  名前か番号で指して解く。マクロのファイルは読まない。serve の `edit` でも `add_macro_solve` が使える。同じ名前（大文字・小文字を
  区別しない）のマクロを 2 つ置くと誤り
- **複屈折材の板の Mode 1・3**: .zmx の複屈折の板の Mode 1・3（異常光の道）を追跡する（前は断って光線を板で止めていた）。
  屈折は一軸結晶の式で解き、板の中は walk-off の向きへ進み、光路長は n(θ) で数える
- **GUI で開く .zmx を要素に分ける範囲**: 前後に傾けて戻す鏡（行の `before`・`after`）、座標ブレーク、膜ファイル（プロジェクトの
  `coating_file`）、OPD のソルブ、玉の中の鏡（裏面鏡・キューブ BS の中の反射）、液浸（空気のブロックの `medium`、要素の設定の欄）、
  ZPL マクロのソルブのある .zmx も要素に分ける。分けられない .zmx（多重構成など）は .zmx を読む要素 1 つで開き、その旨を出す
- **GUI の操作の記録と流し直し**: 「ファイル > 操作を記録」で文書を変える操作（欄の書き換え・元に戻す・レンズデータの
  編集など）を版つきの JSON に書き、「記録した操作を流し直す…」で始めた時の組合せから同じ順に当て直す
- **書き出しの場面の立体の名前**: 3D の書き出しのガラス・鏡の立体の名前の頭に、その面を含むプロジェクトの要素・群の名前を付ける
- **.zmx のメリットファンクション**: OpticStudio が .zmx に保存したメリットファンクション（面の後の行）を読み、項で表せる
  オペランド（近軸量・RSCE などの RMS・OPDC・ZERN・MTFT/MTFS・REAX〜REAC・CENX/CENY・OPGT/OPLT・CONF）を最適化の項にする。
  CLI は `optimize --zmx-merit`。写せない行は行の番号つきの警告にする。`optimize --out` の .zmx にもメリットファンクション
  （読んだ行と `--term` の項）を書く
- **スポット・波面の項の瞳の求積**: `spot`・`wavefront` の項に `quadrature`（`lensbench` が既定、`opticstudio` は OpticStudio の
  RSCE などと同じ腕 2·Ring 本を 90° 回した置き方）を足した。.zmx から読んだ RSCE・RSCH・RWCE・RWCH は `opticstudio` にする
- **厚みの項**: 最適化の項 `thickness`（面 `surf` から `to` までの中心の厚みの和、`min`・`max` も可。`edge` で縁の厚み、`each` で面ごとの
  境界）。.zmx の CTGT・CTLT・CTVA・TTHI・ETGT・ETLT・ETVA と範囲の境界 MNCT〜MNEG をこの項にし、書き出しも同じオペランドにする
- **計算の項**: 最適化の項 `calc`（数・1 つの値の項の和・差・積・商・絶対値・√|a|・逆数・最大・最小・√Σa²・三角関数）。.zmx の SUMM・OSUM・DIFF・PROD・DIVI・ABSO・SQRT・
  RECI・CONS・MAXX・MINN・QSUM・SINE〜ATAN・OPVA・ABGT・ABLT をこの項にする
- **縦収差・色収差の項**: `longitudinal`（軸上の光線の像面からの距離、`wave2` で軸上色収差）と `lateral_color`（倍率色収差）。.zmx の LONA・AXCL・LACL を読み書きする
- **ザイデル収差係数・歪曲の項**: 面ごと・和のザイデル収差係数 `seidel`（spha・coma・asti・fcur・dist [波]）と、最大の視野の実光線の歪曲 `distortion`（% か差）。.zmx の SPHA・COMA・ASTI・FCUR・DIST を読み書きする
- **F ナンバーの項**: 視野ごとの実光線の F ナンバー `fnum`（`dir` は tan・sag）。.zmx の TFNO・SFNO を読み書きする
- **基点の項**: 面の範囲の焦点距離・焦点・主点・節点などを `cardinal` の項にした（`from`・`to`・`wave`）。.zmx の CARD・EFLX・EFLY・EFLA を読み、CARD・EFLY を書く
- **面のパワーなど**: `surface` の項の `q` に `power`・`ynip`・`index`（`wave` も）を足した。.zmx の POWR・YNIP・INDX を読み書きする（ミラーの面の power・ミラーの後の ynip はまだ項の誤り）
- **近軸量**: `paraxial` の項の `q` に `amag`（角倍率）・`linv`（ラグランジュ不変量）・`petc`・`petz`（ペッツバールの曲率・半径）を足した（`wave` も）。.zmx の AMAG・LINV・PETC・PETZ を読み書きする（ミラーのある系の petc・petz・絞りの前にミラーのある linv はまだ項の誤り）
- **近軸量の波長**: `paraxial` の項の `efl`・`pimh` に `wave`（その波長を主波長にした値）を足した。.zmx の EFFL・PIMH の Wave を読み書きする
- **面の量の項**: 最適化の項 `surface`（面の曲率・コーニック・矢高・点の厚み・直径・玉の径・頂点の大域座標）。.zmx の CVGT〜COVA・DMGT〜DMVA・SAGX・SAGY・SSAG・STHI・GLCX〜GLCZ を読み書きする。MXEA・MXEG も書く。縁の厚み（`edge`）の項も、最適化の間に自動の半直径を求め直すようにした（前は始めの半直径のまま）
- **光線の収差・光路長**: `ray` の項の `q` に `trax`・`tray`・`trar`（主波長の主光線からのずれ）と `opth`（光路長）を足した。.zmx の TRAX・TRAY・TRAR・OPTH を読み書きする
- **光線の量**: `ray` の項の `q` に `gx`〜`gn`（グローバル座標の交点・方向）・`r`・`ang`・`tanx`・`tany`・`nl`〜`nn`（面の法線）・`aoi`・`aoe`（入射角・射出角）・`cosi`・`cose` を足した。.zmx の RAGX〜RAGC・REAR・RANG・RETX・RETY・RENA〜RENC・RAID・RAED・RAIN・RAEN を読み書きする。`to` で面の範囲（`opth` は面の間の光路長、`aoi`・`aoe` は `min`・`max` との面ごとのはみ出し）、.zmx の PLEN・MXRE・MNRE・MXRI・MNRI。`paraxial: true` で近軸光線（.zmx の PARX〜PARC・PARR・PANA〜PANC・PATX・PATY）
- **評価関数の Wizard の行**: .zmx の DMFS の後の光線の行（OPDX・TRAR・TRAC・TRAD・TRAE・TRCX・TRCY）を、視野ごとの
  スポット・波面の項にまとめて読む（評価関数の値は OpticStudio と同じ）。そのために `spot`・`wavefront` の項に
  `quadrature: "grid"`（`n` × `n` の升の中心）と `arms`（`opticstudio` の腕の数、0° から等間隔）、`spot` に `axis`（`x`・`y`）を足した
- **利用者定義の形（面の種類 USERSHAPE）**: 外のクレートが `lensbench_sdk::shape::ShapeModel` を実装し
  `Extensions::shape_model` で足すと、二次曲面（`c`・`k`）にモデルの矢高を足した面を使える。屈折・反射・材料・膜・
  偏光・dOPL/dp・着地点の微分・玉の反転はほかの面と同じに効き、設定の数の項目はパラメータ `<面>.u_<項目>` で変数に
  できる。システムファイルは `"shape": {"user": {…}}`。.zmx には書けない。約束は [ライブラリ](docs/rust.md) と
  `check_shape`。雛形 `examples/extension` に例（`example_saddle`）
- **利用者定義の材料**: 外のクレートが `lensbench_sdk::material::MaterialModel`（波長・系の温度・気圧 → 屈折率）を
  実装し `Extensions::material_catalog(名前, …)` で足すと、AGF のカタログと同じにカタログ名・材料名で引ける
  （GCAT・`glass_catalogs`・`材料@カタログ`）。dn・disp・範囲の注意・内部透過率もガラスと同じに効く。雛形に例
  （カタログ `EXAMPLE`）
- **GUI の図の縮尺（スケールバー）**: レイアウト図（2D・3D）の左下に、1・2・5×10^n のきりのよい長さの棒と長さを出す
  （拡大縮小に合わせて選び直す）。スポット図は OpticStudio の Scale と同じく、枠の幅を指定でき（既定は自動のきりのよい長さ）、
  枠の下にその長さの縮尺を出す
- **Rust API**: 計算の要求を型で渡す口（`lensbench_engine::Request`・`request`・`catalog`）。serve の method は
  型の要求に読んでから処理する（JSON の形・誤りの文は前と同じ。誤りが 2 つ重なる要求では出る誤りが前と違う
  ことがある: params の誤りを、システムを読んでいない誤りや `--allow-remote` の検査より先に返す）

### 速さ

- 波面を使う計算（最適化の波面の評価関数・公差・PSF・波面）を少し速くした（値は同じ）
- PSF を複数スレッドで速くした（瞳 → 像の 1 段目も並列に。値は同じ）
- 幾何 MTF を複数スレッドで速くした（周波数ごとに並列。値は同じ）
- カタログの品の一覧・検索を複数スレッドで少し速くした（品ごとの要約と、レンズカタログ（.zmx）の玉ごとの要約を並列に。結果は同じ）
- 最適化の波面の項・MTF（瞳の補間と位相因子の和の作業の配列）の内側を少し軽くした（値は同じ）
- Grid Sag の面を通る光線追跡を速くし、0.4.0 より遅くなっていたのを戻した（値は同じ）
- Zernike の面（Standard・Fringe）を通る光線追跡を 2〜3 倍速くした（面との交点と法線を光線の組のまま計算する。値は同じ）
- PSF を約 2 倍速くした（瞳 → 像の変換をチャープ z 変換（FFT）で計算する。値は丸めの範囲で変わる: 差は 1e-14 以下）
- 幾何 MTF を約 3 倍速くした（位相因子の sin・cos を MTF・Huygens と同じ計算に。値は丸めの範囲で変わる: 差は 1e-15 以下）
- Huygens PSF・Huygens MTF を 5〜7 倍速くした。像のグリッドを小さな区画に分け、瞳の各標本の位相と振幅を区画の中で 2 次式で
  近似する（区画は、グリッドの四隅と中央の区画で確かめて、位相の差が 1e-6 rad 以下になる大きさを自動で選ぶ）。値は変わる: 強度は前の厳密な和と 1e-7 程度で合う。
  前と同じ厳密な和は設定 `exact`（CLI `--exact`、Python `exact=True`、GUI・serve の設定 `exact`）で

### 直した所

- **多重構成の表の空行**: .zmx の多重構成の表にある空の行（BLNK）で開けなかったのを、読み飛ばすようにした
- **GUI のレイアウト図の選択**: 要素を選ぶと図の縮尺が変わる（下の設定の欄の高さが要素で違った）、先頭に自動で置く
  絞り（厚さ 0）が選べない、BS を選んでもレンズデータの表の行が選ばれない（鏡で戻って BS を通る面にも BS の名前を付けた。
  serve の `blocks` の BS の範囲に復路の分も入る）を直した

## 0.5.0（2026-11-18）

0.4.0 からの変更。ファイルの形式・CLI・Python・serve の書き方と、いくつかの解析の値が変わる（互換が切れる版）。

### 互換が切れる所

- **システムファイルを版 2 に**: `system` は内部の構造体でなく、利用者が書いた値（処方）だけを決めた形で書く
  （面は `shape`・`action`・`medium` などの行、曲率は `c`、ソルブは `{"surf": 5, "type": "edge", …}`）。版 1 のファイルは読める。
  求めた屈折率・自動のクリア半径の値・ガラスの式は書かないので、ファイルが小さくなる。システムの hash も変わる
- **プロジェクトファイル**: 形式の印 `"format": "lensbench-project"` と版 `"format_version": 1` が付く。直下の `surface_edits`・
  `surface_ops` をやめて各ブロックに書く。知らないキーはエラー（GUI だけの設定は `gui` 欄）
- **名前をそろえた（旧名はエラー）**: 引数の名前を CLI・serve・Python で 1 つに。`fringe` → `interference`、物体空間 NA の量
  `obna` → `obsn`、PSF・Huygens の格子の間隔 `delta` → `pixel`、`tolerance --save-worst` → `--out` など
- **過去互換の受け口を消した**: lensbench の古い保存の読み分け・評価関数・公差・差し替え探索の古い書き方
- **CLI の終了コード**: 1 失敗、2 使い方、3 ファイル、4 未対応の機能、130 中断に分けた（前は引数のエラーの 2 のほかは全部 1）。
  エラーの文は英語が元で、日本語の表示では訳す。公差の結果の `label` も英語の名前に
- **結果の形**: MTF の `cutoff` は `{"tangential", "sagittal"}`、スポット・PSF・RMS の `unit` は `"mm"`、ズーム軌跡は解けない点を
  `stop: "failed"` で残す、serve の `load` などに `notes`
- **Rust API**: core を分けた（`lensbench-runs`・`lensbench-formats`・`lensbench-catalog`・`lensbench-step` など）、`System` は
  書き換えられない値、面の種類ごとの値は `Form` の枝、追跡の結果は列で受ける、外のクレートから足す口 `lensbench-sdk`

### 結果が変わる所

- 位相の約束を exp(−ikL) にそろえた: POP の場の位相の符号、光線の格子の Jones 行列が前の版の複素共役になる
  （照度・強度は変わらない）
- MTF の子午・球欠は既定で視野の向き。遮断周波数は計算した波長の値。Huygens MTF のナイキストを超える周波数は `null`
- 瞳のアポダイゼーションを幾何の解析の光線の重み・RMS 波面の重みにも入れる（OpticStudio と照合）
- 膜・偏光の振幅を入れた多色の PSF・MTF は、光量の透過率を波長の重みに掛ける
- モンテカルロ公差・検出器の露光の乱数を変えた（同じ種でも試行の値が前と違う。分布は同じ）
- 最適化の止まった理由 `small_step` と `no_progress` の分け方
- 鏡の後ろの面（トロイダル・Zernike・Grid Sag・GRIN・利用者定義の面など）を物理的に正しく裏返す
- 開口の判定の余裕を開口の大きさに対する比に（1e-9 倍）
- 膜のモデルのミラーで、全反射に変わる点（分岐点）のそば・分岐点が測った β の範囲の上端を横切る所とそのすぐ上の波長は
  notimpl を返す（前は OpticStudio と 1e-3〜6e-2 違う値を返した）

### 足したもの

- lensbench が書く JSON の先頭に `$schema`（`https://masa-iwm.github.io/lensbench/schema/v0.5/…`）。ドキュメントとスキーマを版ごとに残す
- GUI はファイルを必ず組合せで開く: .zmx（多重構成・条件で定める視野・像面の傾き・物体／像空間が空気でないものも）とシステムファイル
- 玉の反転を Zernike・Grid Sag・Tilted・Irregular・Cylinder Fresnel・GRIN の玉にも
- プロジェクトに像面 `image`、面の並び・自作の玉に版 2 の面の行 `row`、取り込むプロジェクトの状態と経路の選択
- serve の `write_back`・`save_project`（面の表の編集をブロックへ書き戻す）
- Python の `System.analyze(name, **settings)`、解析のサブコマンドの `--params-file`
- スポットの結果に光線の本数と除いた理由
- 外のクレートから解析・検出器などを登録する口（`lensbench-sdk`）
- Web 版: 解析のパネルの別の窓は窓ごとに計算する
- GUI の読み上げ（AccessKit）
- 近軸レンズ（理想レンズ）が物体を無限遠に結ぶ .zmx（コリメータ）も開ける（系の検査で断らない）
- 読み込みで解けないソルブは外し、保存された値に固定して開く（そのソルブを注意に出す）

### 速さ

- CLI の起動・`info`、Zernike の矢高、MTF の瞳の標本、形のパラメータの微分、検出器の露光を速くした

### 既知の問題

- .zmx の複屈折材の面の Mode 1・3（異常光の道）は読んで持つだけで、その面がある系の追跡・偏光・最適化は面を名指しして断る
  （常光の Mode 0・2 は追跡できる）
- Grid Sag の面がある系の追跡が 0.4.0 よりわずかに遅い（0.94〜0.99 倍、https://github.com/masa-iwm/lensbench/issues/268）
- 近似の膜のいくつかで、硝材側から大きな角度で入るミラーの細い共鳴の所の反射率・透過率が OpticStudio と 1e-3〜1e-2 ほど違うことがある

## 0.4.0 以前

[壊れる変更](docs/changes.md) と GitHub の Releases を参照。
