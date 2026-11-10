# プロジェクトファイル（ブロック、状態、群）

`.json` のプロジェクトファイルは玉・空気・絞り・鏡・面の並び・.zmx の取り込み・群のブロックで系を組む（例は `examples/projects`）。

JSON スキーマは `docs/schema/` にある: `project`（プロジェクトファイル）・`system`（系ファイル）・`user-catalog`（ユーザーカタログ）・
`swap`（差し替え探索の `--spec`）の `.schema.json`。どのファイルも先頭に
`"$schema": "https://raw.githubusercontent.com/masa-iwm/lensbench/main/docs/schema/project.schema.json"` のように書くと、
エディタ（VS Code など）で項目の補完・説明・検査が効く（lensbench は読むときに `$schema` を無視する）。スキーマは serde の型
（`lensbench_project::schema`）から作り、テストで型とずれていないか・例題が合うかを確かめる。型を変えたら
`LENSBENCH_UPDATE_SCHEMA=1 cargo test -p lensbench-project --test schema` で作り直す。

- 検出器のブロック（`"type"` が組み込みのブロックでない名前）は、この lensbench を組み込んだ版が登録した種類
  （`lensbench_project::detectors`）だけ読める。最後のブロックだけに置け、面は足さない（像面に置く。前の空気のブロックが距離）
- .zmx の取り込み（`"type": "zmx"`）は面 `from`〜`to`（既定は面 1〜像面の手前）・構成 `config`。`"stop": false` で取り込んだ面の絞りを外す
  （系の絞りはほかのブロックに）
- 群（`"type": "group"`）は `role`（`zoom`・`focus`・`fixed`）と移動 `shift` [mm] を持つ。`shift` は前の間隔に足し、後の間隔から引くので、
  群の外の合計長は変わらない（鏡を含む群は、鏡の枚数によらず動かせない）。.zmx の取り込みでは `groups` に面の範囲（`from`・`to`、その面に始まる玉・面と間の空気）で群を置ける（玉モード）。
  群にも偏心・傾き・ON/OFF を付けられる。
- 状態（`states`、多重構成に当たる）は名前と変更の組。`set` はブロックの `id` ごとに変える値（`on`・`off`・`t`・`shift`・`decenter`・`tilt`・
  .zmx の構成番号 `config` など、ブロックの JSON の項目）、`project` は系の値（`object_distance`・`aperture`・`fields` など）。
  CLI は `--state 名前` で選ぶ（`info` が状態の一覧を出す）。ライブラリは `lensbench_project::expand_state`。
- 自動の半径: 面の `semi` を `"auto"` にすると、半径を光線から求める（OpticStudio の Automatic と同じ決め方: 全視野・全波長の
  瞳の縁の光線の高さの最大。回転対称な系は視野の値 (x, y) の向きの上下 2 本、そうでない系は縁の 8 本）。系やパラメータを変えるたびに求め直し、
  描画・機構部品の検査・書き出し・Floating 開口に使う。`surface_edits` の `"semi": "auto"`、系ファイルの面の `"semi_auto": true`、
  .zmx の `DIAM 値 0`（自動）も同じ。カタログの玉の半径はファイルの値で固定
- 面の値の上書き（`surface_edits`）: `[{"block": "L1", "surface": 1, "fields": {"r": -40, "semi": 8}}]` のように、ブロック `id` の
  面の値を展開の最後に変える。`surface` はそのブロックの面のうち座標ブレークを除いて数えた番号（0 始まり。偏心・傾きを付け外ししても
  変わらない）、`fields` は serve の `edit` の `set_surface` と同じ名前と、展開した系の値（鏡の後は厚さ・曲率の符号が逆）。
  鏡の `back` で往復するブロックでは、番号は往路の面で数え、復路の同じ面にも当てる（逆の順にたどるので、厚さ・材料は 1 つずれた面に、
  厚さは符号を逆に）。往路の最後の面の厚さ・材料はブロックの外（後の空気のブロック）なので変えられない。
  `fields` の名前が面の値の名前に無ければ、serve の `set_surface_data` と同じ面のデータの項目（`ap`・`before`・`zern` など）として当てる。
  GUI の面の表で組合せの玉の面を変えると、これで保存される
- 面の絞り・ソルブ・ピックアップ（`surface_ops`）: serve の `edit` の `set_stop`・`add_solve`・`add_pickup` と同じ形で、面を
  `{"block": "L2", "surface": 1}`（`surface_edits` と同じ数え方。往復するブロックは往路の面）で書く。例:
  `[{"op": "set_stop", "surface": {"block": "L1", "surface": 1}}, {"op": "add_solve", "surface": {"block": "L2", "surface": 1}, "kind": "marginal_height"},
  {"op": "add_pickup", "dst": {"block": "L2", "surface": 1}, "src": {"block": "L1", "surface": 0}, "what": "c", "scale": -1}]`。
  `surface_edits` の後に当てる。`set_stop` があれば絞りのブロックは無くてよい（あれば絞りは面に移る）。ブロックの順を変えても id で付いて行き、
  状態で外したブロックの操作は当てずに注意を出す

- 視野の種類（`fields.type`）は `angle`・`object_height`・`paraxial_image_height`・`real_image_height` と、条件で定める視野 `solve`。
  `solve` では `fields.solve` の条件について、視野の (x, y) が目標値になる物体（有限なら物点、無限遠なら方向）を主波長の主光線で解く。
  条件は `{"cond": "position", "surface": 面}`（面での位置。像面なら実像高と同じ）、`{"cond": "dir_sum", "s1": 面, "s2": 面}`
  （2 面での方向余弦 l・m の和。法線入射で戻る主光線なら 0）、`dir_diff`（差）。面は番号か名前。パラメータを変えると解き直す。
  .zmx には書けない（OpticStudio にない）。Python は `s.set_field_type("solve", ("dirsum", 1, 6), fields=[(0, 0)])`
- 像空間をアフォーカルにするには `"afocal": "mr"`（角度の単位 `ur`・`mr`・`rr`・`as`・`am`・`dg`。.zmx の Afocal Image Space と同じ）。
  物体側テレセントリックは `"telecentric": true`（有限の `object_distance`、開口は `na` か `cone`、エイミングなし）。どちらも状態の `project` で変えられる。

- ビームスプリッター（`"type": "splitter"`）: 形は `cube`（`size` 一辺・`glass`、斜面が 45°）・`plate`（`thickness`・`glass`・`semi`、
  `wedge` でくさび、`split_face` で分ける面 front・back）・`ideal`（厚さのない面）。分ける面の傾きは `angle`（既定 [45, 0]、鏡の tilt と同じ約束）。
  分岐比は `ratio`（偏光によらない理想膜の強度透過率、既定 0.5）・`coat`（膜ファイル・LBCM の膜）・`pbs`（理想の偏光ビームスプリッター
  `{"tp": 1, "ts": 0}`、反射は 1 − T）のどれか。ほかの面の膜は `ar`（既定）と `faces`（キューブは `a`〜`d`、平板は分けない面の
  `front`・`back`）。理想膜は `I.s=Ts,p=Tp` とも書ける（s・p で別の透過率）。`id` が要る。偏心・傾き・ON/OFF も付けられる（OFF は素通り）。
  - 口: A（入射）、B（透過）、C（反射）、D（C の反対。B から戻る光の反射）。透過は A↔B・C↔D、反射は A↔C・B↔D。
  - 経路: BS を通る順の T・R（`TR` = 往きは透過、戻りは反射）。CLI は `--path`、プロジェクトは `"path"`（状態の `project` でも）、
    省くと各 BS の `arm`（既定 t）。`info` が経路の一覧（`paths`）を出す。Python は `load(..., split_path="TR")`・`System.split_paths(path)`
  - 分岐 `t`・`r`・`d` は口 B・C・D の先のブロック（分岐のある BS は並びの最後）。分岐のない BS では、後ろのブロックが 1 回目に出た口に続く。
  - 鏡の `back` で戻って BS に来たら、経路の文字で口を選ぶ。A へ出れば前のブロックをさらに戻り、ほかの口へ出ればその口の分岐へ
    （分岐がなければ鏡の後のブロックへ）進む。強度・Jones には通った面の係数の積が入り、腕どうしの光路差もそのまま出る
    （例は `P16_twyman_green.json`: 被検の腕 `TR` と参照の腕 `RT` が口 D で重なる）。
  - 面は BS の座標に固定して、通るたびに座標ブレークでその位置へ移して足す。分ける面での反射は、屈折面での反射（ゴーストと同じ、
    反対側の媒質つき）で、膜の係数は `膜|入射側|反対側|R` の境界の反射。キューブの斜面は硝材どうしの境界の膜として扱う
    （OpticStudio の順次の系で斜面を鏡 MIRROR にすると、膜の基板が硝材でなくなるので係数が違う）。
  - 口の軸の原点: キューブは各面の中心、平板は A が表の頂点・B が軸上で厚さだけ進んだ点（傾けた玉と同じ）・C・D が分ける面の頂点、理想面は面の上。
    くさびのある平板でも B の軸は A の軸のまま（光線は曲がる）。
  - 照合（`crates/lensbench-project/tests/splitter.rs`）: 平板の透過は傾けた玉、理想面の反射は折り曲げの鏡、キューブの透過はガラスの塊、
    キューブの反射はガラスの中の 45° の鏡を並べた .zmx と、像面の大域の位置・方向・光路長が 1e-11 で一致。くさびの振れ、Michelson の
    2 つの腕の光路差（腕の差の 2 倍）、分岐比・理想 PBS の s・p の強度も式どおり。
  - OpticStudio 26.1 との照合（`tests/splitter`、8 経路: キューブ・平板（くさび、表・裏で分ける）の透過・反射、往き透過・戻り反射、
    Michelson の 2 つの腕）: 書き出した各経路の .zmx の像面の位置 1.2e-13 mm・方向 8e-15・光路長 7e-13 mm、透過の経路の偏光の電場
    1.5e-9（膜 BS3・AR2、`LBBS.DAT`。ガラスは試験のカタログを OpticStudio に入れて同じにした）。分ける面の膜の反射（硝材どうし・空気から）は
    Reflection vs Angle と R 5e-8・位相 5e-5°。OpticStudio の順次の系では反射の面は膜なしの鏡で書いて形だけを比べる
    （キューブの斜面を MIRROR にすると 45° の BS3 で Rs 0.413 になり、Reflection vs Angle の硝材どうしの境界の値 0.497 と合わない）。
- ユーザーカタログ（JSON）: 玉モードで得た玉や組んだブロック（測定系・被検レンズなど）を名前で保存し、`{"type": "user", "catalog": "my.json", "item": "名前"}`
  のブロックで他のプロジェクトから使う（群と同じに入り、偏心・傾き・ON/OFF・状態も使える）。.zmx の玉は面を直接書いた玉に、.zmx の取り込みと
  ユーザーカタログの参照のパスは絶対パスにして保存する。メーカーのレンズカタログの玉（`vendor`・`part`）は参照のまま保存するので、使う側でも
  同じレンズカタログが要る（CLI は `--lens-catalog`、ライブラリは `Env::catalog`）。玉のガラスのカタログは項目に記録し、使うときに足す。
  項目が自分自身を（間接にも）参照するとエラー。CLI は `catalog-add`（`--from` の .zmx・.json の `--lens k`、または .json の `--block id`。`--block` を 2 つ渡すと最上位の並びのその間の範囲、面の表の変更 `surface_edits` は玉の面に焼き込む）と `catalog-list`。GUI の組合せからも保存できる。
- lensbench 形式のカタログ: ユーザーカタログの .json をレンズカタログのフォルダに置くと、項目を GUI のカタログ・serve `lenses` に
  カタログの品として並べる（メーカーはカタログの `vendor`、無ければフォルダの名前。型番は項目の名前）。鏡・BS のブロック 1 つだけの項目は
  鏡・BS の品（市販の BS・膜付きの鏡など。膜は `coat`、BS の面の膜は `ar`・`faces`）で、組合せに足すと中身を写したブロックになる
  （向き・経路・寸法は組合せで変えられる）。ほかの項目は玉として `{"type": "user"}` で参照する。例: `examples/lenses/LBP/parts.json`
  gzip で圧縮した `.json.gz` もそのまま読む（同じ名前の `.json` があればそちら）。プロジェクトなどで `x.json` を指していて `x.json.gz`
  だけがあるときも、それを読む（リリースのカタログは .json.gz で配る）。
- 偏光子・波長板の面: 面の並び（`surfaces`）の面に `"polarizer": {"axis_deg", "extinction_ratio", "transmittance"}`（理想の直線偏光子。
  透過軸の強度透過率と消光比、省略は 1 と無限）か `"waveplate": {"retardance", "wavelength_um", "axis_deg", "order", "transmittance"}`
  （速い軸 axis_deg、設計波長での遅延 retardance 波長、多次は整数の order。遅延は波長ごとに (order + retardance)·wavelength_um/λ）を書く。
  平面の Jones 面になり、偏光の追跡は波長ごとの行列 R(−θ)·diag(tx·e^{+iφ/2}, ty·e^{−iφ/2})·R(θ)（遅い軸が位相 −。OpticStudio の伝播の位相 e^{−i2πnd/λ}・厚さのある結晶板と同じ向き）を使う。
  .zmx には設計波長の行列の JONESMAT で書く（OpticStudio の JONESMAT は波長によらない）。
- 複屈折材の波長板: `waveplate` に `"material": "QUARTZ"`（常光はガラス QUARTZ、異常光は QUARTZ-E。OpticStudio の
  複屈折材の名前の約束）と `"thickness_mm"` を書くと、厚さのある結晶板（平行平板）になる。厚さを省くと設計波長の遅延
  order + retardance から決める（d = (order + retardance)·λ0/|Δn(λ0)|、Δn = ne − no、カタログ温度の屈折率）。
  `"plates": [{"material", "thickness_mm", "axis_deg"}, …]` は板の組（光の通る順、`axis_deg` は波長板の速い軸からの各板の
  速い軸の向き）で、アクロマティック波長板は速い軸を直交させた 2 材（MgF₂ + 水晶など）で書く。板は面の並びの
  入口（`birefringent` の `in`、後ろは常光の材料、厚さは板の厚さ、結晶軸は面内で遅い軸（ne > no の材料）か速い軸の向き）と
  出口（`out`、後ろは次の板の材料で厚さ 0 か、空気で元の面の `t`）になり、系は板の厚さだけ長くなる。光線は常光の屈折率で進み、
  出口で異常光の成分（結晶軸の光線に垂直な向き）に exp(−i·2π·(n(θ) − no)·L/λ) の位相を付ける
  （n(θ) = 1/√(cos²θ/no² + sin²θ/ne²)、θ は板の中の光線と結晶軸の角、L は板の中の長さ。斜めに入る光の遅延の変化も入る）。
  OpticStudio の Birefringent In（Mode 2、波長板モード）/ Out と同じで、.zmx にもその面で書く（BIRE__IN・BIRE_OUT）。
  位相の向きは OpticStudio と同じ（遅い軸が e^{−iφ}）で、理想の波長板（`material` なし、Jones 面）とも同じ。外側の 2 面の膜は波長板の面の `coat`、無ければ透過率 `transmittance`
  からの理想膜（1 面あたり √T、T = 1 は `I.99999999`）。貼り合わせの面は無膜（Fresnel）。境界の Fresnel は常光・異常光で分けて求める: 板の中の電場を常光の成分（光線と結晶軸を含む面に垂直）と異常光の成分に分け、常光は no、異常光は n(θ) を板の側の屈折率にした係数をかける（入口・出口とも、斜めの光線・膜ファイルの層の膜も。理想膜・実測表・膜のモデルは屈折率によらないので同じ係数）。垂直入射の片面の透過は 2√no/(1+no)・2√ne/(1+ne)。板どうしの貼り合わせ（出口の面の厚さ 0 で、すぐ後に同じ形・膜なしの次の板の入口）は境界 1 回として、前の板の常光・異常光の成分を次の板の常光・異常光の基底へ分け、前の板の屈折率（no₁ か n₁(θ)）から次の板の屈折率（no₂ か n₂(θ)）への係数をかける（垂直入射で 2√(n₁n₂)/(n₁+n₂)）。光線の道は常光のまま（walk-off は扱わない）。無膜の板では OpticStudio と最大 5e-4 違う（OS は面に垂直な光線で、実験室の x・y のうち結晶軸に近い方に ne の透過を入口の面だけで使い、結晶軸の向きと合わない。物理的に正しい式を採る。膜のある板は一致）。テスト用の水晶・MgF₂ は
  `tests/glass/LBCRYSTAL.AGF`（refractiveindex.info、CC0）。GUI の面の並びの波長板にも複屈折材・厚さ（2 材まで）の欄がある。
- 複屈折材の板を面で直接書く: 面の並びに `{"glass": "QUARTZ", "t": 1, "birefringent": {"in": {"axis": [x, y, z]}}}`
  （結晶軸は面の局所座標、`"mode": 0` で常光の成分だけ）と `{"t": 5, "birefringent": "out"}`。入口と出口の間にほかの面は置けない。
  .zmx の BIRE__IN（Mode 0・2）・BIRE_OUT も読める（Mode 1・3 の異常光の道と Gradient Mode は未対応）。
- フィルタ・ダイクロイックの膜: プロジェクトかユーザーカタログの `"coatings": {名前: 膜}` に書いた膜を、面・BS の `coat`
  に名前で使う（TABLE 膜として偏光の追跡・透過に効く。プロジェクトの同じ名前がカタログより優先、膜ファイルの同じ名前より優先）。
  膜は `{"type": "table", "rows": [[入射角 deg, 波長 µm, Rs, Rp, Ts, Tp], …]}`（表そのまま）、`curve`（`wavelengths_nm`・`t`・`r`、
  角度・偏光によらない。色ガラス・吸収型 ND を 1 面で表すときなど）、仕様からの理想の形の `longpass`・`shortpass`（`edge_nm` = 透過 50 %）・
  `bandpass`・`notch`（`center_nm`・`fwhm_nm`）・`nd`（`od`）。理想の形は帯域内の透過 `t`（0.95）、帯域外の OD `od`（4）、
  エッジの 10〜90 % の幅 `width_nm`、斜め入射のエッジのずれ λ(θ) = λ0·√(1 − (sinθ/neff)²) の `neff`（2）、反射は 1 − T
  （`absorptive` で 0）。ダイクロイック BS は BS の `coat` にロングパス・ショートパスを書くと、波長で T・R の経路の強度が分かれる。
  色ガラス・吸収型 ND は、内部透過率（AGF の IT 行）のあるガラスでも表せる（厚さに応じた吸収）。
  理想の形の表は入射角 0〜75°、`curve`・`nd` は 0〜89° で作る。表の角度の範囲より大きい入射角の光線は、表の端の値を使わず
  誤り（「入射角 … が表の範囲の外」）で止まる（`table` は書いた行の角度の範囲）。
  .zmx に書き出すと、これらの膜は表（TABLE）にして `.zmx` の隣の `名前_coat.dat`（COATING.DAT 形式。系の膜ファイルがあれば
  その中身も入れる）に書き、COFN はそのファイルを指す。lensbench は `.zmx` のフォルダから読む。OpticStudio で開くときは
  このファイルを OpticStudio の Coatings フォルダに写す。OpticStudio の TABLE は全部の角度で同じ波長の並びでないと、
  エラーを出さずに表全体を R = T = 0 として読むので、書き出しでは全角度の波長の和集合に並べ直す（lensbench では同じ値）。
  理想の形の表は点が多く（1 膜で 1,500〜3,600 波長 × 13 角度）、膜ファイルが大きいと OpticStudio の読み込みに数十秒かかる。
- 面の傾き: 面に `"tilt": [tx, ty]`（deg）で、その面だけを頂点のまわりに傾ける（後の面の座標は元の軸に戻す。OpticStudio の面の Tilt/Decenter の後を Reverse）。くさびのウィンドウは 2 面目に `tilt`（くさび角 3′ なら 0.05）。
- 機構部品: ユーザーカタログの品に `kind`（tube・cage_plate・mount・rod・adapter）と `mech`
  （`bores`: 光の通る穴 `{"z0", "z1", "diameter"}` の並び、z は部品の基準面から光の進む向き。`length`・`outer_diameter`・
  `thread`・`rod_spacing`・`holds` `{"diameter", "seat_z"}`）を書く（`blocks` は省略できる）。プロジェクトの `mechanics` に
  `{"name", "catalog", "item", "at": {"block": id, "surface": i}, "offset": [dx, dy, dz], "tilt": [tx, ty]}`（`catalog`・`item` の
  代わりに `mech` をその場で書いても、`at` に系の面の番号 `{"surface": n}` だけでもよい）で、ブロックの面の局所座標に置く。
  光学系の位置は変えず、光線の追跡にも効かない。`lensbench mech-check` は瞳の格子の光線を面 1 から追い、面と面の間の直線が
  穴（円柱）の z の範囲で半径の外に出たら切れたとして、部品ごと・視野と波長ごとに切れた割合（`clipped_pct`、ほかで先に切れた
  光線も数える）と最初に切った割合（`first_pct`）、系の開口で切れた割合、通った割合を出す。例は `examples/projects/P19_tube_cage.json`。
  Python は `System.mech_check()`（同じ dict）。
- 枠付きの品: ユーザーカタログの品に `frame`（`mech` と同じ形。z は枠の端から光の進む向き、`holds.seat_z` はガラスの前面の位置）を
  書くと、`blocks`（ガラスだけ）をプロジェクトに `user` のブロックで置いたとき、枠がガラスの最初の面に機構部品（名前は「ブロック id の枠」、
  保持する素子はその品の面）として付き、`mech-check` のケラレ・当たりと描画に入る。品ページの STEP を測ると（GUI の補う窓）、
  組立の置き方どおりに置いた立体のうち、光軸と同じ軸の穴（内壁の円柱）を持つものを枠、持たないものをガラスとして分ける。
  ガラスと枠が 1 つの立体なら、光軸をふさぐ面（縁が 1 つの円だけの面）の間をガラスとする（ガラスの径は見えている径）。仕様表の値と 0.05 mm より違う寸法（有効径は枠の最小の穴より大きい時）は `notes` に書く（serve の `catalog_fill` の `spec`）。
  枠の向きは品の `frame_reversed`（false: 光は枠の z の小さい端から入る、true: 大きい端から）。STEP から測った枠は STEP の
  座標の向きなので、図面の矢印などで決める
- 機構の配置の確認と描画: `mechanics` の項目に `"holds": [ブロック id]` で、部品が保持する素子を書ける。`mech-check` の
  `fit` は、素子の面の縁が部品の内壁（穴の半径と保持径の半分の大きい方）に当たる `collision`、保持する素子が部品の z の範囲から
  はみ出す `protrude`、保持径より素子が 0.5 mm 以上細い `loose`（注意）、部品どうしの壁が重なる `overlap`、同じ軸のケージ板の組の
  ロッドの要る長さ `rod`（注意）とロッドの間隔の違い `rod_mismatch` を出す。`layout` の 2D（SVG・DXF の MECH レイヤー・PNG）・
  3D（glTF・OBJ・STL）・STEP は部品を軸のまわりの輪（内壁〜外径。穴のない区間は詰まった円柱）で描く。
  GUI では、カタログの機構部品の行を足すと組合せの「機構部品」に入り（選んでいる要素に置く）、置く要素・面・ずれ・保持する要素を
  変えられる。レイアウト図・3D 表示に部品を描き、ケラレ・当たりは解析の「機構のケラレ」で見る。
- 機構部品を素子に合わせて置く: `mechanics` の項目に `"seat": true` と `"holds": [ブロック id]` で、部品の座（寸法の
  `holds.seat_z`）を保持する最初の素子の前面に合わせて置く（`at` は要らない。`offset` はそこからのずれ）。GUI では機構部品の欄の
  「座に合わせる」。素子が入る機構部品は `lensbench catalog-search <カタログ> --fits <径>`（保持径が径以上で 0.25 mm より
  太くないもの）で探せ、GUI のカタログの詳細にも「入る機構部品」を出す
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
  （ユーザーカタログ）に普通の品（鏡は平面鏡、ほかは `specs.material` の平行平板。分からない基板は N-BK7 で代用。四角い品は矩形の開口）として保存し、
  次からは未完の品の代わりに並ぶ（元のカタログは書き換えない）。serve は `catalog_fill`。品ページは選んだ品の 1 件だけを開き、
  まとめて取りに行くことはしない。

```json
"states": [
  {"name": "near", "project": {"object_distance": 500}, "set": {"front": {"shift": -0.4}}},
  {"name": "no L3", "set": {"L3": {"on": false, "off": "air"}}}
]
```

`coatings` は全コンフィグの境界（`膜|前の媒質|後の媒質`）と膜の扱い（膜なし、理想膜、膜ファイルの薄膜・表、実測表、モデル）を一覧にし、
偏光追跡に実測表かモデルが要る境界（暗号化された膜など）を `needs_table` に挙げる。

膜ファイル（COATING.DAT 形式）の多層膜は、透過・鏡とも OpticStudio と同じ約束で計算する（層は膜の外側から順、
後ろの媒質が空気なら基板は前の媒質、鏡の基板は最後の層の材料、係数に掛かる位相。`lensbench_core::coating` の
`Coatings::prepare` に説明）。膜厚が波長単位の層は系の主波長で厚さを決める。
