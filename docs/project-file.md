# プロジェクトファイル（ブロック、状態、群）

`.json` のプロジェクトファイルは玉・空気・絞り・ミラー・面の並び・.zmx のインポート・群のブロックでシステムを組む（例は `examples/projects`）。

## JSON スキーマ

JSON スキーマは `docs/schema/` にある: `project`（プロジェクトファイル）・`system`（システムファイル）・`user-catalog`（ユーザーカタログ）・
`swap`（差し替え探索の `--spec`）・`terms`（評価関数の項）・`tolerance`（公差の `--spec`）の `.schema.json`。スキーマはドキュメントの
サイトにも版ごとに置く（`https://masa-iwm.github.io/lensbench/schema/v0.6/project.schema.json` のように lensbench の版の小数第 1 位まで。
リリースの後は書き換えないので、古い版で書いたファイルもその版のスキーマで検査できる）。lensbench（CLI・GUI・Python）が書く
プロジェクト・システム・ユーザーカタログ・公差と差し替えの設定のファイルは、先頭にその版の `"$schema"` を付ける。手で書くファイルも先頭に
`"$schema": "…/schema/v0.6/project.schema.json"` のように書くと、エディタ（VS Code など）で項目の補完・説明・検査が効く
（lensbench は読むときに `$schema` を無視する）。スキーマは serde の型
（`lensbench_project::schema`）から作り、テストで型とずれていないか・例題が合うかを確かめる。型を変えたら
`LENSBENCH_UPDATE_SCHEMA=1 cargo test -p lensbench-project --test it schema::` で作り直す。

## 形式とブロック

- 形式の印 `"format": "lensbench-project"` と版 `"format_version": 1`。lensbench が書くファイルにはいつも付く。省略したファイルは今の版として読み、
  読める版より新しい版のファイルは誤りにする（知らない項目を落として黙って計算しないため）
- 検出器のブロック（`"type"` が組み込みのブロックでない名前）は、この lensbench を組み込んだ版が登録した種類
  （`lensbench_project::detectors`）だけ読める。最後のブロックだけに置け、面は足さない（像面に置く。前の空気のブロックが距離）
- .zmx のインポート（`"type": "zmx"`）は面 `from`〜`to`（既定は面 1〜像面の手前）・構成 `config`。`"stop": false` でインポートした面の絞りを外す
  インポートした .zmx のコーティングファイル（COFN）は、展開したシステムのコーティングファイルになる（コーティングファイルの違う .zmx が 2 つ以上なら最初のもの、注意を出す）
  （システムの絞りはほかのブロックに）。lensbench のプロジェクト（.json）をインポートするときは、そのプロジェクトの状態 `state` と
  BS の経路 `beam_path` を選べる（省略なら状態の変更なし・そのプロジェクトの `path`）。プロジェクトのインポートやユーザーカタログの項目が
  循環すると、循環の道を並べたエラーにする
- 群（`"type": "group"`）は `role`（`zoom`・`focus`・`fixed`）と移動 `shift` [mm] を持つ。`shift` は前の間隔に足し、後の間隔から引くので、
  群の外の合計長は変わらない（ミラーを含む群は、ミラーの枚数によらず動かせない）。.zmx のインポートでは `groups` に面の範囲（`from`・`to`、その面に始まる玉・面と間の空気）で群を置ける（玉モード）。
  群にも偏心・傾き・ON/OFF を付けられる。

## 状態・面の上書き・ソルブとピックアップ

- 状態（`states`、多重構成に当たる）は名前と変更の組。`set` はブロックの `id` ごとに変える値（`on`・`off`・`t`・`shift`・`decenter`・`tilt`・
  .zmx の構成番号 `config` など、ブロックの JSON の項目）、`project` はシステムの値（`object_distance`・`aperture`・`fields` など）。
  CLI は `--state 名前` で選ぶ（`info` が状態の一覧を出す）。ライブラリは `lensbench_project::expand` に `ExpandOpts { state, .. }` を渡す。
- 自動の半径: 面の `semi` を `"auto"` にすると、半径を光線から求める（OpticStudio の Automatic と同じ決め方: 全視野・全波長の
  瞳の縁の光線の高さの最大。回転対称なシステムは視野の値 (x, y) の向きの上下 2 本、そうでないシステムは縁の 8 本）。システムやパラメータを変えるたびに求め直し、
  描画・機構部品の検査・書き出し・Floating 開口に使う。ブロックの `surface_edits` の `"semi": "auto"`、システムファイルの面の `"semi": "auto"`、
  .zmx の `DIAM 値 0`（自動）も同じ。カタログの玉の半径はファイルの値で固定
- 玉の縁でけられる: 半直径を数で固定した面のうち前後で媒質が変わる面（ガラスと空気・別のガラスの境。座標ブレーク・同じガラスどうしの面は除く。前がガラスの鏡の面にも付く）で開口の無い面は、展開で浮動開口に
  なり、縁の外を通る光線はけられる（玉の外の光線は玉を通らない。.zmx の読込・OpticStudio の読込と同じ規則なので、.zmx に書いて
  読み直しても同じ光線がけられる）。半直径を `"auto"` にした面、書いていない面（0）、絞りの大きさで決める系（`float`）の絞りには付けない
- 面の値の上書き（ブロックの `surface_edits`）: 玉・絞り・ミラー・面の並び・zmx・user・群のブロックと、zmx のインポートの群
  （`groups` の項目）に `"surface_edits": [{"surface": 1, "fields": {"r": -40, "semi": 8}}]` のように書き、そのブロックの
  面の値を展開の最後に変える。`surface` はそのブロックの面のうち座標ブレークを除いて数えた番号（0 始まり。偏心・傾きを付け外ししても
  変わらない）、`fields` は serve の `edit` の `set_surface` と同じ名前と、展開したシステムの値（ミラーの後は厚み・曲率の符号が逆）。
  `"coordbrk": true` を付けると `surface` はブロックの面の座標ブレークだけを数えた番号（ブロックの偏心・傾きの入口・戻りの座標ブレークは数えない）になり、座標ブレークの値（`cb`・`cb_order`・`ignore`・
  `aperture` など）を変える（往復するブロックでは使えない）。
  ミラーの `back` で往復するブロックでは、番号は往路の面で数え、復路の同じ面にも当てる（逆の順にたどるので、厚み・材料は 1 つずれた面に、
  厚みは符号を逆に）。往路の最後の面の厚み・材料はブロックの外（後の空気のブロック）なので変えられない。
  `fields` の名前が面の値の名前に無ければ、serve の `set_surface_data` と同じ面のデータの項目（`ap`・`before`・`zern` など）として当てる。
  カタログの玉は型番のまま上書きを持てる。id の無いブロックにも書ける。GUI の面の表で組合せの玉の面を変えると、これで保存される
- 面の絞り・ソルブ・ピックアップ（ブロックの `surface_ops`）: serve の `edit` の `set_stop`・`add_solve`・`add_macro_solve`・`add_pickup`・`remove_solve`・
  `remove_pickup` と同じ形で、当てる面（`surface`・`dst`）はそのブロックの面の番号（`surface_edits` と同じ数え方。往復するブロックは
  往路の面）。参照する面（`src`・`other`）は、ほかのブロックの面 `{"block": "L1", "surface": 0}` か、展開したシステムの面の番号・名前。
  座標ブレークは `{"block": "S3", "surface": 0, "coordbrk": true}`（ブロックの座標ブレークだけを数えた番号。`dst` も、自分のブロックでもこの形）で指す。
  ほかの操作・知らないキーは読むときにエラー。例（id L2 の玉に）:
  `"surface_ops": [{"op": "add_solve", "surface": 1, "kind": "marginal_height"},
  {"op": "add_pickup", "dst": 1, "src": {"block": "L1", "surface": 0}, "what": "c", "scale": -1}]`。
  `surface_edits` の後に、ブロックの順に当てる。`set_stop` があれば絞りのブロックは無くてよい（あれば絞りは面に移る）。
  状態で外したブロック・通らない経路のブロックの上書き・操作は当てない。参照するほかのブロックが無ければ当てずに注意を出す。
  `remove_solve`・`remove_pickup` はブロックの元（.zmx など）にあるソルブ・ピックアップを外す
  （値はそのまま残る。外すものが無ければ当てずに注意）。GUI の面の表で組合せの玉の元のソルブを外すと、これで保存される

## システムの値（視野・像面・物体・アフォーカル・瞳・環境）

- 視野の種類（`fields.type`）は `angle`・`object_height`・`paraxial_image_height`・`real_image_height` と、条件で定める視野 `solve`。
  `solve` では `fields.solve` の条件について、視野の (x, y) が目標値になる物体（有限なら物点、無限遠なら方向）を主波長の主光線で解く。
  条件は `{"cond": "position", "surface": 面}`（面での位置。像面なら実像高と同じ）、`{"cond": "dir_sum", "s1": 面, "s2": 面}`
  （2 面での方向余弦 l・m の和。法線入射で戻る主光線なら 0）、`dir_diff`（差）。面は番号か名前。パラメータを変えると解き直す。
  .zmx には書けない（OpticStudio にない）。Python は `s.set_field_type("solve", ("dirsum", 1, 6), fields=[(0, 0)])`
- 視野の正規化は `fields.norm`（`radial` 既定・`rect`、.zmx の FTYP の正規化と同じ）。ケラレ係数は `fields.vignetting` に視野ごとの
  `[VDX, VDY, VCX, VCY, VAN]`（VAN は度、省略した視野は 0。.zmx の VDXN〜VANN と同じ）。
- 大域座標の基準の面（.zmx の GLRS、書き出し・3D の座標の原点）は `"glrs"`。ブロックの面 `{"block": "L1", "surface": 0}`（数え方は
  `surface_edits` と同じ）か、展開したシステムの面の番号か名前。省略で面 1
- 像面の形・傾きは `"image"`（省略は平面で傾きなし）。面の並びの `row` と同じシステムファイル（版 2）の面の行で、
  `shape`（曲がった像面など）・`before`（像面の傾き偏心）・`aperture` などを書く。`t`・`medium`・`stop`・`coordinate_break` は書けない。
  GUI が像面を傾けた .zmx を組合せで開くときもこれで書く。
- 物体面の形・開口は `"object"`（省略は平面で開口なし）。`image` と同じ面の行で、`shape`（傾いた平面など）・`aperture` を書く。
  `medium` は物体空間の媒質（液浸など。省略は空気）。厚みは `object_distance` で、`t`・`stop`・`coordinate_break`・`before`・`after` は書けない。
  GUI が物体空間・像空間の空気でない .zmx を組合せで開くときは、物体空間の媒質をこれで書き、像空間の媒質は最後の空気のブロックの
  `medium`（要素に分けられないときは zmx ブロックの最後の面）が持つ。
  面 1 から取り込んだ .zmx の物体空間の媒質がこれと違えば注意を出す（プロジェクトの媒質を使う）。
- 像空間をアフォーカルにするには `"afocal": "mr"`（角度の単位 `ur`・`mr`・`rr`・`as`・`am`・`dg`。.zmx の Afocal Image Space と同じ）。
  物体側テレセントリックは `"telecentric": true`（有限の `object_distance`、開口は `na` か `cone`、エイミングなし）。どちらも状態の `project` で変えられる。
- クリア半径を固定した面（浮動開口）の外の光線をけないようにするには `"floating_clip": false`（既定はける。OpticStudio と同じ）。
- 定義した視野以外の点のケラレ係数の求め方は `"vig_interp"`（`"continuous"`（既定、視野の間で連続）・`"opticstudio"`（OpticStudio と同じ、視野の間で段差）。
  システムファイルの同じ名前の項目と同じ）。
- 瞳の振幅の分布は `"apodization"`（`"uniform"`（既定）・`{"gaussian": G}`・`"cosine_cubed"`。.zmx の GFAC と同じ）、OPD の基準は
  `"ropd"`（既定 2。.zmx の ROPD と同じ）。
- 温度・気圧は `"environment": {"temperature_c": 25, "pressure_atm": 1, "adjust_index": true}`（省略で 20 °C・1 atm・補正なし。
  .zmx の ENVD と同じ）。ガラスの屈折率はこの環境で計算し、インポートした .zmx の環境と違えば注意を出す。
- 知らないキー（綴りの違いなど）はエラーにする。GUI は自分の編集の状態を `gui` に残す。

## ビームスプリッター

- ビームスプリッター（`"type": "splitter"`）: 形は `cube`（`size` 一辺・`glass`、斜面が 45°）・`plate`（`thickness`・`glass`・`semi`、
  `wedge` でくさび、`split_face` で分ける面 front・back）・`ideal`（厚みのない面）。分ける面の傾きは `angle`（既定 [45, 0]、ミラーの tilt と同じ約束）。
  分岐比は `ratio`（偏光によらない理想コーティングの強度透過率、既定 0.5）・`coat`（コーティングファイル・LBCM のコーティング）・`pbs`（理想の偏光ビームスプリッター
  `{"tp": 1, "ts": 0}`、反射は 1 − T）のどれか。ほかの面のコーティングは `ar`（既定）と `faces`（キューブは `a`〜`d`、平板は分けない面の
  `front`・`back`）。理想コーティングは `I.s=Ts,p=Tp` とも書ける（s・p で別の透過率）。`id` が要る。偏心・傾き・ON/OFF も付けられる（OFF は素通り）。
  - 口: A（入射）、B（透過）、C（反射）、D（C の反対。B から戻る光の反射）。透過は A↔B・C↔D、反射は A↔C・B↔D。
  - 経路: BS を通る順の T・R（`TR` = 往きは透過、戻りは反射）。CLI は `--beam-path`、プロジェクトは `"path"`（状態の `project` でも）、
    省くと各 BS の `arm`（既定 t）。`info` が経路の一覧（`paths`）を出す。Python は `load(..., beam_path="TR")`・`System.beam_paths(path)`
  - 分岐 `t`・`r`・`d` は口 B・C・D の先のブロック（分岐のある BS は並びの最後）。分岐のない BS では、後ろのブロックが 1 回目に出た口に続く。
  - ミラーの `back` で戻って BS に来たら、経路の文字で口を選ぶ。A へ出れば前のブロックをさらに戻り、ほかの口へ出ればその口の分岐へ
    （分岐がなければミラーの後のブロックへ）進む。強度・Jones には通った面の係数の積が入り、腕どうしの光路差もそのまま出る
    （例は `P16_twyman_green.json`: 被検の腕 `TR` と参照の腕 `RT` が口 D で重なる）。
  - 面は BS の座標に固定して、通るたびに座標ブレークでその位置へ移して足す。分ける面での反射は、屈折面での反射（ゴーストと同じ、
    反対側の媒質つき）で、コーティングの係数は `コーティング|入射側|反対側|R` の境界の反射。キューブの斜面は硝材どうしの境界のコーティングとして扱う
    （OpticStudio の順次のシステムで斜面をミラー MIRROR にすると、コーティングの基板が硝材でなくなるので係数が違う）。
  - 口の軸の原点: キューブは各面の中心、平板は A が表の頂点・B が軸上で厚みだけ進んだ点（傾けた玉と同じ）・C・D が分ける面の頂点、理想面は面の上。
    くさびのある平板でも B の軸は A の軸のまま（光線は曲がる）。
  - 照合（`crates/lensbench-project/tests/splitter.rs`）: 平板の透過は傾けた玉、理想面の反射は折り曲げのミラー、キューブの透過はガラスの塊、
    キューブの反射はガラスの中の 45° のミラーを並べた .zmx と、像面の大域の位置・方向・光路長が 1e-11 で一致。くさびの振れ、Michelson の
    2 つの腕の光路差（腕の差の 2 倍）、分岐比・理想 PBS の s・p の強度も式どおり。
  - OpticStudio 26.1 との照合（`tests/splitter`、8 経路: キューブ・平板（くさび、表・裏で分ける）の透過・反射、往き透過・戻り反射、
    Michelson の 2 つの腕）: 書き出した各経路の .zmx の像面の位置 1.2e-13 mm・方向 8e-15・光路長 7e-13 mm、透過の経路の偏光の電場
    1.5e-9（コーティング BS3・AR2、`LBBS.DAT`。ガラスは試験のカタログを OpticStudio に入れて同じにした）。分ける面のコーティングの反射（硝材どうし・空気から）は
    Reflection vs Angle と R 5e-8・位相 5e-5°。OpticStudio の順次のシステムでは反射の面はコーティングなしのミラーで書いて形だけを比べる
    （キューブの斜面を MIRROR にすると 45° の BS3 で Rs 0.413 になり、Reflection vs Angle の硝材どうしの境界の値 0.497 と合わない）。

## カタログ

- ユーザーカタログ（JSON）: 玉モードで得た玉や組んだブロック（測定系・被検レンズなど）を名前で保存し、`{"type": "user", "catalog": "my.json", "item": "名前"}`
  のブロックで他のプロジェクトから使う（群と同じに入り、偏心・傾き・ON/OFF・状態も使える）。.zmx の玉は面を直接書いた玉に、.zmx のインポートと
  ユーザーカタログの参照のパスは絶対パスにして保存する。メーカーのレンズカタログの玉（`vendor`・`part`）は参照のまま保存するので、使う側でも
  同じレンズカタログが要る（CLI は `--lens-catalog`、ライブラリは `Env::catalog`）。玉のガラスのカタログは項目に記録し、使うときに足す。
  ガラスの名前（面の `glass` など、プロジェクトでもユーザーカタログでも）は「名前@カタログ」でもよく、そのカタログ（`glass_catalogs`
  か、ガラスのフォルダ）のその名前のガラスを引く。zmx ブロックが `glass_catalogs` と同じ名前で値の違うガラスを使うと、取り込んだ方を
  この名前で持つ（同じ名前のカタログ 2 つとぶつかれば「名前@カタログ#2」）。
  項目が自分自身を（間接にも）参照するとエラー。CLI は `catalog-add`（`--from` の .zmx・.json の `--lens k`、または .json の `--block id`。`--block` を 2 つ渡すと最上位の並びのその間の範囲、ブロックの `surface_edits`（面の表の変更）は玉の面に焼き込む）と `catalog-list`。GUI の組合せからも保存できる。
- レンズカタログの束（.lbcat）: メーカーのカタログをインポートのときに 1 回だけ変換した、玉の .zmx の束。zlib で圧縮した UTF-8 の文で、1 行目が
  `LBCAT 1<TAB>メーカー<TAB>変換元`、続いて玉ごとに `型番<TAB>焦点距離<TAB>.zmx のバイト数` の行と .zmx の本文（`lensbench_core::catalog::encode_lbcat`）。
  レンズカタログのフォルダに置くと .zmx の玉と同じに並ぶ（メーカーは束の値、無ければファイル名）。
- lensbench 形式のカタログ: ユーザーカタログの .json をレンズカタログのフォルダに置くと、項目を GUI のカタログ・serve `lenses` に
  カタログの品として並べる（メーカーはカタログの `vendor`、無ければフォルダの名前。型番は項目の名前）。ミラー・BS のブロック 1 つだけの項目は
  ミラー・BS の品（市販の BS・コーティング付きのミラーなど。コーティングは `coat`、BS の面のコーティングは `ar`・`faces`）で、組合せに足すと中身を写したブロックになる
  （向き・経路・寸法は組合せで変えられる）。ほかの項目は玉として `{"type": "user"}` で参照する。例: `examples/lenses/LBP/parts.json`
  gzip で圧縮した `.json.gz` もそのまま読む（同じ名前の `.json` があればそちら）。プロジェクトなどで `x.json` を指していて `x.json.gz`
  だけがあるときも、それを読む（リリースのカタログは .json.gz で配る）。

## 偏光子・波長板・複屈折材

- 偏光子・波長板の面: 面の並び（`surfaces`）の面に `"polarizer": {"axis_deg", "extinction_ratio", "transmittance"}`（理想の直線偏光子。
  透過軸の強度透過率と消光比、省略は 1 と無限）か `"waveplate": {"retardance", "wavelength_um", "axis_deg", "order", "transmittance"}`
  （速い軸 axis_deg、設計波長での遅延 retardance 波長、多次は整数の order。遅延は波長ごとに (order + retardance)·wavelength_um/λ）を書く。
  平面の Jones 面になり、偏光の追跡は波長ごとの行列 R(−θ)·diag(tx·e^{+iφ/2}, ty·e^{−iφ/2})·R(θ)（遅い軸が位相 −。OpticStudio の伝播の位相 e^{−i2πnd/λ}・厚みのある結晶板と同じ向き）を使う。
  .zmx には設計波長の行列の JONESMAT で書く（OpticStudio の JONESMAT は波長によらない）。
- 複屈折材の波長板: `waveplate` に `"material": "QUARTZ"`（常光はガラス QUARTZ、異常光は QUARTZ-E。OpticStudio の
  複屈折材の名前の約束）と `"thickness_mm"` を書くと、厚みのある結晶板（平行平板）になる。厚みを省くと設計波長の遅延
  order + retardance から決める（d = (order + retardance)·λ0/|Δn(λ0)|、Δn = ne − no、カタログ温度の屈折率）。
  `"plates": [{"material", "thickness_mm", "axis_deg"}, …]` は板の組（光の通る順、`axis_deg` は波長板の速い軸からの各板の
  速い軸の向き）で、アクロマティック波長板は速い軸を直交させた 2 材（MgF₂ + 水晶など）で書く。板は面の並びの
  入口（`birefringent` の `in`、後ろは常光の材料、厚みは板の厚み、結晶軸は面内で遅い軸（ne > no の材料）か速い軸の向き）と
  出口（`out`、後ろは次の板の材料で厚み 0 か、空気で元の面の `t`）になり、システムは板の厚みだけ長くなる。光線は常光の屈折率で進み、
  出口で異常光の成分（結晶軸の光線に垂直な向き）に exp(−i·2π·(n(θ) − no)·L/λ) の位相を付ける
  （n(θ) = 1/√(cos²θ/no² + sin²θ/ne²)、θ は板の中の光線と結晶軸の角、L は板の中の長さ。斜めに入る光の遅延の変化も入る）。
  OpticStudio の Birefringent In（Mode 2、波長板モード）/ Out と同じで、.zmx にもその面で書く（BIRE__IN・BIRE_OUT）。
  位相の向きは OpticStudio と同じ（遅い軸が e^{−iφ}）で、理想の波長板（`material` なし、Jones 面）とも同じ。外側の 2 面のコーティングは波長板の面の `coat`、無ければ透過率 `transmittance`
  からの理想コーティング（1 面あたり √T、T = 1 は `I.99999999`）。貼り合わせの面はコーティングなし（Fresnel）。境界の Fresnel は常光・異常光で分けて求める: 板の中の電場を常光の成分（光線と結晶軸を含む面に垂直）と異常光の成分に分け、常光は no、異常光は n(θ) を板の側の屈折率にした係数をかける（入口・出口とも、斜めの光線・コーティングファイルの層のコーティングも。理想コーティング・実測表・コーティングのモデルは屈折率によらないので同じ係数）。垂直入射の片面の透過は 2√no/(1+no)・2√ne/(1+ne)。板どうしの貼り合わせ（出口の面の厚み 0 で、すぐ後に同じ形・コーティングなしの次の板の入口）は境界 1 回として、前の板の常光・異常光の成分を次の板の常光・異常光の基底へ分け、前の板の屈折率（no₁ か n₁(θ)）から次の板の屈折率（no₂ か n₂(θ)）への係数をかける（垂直入射で 2√(n₁n₂)/(n₁+n₂)）。光線の道は常光のまま（walk-off は扱わない）。コーティングなしの板では OpticStudio と最大 5e-4 違う（OS は面に垂直な光線で、実験室の x・y のうち結晶軸に近い方に ne の透過を入口の面だけで使い、結晶軸の向きと合わない。物理的に正しい式を採る。コーティングのある板は一致）。テスト用の水晶・MgF₂ は
  `tests/glass/LBCRYSTAL.AGF`（refractiveindex.info、CC0）。GUI の面の並びの波長板にも複屈折材・厚み（2 材まで）の欄がある。
- 複屈折材の板を面で直接書く: 面の並びに `{"glass": "QUARTZ", "t": 1, "birefringent": {"in": {"axis": [x, y, z]}}}`
  （結晶軸は面の局所座標、`"mode": 0` で常光の成分だけ、`1`・`3` で異常光の道）と `{"t": 5, "birefringent": "out"}`。入口と出口の間にほかの面は置けない。
  .zmx の BIRE__IN・BIRE_OUT も読める。Gradient Mode は未対応。
- 異常光の道（Mode 1・3）: 光線は入口で異常光として屈折し（波数ベクトル m = n(θ) k̂ の接線の成分が続く。θ は波面法線 k̂ と結晶軸の角）、
  板の中はエネルギーの向き（m/ne² + (1/no² − 1/ne²)(m·a) a、walk-off）へ進み、出口で m から等方な媒質へ屈折する。光路長は異常光で数える
  （光線の長さ L に m·ŝ をかける。垂直入射の平行な板では n(θ)·厚み）。Mode 1 は出口で異常光の成分だけを通し、Mode 3 は常光の成分に
  異常光に対する位相 exp(+i·2π·(n(θ) − no)·L(k̂·ŝ)/λ) を付ける（Mode 2 と同じ位相の差を、異常光の道で）。境界の Fresnel は波面法線の向きで求める。
  板の中に鏡・GRIN の媒質・屈折する面があると追跡の入口で断る。近軸量は常光の屈折率で出る。
  OpticStudio 26.1 と照合（結晶軸を法線から 30° 傾けた水晶 1 mm、`tests/birefringent/os_b5.json`）: 光線の位置・向き（walk-off を含む）と
  Mode 1 の偏光は一致する。OS は異常光の道の光路長を no × 光線の長さで数え（垂直入射で 1 mm あたり約 2e-3 mm 短い）、Mode 2・3 の常光・異常光の
  位相の差にも no·(光線の長さ − 厚み) を足す（この例で 0.02〜0.04 波）。どちらも物理の式（波数ベクトルの位相）を採る。.zmx を読むと、この違いが出る板（Mode 1・3 と、結晶軸を法線から傾けた Mode 2・3）を読込の注意で知らせる。

## 特別な面・鏡・コーティング・マクロ

- ほかの欄で書けない面: 面の並び・自作の玉の面に `{"row": {…}}` と書くと、システムファイル（版 2）の面の行
  （`shape`・`action`・`medium`・`aperture`・`semi`・`t`・`ignore`・`dn`・`disp`・`comment`・`coating` など）で面を書ける。
  Zernike Standard / Fringe Sag・Grid Sag・Irregular・Tilted・Binary 2・Cylinder Fresnel・理想レンズ・GRIN の媒質・無視する面などに使う。
  `row` を書いた面にはほかの鍵を書かない。行の `stop`（絞りはブロックの `set_stop`）・`object`・`image` は書けない。玉の最後の面の
  厚み・媒質は使わない（後ろの空気のブロック）。行には面の前後の傾き偏心（`before`・`after`）も書ける。そう書いた面を含む玉は
  裏返せず、面の並びは復路を通れない。GUI が .zmx を要素に分けるときもこの形で書く。座標ブレークの面（`coordbrk`）も玉の面に書け（開口（浮動の開口も）・`ignore` も書ける）、面の上書き・操作の面の番号では数えない。面の `ignore: true` はその面を無視する（OpticStudio の Ignore This Surface）。
- 媒質で満たす間隔（液浸）: 空気のブロックに `"medium"`（硝材の名前か屈折率の数）を書くと、前の面の後をその媒質にする
  （`{"type": "air", "t": 0.2, "medium": "WATER"}`）。最後の空気のブロックなら像空間の媒質。前が鏡・硝材の間隔と、復路では書けない。
  GUI が像空間の空気でない .zmx を要素に分けるとき、最後の間隔をこれで書く（後ろに要素を足すと、その要素は媒質の中に入るので注意が出る）。
- 玉の中の鏡（裏面鏡・プリズムやキューブ BS の中の反射）: 自作の玉の面に、働きが鏡の行（`{"row": {"action": "reflect", …}}`、
  媒質は書かない）を書く。厚み `t` は鏡から次の面まで、光の進む向きで正（展開で鏡の後の向きの符号にする）。鏡の後の面は
  ほかの鏡の後と同じく裏返して置く。最初と最後の面は鏡にできない。鏡のある玉は裏返せず、復路も通れない。傾け・偏心して置くこと
  （`decenter`・`tilt`）と、外して空気にすることもできない（鏡は自分で傾ける）。面の並び（空気中）には鏡を書けない（鏡のブロックで）。
  GUI が .zmx を要素に分けるとき、ガラスの中の鏡はこの形で書く。
- radius・conic で書けない鏡（Zernike・Grid Sag など）: 鏡のブロックに `{"type": "mirror", "row": {…}}` と書くと、鏡の形を面の行で
  書ける。働きは鏡（`"action": "reflect"`、または `mirror` の付く働き）。`row` を書くときは radius・conic・semi・aperture・coat を
  書かず行に書く（`semi`・`aperture`・`coating`）。行の厚み・媒質・`stop`（絞りはブロックの `set_stop`）は書けない（間隔は後ろの
  空気のブロック）。面の前後の傾き偏心（`before`・`after`、.zmx の面の傾きをそのまま）は行に書ける。ブロックの `tilt`（折り曲げ）と
  一緒には書けない。鏡の後（向きが逆の所）に置くと、ほかの面と同じく裏返して置く。裏返すのは面の形だけで、行の `before`・`after` は
  そのまま（鏡の後の座標の値として、OpticStudio と同じに読む。面の並び・自作の玉の行も同じ）。
  GUI は .zmx の空気中の鏡を鏡の要素に分け、標準面でないものはこの形で書く（面の表で変えた r・k・semi は面の上書き）。
- コーティングファイル: `"coating_file": "COATING.DAT"`（OpticStudio のコーティングファイルの形。相対パスはプロジェクトのフォルダから、そこに無ければ .zmx のコーティングファイルと同じく材料の検索フォルダから）。面の `coat` の名前を、
  `coatings` に無ければこのファイルで引く。取り込んだ .zmx のコーティングファイルより優先（違えば注意）。GUI が .zmx を要素に分けると、
  その .zmx のコーティングファイルをここに書く（材料の検索フォルダで見つけたものはそこからの相対パス）。
- ZPL マクロのソルブ: `"macros": [{"name": "MIRR", "source": "…"}]`（.ZPL の中身をそのまま。マクロのファイルは読まない。同じ名前は 2 つ書けない）と書き、
  ブロックの `surface_ops` に `{"op": "add_solve", "surface": 1, "kind": "macro_curv", "macro": "MIRR"}`（厚みは `macro_thick`）で
  `macro` にマクロの名前（大文字・小文字は区別しない。0 始まりの番号でもよい、`param` に番号でも読む）を書く。円錐係数・半直径・
  面タイプのパラメータは `{"op": "add_macro_solve", "surface": 0, "macro": "K", "what": "k"}`（`what` は `k`・`semi`・`p1` …）。
  OpticStudio の Macro Solve と同じに解く（.zmx のマクロのソルブと同じ）。マクロの中の面の番号は展開したシステムの面の番号
  （`SOSO(0)` からの相対でなければ、前に面を足し引きすると指す面が変わる。OpticStudio と同じ）。
  GUI がマクロのソルブのある .zmx を要素に分けると、マクロをここに書き、ソルブは名前で指す。
- フィルタ・ダイクロイックのコーティング: プロジェクトかユーザーカタログの `"coatings": {名前: コーティング}` に書いたコーティングを、面・BS の `coat`
  に名前で使う（TABLE コーティングとして偏光の追跡・透過に効く。プロジェクトの同じ名前がカタログより優先、コーティングファイルの同じ名前より優先）。
  コーティングは `{"type": "table", "rows": [[入射角 deg, 波長 µm, Rs, Rp, Ts, Tp], …]}`（表そのまま）、`curve`（`wavelengths_nm`・`t`・`r`、
  角度・偏光によらない。色ガラス・吸収型 ND を 1 面で表すときなど）、仕様からの理想の形の `longpass`・`shortpass`（`edge_nm` = 透過 50 %）・
  `bandpass`・`notch`（`center_nm`・`fwhm_nm`）・`nd`（`od`）。理想の形は帯域内の透過 `t`（0.95）、帯域外の OD `od`（4）、
  エッジの 10〜90 % の幅 `width_nm`、斜め入射のエッジのずれ λ(θ) = λ0·√(1 − (sinθ/neff)²) の `neff`（2）、反射は 1 − T
  （`absorptive` で 0）。ダイクロイック BS は BS の `coat` にロングパス・ショートパスを書くと、波長で T・R の経路の強度が分かれる。
  色ガラス・吸収型 ND は、内部透過率（AGF の IT 行）のあるガラスでも表せる（厚みに応じた吸収）。
  理想の形の表は入射角 0〜75°、`curve`・`nd` は 0〜89° で作る。表の角度の範囲より大きい入射角の光線は、表の端の値を使わず
  エラー（「入射角 … が表の範囲の外」）で止まる（`table` は書いた行の角度の範囲）。
  .zmx に書き出すと、これらのコーティングは表（TABLE）にして `.zmx` の隣の `名前_coat.dat`（COATING.DAT 形式。システムのコーティングファイルがあれば
  その中身も入れる）に書き、COFN はそのファイルを指す。lensbench は `.zmx` のフォルダから読む。OpticStudio で開くときは
  このファイルを OpticStudio の Coatings フォルダに写す。OpticStudio の TABLE は全部の角度で同じ波長の並びでないと、
  エラーを出さずに表全体を R = T = 0 として読むので、書き出しでは全角度の波長の和集合に並べ直す（lensbench では同じ値）。
  理想の形の表は点が多く（1 コーティングで 1,500〜3,600 波長 × 13 角度）、コーティングファイルが大きいと OpticStudio の読み込みに数十秒かかる。
- 面の傾き: 面に `"tilt": [tx, ty]`（deg）で、その面だけを頂点のまわりに傾ける（後の面の座標は元の軸に戻す。OpticStudio の面の Tilt/Decenter の後を Reverse）。くさびのウィンドウは 2 面目に `tilt`（くさび角 3′ なら 0.05）。

## 機構部品と品の形

- 機構部品: ユーザーカタログの品に `kind`（tube・cage_plate・mount・rod・adapter）と `mech`
  （`bores`: 光の通る穴 `{"z0", "z1", "diameter"}` の並び、z は部品の基準面から光の進む向き。`length`・`outer_diameter`・
  `thread`・`rod_spacing`・`holds` `{"diameter", "seat_z", "max_diameter"}`（`max_diameter` は上下から挟むホルダー（バー型・ジョー型）のように径を選ばない部品の入る径の上限で、`diameter`〜`max_diameter` の素子が入る。保持径が太すぎる確認はしない）・`front_thread`・`back_thread`・`outline`）を書く（`blocks` は省略できる）。プロジェクトの `mechanics` に
  `{"name", "catalog", "item", "at": {"block": id, "surface": i}, "offset": [dx, dy, dz], "tilt": [tx, ty]}`（`catalog`・`item` の
  代わりに `mech` をその場で書いても、`at` にシステムの面の番号 `{"surface": n}` だけでもよい）で、ブロックの面の局所座標に置く。
  光学系の位置は変えず、光線の追跡にも効かない。`lensbench mech-check` は瞳の格子の光線を面 1 から追い、面と面の間の直線が
  穴（円柱）の z の範囲で半径の外に出たら切れたとして、部品ごと・視野と波長ごとに切れた割合（`clipped_pct`、ほかで先に切れた
  光線も数える）と最初に切った割合（`first_pct`）、システムの開口で切れた割合、通った割合を出す。例は `examples/projects/P19_tube_cage.json`。
  Python は `System.mech_check()`（同じ dict）。
- 枠付きの品: ユーザーカタログの品に `frame`（`mech` と同じ形。z は枠の端から光の進む向き、`holds.seat_z` はガラスの前面の位置）を
  書くと、`blocks`（ガラスだけ）をプロジェクトに `user` のブロックで置いたとき、枠がガラスの最初の面に機構部品（名前は「ブロック id の枠」、
  保持する素子はその品の面）として付き、`mech-check` のケラレ・当たりと描画に入る。品ページの STEP を測ると（GUI の補う窓）、
  組立の置き方どおりに置いた立体のうち、光軸と同じ軸の穴（内壁の円柱）を持つものを枠、持たないものをガラスとして分ける。
  ガラスと枠が 1 つの立体なら、光軸をふさぐ面（縁が 1 つの円だけの面）の間をガラスとする（ガラスの径は見えている径）。ガラスの無い機構部品（チューブ・ケージ板・マウント）の STEP は、全体を枠として穴・長さ・外径（ケージ板・キューブは一辺）・内側のねじ（SM05〜SM3・C マウント、保持する素子の径はねじの呼び。ねじは外径の円柱で描く STEP（Thorlabs）と、メスを山の径（内径）の円柱で描く STEP（Edmund）のどちらも読む。C マウントのメスを外径 25.4 で描く STEP は、1 インチの素子の穴と区別するため、端のねじで奥に逃げの溝があるときだけ）・ロッドの間隔（規格の 16・26.87（Edmund の 25・30 mm のケージ）・30・60 mm に近い穴を優先。規格に合わなければちょうど 4 つの穴の組）。ねじとロッドの間隔の規格の表は同梱の JSON（`crates/lensbench-catalog/src/step/geom/standards.json`: `threads` の `name`・`major_mm`（外径）・`pitch_mm`・`holds_mm`（入る素子の径、0 は入れない）・`female_major`（メスを外径の円柱でも判定するか）と `rod_spacings_mm`）で、環境変数 `LENSBENCH_STEP_STANDARDS` に同じ形の JSON のファイルを与えると、そのねじ（同じ名前は置き換え）と間隔を足す（読めなければ測りはエラー）。光軸は半周以上の弧の円のうち最大のもの（ロッドの穴の周りの溝のような短い弧は除く）、光軸の穴の無い板はロッドの穴の正方形の中心。ロッドの間隔の規格に合う組が 2 つ（間隔を変える板）なら大きい方・座（止め輪の奥の面か、枠の端から続く保持径より細い穴の段）を測る（`lensbench catalog-step`）。付属の止め輪（本体より短く、外径が本体の穴に入る輪）は素子に合わせて動かすので枠の穴に含めず、`retaining_rings`（z・内径・外径）に分けて返す。仕様表の値と 0.05 mm より違う寸法（有効径は枠の最小の穴より大きい時）は `notes` に書く（serve の `catalog_fill` の `spec`）。
  枠の向きは品の `frame_reversed`（false: 光は枠の z の小さい端から入る、true: 大きい端から）。STEP から測った枠は STEP の
  座標の向きなので、図面の矢印などで決める
- 機構の配置の確認と描画: `mechanics` の項目に `"holds": [ブロック id]` で、部品が保持する素子を書ける。`mech-check` の
  `fit` は、素子の面の縁が部品の内壁（穴の半径と保持径の半分の大きい方）に当たる `collision`、保持する素子が部品の z の範囲から
  はみ出す `protrude`、保持径より素子が 0.5 mm 以上細い `loose`（注意）、部品どうしの壁が重なる `overlap`、同じ軸のケージ板の組の
  ロッドの要る長さ `rod`（注意）とロッドの間隔の違い `rod_mismatch` を出す。`layout` の 2D（SVG・DXF の MECH レイヤー・PNG）・
  3D（glTF・OBJ・STL）・STEP は部品を軸のまわりの輪（内壁〜外径。穴のない区間は詰まった円柱）で描く。寸法に
  `"outline": "square"` を書くと外形を正方形の柱（一辺は `outer_diameter`、辺は部品の x・y 軸に沿う。ケージ板・キューブ）で描く
  （省略時は `"round"`、円柱）。`catalog-step` は、光軸に直交する向きの幅の最小が軸の円の径より太ければ角とする。
  GUI では、カタログの機構部品の行を足すと組合せの「機構部品」に入り（選んでいる要素に置く）、置く要素・面・ずれ・保持する要素を
  変えられる。レイアウト図・3D 表示に部品を描き、ケラレ・当たりは解析の「機構のケラレ」で見る。
- 機構部品を素子に合わせて置く: `mechanics` の項目に `"seat": true` と `"holds": [ブロック id]` で、部品の座（寸法の
  `holds.seat_z`）を保持する最初の素子の前面に合わせて置く（`at` は要らない。`offset` はそこからのずれ）。GUI では機構部品の欄の
  「座に合わせる」。素子が入る機構部品は `lensbench catalog-search <カタログ> --fits <径>`（保持径が径以上で 0.25 mm より
  太くないもの）で探せ、GUI のカタログの詳細にも「入る機構部品」を出す
- 機構部品をつなぐ: `mechanics` の項目に `"after": "前の部品の名前"` で、前に書いた部品の z の大きい端につなぐ（`at`・`seat` は
  使わない。同じ面・傾きに置き、`offset` は前の部品の座標でのずれ、`tilt` は使えない。同じ名前が複数あれば最後のもの）。
  寸法の `front_thread`（z 0 の端）・`back_thread`（z の大きい端）に `{"thread": "SM1", "male": true, "length": 3.0}` を書くと、
  つなぐ両端のねじが同じ名前でオスとメスか確かめ（違えば注記して突き当てる）、オスのねじの長さだけ重ねる（ねじ込む）。
  ねじの穴の端が部品の外形の端より奥にあれば `recess`（引っ込み、mm）を書き、オスはその分も奥へ入る。
  `catalog-step` は両端のねじも読む（端に接するねじの谷の径の外向きの円柱がオスで、長さは端から外径より細い部分。端から最も近い
  内壁がねじの谷の径ならメスで、1 mm を超えて奥にあれば `recess`）。
  GUI では機構部品の欄の置く所で「〜の後ろにつなぐ」。`"reversed": true` で部品を反転して置く（光が z の大きい端から入る。穴・座・
  両端のねじも入れ替わる。`seat` なら座は保持する素子の後ろの面に当たる）。チューブの外側のねじ（オス）をケージ板の SM1 の
  穴（メス）にねじ込む組み方もこれで書ける（ケージ板の後ろにチューブ、または反転たチューブの後ろにケージ板）。ケージ板どうしは
  `after` と `offset`（間隔）で並べ、ロッドの間隔の違い・要る長さは `mech-check` の `fit` で見る
- 品の詳しい形（公式の STEP）: カタログには機構部品の簡易外形（`mech`）だけを入れ、STEP は入れない。品に
  `"cad": {"step": "<STEP の URL>", "pdf": "<図面の URL>"}`（メーカーの配布元。STEP は .step・.stp か、それを含む zip）を書くと、
  `lensbench cad-step --catalog <カタログ> --item <品>`（serve の `cad_step`、GUI のカタログの詳細の「詳しい形（STEP）を取る」）で、
  その品の STEP を 1 件だけ取ってキャッシュ（`--cache`、省略で環境変数 `LENSBENCH_CAD_CACHE`、なければ Windows は
  `%APPDATA%\lensbench\cad`・ほかは `~/.cache/lensbench/cad`。GUI は設定のフォルダの `cad`）に置く。キャッシュのファイルは品の名前と
  URL で決まり（`<品>-<URL のハッシュ>.step.gz`、gzip の STEP。以前の `.step` も読む）、2 回目からはキャッシュを使う（URL が変われば取り直す）。裏でまとめて取りには行かない。取った STEP は `catalog-step` と
  同じ読み方で測り、簡易外形と長さ・外径が 0.05 mm より違うか、外形の形（丸・角）が違えば知らせる。取れなければ（中身が STEP で
  ない・つながらない）エラーで、品ページの URL を示す（表示は簡易外形のまま）。
  **共有の置き場**: 自前の HTTP の口（`GET <URL>/<鍵>` で STEP、無ければ 404。`PUT <URL>/<鍵>` で上げる。鍵はキャッシュの
  ファイル名から `.gz` を除いたもの。STEP は gzip で上げ、取るときに展開する）を決めておくと、キャッシュに無い品は配布元より先に置き場を見て、配布元から取った STEP は置き場にも上げる。
  `cad.step` の無い品も、置き場に上げてあれば取れる。決め方は、GUI の「カタログのフォルダ」の窓の「品の STEP の置き場」、
  環境変数 `LENSBENCH_CAD_STORE`（URL）、配る版がビルドのときに組み込んだ置き場（`lensbench_cad::set_default_store`）の順。
  Cloudflare Access で守った口（Cloudflare R2 を Pages Functions で返すなど）には、Access のサービストークンを GUI の
  Client ID・Client Secret か環境変数 `LENSBENCH_CAD_STORE_ID`・`LENSBENCH_CAD_STORE_SECRET` で渡す
  （見出し `CF-Access-Client-Id`・`CF-Access-Client-Secret`。トークンを送るのは https の口だけ。GUI の Secret は設定のファイルに平文で残る）。置き場に届かなくても配布元から取る（結果に理由を添える）。
  手元にある STEP を上げるのはライブラリの `lensbench_cad::upload_jobs`・`upload_file`（ファイル名に品の名前を含む .step・.stp・
  .zip を機構部品の品に当てる。STEP を 2 つ以上束ねた zip は中の STEP を 1 つずつ当てる。カタログにフォルダを渡すと
  中の .json・.json.gz を全部読む。キャッシュのファイルをそのまま上げることもできる）。置き場にある鍵は上げない（上げ直す指定もある）。
  置き場を持つ配る版は、これを呼ぶ保守の道具を自分の exe に足す（公開版の CLI には無い）。
  STEP を取ってある品を `mechanics` に置き、`lensbench layout --mech-detail`（serve の `layout`・`layout_export` の
  `mech_detail`、GUI のレイアウトの設定の「機構部品を詳しい形で」）で描くと、3D（glTF・OBJ・STL と GUI の 3D 表示）は
  STEP を三角形にした形（面からのずれ 0.2 mm まで。三角形にできない面・境の読めない面は除く）、STEP の書き出しは元の立体を
  写したもの（名前は「部品の名前 detail 番号」）で描く。STEP の立体は catalog-step と同じ測り方で部品の座標（z は枠の端から光軸の向き、
  x は角の部品の辺に直交）に置くので、簡易外形と同じ所に重なる（`reversed` なら反転する）。2D の断面図・`mech-check` の
  ケラレ・当たりは簡易外形のまま

## カタログの品の種類・未完の品

- カタログの品の種類・仕様・リンク: 項目に `kind`（lens・mirror・splitter・polarizer・waveplate・window・filter・prism・other、
  省略時はブロックから）、`url`（製品ページ）、`specs`（名前 → 数・[下限, 上限]・文字列。名前の決まったものは
  `lensbench catalog-search x --list-specs`: wavelength_nm・diameter・thickness・coating・split_ratio・extinction_ratio・retardance・wedge・flatness など）、
  `tags`、出典 `source`・確認日 `checked`。カタログ全体の `url_template`（`{part}` を型番に）と `overlay`（型番のパターン `*`・`?` → `specs`・`url`・
  `tags`・`description`・`kind`）は、同じメーカー（大文字小文字・空白・記号は無視）のほかのカタログの品（ZMF・.zmx の玉など）にも当てる。
  品が自分で持つ値が先、overlay は `*` の無い型番、長いパターンの順。品の無い overlay だけの .json も置ける。
  `lensbench catalog-search` は種類（`--kind`）・メーカー（`--vendor`）・語（`--text`）・仕様（`--spec 名前=値`。数は一致、範囲の仕様は値を含む、
  文字列は含む。`名前=a..b`、`名前>=値`・`<=`・`>`・`<`（範囲の仕様は重なりで）、`名前` だけはある品）で絞り込む。`efl`・`diameter`・`thickness` は
  一覧の値でも比べる。serve は `catalog_search`。
- 未完の品: blocks の無い項目に `missing`（足りない値の名前 `thickness`・`diameter`）を書くと、配置できない「未完の品」として並ぶ
  （`title`・`notes` も持てる）。GUI でこの品を選ぶと補う窓が開き、品ページの仕様表（`Thickness (mm):` など）か、CAD 欄から落とした
  STEP を測って足りない値を入れ、ユーザーが確かめて確定する。確定した品は設定のフォルダの `catalog-fill/<メーカー>.json`
  （ユーザーカタログ）に普通の品（ミラーは平面ミラー、ほかは `specs.material` の平行平板。分からない基板は N-BK7 で代用。四角い品は矩形の開口）として保存し、
  次からは未完の品の代わりに並ぶ（元のカタログは書き換えない）。serve は `catalog_fill`。品ページは選んだ品の 1 件だけを開き、
  まとめて取りに行くことはしない。

## 状態の例

```json
"states": [
  {"name": "near", "project": {"object_distance": 500}, "set": {"front": {"shift": -0.4}}},
  {"name": "no L3", "set": {"L3": {"on": false, "off": "air"}}}
]
```

## コーティングの境界

`coatings` は全コンフィグの境界（`コーティング|前の媒質|後の媒質`）とコーティングの扱い（コーティングなし、理想コーティング、コーティングファイルの薄膜・表、実測表、モデル）を一覧にし、
偏光追跡に実測表かモデルが要る境界（暗号化されたコーティングなど）を `needs_table` に挙げる。

コーティングファイル（COATING.DAT 形式）の多層膜は、透過・ミラーとも OpticStudio と同じ約束で計算する（層はコーティングの外側から順、
後ろの媒質が空気なら基板は前の媒質、ミラーの基板は最後の層の材料、係数に掛かる位相。`lensbench_core::coating` の
`Coatings::prepare` に説明）。膜厚が波長単位の層はシステムの主波長で厚みを決める。
