# 壊れる変更

版を上げるときに読むための、使い方が変わる変更の一覧（新しい順）。細かい経緯は各 PR。版ごとのまとめは
[CHANGELOG](https://github.com/masa-iwm/lensbench/blob/main/CHANGELOG.md)、版の付け方は [版の付け方](versioning.md)。

## 次の版（v0.5.0 の後）

### .zmx の読込の場所
- `lensbench_core::zmx`（`load`・`load_with`・`LoadOptions`・`parse_text`・`build` など）を `lensbench_formats::zmx` に移した。
  `lensbench_core::prelude` から `zmx`・`LoadOptions` を外し、それを足した `lensbench_formats::prelude` を置いた。

### 最適化の項
- `zernike` の項を 37 項以上で当てはめた係数にした（前は `term` 項までで当てはめていた）。格子の上の最小二乗では当てはめる項の
  数で低い次の係数が動き、Double Gauss 28 の視野 2 で Fringe Z9 が 0.0944 → 0.0917 波（OpticStudio 26.1 の ZERN と 9 桁一致）、
  Standard Z11 が 0.0422 → 0.0410 波（OpticStudio は 11 項で当てはめた 0.0422）。目標を前の値で決めていたら見直す。前と同じ値は
  当てはめる項の数 `fit` に `term` と同じ数を書く。
- Rust の `lensbench_optim::TermKind::Spot` に測る向き `axis: SpotAxis` を足した（`SpotAxis::Both` が前と同じ）。`Quad` に
  `Arms(n)`・`Grid(n)` が増えたので、`Quad` を網羅する `match` は書き足す。
- `TermKind` に計算の項 `Calc { op, args, target }` が増えた（`TermKind` を網羅する `match` は書き足す）。`TermKind::target()` は外から使える。
- `TermKind::Thickness` に `edge: Option<f64>`（縁の厚みの高さ）と `each: Option<Each>`（面ごとの境界）を足した（どちらも `None` が前と同じ）。
- `TermKind::Paraxial` に `wave: Option<usize>`（efl・pimh の波長、`None` が前と同じ主波長）を足した。
- `CalcOp` に `Max`・`Min`・`Qsum`・`Sin`〜`Atan` が増えた（`CalcOp` を網羅する `match` は書き足す）。
- `TermKind::Surface` に `wave: Option<usize>`、`SurfQ` に `Power`・`Ynip`・`Index` が増えた。
- `ParaxQ` に `Amag`・`Linv`・`Petc`・`Petz` が増えた（`ParaxQ::ALL` は 17 個）。
- `TermKind` に `ParaxRay { field, wave, px, py, surf, q, target }`（近軸光線の量）が増えた。
- `TermKind` に `RayPath { field, wave, px, py, surf, to, q, upper, target }`（面の範囲の光線の量）が増えた。
- `TermKind` に `Surface { surf, q: SurfQ, x, y, target }`（面の量）が増えた（`TermKind` を網羅する `match` は書き足す）。
- `RayQ` に `TraX`・`TraY`・`TraR`・`Opth`・`Gx`〜`Gn`・`R`・`Ang`・`TanX`・`TanY`・`Nl`〜`Nn`・`Aoi`・`Aoe`・`CosI`・`CosE` が増えた（`RayQ` を網羅する `match` は書き足す）。

### ユーザーカタログの項目の型
- `lensbench_project::catalog::UserItem` の中身を、レンズ・機構部品・未完成で分けた `body: ItemBody`（`Optic { blocks, frame }`・`Mech(MechSpec)`・
  `Incomplete { missing }`）にした。前の `blocks`・`frame`・`mech`・`missing` の欄は同じ名前の読む口（`blocks()`・`frame()`・`mech()`・`missing()`）から引く。JSON の形は変わらない。
- `lensbench_catalog::spec` の絞り込みは、行を一度だけ読んだ `CatalogRow` に対して行う（`SpecFilter::matches`・`CatalogQuery::matches` の引数）。

### 過去の形式の読込をやめた
- システムファイルの版 1（core の System の serde をそのまま書いた形）は読まない。誤り「version 1 is no longer read」。lensbench 0.5 で
  開いて保存し直すと版 2 になる。
- core の列挙（`ApertureKind`・`Aiming`・`FieldNorm`・`AfocalUnit`・`VigInterp`）の旧名の別名（`"Epd"`・`"FNum"`・`"Real"`・
  `"Milliradians"`・`"OpticStudio"` など）を消した。今の名前（`"epd"`・`"fnum"`・`"real"`・`"milliradians"`・`"opticstudio"`）だけを読む。
- GUI の `lensbench-gui --convert-combo` を消した（前の版の `gui.combo` 入りの保存の変換）。
- Python の pickle は `glass_dirs` の入ったもの（0.5 以降が書いたもの）だけを読む。

### 条件で定める視野
- core の `FieldCond` の JSON を、システムファイル版 2 の `field.solve` と同じ形（`{"cond": "position", "surface": 5}`・
  `{"cond": "dir_sum", "s1": 2, "s2": 7}`）にした。系ファイルの `FieldSolve` は `FieldCond` そのものになった。
  serve の `system_data`・`set_system_data` の `field_type` の `Solve` の中身もこの形。

## v0.4.0 の後・v0.5.0 まで

### システムファイル
- 面の量の型を 1 つにした（IOO-08）。ピックアップの量 `lensbench_core::pickup::PickKind` をやめ、パラメータの種類 `lensbench_core::param::Kind` に
  `Semi`・`Parm(番号)`・`Before(i)`・`After(i)`・`Coating` を足した（これらはピックアップ・マクロのソルブだけの量で、パラメータには使えない。
  `Kind::is_pick`・`is_pick_only`）。名前（`Kind::parse`・`name`）も 1 つで、`pick_kind_parse`・`pick_kind_name` はそれを引く。
  マクロのソルブ `MacroParm` は `parm`・`col`（`MacroCol`）をやめて量 `what`（`Parm(番号)`・`Conic`・`Semi`）を持つ。システムファイル版 2 の
  `macro_parms` は `{"surf": 3, "id": 0, "what": {"Parm": 2}}` の形。
- パラメータ（`lensbench_core::param::Param`）の `surf` をやめ、付く先 `target`（`Target::Surf(面)`・`Target::Unit(玉・群)`・`Target::System`）にした
  （IOO-08。番号の意味が種類で変わらない）。システムファイル版 2 の多重構成の表の行の `param` は `{"target": {"surf": 3}, "kind": "Thick"}` の形。
  Rust は `Param::new(番号, 種類)`（付く先は種類から）・`Param::unit`・`Param::system` で作り、番号は `surface()`・`unit_index()`・`index()`。
- ソルブの書き方を変えた（IOO-19）: システムファイル版 2 の `solves` は `{"surf": 5, "type": "edge", "thickness": 1.5, "radius": 4}` のように
  種類（`type`、snake_case）と種類ごとの値の欄を持つ（前の `height` と `kind` はやめた）。版 1 のファイルは読むときに直す。
  Rust は `lensbench_core::param::ThickSolve` を `lensbench_core::solves::Solve { surf, kind }` に、`SolveKind` の各種類が値を持つ
  （`MarginalHeight { height }`・`Edge { thickness, radius }`・`Position { other, length }` など。`value()`・`with_value()` で条件の値）。
  `solve_kind_parse(名前, value, param, other)`・`solve_kind_info` は value も扱う。ピックアップは `lensbench_core::pickup`
  （`PickKind`・`Pickup`・`pick_kind_parse`・`pick_kind_name`・`surf_parm`）、多重構成の表は `lensbench_core::configs`
  （`Configs`・`ConfigRow`・`ConfigPick`）、`SOLVE_KINDS` は `lensbench_core::solves` に移した。`sweep::solve` は `linalg::solve_or_zero`。
- システムファイルを版 2 にした（PRJ-24・IOO-10）。`system` は内部の構造体の serde でなく、利用者が書いた値（処方）だけを決めた形で書く
  （[JSON スキーマ](reference/schema.md) の system）。面は形 `shape`・働き `action`・媒質 `medium` などの行で、曲率は `c`。求めた屈折率
  `n`・`n_base`、自動のクリア半径の値、ガラスの式（カタログ全体）は書かない（玉 2 枚の ACH-45 は 760 KB から 1.8 KB）。ガラスは名前と、使った
  カタログの名前・ファイルの hash（`catalogs`）だけを書き、読むときにカタログを `glass_dirs`・システムファイルのフォルダ・LENSBENCH_GLASSCAT・
  そのプロセスで読んだカタログから探す（中身が保存時と違えば注意を出し、今のカタログの値で追う。見つからなければエラー）。
  書くときにシステムのガラスと値が同じカタログのファイルが見つからなければ、そのカタログの hash は空にして注意を出し（CLI・Python の警告、
  serve の `save`・`save_range` の `notes`）、hash が空のカタログを読むときは値を確かめられないと注意を出す。
  版 1 のファイルは読めるが、書くのはいつも版 2。`field_wave`・`import_notes`・`macro_notes` は書かない。
- システムの hash（`system_hash`・`s.hash`・解析の結果の `system_hash`）は中身が変わったので値が変わる。前の結果に刻んだ hash とは合わない。
- 光線が届かず求め直せない自動のクリア半径は、読み直すと 0 になる（前は保存した値が残った）。
- Python の `System.from_json` に `glass_dirs=` を、pickle にカタログのフォルダを足した（古い pickle も読める）。Rust は
  `sysfile::from_str_with`・`load_with`・`catalog_files`、`lensbench_core::glass::loaded_files`・`load_agf_shared`（公開に）。

### プロジェクトの面の上書き・操作
- zmx ブロック（と、取り込んだ系ファイル・入れ子のプロジェクト）の面のガラスは、その .zmx の GCAT で引いたものを使う
  （前は `glass_catalogs` を先に引いた）。`glass_catalogs` に同じ名前で値の違うガラスがあれば、取り込んだ方を「名前@カタログ」の
  名前で持ち、読込の注意（`import_notes`）を出す。.zmx に書き出すときは元の名前で書く。システムファイル・プロジェクト・
  ユーザーカタログのガラスの名前にも「名前@カタログ」を書け、そのカタログ（`catalogs`・`glass_catalogs` か、ガラスのフォルダ）の
  その名前のガラスを引く（付け替えた名前のまま保存しても読み直せる）。複数のカタログに同じ名前で値の違うガラスがあると、
  前のカタログのものを名前だけで、後のものを「名前@カタログ」で引ける
- 鏡のブロック（Rust の `Block::Mirror`）に `row`（`Option<Box<sysfile_v2::Row>>`、radius・conic で書けない鏡の面の行）を足した
  （GAR-02）。欄を全部並べて `Block::Mirror` を作るか分けていたら `row` を足す。鏡の面を行にする `lensbench_project::mirror_row` を足した
- Rust の `expand` の誤りを `ExpandError`（誤り `error` と、ブロックの中で起きたときの位置 `block: Option<BlockAt>`）にした
  （PRJ-12）。`BlockAt` は木の中の位置 `node`（`NodePath`）と名前 `name`（各段の id、id の無いブロックは「#番号」、例 `G1/#2`）。
  `?` で `lensbench_core::Error` に直せ、そのとき文の前に「block G1/#2: 」が付く。`message()`・`kind()`・`code()` はそのまま使える
- Rust の `lensbench_project::open::Source` の `Zmx`・`SystemFile` を `{ path, text }` にした（CLI-21）。読んだときの中身を持ち、
  別の構成・波長の置き換え・構成の数（`configurations()`・`zmx_configs()`）はファイルを読み直さずにそこから作る（serve の
  config_table・config_diff・configs・重ね描き・swap も）。core に `zmx::load_text_with`・`zmx::load_configs_text` を足した
- Rust の `ExpandOpts` に `within`（外側で展開中のファイル・項目、ふつうは空）を足した。欄を全部並べて作っていたら
  `..Default::default()` を足す（PRJ-28）。プロジェクトのインポートの循環は深さ 8 の上限でなく循環の道で見つけ、
  エラーの文は「a loop of imports: a.json → b.json → a.json」になった（ユーザーカタログの項目の循環も同じ文）
- プロジェクトファイルに形式の印 `"format": "lensbench-project"` と版 `"format_version": 1` を足した（PRJ-23）。書くときはいつも付け、
  読むときは省略を許すが、印が違うか版が新しいファイルは誤りにする。Rust の `Project` に `format`・`format_version`（`Option`）が増えた。
  この版より前の lensbench は、この版が書いたプロジェクトファイルを「unknown key format, format_version」で読めない
- プロジェクトの直下の `surface_edits`・`surface_ops` をやめ、各ブロック（玉・絞り・ミラー・面の並び・zmx・user・群と zmx のインポートの群）が
  持つようにした（PRJ-04）。`surface_edits` の項目は `block` を書かず `{"surface": i, "fields": {…}}`、`surface_ops` の当てる面
  （`surface`・`dst`）はそのブロックの面の番号（参照する `src`・`other` は前どおり `{"block": id, "surface": i}` かシステムの面）。
  直下に書いた前のファイルは「unknown key」のエラーになるので、項目をそのブロックへ移す。状態で外したブロックの上書き・操作は
  注意なしで当てない（前は「block … not found」の注意）。surface_ops のエラーの文は `block L2 surface_ops 3: …` のようにブロックを示す。
  Rust は `Project::surface_edits`・`surface_ops` をやめ、`Block::faces()`・`ZmxGroup::faces`（`FaceMods`）、`SurfaceEdit` から `block` を外した。
- 面の並び・自作の玉の面に `row`（システムファイル版 2 の面の行）を書けるようにした。Zernike・Grid Sag・Irregular・Tilted・Binary 2・
  理想レンズ・GRIN の媒質・無視する面なども、プロジェクトのブロックで書ける。GUI は鏡・座標ブレークの無い .zmx を、これで要素に分けて開く。
- プロジェクトに像面の `image`（版 2 の面の行: 曲がった像面・傾き偏心など）を足した。像面を傾けた .zmx も組合せで開ける。
- GUI はシステムファイル（`.lbsys.json`）も .zmx と同じく組合せで開く。保存は元のファイルに上書きせず、プロジェクトファイル（.json）として保存先を選ぶ。
- GUI は多重構成の .zmx も組合せで開く。.zmx の取り込み 1 つと、構成ごとの状態「Config 1」〜「Config n」（取り込みの構成の番号と、構成で変わる開口・視野・物体距離など）になる。
  要素の設定の「構成」は、状態を選んでいればその状態の構成の番号。
- serve の `save_project`・`write_back` は、`set`・`set_param` の厚さ・半径・コーニック以外の面のパラメータ（非球面係数・Zernike・偏心など）も、変わった面のデータの項目としてブロックの面の上書きに書き戻す（前は書き戻さずに注意）。GUI は最適化などの値を組合せに当てるとき、これで厚さ・半径・コーニック以外も要素に入れる。
- `zmx_project`（GUI が .zmx を組合せとして開く）は、条件で定める視野の .zmx も書く。
- GUI の組合せでは、多重構成のパネルに状態の表（行 = 状態で変えた値、列 = 基本と状態）を出す。
- GUI はファイルを必ず組合せで開く。読めるが組合せで同じシステムを書けないファイルは開かずに理由を出す（前は「開いたシステム」として開いた）。読めないファイル（ガラスが無いなど）の案内は前と同じ。状態で変える欄の無い系の値（温度・気圧など）は組合せがそのまま持って書き戻す。モデルガラスにした写し・ツアーの例も組合せで開く。開いたシステムだけの画面（システムの設定の欄・「このシステムを組合せにする」・開いたシステムの図やレンズデータへのカタログの差し込み）を消した。開いたシステムの多重構成の表（Multi-Config のパネルは状態の表だけになる）と玉・群・機構部品の欄も消した。Ctrl+A・Ctrl+Shift+A は組合せの次・前の状態にする。ツアーの「玉・群」の段も無くした。レンズデータの開いたシステムだけの操作（玉・群を作る・外す、面を挿入して貼り付け、ファイルごとのブロックのロック）と、差し替え探索のプロジェクトのブロックを使う形も消した。レンズデータの行の右クリックの「前・後に面を挿入」「上書きで貼り付け」「この面を消す」は組合せでも出す。物体面のコメント・物体面を元にするピックアップ・マクロのソルブの注意のある .zmx も組合せで開く（zmx ブロックの展開がこれらを持ち越す）。zmx ブロックの展開は ZPL マクロとマクロのソルブ（パラメータ・円錐係数・半直径）も持ち越す（前は Macros のフォルダが見つかる .zmx が「no macro」で開けなかった）。プロジェクトに瞳の振幅の分布 `apodization` と OPD の基準 `ropd` を足し、.zmx のこれらの値も組合せに残る。無限遠系の単位の大文字・小文字（.zmx の MR など）でも開く。プロジェクトに物体面の形・開口 `object` を足し、物体面を傾けた・物体面に開口のある .zmx も組合せで開く。像面の半直径のピックアップ、無限の厚み（物体面）を写す厚さのピックアップ（1e10 として写す。.zmx に書かれた値と同じ）のある .zmx も開く（像面の厚さは組合せに書かない）。半直径の行の無い像面の浮動開口も半径を持ち越す。.zmx を組合せにするときは .zmx のガラスカタログの指定（GCAT）で引いたガラスと同じになる `glass_catalogs` を書く（前は既定の SCHOTT が先で、同じ名前のガラスが別のカタログにもあると屈折率が変わった。そういう .zmx を組合せにした結果は変わる）。プロジェクトはガラスのカタログを zmx ブロックの .zmx のフォルダでも探す（.zmx を読むときと同じ）。物体空間・像空間が空気でない .zmx（液浸・像側の硝材・硝材の中へ折り返すミラーなど）も開く（物体空間の媒質はプロジェクトの `object` の `medium` に書く。面 1 から取り込んだ .zmx の物体空間の媒質がこれと違えば、展開の注意 `NoteKind::Medium` を出してプロジェクトの媒質を使う）。マクロが大域座標を読む .zmx は基準の面を決めてから解き直す。多重構成のどれかの構成が読めないファイルは、開けない理由にどの構成かを出す。GRIN の面の材料名（Gradium の分布の名前など）がガラスのカタログに無い .zmx も開く。
- GUI は間に空気の無い要素（絞りの直後の玉など）のあるプロジェクトも、空気 0 として組合せで開く（examples・tests のブロックのプロジェクトは全部が組合せで開く）。
- プロジェクトの zmx ブロックで取り込んだ .zmx の膜ファイル（COFN）を、展開したシステムの膜ファイルにする（前は膜ファイルが外れ、面の膜を既定の表で引いた）。
  膜ファイルの違う .zmx を 2 つ以上取り込むと、最初のものを使い注意を出す。

### 名前・設定の書き方
- ズーム軌跡（CLI `zoom`・serve `zoom`・Python `zoom`）の目標の満たし方の既定を拡大ラグランジュ法 `lagrange` にした（IOO-27、前は `penalty`）。
  目標が始めの値から遠いと像の評価関数が前より小さくなり、値が変わる。前の結果を出し直すには `--method penalty`（serve・Python は `method`）。
- 解析のサブコマンドに `--params-file`（設定を JSON のファイルで渡す。コマンド行で明示した旗が優先、[CLI](cli.md)）。
- 開口の種類・視野の正規化・エイミング・無限焦点の単位・ケラレの補間を、core・プロジェクト・システムファイルで同じ列挙にした（XCT-09）。
  serve の `system_data`・`set_system_data` の `data`（GUI のシステムの設定もこれを使う）と Rust の core の列挙の serde の名前はシステムファイル版 2 と同じ小文字（`"epd"`・`"fnum"`・`"na"`・
  `"float"`・`"pwfn"`・`"cone"`、`"off"`・`"paraxial"`・`"real"`、`"radial"`・`"rect"`、`"degrees"` など、`"opticstudio"`・
  `"continuous"`）。前の名前（`"FNum"`・`"Real"`・`"Rect"` など）も読む。Rust の `lensbench_project::model::ApertureType`・
  `FieldNormSpec`・`AimingSpec` は消し、`lensbench_core::system` の `ApertureKind`・`FieldNorm`・`Aiming` を使う。
- 物体空間 NA の量の名前を `obna` から OpticStudio の演算子名 `obsn` にした: 評価関数の `{"type":"paraxial","q":"obsn"}`、Python の
  `paraxial()` の鍵、`report::SystemData::obsn`、GUI の最適化パネルの選択肢。旧名 `obna` は受けない（エラーになる）。.zmx のシステムの行 `OBNA`（絞りの種類）はそのまま。
- lensbench 自身の古い保存の読み分けを消した（XCT-24・GAR-23、11/16 masa）: 自動のクリア半径を入れる前の lensbench が書いた .zmx
  （`VERS 260327 42 20120530 20120530` で MEMA の行が無い）のクリア半径を固定として読む分岐と、面の行の `IGNR 1`。そういうファイルは
  OpticStudio と同じく `DIAM 値 0` を自動として読むので、固定にしたい面は `DIAM 値 1` に直す（同梱の例のレンズと BS の多重構成の試験のシステムは直した。OpticStudio と照合した試験のシステムは OpticStudio と同じ自動で読む）。
  GUI の組合せの視野・波長の数だけの並び（今は `{"y": …}`・`{"um": …}`）、初めてのポータブルで OS の場所の設定を写す処理
  （ポータブルは空の設定と exe の隣の `catalogs/` から始める）、取った品の STEP のキャッシュの gzip でない `.step` も読まない。
- 過去互換の受け口を消した（XCT-34、11/16 masa）: 評価関数・公差・差し替え探索の `wfe`（`wavefront` に）と MTF の向き `t`・`s`
  （`tan`・`sag` に）の別名、serve の `analyze` が数の文字列（`"2"`）・カンマ区切りの文字列（`"10,20"`）・1 つの数の並び・JSON の
  文字列を読み替えること（値は JSON の型で渡す）、旧名（`glass_dir`・`lens_catalog`・`path`・`ref`・`steps`）を新しい名前で示すエラー
  （serve の `load`・`glasses`・`lenses`・`catalog_search` は知らない鍵をエラーにする。解析は知らない設定のエラー）、環境変数
  `LENSBENCH_FLOATING_CLIP` の注意、GUI の公差パネルの設定の `ref`・CAD 置き場の設定に書いた Client Secret の読み込み。
- メーカー名の比べ方を 1 つにした（XCT-12・PRJ-17）: 大文字小文字・空白・記号を無視する。差し替え探索の `filter.vendor` は前の
  「一致」から、検索と同じ「カタログのメーカー名に含まれれば合う」に。公差の `catalog_tols` は複数のメーカー名が合えばいちばん
  長い名前を使う（前は書いた順の最初）。ユーザーカタログの品の欄の意味のない組み合わせ（`blocks` と `mech` の両方、面の無い
  `frame`、枠の無い `frame_reversed`、面か `mech` のある品の `missing`）は読むときにエラー。
- 名前を CLI・serve・Python でそろえた（CLI-14・CLI-17、11/16 masa 決定。旧名は受けない）:
  Python の `System.geometric_mtf`・`through_focus_mtf`・`field_mtf` は登録簿の名前どおり `geo_mtf`・`mtf_focus`・`mtf_field`。
  `trace`・`trace_rays` の結果の波長の番号の鍵 `wavelengths` は `waves`（引数と同じ）。
  番号の並びを取る `rays`・`mech-check`・`layout` の視野・波長は `--fields`・`--waves`（serve・`analyze` の設定は `fields`・`waves`。
  1 つの番号を取る解析は前どおり `--field`・`--wave`）。`ghost` の書き出し先のフォルダ `--out-dir`（`out_dir`）は `--out`・`-o`（`out`）。
- FFT PSF の既定の面を像面の上にした（ANA-08）: 格子の座標は像面の局所座標の x・y（`axes` は x̂・ŷ）で、回折 MTF と同じ
  周波数の面（PSF をフーリエ変換すると回折 MTF になる。x・y の両方に角度のある視野では、回折 MTF の子午・球欠は前と同じく
  瞳の py・px の向きのずれなので、像面の上では少し傾いた向きの周波数に当たる）。x・y の両方に角度のある視野では瞳の格子を
  写して像面の上の位相の傾き J = ∂(L, M)/∂(px, py) の 2×2 全部を入れる（Huygens PSF と同じ形になる）。斜めの視野では子午の既定の間隔が 1/cos だけ広がる
  （E1 の視野 3・主波長で Δy 5.588e-4 → 6.324e-4 mm）。前の主光線に垂直な面は `PsfOpts::plane = PsfPlane::ChiefPerp`
  （CLI・serve の `psf` の `--plane chief`、Python の `psf(plane="chief")`、プラグインの `plane = "chief"`）で、OpticStudio の
  FFT PSF との照合用。`PsfOpts`・`Psf` に `plane` を足した（構造体の式で作るなら `..Default::default()` を）。
  Huygens PSF の既定の間隔も像面の上の値（前は主光線に垂直な面の傾きから決めていた）。
- 瞳の格子を `lensbench_core::pupil::PupilGrid` に、格子の光線と OPD を `PupilFunction` にまとめた（ANA-04）:
  `lensbench_analysis::interference::opd_grid` は格子の座標の並びでなく `PupilGrid` を受ける（前の `pupil_grid(n)` の並びは
  `PupilGrid::Interferogram(n)`）。`analysis::pupil_grid`・`wavefront_grid`・`pupil_gauss`・`interference::pupil_grid`・
  `mech::pupil_grid` は残し、`PupilGrid` の値を返す。値は同じ。
  `PupilFunction::trace`・`trace_amp` は `Arc<PupilFunction>` を返す。結果は系にしまい、同じ系・視野・波長・格子なら追い直さない
  （系を直すと捨てる。膜・偏光の振幅つきはしまわない）。
- GUI の組合せの保存は、ブロックが元になった（文書モデル）。`gui.combo`（組合せの編集状態の写し）は書かず、読まない。前の版で
  保存したファイルはブロックから組合せに戻るので、システムはそのまま開けるが、ロック・今の状態・カタログのミラーや平板の品名は外れる。前の版で保存したファイルは `lensbench-gui --convert-combo ファイル…` で一度だけ今の形に直す（元は `.bak` に残す。この変換は次の版で消す）。
- 最適化（`optimize`）とズーム軌跡（`zoom`）の設定を CLI・serve・Python・GUI で 1 つの型にした（XCT-02）。serve の `optimize`・`zoom` の
  params は知らないキーがエラー（前は無視）、`merit`・`reference`・`jac`・`method` は決まった綴りだけ（大文字・小文字は問わない。
  エラーの文に鍵の名前が付く）。serve の `optimize` の変数に CLI と同じ範囲の書き方 `"7.t=0.5:3"` を、`zoom` の `efl`・`pmag` に CLI と同じ
  `"50,-,80"` の文字列を足した。`optimize --json` の `merit` は打った綴りでなく正式の名前。
  `zoom` で `efl` と `pmag`（CLI は `--efl` と `--pmag`）を両方与えるとエラー（前は `efl` を使った）。
- Python に、登録簿の解析を名前で回す `System.analyze(name, **settings)`、解析の一覧 `lensbench.analyses()`、既定の設定
  `lensbench.defaults(name)` を足した（CLI のサブコマンド・serve の `analyze` と同じ設定・結果）。
- Rust の 2 次元の結果を `lensbench_core::grid::Grid2` に（ANA-18）: `MtfSweep` の `tan`・`sag`（x = 横軸の点、y = 周波数。
  `tan[k][i]` はそのまま。`x`・`freqs` は同名のメソッド）、`DetectorFringe` の画素の値（`n`・`x0`・`y0` は消し、座標は格子の
  `x()`・`y()`、`n()` はメソッド）、`PolGrid` の格子点の値（`nx`・`ny` はメソッド、値は `values()`）。POP の `Beam` は伝搬の作業用の
  形のまま（`u`・`x0`・`dx`）で、座標つきの格子は `Beam::grid()`・`irradiance_grid()`。プラグインの配列は `plugin::Array::grid` で作る。
  Python・CLI・serve の出力は同じ。
- PSF・Huygens PSF・Huygens MTF の像の格子の間隔 `delta` を `pixel` に改名した（XCT-18。干渉縞の位相差 `delta` と同じ名前で別の意味だった）:
  CLI の `--delta` は `--pixel`、Python の `psf()`・`huygens_psf()`・`huygens_mtf()` の引数・結果の鍵、プラグインの設定・出力、Rust の
  `PsfOpts::pixel`・`Psf::pixel`、保存した設定（GUI・serve の JSON）の鍵。旧名 `delta` は残さない（エラー）。
- 差し替え探索の `vars`・`state_vars` にオブジェクトの形 `{"name": "d1.t", "min": 0.5, "max": 40}` を足した（PRJ-30。文字の形も前どおり）。
  Rust の `SwapSpec::vars` は `Vec<VarSpec>`（`"d1.t".into()` で作れる）。
- 波長の番号のエラーはどの口でも「no wavelength 9 (1 to 3)」（XCT-18、`lensbench_core::waves::index`）。
- Rust のモジュールの置き場所を変えた（XCT-04）: `lensbench_core::zmx_write` は新しいクレートの `lensbench_formats::zmx_write`、
  `lensbench_core::{ghost, interference, polgrid}` は `lensbench_analysis::{ghost, interference, polgrid}`。中身・Python・CLI は同じ。
- カタログ・STEP のクレートを分けた（PRJ-05/06/21・XCT-11）: `lensbench_project::catalog_fill` は `lensbench_catalog::fill`、
  `catalog_spec` の検索・絞り込み（`CatalogQuery`・`SpecFilter`・`spec_of`・`apply_overlays` など）は `lensbench_catalog::spec`
  （型の `ItemKind`・`Overlay`・`SPEC_NAMES`・`spec_unit` は project に残し、spec からも見える）。STEP の読み取りは新しいクレート
  `lensbench_step`（`StepFile`）の 1 本で、寸法の測定（`read_step`・`StepSize`）は `lensbench_catalog::step`。`cad_cache_*` は
  `lensbench_cad` に。機構部品の STEP は大域の差し込み（`install_detail_loader`）をやめ、`Env::step_files`・`open_with` と
  `Options3D`・`OptionsStep` の `mech_detail: Option<Details>`（`lensbench_cad::detail::details()` で作る）で渡す。CLI・Python・GUI は同じ。
- レイアウトの描き方（光線の選び方・書き出しの形）の既定値と範囲を CLI の `layout` と serve の `layout`・`layout_export` で
  1 つにした（`lensbench_engine::layout`、CLI-02）。CLI の `layout --plane xz` で `--fan` を省くと x の扇（前は y の扇。serve と同じ）、
  `--rays` は 1〜200、`--width`・`--segments`・`--rings` は serve と同じ範囲に収める。`--tube` は正の数だけ（0・負の値は使い方のエラー、
  終了コード 2。前は黙って受けた）。`--fan`・`--plane`・`--color` の不正な値は引数の解釈でエラー（終了コード 1 → 2、選択肢を出す）。
  serve の `field`・`wave` に `"all"` を足した。
- 最適化・ズームの結果を型にした（GAR-04）: `lensbench_engine::session::views::{OptimizeView, ZoomView}`。serve・CLI はこの型を JSON に、
  GUI は同じ型で読む（鍵が足りなければエラー）。Rust の `lensbench_engine::zoom::zoom_systems` は `Value` でなく `ZoomView` を返す。
- 拡張の登録を拡張の表 1 つに（XCT-07）: 検出器の種類（`lensbench_project::detectors`）・拡張の解析（`lensbench_engine::register`）・
  GUI の見せ方（`lensbench_gui::ext`）・Python の模擬の組（`register_native_task`）・Web 版の同梱ファイル（`set_extra`）・膜のモデルを
  取る口（`coating::set_remote_loader`）は、それぞれのプロセス大域の表をやめ、`lensbench_core::ext::Ext` の型ごとの欄
  （`Ext::slot`・`slot_mut`）に載せた。登録の関数は前どおりプロセスの既定に足し、`ext::with` で渡した表はその計算だけに効く。
  `Extensions::core_ext()` は `Extensions::ext()`（名前が組み込みと重なればエラー）に、表に足すのは `add_to`。
- 単色の干渉縞の解析 `fringe` を `interference` に改名した（CLI のサブコマンド・serve の `analyze` の `cmd`・GUI の解析パネル・
  `lensbench_client` の `interference()`）。Python の `fringe_detector` は `interference_detector`、Rust の `lensbench_core::fringe` は
  `lensbench_core::interference`。旧名は残さない（GUI の設定に残った `fringe` のパネルは開き直す）。Zernike の並び `kind = "fringe"` は変えない。
- 引数の名前を CLI・serve・Python で 1 つにそろえ、旧名は消した（旧名はエラー。serve は「X は Y に名前を変えた」と出す）。
  対応表は [Python API の約束](python.md) の「引数の名前」。Python API は 2.0。
- プロジェクトの知らないキーはエラー（GUI だけの設定は `gui` 欄）。`environment` 欄（既定 20°C・1 気圧）。
- 差し替え探索の設定の `merit`・`reference`・`jac` は決まった綴りだけ（大文字小文字を区別する）。
- serve の `analyze` は CLI と同じ `--flag=値` の並びで受ける。
- `tolerance --save-worst` は `--out` に（最も悪い試行のシステムの書き出し先。システムを書き出す旗はどれも `--out`）。`--out` はどのコマンドでも `-o` でも渡せる。
- 評価関数の項（`optimize --term`・`terms` 欄）は種類ごとの型で読む（スキーマ `terms.schema.json`）。`dir`・`reference`・`kind`・
  centroid の `q` は決まった綴りだけ（小文字。`type` は前どおり大文字も可）。型の合わない値・知らないキーのエラーの文は serde のもの
  （例「ray の項: unknown field `taget`, expected one of …」）。
- serve の `edit` の操作とプロジェクトの `surface_ops` は操作ごとの型で読む（スキーマ `edit-ops.schema.json`、`surface_ops` は
  `project.schema.json` の中）。知らないキー（以前は黙って捨てた）・型の合わない値（以前は `scale`・`value` などは既定値にした）はエラー。
  `edit` の読みのエラーは invalid params（-32602、以前は -32000）で、どの操作も当てない。`surface_ops` は `set_stop`・`add_solve`・
  `add_pickup` だけで、ほかの操作は展開のときでなく読むときにエラー。書き直したプロジェクトの `surface_ops` の数は実数で書く（`-1` → `-1.0`）。
- プラグインの約束（ANA-19）: 設定の説明は `Plugin::params()` だけ（`Spec::config` を消した）。同梱の解析も型・既定・単位つきの
  項目を持つ。Python の `plugin_spec(name)["config"]` は (名前, 説明) の組でなく項目の dict（name・type・default・unit・doc・choices・len）
  の並び、serve の `plugins` の各プラグインから `config` を消した（`params` だけ）。設定の値に真偽（true・false）を足し、真偽の項目は
  true・false か 0・1 を受ける（Python は True・False がそのまま渡る。前は bool をエラーにした）。省略できる数・整数の項目に NaN を書くと
  「省略」でなくエラー。出力の説明も名前・型（float64・complex128・bool）・形・単位つき（`Spec::outputs` は `OutputSpec` の並び）で、
  Python の `plugin_spec(name)["outputs"]` と serve の `plugins` の `outputs` は dict の並び。
- 公差の設定（`tolerance --spec`）も型で読む（スキーマ `tolerance.schema.json`）。`wedge_face` は小文字の `back`・`front` だけ、
  `relative`・`catalog`・`samples` などの真偽に真偽でない値はエラー（前は無視）。エラーの文は欄の道つきの serde のもの
  （例「tolerance.lenses.L1: unknown field `tlit`, …」）。先頭の `$schema` は受ける。
  `groups` の値は `GroupTol`、`dist` は `Dist` の型で直に読む（XCT-11）: `groups` の欄の `null`（以前は 0 扱い）と、
  分布の名前のエラーは読むときのエラー（文は `tolerance.tols[0].dist: distribution …` のように欄の道つき）。

- エラーの符号: core のエラーに機械で見分ける符号（`Error::code`、`lensbench_core::error::code`）を足した。serve のエラーは `data.code`、
  Python の例外は `code`。プロジェクトを読む所（`lensbench_project` の関数）は `Result<_, String>` から core の `Error` に変わり、
  種類（読込・不正）と符号が CLI の終了コード・serve・Python まで残る。表示の文は種類の前置き（`invalid:` など）が付くことがある。
  検出器の種類の `check` は `lensbench_core::Result<()>` を返す。`par::try_map`（エラーが出たら残りを始めない）を足した。
  ズーム軌跡も進み具合（`Event::Count`）を知らせ、CLI の `zoom` は標準エラーに「k / n」を出す。

### 終了コード
- CLI の終了コードをエラーの種類で分けた: 1 失敗（システム・値の不正・計算・照合の不合格・一部の失敗）、2 使い方（引数・設定ファイル）、
  3 ファイルが無い・読めない・書けない、4 未対応の機能、130 中断（以前は引数のエラー 2 のほかは全部 1）。表は [使い方（CLI）](cli.md)・
  `lensbench --help` の末尾。エラーの文の次の行に `案内:` を出すことがある。
- エラーの文の言語: 英語が元で、日本語の表示（`LENSBENCH_LANG=ja`、無ければ OS の言語が日本語のとき）では訳す。種類の前置きは
  英語の表示で `invalid:`・`read:`・`not supported:`・`cancelled`（日本語の表示は前どおり `不正:` など）、案内の行は `hint:`（日本語は `案内:`）。
  引数のエラーは英語の表示で clap の文のまま、日本語の表示では訳す（`lensbench: 知らない引数 --ring（--rings のこと？）`）。
  個々のエラーの文は順に英語へ移す（それまでは日本語だけの文が残る）。
- 最適化の OPD・波面の項は、OPD の基準（ROPD）が射出瞳・無限遠のどちらでもないとき、解析の OPD と同じく「未対応」（終了コード 4、
  前は 1 の不正）にした（XCT-19）。

### 結果の形
- `rays --save` の csv・npz は、表を全部メモリに作らずに視野・波長の組ごとに書く（PRJ-27。中身は前と同じ）。Rust に
  `table::rays_each`・`table::save_rays`、解析の定義に `Analysis::run_to_file`（書き出すときに run・save の代わりに呼ぶ）。
- STEP の書き出しで、非球面の面を周 9 点 × 断面の有理 B スプライン曲面から、断面の B スプライン曲線を回した
  `SURFACE_OF_REVOLUTION` にした（PRJ-20。回転が厳密、断面の区間は許容差 1e-6 mm から決める）。Rust の `OptionsStep` に `profile_tol`。
- 回折 MTF（`mtf`・`geo-mtf`・`huygens-mtf`）の `cutoff` は `[x, y]` から `{"tangential", "sagittal"}` に（JSON・Python）。
  プラグインの出力 `cutoff` は (2,) のまま、並びを [子午, 球欠] にした。
- 遮断周波数は計算した波長の値（以前は単色でも主波長）。多色は主波長のまま。単色の MTF で周波数を省いたときの範囲も
  その波長の遮断周波数の 0〜0.95 になる。

- Huygens MTF（`huygens-mtf`）は、像の格子の間隔 Δ のナイキスト 1/(2Δ) を超える周波数を `null`（NaN）にする（以前は折り返した値）。
  多色の MTF（`mtf-focus`・`mtf-field`・Python の `through_focus_mtf`・`field_mtf` も）で重みのある波長が無いときは、全部 NaN でなくエラー。
- スポット・PSF・RMS の結果の `unit` は `"lens"` から `"mm"` に（意味は同じ。アフォーカル像空間は角度の単位のまま）。
- serve の `load`・`reload`・`system`（と元に戻すで開く前へ戻した `loaded`）に `notes`（読んだときの注意）を足した（CLI-10）。
  プロジェクトの展開の注意は解析の `notes` にも入る。Rust の `lensbench_engine::load::load_file` は注意を標準エラーに書かなくなった
  （注意が要るなら `load_file_notes`）。`overlay_systems` はシステムと注意の組を返す。
- serve: 読むだけの要求は並行に、`optimize`・`tolerance`・`swap`・`zoom` は始めたときのシステムで回る。`optimize` の `apply` は
  回している間にシステムが変わっていれば当てない（`applied: false` と `notes`）。
- 公差（`tolerance`・Python の `tolerance`）の結果の `label`（`tolerances`・`sensitivity`・`correlation`）は英語の名前に
  （例: `L1 偏心x` → `L1 decenter x`、`L1 厚さ1` → `L1 thickness 1`、`G1 群の偏心x` → `G1 group decenter x`、`焦点距離` → `focal length`、
  `後の間隔` → `space after`）。名前で項目を探すスクリプトは書き換えが要る。CLI の文字の出力と GUI は表示の言語で出す。
- 最適化の止まった理由（`stop`）: 減衰を強めて一歩が小さくなったときは `small_step` でなく `no_progress`（進めない）に
  （IOO-13）。`small_step` は減衰を強めずに求めた一歩が小さいときだけ。変数・評価関数の値は変わらない。
- ズーム軌跡（`zoom`・serve の `zoom`・Python の `zoom`）は、1 点が解けなくても全体をエラーにせず、その点を `stop: "failed"`
  （理由は `error`、変数は解き始めの値、近軸量・評価関数は `null`）として残して続け、`warnings` に出す（IOO-27）。中断は前どおりエラー。
- モンテカルロ公差の乱数・抽選を一括実行（`lensbench_runs`）のものと共通にした（IOO-14）。同じ種でも試行のずれと統計は前の版と
  違う（統計としては同じ分布。切った正規分布は棄却法から逆関数法に）。感度・公称の値は変わらない。
- 位相の約束を exp(i(ωt − kz))（光路長 L の位相は exp(−ikL)）にそろえた（ANA-13、`lensbench_core::phase`）。POP・
  Huygens PSF・像面上の干渉縞・光線の格子（`raygrid`）を変えた。POP（`pop`）を保存した場（JSON の `grid.phase`、表の位相の列）の位相は符号が逆になる
  （照度は変わらない。前の版の値は −1 倍で移る）。Rust の `pop::Beam` の `u` は前の版の複素共役で、全体の場は
  `u·exp(−ik·パイロット)`（`Beam::from_fn` に渡す残りの場もこの約束で）。プラグインに渡る POP の場（`u`・`input_u`）も前の版の共役。Huygens PSF・干渉縞の強度は変わらない。
  光線の格子（`raygrid::trace_path`・`trace_jones`）の場 `ex`・`ey` と Jones 行列は前の版の複素共役で、入射の偏光
  `InputPol::CircularPlus` の Jones は `(1, −i)/√2`（前は `(1, i)/√2`。同じ向きの円偏光で、`jones()` の値だけが変わる）。
  `lensbench_core::phase::pol_to_scalar` はなくした（約束が 1 つなので要らない）。

### 光学系
- 膜のミラーのモデルが notimpl を返す範囲を広げた: 補間した分岐点（膜の外の媒質で全反射に変わる点）のそばと、分岐点が波長で測った β の
  上端を横切るところ・上端のすぐそばにあるところ（[膜](coatings.md)）。前はそこで OpticStudio と 1e-2〜3e-2 違う値を返した。
- 開口の判定の余裕（縁の上に狙った光線をけらないための幅）を、1e-9 mm の固定から開口の大きさの 1e-9 倍に変えた（`tol::aperture_edge`。外径は外径の、輪帯・遮蔽の内径は内径の比）。光線の狙い（エイミング）の収束の許容・届いたとみなす残差も、1 mm の下限をやめて絞りの半径に対する比にした（`tol::aim_scale`。下限は物体面から絞りまでの長さの 1/100 で、座標の丸めに合わせる）。縁から 1e-9 mm ほどの光線の通る・けられるが変わることがある。楕円の開口は正規化した半径²に 2e-9（前は 1e-9）
- 玉の反転（ブロックの `reversed`、GUI の反転）が、Zernike Standard・Fringe・Grid Sag・Tilted・Irregular・Cylinder Fresnel の面のある玉にも
  できるようになった（形を y 軸のまわりに 180° 回す。Grid Sag は格子を映すので、書き出す .zmx の FIMP は空）。理想レンズ・ユーザー定義の面・
  GRIN の媒質のある玉は今まで通り反転しない。
- ミラーで往復する玉の復路で、面の開口の x の偏心と、x について対称でない面の形が左右に映っていたのを直した（復路は同じ玉を鏡の後の
  座標で書くので映さない）。開口を x に偏心した玉を往復させた系は、結果が変わる。
- ミラーの後に置いた玉・面の並びの Zernike の項・Grid Sag の格子・Cylinder Fresnel の屈折の面が、ほかの形と一緒に裏返るようになった
  （前は裏返らず、ミラーの後のこれらの面の形が逆だった）。これらの面をミラーの後に置いた系は、結果が変わる。
- .zmx の ZPL マクロのソルブ（IOO-22）: 途中でエラーになって止まったマクロは、値は前と同じく OpticStudio に合わせる（それまでの
  SOLVERETURN の値か 0、または変えない）が、読込の注意（`System.macro_warnings`、CLI の `warning:`）に止まったこととエラーを出す。
  読込で解き直せないときは、前はすべてのマクロのソルブを外したが、解き直せなくするソルブだけを外し、ほかは残す。
  既定はこのまま（OpticStudio と同じ値＋注意）で、CLI の `--strict-macros`・serve の `load` の `strict_macros: true` で、解けない
  マクロのソルブ（止まった・外した）があればエラーで止める（Rust は `lensbench_engine::SysSpec::strict_macros`）。
- ガラスのフォルダのどこにも無いカタログ SCHOTT・OHARA は、同梱のカタログ（分散式のみ、GUI と同じもの）から取る（CLI-31）。
  CLI・serve・Python でも、前は「glass catalog not found」で止まったシステムがフォルダの指定なしで開く。同梱から取ったガラスは
  読込の注意（CLI は標準誤差の `warning:`、`System.import_notes`）に出る。同じ名前の AGF がフォルダにあればそちらが先なので、
  フォルダを渡している使い方の結果は変わらない。同梱のカタログは内部透過率・熱膨張を持たない。
  プロジェクトの「カタログが無い」・「レンズカタログに無い」のエラーに、`--glass-dir`・`--lens-catalog`（serve・Python の名前も）の案内を付けた。
- 共軸の面の間（前の面の後と次の面の前に傾き・座標ブレークがない区間）の座標の変換は、大域の座標の差でなく面の厚さそのものを
  使う。追跡の結果は丸めの分だけ変わる（公開のテスト系 38 本のうち 29 本。光線の座標 1e-12 前後、RMS・波面は相対 5e-10 まで、
  傾きのある C1 で 3e-9 まで、MTF は値が小さい周波数で相対 1e-5 まで）。自動のクリア半径・実光線のソルブの値も丸めの分変わるので `system_hash` も変わる。
- 回折 MTF で、瞳が開口・遮蔽で切れているか（縁に合わせた線ごとの求積にするか）を、求積点がけられた点に当たったかでなく、けられの面積
  （縁を二分法で詰めて測る。単位円の 1e-4 を超えるとき）で決める。外挿しない Zernike 面の縁で光線が落ちる細い帯のような、面積の小さい
  けられでは線ごとの求積に切り替えない（前は点数や点の置き方で切り替わり、値が跳んだ）。けられのありうる系では周波数 0 がちょうど 1 になる
  （前は 1 から 1e-4 ほどずれることがあった）。公開のテスト系では Z3 だけ値が変わる（1e-3 ほど）。
- 2 軸の折り曲げミラー・ビームスプリッターの後の座標の軸の向きが変わる。
- 瞳のアポダイゼーションをシステムの設定にした（ANA-05）: .zmx のシステムの行 `GFAC 係数 種類`（種類 1 ガウス・2 Cosine Cubed）を
  `System::apodization`（システムファイルの `apodization`、`set_system_data` でも変えられる）に読み、FFT PSF・Huygens PSF・回折 MTF の
  瞳の振幅に常に入れる。前は一様でない GFAC は読めないエラーだった。一様のシステムの結果はビット単位で同じ。.zmx に書き出すと
  一様でなければ `GFAC` の行を書く。一様でない GFAC を読むと、振幅が回折の解析にだけ入る（スポット・RMS 波面・Zernike・幾何 MTF には入らない）ことを読込の注意に出す。膜の透過率・偏光の振幅は選択式（既定は off）で、Rust は
  `lensbench_analysis::pupil_amp::PolAmp` を `psf::psf_with`・`huygens::huygens_psf_with`・`huygens_mtf_with`・
  `analysis::mtf_amp`・`mtf_waves_amp` に渡す。CLI・serve の `psf`・`mtf`・`huygens-mtf` は `--polarization`。
  `PupilFunction` に振幅の `amp` を足した。
- 膜・偏光の振幅（`--polarization`、`PolAmp`）を入れた多色の PSF・Huygens PSF・MTF は、波長の重みに瞳の上で平均した光量の
  透過率を掛けて足す（ANA-34）。前は波長ごとに光量を 1 に揃えてから重みで足したので、膜の透過率が波長で違うと像面に届く光量の比と
  違った。振幅なし・一色の結果は変わらない。重みは `lensbench_core::pupil::wave_weight`。
- 瞳のアポダイゼーション（.zmx の GFAC）を、回折の解析に加えて幾何の解析の光線の重みにも入れる（ANA-33）: スポットの重心・
  RMS 半径（`spot`・`rms` とその掃引）、RMS 波面・ストレール比、幾何 MTF、最適化の RMS スポット・RMS 波面・重心の項。重みは
  スポット・重心・幾何 MTF が強度 A²、RMS 波面・ストレール比が振幅 A（収差の無い瞳で規格化したストレール比を波面の 2 次まで
  展開した分散の重み。OpticStudio 26.1 のガウス求積の RSCE・GMTT・RWCE と同じ重みで、Double Gauss 28 のガウスの瞳で RSCE は
  9 桁、RWCE は 1e-5 で一致）。Zernike の当てはめには入れない
  （OpticStudio も同じ）。一様なら値は変わらない。一様でない GFAC を読んだときの注意（回折の解析にだけ入る）は出さない。
  Rust は `lensbench_core::analysis::apodized`（求積の重みに A² を掛ける）。
- MTF の子午・球欠の向きを、既定で視野の向きにした（ANA-11）: 子午は視野の座標 (x, y)（角度の視野は (tan θx, tan θy)）の向き、
  球欠はそれに直交する向きの周波数。回転対称なシステムでは、視野の方位によらず同じ MTF になる。前は（OpticStudio の FFT MTF と同じく）
  子午がいつも瞳の py の向きだったので、x だけの視野では子午と球欠が入れ替わり、x・y の両方に角度がある視野では子午面の向きでなかった。
  軸上と y だけの視野の結果はビット単位で同じ。回折 MTF（補間・FFT・けられの求積）・スルーフォーカス・フィールド MTF・幾何 MTF・
  Huygens MTF・最適化の MTF の項が対象。前の向きは設定 `axes` の `pupil` で選べる（CLI・serve・GUI の各 MTF の解析、Python の
  `mtf`・`mtf_focus`・`mtf_field`・`geo_mtf`・`huygens_mtf` の `axes="pupil"`）。`mtf-field` の `axis` が x のときは、既定の子午・球欠が
  前と入れ替わる。Rust は `lensbench_core::analysis::MtfAxes`、`mtf_amp`・`mtf_waves_amp`・`sweep::through_focus_mtf`・`field_mtf` に
  引数 `axes` を足し、`psf::geometric_mtf_axes`・`huygens::huygens_mtf_axes`・`huygens::psf_mtf_dir`・`analysis::mtf_cutoff_axes`・
  `mtf_tangential` を足した。
- 形のパラメータの微分（光路長の微分 `opl_grad`・着地点の微分 `land_grad`、それを使う最適化・公差）を、面の形どおりに求める（KRN-32）。
  以前は `asph` をどの型でも偶数次非球面の係数とみていた。Odd Asphere の `a2`, `a4`, … は β1 r, β2 r², …、Tilted の `a2`・`a4` は
  tan α・tan β（z = x tan α + y tan β）、Irregular の `a2`〜`a16` は偏心 x・y、傾き x・y、Zs・Za・Zc、角度で、偏心・傾けた面の c・k の
  微分も変わる。形に効かない値はパラメータにできない（エラー「surface j (TiltSurf): c doesn't change the shape of this surface type」）:
  Tilted の c・R・k と 3 番目からの `asph`、Zernike Sag・Grid Sag でない面に残った Zernike の項（以前は効かない値を変数にでき、
  Zernike の項は効かない微分が付いた）。追跡・近軸量は変わらない。
- トロイダル面の曲率のソルブ（周辺光線・主光線の傾き・垂直・不遊・F/#）が y² の項を近軸の曲率に入れる（KRN-33。近軸の追跡と同じ
  c + 2 a₁）。以前は y² の項がある面で解いた曲率が目標から外れていた（例: 周辺光線の傾き −0.05 の目標で −0.0636）。
- `freeform::sag_xy`（機構部品の確認の素子の縁、Huygens PSF の像面の矢高）が、トロイダル面・Cylinder Fresnel の基板・Fresnel・
  Gradient 9 の傾きの項なども追跡と同じ形で求める（KRN-34。以前は回転対称でない面のほかを c・k・asph の偶数次非球面として求めていた）。
  Fresnel・理想レンズ・Jones の面は平らな基板（矢高 0）。
- 像面上の干渉縞（`interference` の検出面、偏光あり）で円偏光の入射（`+`・`-`）の回る向きが逆に追跡されていた（偏光追跡へ
  Jones ベクトルを約束を直さずに渡していた。ANA-13 で見つけた）。光線の格子と同じ向きに直した。直線偏光・偏光なしは変わらない。

- `spot` の各視野に `rays`（追跡した本数 `total` と、除いた光線の理由ごとの本数 `vignetted`・`missed`・`tir`・`aim_failed`。
  波長の分を足し、主光線は数えない）を足した（KRN-09）。Rust は `analysis::Spot::rays`（`trace::RayCounts`）、`RayBuf::counts`・`Trace::counts`。
- 鏡の後ろの面（KRN-05 の 3 段目）: プロジェクトで鏡の後に置いたトロイダル面・逆向きのレンズのトロイダル面は、c と同じく x 方向の
  曲率 cx の符号も変える（前は c だけ変えて形が違った）。鏡の後の Irregular は傾きと Zs・Za・Zc の符号だけを変え、偏心と項の角度は
  そのままにする（前は 8 つの値を全部変えた）。どちらも結果が変わる。鏡の後にこれらの面が無いシステムは変わらない。
- 曲率の残った Jones の面は、値をそのまま置く作り方（`edit_raw`・`build_raw`・Python の系の組み立て）でもエラーにした（前は交点を頂点の
  平面で、法線を曲率で求めていた）。.zmx の読込・`edit` は前からエラー。
- 形に効かない値で交点の求め方を変えない（KRN-05 の 3 段目）: c = 0 で非球面の係数・cx も 0 の面は、残った並び（0 の係数）があっても
  平面として交点を求める。Tilted の交点のニュートン法の初期値は頂点の平面（前は c・k の二次曲面。Tilted の形は c・k を使わない）。
  結果は丸めの範囲で変わる。
- 鏡の後ろの媒質: プロジェクトで鏡の後に置いた GRIN の媒質も z について裏返す（Gradium は分布を逆向きに、Gradient 9 は形の傾き
  tan を）。複屈折材の板の結晶軸も z 成分の符号を変える。前は面の形だけ裏返り、媒質はそのままだった（結果が変わる）。
  裏返した Gradium の分布は分布のファイルの組と違うので .zmx には書けない（エラー。システムファイルには書ける）。
  プロジェクトの行（`row`）で書いた GRIN の媒質が「カタログに無いガラス」のエラーで展開できなかったのを直した。
- 鏡の後ろの利用者定義の面は、モデルが裏返しに対応していればモデルが裏返した設定で置き、対応していなければエラーにした（前は
  裏返さずにそのまま置いた）。同梱の `ideal_mapping` は対応する。玉の反転も、対応するモデルの利用者定義の面なら反転できる。
  複屈折材の板のある玉の反転はエラーにした。
- GRIN の媒質のある玉も反転できる（前はエラー）。媒質は反対側の面の後ろに移り、分布を z について裏返して厚さだけずらす。
  Gradient 9 の形の傾き tan は面に残る（傾けた平面は Tilted と行き来する。傾けた曲面の後ろが一様な媒質になるとき、
  Gradium の媒質が傾けた面で終わるとき、玉の最後の面の後ろに GRIN の媒質があるときはエラー）。`elements::reversible` は
  Gradium・Gradient 9 の面も true を返す。復路の面の並びの GRIN は今までどおりエラー。
  GRIN の媒質の次の面に Tilted を置けるようにした（傾けた平面で交点はそのまま求まる。前はエラー）。
- ZPL マクロのソルブの `RADI` は、平面と無い面の番号で 0 を返す（OpticStudio 26.1.1 で測った値。前は未対応のエラーでソルブを外した）。
- 物体が無限遠のときの理想レンズ（`PARAXIAL`・`ideal_mapping`）の OPD: 光が −z へ進むとき（ミラーの後）、入ってくる平行光の
  面内の位相の符号が逆だった（OPD に視野の角に比例する傾きが残った）のを直した。+z へ進む光は変わらない（ビット同じ）。
- Python の `set_param`・`set_params` は、自動の半直径が追跡に効かない系では半直径の求め直しを後回しにし、系を読むときに最後の
  系から 1 回求める（ループで繰り返す `set_param` が 7〜60 倍速い）。最後の系で光線が届く面の半直径は毎回求めたときと同じ。
  届かない面は求め直しで値を変えないので、途中の系で求めた値ではなく、後回しにする前の値が残る。
### Rust API
- `lensbench_project::catalog_system`（レンズカタログの品の .zmx を系にする。構成 1、径はファイルの値で固定）を足した。展開・カタログの一覧・差し替え探索が同じ設定で品を読む。
- .zmx の読込を 2 段に分けた（足しただけで、今の口は変わらない）: `lensbench_core::zmx::document`（`Raw` から系の文書
  `Prescription` と ZPL マクロのソルブの依頼 `import::MacroRequest` を作る）と `System::from_import`（ピックアップを解き、
  マクロを今の系で試して付け、ソルブ・自動のクリア半径を求める。新しいモジュール `lensbench_core::import`）。外の読み手のために
  `grin::GrinData`（`load`・`gradium`・`grad9`）・`glass::missing_note`・`pickup::kind_of_pick`・`form::asph_parm_base` を公開にした。
- 外のクレートから足す口: 新しいクレート `lensbench-sdk`（約束の型の再輸出、入口 `cli::main`・`gui::main`・`py::module`、
  足し忘れの検査 `check`）と雛形 `examples/extension`（[ライブラリ（Rust）](rust.md) の「拡張の約束」）。
  - **壊す変更**: `lensbench_engine::Extensions::merge` は `Result` を返す（2 つの拡張が同じ名前のものを足すと誤り。
    置き換えは後の方に `replacing()`）。
  - `Extensions` に `named`・`replacing`・`with_ext`（表に足す関数）・`on_install`・`messages`（誤りの文の訳）。
  - GUI の `lensbench_gui::ext::GuiExtensions` と `lensbench_gui::main_with`、Python の `lensbench::ext::PyExtensions`
    と `module_with`、CLI の `lensbench_cli::main_with`。Python の `lensbench::ext::system_method`（系を最初の引数に取る
    関数を `lb.System` のメソッドにする。説明と引数の並びはその関数のもの）。
  - `SurfaceModel::example_config`（既定は `params` の既定値。`check` が `check_flip` に渡す）。
  - **壊す変更**: GUI の見せ方の型 `Analysis`・`ObjectArg`・`Detector` はフィールドを隠し、`Analysis::new(cmd, label)` と
    `select`・`figure`・`figure_params`・`object` などのビルダーで作る。大域の `register`・`register_detector`・
    `translations` は消した（`GuiExtensions` の `gui_analysis`・`gui_detector`・`gui_translations` を使う）。
  - Web 版の入口: SDK の feature `web`・`web-worker`・`web-threads` と `lensbench_sdk::web`（`Extra`・`set_extra`・
    wasm-bindgen の再輸出）。`scripts/build_web.sh` は `CRATE`・`MANIFEST` で外のクレートの wasm を作る
    （出力先の相対パスは呼んだ場所から）。雛形に `web` の入口。
  - GUI の設定のオブジェクトの欄 `ObjectArg::specs_from(arg)`: 項目を、解析の引数 arg で選んだプラグインの設定から作る
    （`chosen_plugin`・`specs_for`・`json_for`）。
  - SDK の再輸出に `plugin::Linear`・`plugin::check_cancel`・`surface::ZernSag` を足した。
- lensbench-core の feature `serde`・`schema` を消した。core の型は常に `serde::Serialize`・`Deserialize`・`schemars::JsonSchema` を実装する（依存の `features = ["serde", "schema"]` は消す）。lensbench-runs の `serde`・`schema` は残る。
- `System::semi_affects_trace` を公開した。
- 検出器の露光（`lensbench_runs::SensorNoise::expose`）の正規乱数を Ziggurat 法（`Rng::normal_fast`・`Rng::poisson_fast`）にした。
  分布は厳密に同じで手間はおよそ 1/6、同じ種でも雑音の値は前と違う。`Rng::normal`・`Rng::poisson` はそのまま
  （公差のモンテカルロなどの値は変わらない）。
- 利用者定義の面のモデル `SurfaceModel` に裏返しの口 `flip(config, how) -> Result<Option<Config>>` を足した（既定は None で
  「対応しない」）。`how` は `usersurf::Flip::Z`（z → −z）・`Flip::X`（x → −x）。約束は 2 回で元の設定に戻ること、裏返した面の
  `deflect` が元の面を映したものと合うこと、`paraxial_power` が同じこと。約束は `usersurf::check_flip(model, config)` で確かめられる。
  `Surface::flip_z`・`mirror_x` は `Result<()>` を返す（対応しない利用者定義の面で誤り）。`UserSurf::flipped(how)`。
  Gradium の分布 `GradiumProfile` に `flipped`（裏返したもの）を足した。
- 偏光追跡の電場 `PolTrace::e` を光線ごとの `Vec` の `Vec` から 1 本の配列 `PolField` にした。`t.e[i][m]`・`for e in &t.e`・`t.e.len()` は
  そのまま使える（`t.e[i]` は `&[CV3]`、`Vec` が要るなら `to_vec()`）。並びのまま読むのは `as_flat()`・`into_flat()`、偏光の数は `pols()`。
- `raygrid::JonesGrid` に瞳の縁までの距離 `edge`・その縁の開口の面 `edge_surf`・距離の瞳座標での勾配 `edge_dm` を足した。縁を開口の縁で
  点ごとに決める追跡 `raygrid::trace_jones_edge`（単位円の外 3 格子まで追い、けられた光線の値も残す）を足した。
- 非球面の係数の並び（`Surface::asph`）も種類の枝に移した（KRN-05 の 2 段目）。`s.asph` は `s.asph()`（読む、係数を持たない種類は空）、
  書くのは `s.asph_mut()`（長さを変えられる種類）・`s.set_asph(v)`・`s.set_asph_at(i, x)`。Tilted は `Form::TiltSurf { tan }`、Irregular は
  `Form::Irregular { p }`（決まった数の値、`asph()` では前と同じ並び）。標準面・Gradium・Gradient 9・Jones・理想レンズ・利用者定義の面・
  座標ブレークは係数を持てない（`set_type` で捨て、`edit` で置くと誤り）。serde の形（`asph` の欄）は同じ。
- 面のデータを serde で読むとき（serve の `set_surface_data`・系ファイル版 1・pickle）、種類に無い欄（標準面の `cb`・`focal`・0 でない
  `asph` など）に既定でない値があれば誤りにした（前は黙って捨てた）。`set_surface_data` で `typ` を変えるときは、書いていない種類ごとの
  値は新しい種類に合わせる（持ち越せない値は既定に）。
- 面の種類ごとの値を種類の枝にまとめた（KRN-05 の 1 段目）: `Surface` の `typ`・`cx`・`zern`・`grid`・`grid_file`・`bin2`・`cylf`・`grin`・
  `cb`・`cb_order`・`focal`・`opd_mode`・`jones`・`user` をやめ、`form`（`lensbench_core::surface::Form`、`#[non_exhaustive]`）が種類と
  その種類にしか意味の無い値を持つ（例 `Form::CoordBrk { cb, order }`・`Form::Paraxial { focal, opd_mode }`）。曲率・円錐係数・`asph` は
  `Surface` に残る。読むのは `s.typ()`・`s.cb()`・`s.zern()` など（種類が違えば既定の値か `None`）、書くのは `s.set_type(t)`（Zernike の項と
  屈折率分布は持ち越し、ほかは既定に戻す）・`s.form = Form::…`・`s.cb_mut()`・`s.zern_mut()` などの置き場（種類が違えば `None`）。
  種類の枝は `Form::of(t)`・`Form::typ()`。面の量の書き換え（`edit`）で、種類に無い値（トロイダル面でない面の `cx` など）を既定でない値に
  するのは誤りになった（前は使われない値が黙って残った）。追跡・解析の結果は変わらない（全試験・OpticStudio との照合が通る）。
  serde の形（serve の `surface_data`・`set_surface_data`、Python の pickle）は前と同じ平たい欄（`typ`・`cb`・`jones` など）だが、
  読むときは種類に無い値の欄を捨てる。利用者定義の面にするには `typ` と `user` を同じ `set_surface_data` で書く（前は `user` を先に
  書いてから `set_surface_type`）。
- Grid Sag から Zernike Sag に種類を変えた面に残っていた格子をやめた（近軸の曲率にだけ効いていた）。システムファイル版 2 の Zernike の形の
  `grid` もやめた。
- 偏光追跡の入口の引数をまとめた（KRN-17）: 入力の偏光・膜・伝搬の位相は `system::PolIn`（`PolIn::new(pols, coat)`、位相を
  かけないなら `.no_phase()`）。`try_trace_pol(field, px, py, w, PolIn)`、`trace_pol_launch(l, w, PolIn, rec)`、`trace_pol_spec` は
  ケラレ係数・視野を `TraceOpts` で受けて `trace_pol_spec(spec, px, py, w, TraceOpts, PolIn)`（`trace_spec` と同じく止める面・面ごとの
  記録も選べる）。`try_trace_pol_opts` は消した（`PolIn::new(..).phase(phase)` で同じ）。値はビット単位で同じ。
- OPD の基準の選び方を `lensbench_core::pupil::RefSurface::of` の 1 か所にした（XCT-19）: OPD の値（`analysis::to_ref`）・最適化の微分・
  Huygens PSF が同じ判定（`RefSurface::plane_for`・`pupil::exit_pupil_near`、閾値は `lensbench_core::tol`）を使う。値はビット単位で同じ。
  Huygens PSF は射出瞳が求まらないシステムで、平面の基準に黙って切り替えず OPD と同じエラーを返す（`pupil::check_opd_reference`）。
- 小さな線形代数を `lensbench_core::linalg` に集めた（ANA-17）: 最小二乗 `analysis::lstsq` は `linalg::lstsq`（列優先は `lstsq_cols`、
  右辺を何本も解く・残差を出すのは `linalg::Qr`）、`linalg::cholesky_solve(&mut a, &mut b, n)` は `linalg::Cholesky::new(&a, n)?.solve(&b)`
  （対角の残りが 1e-14 · 対角以下なら None）。三重対角は `linalg::solve_tridiag`。補間は `lensbench_core::interp` に: 3 次スプライン
  `coating::Spline` は `interp::Spline`（節点の傾きだけは `Spline::not_a_knot_slopes`）、等間隔の Lagrange の重みは `interp::lagrange::<K>`。
  膜のアドミタンス `coating::admittance` を公開にした。Zernike の当てはめ・MTF の瞳の補間はビット単位で同じ。エイミングの当てはめ（KRN-30）・
  減衰最小二乗の一歩・偏光の格子の傾き・自己相関と Huygens MTF の 4 点補間は解き方が変わり、値は丸めの分だけ変わる。
- `lensbench_export::write_layout` は書いた形式の代わりに `Written { format, notes }` を返す。`notes` は STEP に写した品の
  STEP が mm でなく、長さを直す表に無い実体があったときの注意（PRJ-22）。CLI の `layout` は `written` の行に `notes`、serve の
  `layout_export` は `notes` に足す。`step_embed::embed_shell` は直せなかった型を集める引数を、`StepReport` は `notes` を持つ。
- `lensbench_design::dls_spec::OptimizeParams`・`ZoomParams` と `lensbench_optim::zoom::ZoomSpec` に `stopped_as_vignetted`
  （既定 false。止まった光線をけられた光線と同じく除く、IOO-28）を足した。構造体を全部の欄で書いていれば足す（`..Default::default()` なら要らない）。
  CLI の `optimize`・`zoom` は `--stopped-as-vignetted`、serve・Python は `stopped_as_vignetted`。
- `System` を書き換えられない値にした（KRN-04・XCT-05）: 利用者が書く値は `lensbench_core::Prescription`（欄は前の `System` と同じ）に
  移し、`System` はそれを共有して持つ（複製は軽い）。`sys.surfaces` などは前どおり読めるが、`sys.surfaces[j].t = …` のような書き換えと
  `changed()`・`refresh()`・`resolve_pickups()`・`solve_thickness()`・`update_indices()` は外から使えない。直すのは
  `sys.edit(|p| { … Ok(()) })`（屈折率・ピックアップ・ソルブ・自動のクリア半径を作り直す。前の `refresh()` と同じ）か、値をそのまま置く
  `sys.edit_raw(|p| { … Ok(()) })`（前の「書き換えて `changed()`」と同じ）。`edit` の引数は `&mut System` から `&mut Prescription` に。
  足したピックアップ・ソルブを効かせるのは `resolve_links()`。文書からシステムを作るのは `Prescription::build()`（`edit` と同じ後始末）・
  `build_raw()`（確かめだけ）、取り出すのは `sys.prescription()`・`into_prescription()`。結果は前とビット単位で同じ。
- 屈折率を面から外した（KRN-25・IOO-09）: `Surface::n`・`n_base` を消した。求めた屈折率は `sys.n(j)`（面 j の後の媒質、波長ごと）と
  `sys.base_index(j, w)`（ずれ `dn`・`disp` をかける前）で読む。屈折率を求めるのはシステムを作る・直すとき（`edit`・`edit_raw`・`build`）の
  1 か所で、ミラー・座標ブレークの後は前の媒質。カタログに無い分散のない媒質（.zmx のモデルガラス "nd,0"・屈折率だけ決めた媒質）は
  `Surface::index`（どの波長でも同じ値）に書く（前は `n` に値を置いた）。`Element` に `n`（`split` が埋める）を足した。
  `set_surface_data` の項目は `n`・`n_base` が無くなり `index` を書ける。版 1 のシステムファイルは、名前から屈折率が決まらない分散のない
  媒質だけ、書いてあった `n` を `index` にして読む。結果は前とビット単位で同じ。
- 面の材料を型にした（IOO-09）: `Surface::mat` は文字列から `lensbench_core::surface::Material`（`Air`・`Glass(名前)`・
  `Model { nd, vd, dpgf }`・`Index(n)`・`Mirror`・`Inherit`（座標ブレーク））に。`Surface::mirror` と `Surface::index` を消した
  （ミラーは `d.is_mirror()`、屈折率だけの媒質は `Material::Index`）。名前から作るのは `Material::parse`、名前に戻すのは `name()`
  （serde・システムファイル版 1 の書き方は前と同じ文字列。分散のないモデルガラスは丸めない "nd,0"）、`d.mat == "N-BK7"` のように名前と
  比べられる。`Prescription::material_index`・`material_index_at` は `&Material` を受ける。`Element::materials` は名前の `Vec<String>`。
  玉の展開（project）は分散のあるモデルガラスの玉も受ける（前は「カタログに無い」のエラー）。結果は前とビット単位で同じ。
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
  `load_registered` のエラーは `String` から core の `Error` に（読めないファイルは `Error::Io`）。
- core は環境変数 LENSBENCH_GLASSCAT・LENSBENCH_MACROS を読まない（XCT-08）。CLI・GUI・Python は前どおり起動時に読む。
  Rust から使うときは `lensbench_core::init_from_env()` を呼ぶか、`glass::set_default_dirs`・`zpl_solve::set_default_dirs` で決める。
- 2 次元の結果は `lensbench_core::grid::Grid2`（座標の並び x・y と行優先の値、`g[iy][ix]`・`rows()`・`data()`）（ANA-18）:
  `Psf.x`・`y`・`i` は `Psf.i: Grid2`（座標は `i.x()`・`i.y()`）、`Wavefront.p`・`w` は `Wavefront.w: Grid2`、`Interferogram.p`・
  `delta`・`intensity` は `delta`・`intensity: Grid2`。CLI・serve・Python・プラグインの出力の形は変えない。
- 外から使われていない core の pub の項目（約 50: 追跡の中の関数 `trace_ray`・`trace_lanes`・`isect`・`paraxial_curv`、
  ファイルの読みの部品 `decode_text_enc`・`parse_grd`・`parse_sgrin`・`parse_grad9`、定数 `AUTO_FFT_N`・`LBCAT_VERSION` など）を
  クレートの中だけに（XCT-22）。`trace_ray_pol` は使われていないので消した。よく使う型は `lensbench_core::prelude`
  （`use lensbench_core::prelude::*;`）。CLI のクレートの `error`・`run`・`serve` のモジュールは外に出さない。
- 追跡のカーネルを 1 本にした（KRN-06・IOO-20）: `batch::Plan::new(sys, frames, w, last, floating_clip)` は面 1 から（`first` を取らない）。
  1 本ずつの追跡も同じカーネル（幅 1）。結果はビット単位で前と同じ。
  面ごとの準備（`Plan`）はシステムのキャッシュに持つ（システムは書き換えられないので、`edit`・`edit_raw` で直すと作り直す）。
- 光線の状態（KRN-10）: `RayOut` の `err: usize`・`tir: bool` を `status: RayStatus`（`Ok`・`Missed { surf }`・`Tir { surf }`・`AimFailed { stop }`）にした。
  `r.err == 0` は `r.status.is_ok()`、面の番号は `r.status.surf()`（届いたなら None）か `r.status.code()`（前と同じ数、届いたなら 0）。
  光線のエイミングに失敗した光線は、前は主光線の値のまま `err` に絞りの面を入れていたが、位置・方向・光路長・開始の位置・面ごとの記録・電場をすべて NaN にした。
  Python・JSON の `err` の数は前と同じ。
- 追跡の結果を列で受ける入れ物（KRN-09）: `System::try_trace_into`・`trace_launch_into` は `RayBuf`（`RayBuf::with_cols` で選んだ列と状態・けられ。列は `cols()` で読むだけ）に書く。
  値は `Trace` と同じビットで、入れ物は使い回せる（要らない列は確保しない）。`analysis::Opd` の `trace: Trace` は `rays: RayBuf`（`Cols::WAVE`）に。
- 波長を足したシステム `sweep::with_wavelength(&s, lam)` は `System::with_wavelength(lam)` に（ANA-28）。屈折率は材料から作り直す（ガラスの温度の補正は
  システムの Adjust Index の設定のまま。前はシステムの値に合う方を 2 通りから選んだ）。材料の屈折率を任意の波長で `System::material_index_at`。
- 膜のモデルの書き出し `format_models`・`format_models_bin` は `lensbench_design::coat_fit` から `lensbench_core::coating` へ移した
  （読みと同じ所。`coating` はモジュールのフォルダに分けた。ほかの公開の名前は変えない）（ANA-16）。

- モンテカルロ公差の乱数・抽選・実行を一括実行と共通に（IOO-14）: `lensbench_design::montecarlo::Rng` を消した（乱数は
  `lensbench_runs::Rng`）。試行のずれは `McSpec::draws()`（`lensbench_runs::Draws`）で引き、公差ごとの分布は `Tol::sample_dist()`。
  `draw_trial` は `Result<Vec<f64>>` を返す。`montecarlo::run` は感度と試行を `lensbench_runs::run` で回す。
- DLS（IOO-13）: `DlsOpts` に `gtol`（勾配の判定、既定 0 で判定しない）・`qr`（一歩を QR で解く、既定 false）・`active_set`
  （境界の有効制約法、既定 false）を足した（構造体を `..Default::default()` なしで書いていれば直す）。`Stop` に `SmallGradient`
  （`small_gradient`）を足した。既定の値の結果は前と同じ。
- ズーム軌跡（IOO-27）: `ZoomPoint::stop` は `PointStop`（`Solved(Stop)`・`Failed(String)`）。`zoom::solve` は点が解けなくても
  `Err` を返さない（中断だけ `Err(Cancelled)`）。目標の満たし方 `ZoomSpec::method`（`TargetMethod::Penalty`（既定、前と同じ）・
  `Lagrange`（拡大ラグランジュ法））を足した。設定のエラー（`Problem::check` を通らない: 動かすパラメータが状態のシステムにない、
  範囲が逆など）と `DlsOpts` のエラーは `Failed` の点でなく `Err` を返す。`lensbench_engine::zoom::ZoomOpts` に `method` と
  `dls`（`DlsChoice`: `qr`・`gtol`・`active_set`）を足した（構造体を全部の欄で書いていれば直す）。
- DLS の設定の確かめ（IOO-13）: `DlsOpts::check` を足し、`dls` は `ftol`・`xtol`・`gtol` が負か NaN、`damping` が正の有限の値でないと
  エラー（以前は黙って使った）。試しの一歩の評価の途中の中断は `no_progress` でなく `cancelled`。
- 最適化の項の値（IOO-28）: `Problem::term_values` は `Vec<TermValue>`、`merit_and_terms` は `(f64, Vec<TermValue>)` を返す
  （`TermValue::Quantity`（目標のある項の量そのもの）・`Rms`（目標のない項の √Σr²）、数は `value()`）。`Problem` に
  `stopped_as_vignetted`（止まった光線をけられた光線と同じく除く。既定 false）と `ray_counts` を足した。JSON・Python の数の
  並びは前と同じで、値の意味を `value_kind`・`term_value_kinds` で足した。
- `lensbench_runs::Sampler` は `params` と `draws: Draws` に（`vars`・`seed` の欄はなくした。種は `draws.seed()`）。`Dist` に
  `End`（lo か hi を半々）と `Disc`（円の中の分布の 1 成分）、組の分布 `Joint::Disc` と `Sampler::with_joints` を足した。
- OpticStudio の MTF システムの基準（mtf.json・mtf_weights.json・mtf_family.json・psf_os.npz）との照合を core の試験から
  `lensbench_verify::mtf_refs`（`fft_mtf`・`mtf_weights`・`through_focus`・`mtf_vs_field`・`rms_family`・`geometric_mtf`・`fft_psf`）に
  移した（PRJ-19）。`zemax_refs::check_dir` と同じく基準・ガラスのフォルダを渡して、別の基準にも同じ照合をかけられる。
- 展開の入口を 1 本にした（PRJ-12）: `lensbench_project::expand(&project, &env, &ExpandOpts { state, path, cache })` が
  `Expansion { system, notes, gaps }` を返す。消したのは `expand_state`・`expand_report`・`expand_gaps`・`expand_gaps_cached`・
  `expand_path`・`load`・`load_state`・`load_state_report`・`load_path_report`・`expand_json`・`expand_json_path`。ファイルからは
  `Project::load(path)` で読み、環境は `Env::for_file(path, glass_dirs, catalog)`（相対パスはそのフォルダから）か
  `Env::in_dir(base, …)` で作る。JSON の本文からは `Project::from_json`。前の `expand(p, env)` は `expand(p, env, &ExpandOpts::default())?.system`。
- `LensCache` を `SourceCache` にした（PRJ-13）: カタログの玉に加えて、`.zmx`・システムファイルのブロックのシステムも覚える（ファイルの
  更新時刻と大きさが変われば読み直す）。`ExpandOpts::cache` に渡す。BS の経路の一覧を同じ使い回しで求めるのは
  `paths_cached`。`open::Source::Project` に欄 `cache` を足した（状態・経路ごとの展開で使い回す）。
- 面の種類ごとのパラメータを番号で読み書きする `Surface::parm(i)`・`Surface::set_parm(i, v)` を足した（KRN-05。.zmx の PARM の番号、
  置けない番号は None・false）。`pickup::surf_parm` はこれを使う。
- 展開の注意 `Expansion::notes` を `Vec<String>` から `Vec<Note>` にした（PRJ-12）。`Note { kind, block, text }` は種類
  （`NoteKind`: `Pickup`・`Environment`・`Coating`・`Unit`・`SurfaceOp`・`Glrs`・`Mechanics`）、元のブロック・群の id（分かるとき）、
  文を持つ。文は前と同じ（`Display`・`String::from(note)`）。serve・CLI・Python の注意は今までどおり文。
- BS の経路の一覧（`paths`・`paths_cached`）は、経路ごとにブロックをたどるだけにした（PRJ-12）。系を組んで確かめる・自動の
  半直径を光線で求める・機構部品を置く段はしないので速い。その段の誤りは一覧では出ず、その経路を展開したときに出る。
- メーカー名の比べ方を `lensbench_core::catalog::VendorKey` 1 つに（XCT-12・PRJ-17。英数字だけを大文字に）。同じメーカーかは鍵の一致、
  人が書いた名前での絞り込みは `VendorKey::matches`（鍵を含む）。`lensbench_catalog::spec::vendor_key` はなくした。
  `LensCatalog::load_registered` は `Arc<LensCatalog>` を返し、読んだファイルの更新時刻・大きさが変わらなければ読み直さない。
  `UserItem::body` で品の中身（`ItemBody::Optic`・`Mech`・`Incomplete`）を引く。
- `lensbench_runs::run`（IOO-24）: `RunOpts::keep_going` で失敗したタスクを結果 `{"error": "…"}` の行にして続ける
  （`RunSummary::failed` に数）。結果のファイルは `lensbench_core::vfs` を通して読み書きし（Web 版でも使える）、再開でファイルを 1 回だけ読む。
- 差し替え探索（PRJ-07）: 置いた玉の面が同じ候補（型番違いの同じ玉など）の組は 1 回だけ最適化して結果を使い回す
  （`SwapSearch::distinct_count` はまとめた後の組の数）。
- 展開の結果 `Expansion` に `map: Vec<SurfOrigin>`（システムの面ごとの出どころ: 面を足した id のあるブロック・群、往路・復路、
  反転た玉）を足した（PRJ-04）。ブロック・群の面の範囲は `block_runs(&map, id, back)` で引く。
- 膜のモデル作りの道具（[膜](coatings.md)）: `lensbench coat-model --check 表 --check-tol 値` でモデルを作った点とは別に測った表と比べ、
  最大の差と足す波長を JSON の `check` に出す。`lensbench coat-kinks --min-score`（既定 100）で折れ目の候補にする強さの下限を変えられる。
  `lensbench_design::coat_fit::detect_kinks` は数点の間に並んだ 2 つの折れ目も拾うようになり（同じ表でも折れ目が増えることがある）、
  下限を渡す `detect_kinks_with(m, min_score)` を足した。`lensbench_core::coating::CoatModel::iface(KEY, 波長, n1, n2, 位相)` で
  1 つのモデルから面の境界（`Iface`）を作れる。

### GUI
- 組合せの図のキー: ロックは L から Ctrl+L に。R・F は Ctrl などを押していないときだけ効く。要素を外すのは Delete だけ（Backspace では外さない。文字を消すつもりの誤操作を防ぐ）。
- 既定の組合せの名前は画面の言語で付く（英語の画面では Catalog lens combination）。
- CAD の置き場の Secret は保存しない（環境変数 `LENSBENCH_CAD_STORE_SECRET`）。
