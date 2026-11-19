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
- **倒立した像の歪曲の長さの符号**: `distortion` の `absolute: true` の値を、像の y の差ではなく、予測より像が内側なら負
  （OpticStudio の DISG と同じ）にした。近軸像高が負の系（倒立した像）では前と符号が逆になる。% の値は変わらない

### 変えたもの

- **玉の最大の直径（`surface` の `mech_diam`、.zmx の DMVA Mode 0）**: 無視した面は無いものとして玉をつなぐ（OpticStudio 26.1 と同じ）。
  円形の開口の面は、半直径の代わりに開口の最大の半径を縁にする（OpticStudio 26.1 と同じ）
- **レンズ単位が mm でない .zmx の多重構成の視野**: XFIE・YFIE の長さの換算を構成ごとの視野の種類（FLTP）で決める（前は元の種類で
  決め、FLTP が構成で変わると 25.4 倍ずれた）。角度と高さが入れ替わる構成で行の無い視野があれば断る。FLTP の 4（経緯儀の角度）は
  角度として表に読む
- **評価関数の項の誤りと注意**: `calc` の asin・acos の引数が −1〜1 の外なら、その項の誤りにする（前は値が有限でないとだけ出た）。
  `seidel`・`distortion` の項は回転対称でない系では誤りにする。回転対称を使った Wizard の行（Assume Axial Symmetry）を
  回転対称でない系の .zmx から読むと注意を出す（項は瞳全体を使うので OpticStudio と値が違う）。.zmx の多重構成で同じ数を
  違う書き方にした行（10 と 1.0E+01）を構成で変わる行と見なさない
- **GUI の最適化・公差・差し替え探索・ズームの表**: レンズデータの表と同じ操作にした。セルをクリックして打つか Enter・F2・
  ダブルクリックで書き換え、矢印・Tab で移り、Esc でやめる。Delete で空に、Insert で行を挿入、Ctrl+D で行を複製、Ctrl+Shift+↑↓（右クリックの「上へ」「下へ」）で行を動かし、Ctrl+Delete で行を削除、Ctrl+C・V で
  タブ区切りのコピー・貼り付け。数は Alt+ドラッグ、セルの右端のつまみのドラッグ、Alt+↑↓（Shift で 10、Ctrl で 0.1）で増減する。
  評価関数の項は 行＝項、列＝タイプ・パラメータ・重み・構成・値・寄与 % の表にし、パラメータの列の見出しは今の行のタイプの鍵の名前にした。
  「評価」で今のシステムを変えずに評価関数と項ごとの値・寄与を求める（変数なしで回す）
- **GUI の表の範囲の選択・補う入力・下の余白**: 最適化・公差・差し替え探索・ズーム・多重構成の表で、ドラッグ・Shift+クリック・Shift+矢印で四角の範囲を（数のセルの Alt+ドラッグ・右端のつまみは今までどおり値の増減）、
  行の番号で行全体、列の見出しで列全体、左上の # か Ctrl+A で表全体を選ぶ。コピー・Delete は範囲に効き、1 つの値を範囲に貼ると範囲を埋める。
  候補のあるセルは打った文字で始まる最初の候補で補う（補った部分は選んだ状態で、Backspace で打った文字だけに）、右の ▼ を押すと一覧から選べる。
  表の下に余白を置き、横のスクロールバーが最後の行に重ならない（結果の表とレンズデータの表も）。列の見出しのクリックは見出しのセル全体で受ける
- **GUI の評価関数の表のパラメータの列を畳む**: 「◀ パラメータの列を折りたたむ」でタイプごとのパラメータの列を「q efl · target 50」の要約の 1 列にする
- **serve の `optimize` の項ごとの寄与**: `terms` の各項に評価関数への寄与 [%] `contrib0`・`contrib`（重み × Σ残差² の割合）を足した
- **Web 版の解析の別の窓**: 開いた後の編集は差分だけを送る（窓が写しを取り違えたら全体を送り直す）。前の写しの計算の誤りは数えない。
- **非順次の物体を含む .zmx**: 非順次の物体の行（NSOH）を含む系（非順次モード・混合モード）は、行の名前だけの誤りでなく
  非順次の物体には対応していないと分かる文で断る

### 足したもの

- **評価関数のコメントの行**: 項の種類 `comment`（`{"type": "comment", "text": "…"}`、OpticStudio の BLNK）。項の並びを読むときに飛ばし
  （番号は数える）、.zmx のメリットファンクションに書き出すと BLNK の行になる。GUI の評価関数の表のタイプ「comment」
- **多重構成の表に面の材料**: 面の材料 `N.material` を多重構成の表の行にできる（値は構成ごとの材料の名前）。.zmx の GLSS を
  表に読み、書き出す（前は選んだ構成の材料だけを読んだ）。材料をピックアップする面も同じ行を持つ。編集の操作
  `set_config_materials`（param・names）、`add_config_row` も材料の行を足せる。serve の `config_table` の行に `names`
- **多重構成の表に開口の最大の半径**: 円形の開口・遮蔽の最大の半径 `N.apmax` を多重構成の表の行にできる。.zmx の APMX を
  表に読み、書き出す（前は選んだ構成の値だけを読んだ）
- **多重構成の表に半直径と面の無視**: 面の半直径 `N.semi`（固定の半直径。浮動開口の面はその半直径でける）と面の無視 `N.ignore`（0・1）を多重構成の
  表の行にできる。.zmx の SDIA・IGNR を表に読み、書き出す（前は選んだ構成だけを読んだ）。OpticStudio 26.1 の光線追跡と同じく、
  浮動開口（FLAP）のある面は構成ごとの半直径でける
- **多重構成の表に視野の種類**: 視野の種類 `ftype`（OpticStudio の番号 0〜3: 角度・物体高・近軸像高・実像高）を多重構成の表の
  行にできる。.zmx の FLTP を表に読み、書き出す（前は FLTP が構成で変わる .zmx は読めなかった）
- **多重構成の表に開口の種類と絞りの面**: 開口の種類 `atype`（OpticStudio の番号 0〜5）と絞りの面 `stop` を多重構成の表の行に
  できる。.zmx の SATP・STPS を表に読み、書き出す（前は選んだ構成だけを読んだ）。GUI の評価関数の項の欄に `min`・`max` を足し、
  `semi` と面ごとの `thickness`（`each`）の項を GUI から足せるようにした
- **多重構成の表に系の値**: 波長 `wave1`〜、波長のウェイト `wwt1`〜、主波長 `primary`、開口の値 `aper`、視野 `fx1`〜・`fy1`〜・
  ウェイト `fwt1`〜 を多重構成の表の行にできる。.zmx の WAVE・WLWT・PRWV・APER・XFIE・YFIE・FLWT を表に読み、書き出す
  （前は選んだ構成だけを読んだ。WLWT・PRWV・FLWT が構成で変わる .zmx は読めなかった）。ガラス（GLSS）が構成で変わる .zmx は、
  前は黙って構成 1 のガラスにしていたが、選んだ構成だけを読んだと知らせる
- **プロジェクトの ZPL マクロ**: プロジェクトに `macros`（マクロの名前と .ZPL の中身）を書き、ブロックの `surface_ops` の
  `add_solve`（`macro_curv`・`macro_thick`）と新しい `add_macro_solve`（円錐係数・半直径・面タイプのパラメータ）でマクロを
  名前か番号で指して解く。マクロのファイルは読まない。serve の `edit` でも `add_macro_solve` が使える。同じ名前（大文字・小文字を
  区別しない）のマクロを 2 つ置くと誤り
- **複屈折材の板の Mode 1・3**: .zmx の複屈折の板の Mode 1・3（異常光の道）を追跡する（前は断って光線を板で止めていた）。
  屈折は一軸結晶の式で解き、板の中は walk-off の向きへ進み、光路長は n(θ) で数える（OpticStudio は光線の長さあたり no で
  数えるので光路長・OPD が違い、読込の注意に出す）。近軸の量（像の位置など）は常光で数える
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
- **硝材の量と面の向きの項**: `surface` の `q` に `nd`・`vd`・`dpgf`・`cost`・`tce`（面の後の硝材。カタログの ED・OD の行を
  読む）と `ga`・`gb`・`gc`（面の z 軸の向き）。`to` を書くと面の範囲を面ごとに `min`・`max` と比べてはみ出しを足す。.zmx の
  GCOS・GTCE・GLCA〜GLCC・MNIN・MXIN・MNAB・MXAB・MNPD・MXPD を読み書きする
- **包む厚み・面のパラメータ・体積と質量の項**: `surface` の `q` に `total`（クリア半径の矢高で包む厚み、`x`・`y` で向き）と
  `param`（面タイプのパラメータ）、新しい種類 `volume`（面の間の要素の体積 [cm³]、`mass: true` で硝材の質量 [g]）。.zmx の
  TTVA・TTGT・TTLT・PMVA・PMGT・PMLT・VOLU・TMAS を読み書きする
- **角の収差・瞳の微分・矩形の RMS**: `ray` の `q` に `anax`・`anay`・`anar`（方向余弦の主光線との差）と `dxdx`・`dxdy`・
  `dydx`・`dydy`（交点の瞳座標での微分）を足した。.zmx の ANAX〜ANAR・DXDX〜DYDY・TRAI を読み書きし、RSRE・RSRH・RWRE・RWRH を
  格子の `spot`・`wavefront` の項として、SPCH を縦の色収差の差の `calc` の項として読む（RSRE などは書きもする）
- **一般の歪曲・像面湾曲**: `distortion` に `field`・`reference`（基準の視野からの線形の予測との差）・`peak`（軸上から
  視野までの最大）・`ftheta`（校正した f-θ 歪曲）を、`ray` の `q` に `fcgt`・`fcgs`（像面湾曲）を足した。.zmx の DISG・DIMX・
  DISC・FCGT・FCGS を読み書きする
- **グリッド歪曲の係数**: `distortion` に `coef`（基準の視野での A〜D）・`abcd`（予測に使う係数）・`component`（radial・x・y）
  を足した。.zmx の ABCD・DISA を読み書きし、BSER を読む
- **像の質の項**: `mtf` に `kind`（square 矩形波の応答・geometric 幾何 MTF・huygens Huygens MTF）と `dir: "avg"`（平均）を、
  新しい種類 `energy`（包囲エネルギーの大きさ・割合）・`strehl`（ストレール比）・`illum`（相対照度・実効 F ナンバー）を足した。.zmx の
  MTFA・MSWT・MSWS・MSWA・GMTT・GMTS・GMTA・MTHT・MTHS・MTHA・GENC・GENF・DENC・DENF・STRH・RELI・EFNO を読み書きする
- **波長・円柱の体積の項と、評価関数の流れの行**: `paraxial` の `q` に `wlen`（波長）を、`volume` に `cylinder`（面を包む円柱の体積）を
  足した。.zmx の WLEN・CVOL を読み書きし、ENDX・GOTO（後の行・飛ばす行を項にしない）と FDMO・FDRE（視野を変える）を読む
- **半直径の境界の項**: `semi`（面ごとの半直径を `min`・`max` と比べてはみ出しを足す）。.zmx の MNSD・MXSD を読み書きする
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
- **GUI の図の拡大縮小の道具**: レイアウト図（2D・3D）・スポット図・ガラスマップの右上（ポインタが図の上にあるとき）に
  ＋・－（押すたびに 1.25 倍ずつ）・囲んで拡大・全体のボタン。囲んで拡大は、ボタンを押してから左ドラッグ 1 回か、Ctrl（Mac は ⌘）＋左ドラッグで、
  囲んだ所を図いっぱいに広げる（Esc でやめる）。ホイール・右／中ドラッグなどの前からの操作は変わらない
- **GUI の多重構成の表**: 「多重構成」の窓（状態の表）を OpticStudio の Multi-Configuration Editor に当たる表にした。
  列は構成（状態）で、「+ 構成」で今の構成を写して足し、列の見出しの右クリックで写す・名前を変える・消す。「+ 行」で
  システムの値（波長・波長のウェイト・主波長・開口の種類と値・視野の種類・視野の x・y・ウェイト・物体距離・物体空間
  テレセントリック）、絞りの面と、要素の面の値（曲率半径・厚み・コーニック・非球面係数・クリア半径・無視・材料・
  コーティング）の行を足し、行の名前の右クリックで消す。面の表のセルの右クリック「多重構成の行にする」
  でも足せる。面の値はプロジェクトの状態の `set` の `surface_edits`、構成ごとの絞りは `surface_ops` に書く
  （要素の面で指すので、要素を足しても行はずれない）
- **多重構成の表のピックアップと構成の操作**: セルの右クリックで、その構成の値をほかの構成（か基本）の値 × 倍率 ＋ オフセットに
  保つピックアップ（OpticStudio の構成のピックアップ。プロジェクトの `gui.view.mc_picks` に保存）。列の見出しの右クリックで基本の値で足す・
  ピックアップで足す（Insert Config with Pickups）・左へ・右へ。開口の最大の半径（APMX）の行（円形の開口・遮蔽）
- **多重構成の表の操作をレンズデータの表と同じに**: 共通の表の部品で描く。矢印・Tab でセルを移り、打つ・Enter で書き換え、
  Alt+↑↓・Alt+ドラッグで数を増減、Ctrl+C・Ctrl+V、Ctrl+Shift+↑↓ で行を入れ替え、Ctrl+Delete で行を消す。セルを空にすると基本の値に戻す
- **開口・無視のある座標ブレークの .zmx も要素に分ける**: プロジェクトの面に `ignore`（面を無視する）を足し、座標ブレークの面にも
  開口（浮動を除く）と `ignore` を書けるようにした。開口や「無視」の付いた座標ブレーク・無視する面のある .zmx も要素に分けて開く
  （前は .zmx を読む要素 1 つで開いていた）
- **多重構成の表の行の足し方・別の行からのピックアップ**: 「+ 行」をシステムの値（波長・開口・視野・その他）・絞りの面・要素ごと
  （id と種類で見分け、ON/OFF・後の空気・偏心と傾き・面ごとの値）・群の移動の欄に分けた。セルの右クリック「別の行からピックアップ」で、
  同じ要素の別の行やシステムの値の行の値（自分の構成も）を元にできる。構成の切り替えは列の見出しのダブルクリック
- **多重構成の .zmx も要素に分けて開く**: GUI で開く多重構成の .zmx を構成 1 で要素に分け、構成ごとの違い（要素の後の間隔・
  面の値・材料・無視・絞りの面・システムの値）を状態「Config k」の値（多重構成の表の行）にする。構成で面の数・ソルブ・ピックアップが
  違うなど状態で書けない .zmx は、前と同じく .zmx を読む要素 1 つで開き、状態ごとに構成の番号を変える
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

- **.zmx の ABCD の目標の単位**: 角度でない視野（物体高・像高など）の .zmx で、ABCD・DISA の行の目標をレンズ単位から mm に直していた（係数は単位のない量）
- **GUI の材料の候補**: 材料の欄を開いた直後にガラスカタログが届くと、今の材料ではなく同じ位置の別の硝材（F2 のつもりが MGF2 など）が
  選ばれていたのを、今の材料を選び直すようにした。打って絞るときも名前の完全一致を先頭に出す
- **多重構成の表の空行**: .zmx の多重構成の表にある空の行（BLNK）で開けなかったのを、読み飛ばすようにした
- **GUI のレイアウト図の選択**: 要素を選ぶと図の縮尺が変わる（下の設定の欄の高さが要素で違った）、先頭に自動で置く
  絞り（厚さ 0）が選べない、BS を選んでもレンズデータの表の行が選ばれない（鏡で戻って BS を通る面にも BS の名前を付けた。
  serve の `blocks` の BS の範囲に復路の分も入る）を直した
- **GUI**: スポット図の欄「縮尺」を「枠の幅」にした（効くのは枠の幅で、縮尺の長さはそれに合わせる）。行で書いたミラーの要素の
  半直径を行から取る（前は 0 で、配置図が下限の 1 mm で描いた）
- **GUI のスポット図**: パネルが低いと最後の行の縮尺・視野の名前・RMS が下に切れていたのを、枠の大きさを残りの幅と高さの
  両方に合わせて全視野が入るようにした（狭いときは視野の名前と RMS を小さい字で 2 行に）。縮尺の欄の高さを字体から測り、
  行の高い字体（日本語）で縮尺の長さの文字の上が切れないようにした
- **serve**: `preview` を読むだけの要求として、ほかの読む要求と並行に処理する

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
