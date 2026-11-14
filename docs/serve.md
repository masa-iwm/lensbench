# 別アプリとの解析ループ（serve・watch）

「lensbench で解析 → 別アプリで系を更新 → lensbench で再解析」のループを、毎回の起動・読み込み・前処理なしで回す。

| 方法 | 使いどころ |
| --- | --- |
| `lensbench serve` | 系を読み込んだまま待ち受け、JSON-RPC で値の変更・解析を繰り返す。どの言語からでも呼べる |
| `lensbench watch` | 系のファイルの更新を見張り、変わるたびに解析して結果のファイルを書く。ファイルしか扱えないアプリ向け |
| Python バインディング（`import lensbench`） | 同じプロセスの中で一番速く呼ぶ（[Python から使う](python-guide.md)） |

```sh
lensbench --threads 1 serve system.zmx --glass-dir catalogs      # 127.0.0.1:7010 で JSON-RPC（配列はバイナリ）
lensbench watch -- spot system.zmx --json --save out.json       # 系のファイルが変わるたびに再解析して書く
```

## serve

```sh
lensbench --threads 1 serve system.zmx --glass-dir catalogs            # 127.0.0.1:7010 で待つ
lensbench serve --port 0 --port-file port.txt                          # 空いているポート（最初の行と port.txt に出す）
lensbench serve --pipe lb1 system.zmx                                  # TCP の代わりにパイプ（下の「パイプ」）
```

- 起動すると最初の行に `{"listening": "127.0.0.1:7010", "port": 7010, "version": "..."}` を出す（`--pipe` なら `{"listening", "pipe", "version"}`）
- 既定は 127.0.0.1 だけで待ち受ける。認証はないので、ほかのアドレスは `--allow-remote` を付けたときだけ
- 要求で与えたパス（`load` の file・glass_dir・lens_catalog、`write` の path、`analyze` の save・out・out_dir・coatings）の扱い:
  - 既定（localhost だけ）: そのまま読み書きする（同じ利用者からの要求なので）
  - `--root DIR`: DIR の下だけ（相対パスは DIR から。シンボリックリンクは先の実体で確かめる）。外は `--root の外` のエラー
  - `--allow-remote` で `--root` なし: 要求ではファイルを読み書きしない。起動時に与えた系と、その読み方での `reload`・ファイルを書かない解析だけ
  - プロジェクトファイル（.json）が中で参照するファイル（.zmx のブロックなど）は検査しない。遠隔から使うときは、信頼できるファイルだけを DIR に置く
- 1 つの要求の配列の合計は `--max-request-bytes`（既定 2 GiB）まで。形の積があふれるもの・上限を超えるものは確保の前に断り、接続を閉じる
- 系は 1 プロセスに 1 つ。要求は 1 つずつ順に処理する（複数の接続も順番待ち）。別の系を並べて回すときはプロセスを分ける。
  `--threads 1` で 1 プロセス 1 スレッド（結果はスレッド数によらない）
- `--cfg`・`--state`・`--path`・`--lens-catalog` は CLI と同じ（系の読み方。`load` でも選べる）

### 書式

1 行 1 要求の JSON-RPC 2.0（改行で終わる JSON）。応答も 1 行の JSON で、`elapsed_ms`（サーバーの中の処理時間）が付く。

```json
{"jsonrpc": "2.0", "id": 1, "method": "set", "params": {"values": {"3.t": 10.5}}}
{"jsonrpc": "2.0", "id": 1, "result": {"changed": ["3.t"]}, "elapsed_ms": 0.02}
```

配列はバイナリで渡す。ヘッダの `"bin"` に `{"name", "dtype", "shape"}` を並べ、改行のすぐ後にその順で生のバイト列を続ける
（リトルエンディアン、C 順。dtype は `<f8`・`<i4`・`|u1`・`|b1`（真偽）・`<c16`（複素数、実部・虚部の順））。要求・応答とも同じ形で、`bin` のないメッセージは JSON の 1 行だけ。
配列は共有メモリにも置ける（下の「共有メモリ」）。

```
{"jsonrpc":"2.0","id":2,"method":"trace","params":{"field":2},"bin":[{"name":"px","dtype":"<f8","shape":[1000]},{"name":"py","dtype":"<f8","shape":[1000]}]}\n
<px の 8000 バイト><py の 8000 バイト>
```

エラーは `{"error": {"code", "message"}}`（-32700 JSON が読めない、-32601 method がない、-32602 引数の誤り、-32000 解析の失敗）。
計算の誤りには `data.kind`（`invalid` 値・構成が不正、`not_implemented` 対応していない機能、`io` 読込、`cancelled` 中断した）が付く（Python のクライアントは `RpcError.kind`）。

### method

| method | params | result |
| --- | --- | --- |
| `ping` | | `version` |
| `progress` | | 処理している要求を待たずに答える。`running`（要求を処理しているか）・`optimize`（反復ごとの評価関数の値）・`tolerance`・`swap`（(終えた数, 全体の数)）。走っていなければ null |
| `cancel` | | 処理している要求に途中でやめるよう頼む（待たずに答える。頼んだ要求があれば true）。解析は「中断した」の誤りで返る。最適化・公差・差し替え探索は止まらないことがあるので `stop` で止める |
| `stop` | `what`（`optimize`・`tolerance`・`swap`） | 最適化・公差・差し替え探索を止め、そこまでの結果を返させる（待たずに答える） |
| `load` | `file`、`cfg`・`state`・`path`・`glass_dir`・`lens_catalog`・`history`（省略可） | 系の概要（面の数・視野・波長など）。`history` は元に戻す段の扱い: `clear`（既定、段を空に）・`push`（前の系を 1 段積み、開くを `undo` で戻せる）・`keep`（段はそのまま）。`push`・`keep` は結果に `undo`・`redo`・`undo_list`・`redo_list` |
| `reload` | `history`（`load` と同じ） | 同じ読み方でファイルを読み直す（`set` で変えた値は捨てる）。別アプリが系のファイルを書き換えた後に |
| `system` | | 系の概要 |
| `undo`・`redo` | `steps`（まとめて戻す段の数、省略で 1） | 読み込んだ後の `set`・`edit`（最適化の結果を当てたものも）を 1 段ずつ元に戻す・やり直す（100 段まで、同じパラメータの `set` が 1 秒以内に続けば 1 段、戻した後に変えるとやり直す段は消える、`load`・`reload` で空。`history: "push"` で読んだときは読む前の系も 1 段）。`surfaces`・`system_hash`・残りの段の数 `undo`・`redo`。開く・読み直すを越えて戻した・やり直したときは `loaded`（その系の概要、`load` の結果と同じ）。`set`・`edit` の結果にも `undo`・`redo` |
| `history` | | 元に戻せる段・やり直せる段の数 `undo`・`redo` と、その変更 `undo_list`（新しい順）・`redo_list`（次にやり直す順）。set は `{"set": [名前]}`、開く・読み直すは `{"open": ファイル名}`・`{"reload": ファイル名}`、edit は `{"edit": [{op, surface, param, fields（項目の名前）…}]}`。`set`・`edit`・`undo`・`redo` の結果にも |
| `get` | `param`（1 つ）か `params`（並び） | `value` か `values` |
| `set` | `values`: `{"3.t": 10.5, "G1.shift": 0.2}` か `[["3.t", 10.5], ...]` | `changed`（まとめて変える） |
| `write` | `path` | 今の系を .zmx に書く（別アプリに渡す）。`written`・`notes`（.zmx に書けず落とした・変えたものの注意） |
| `save` | `path` | 今の系（`set`・`edit` の後）を系ファイル（.json）に書く。`written`・`system_hash` |
| `save_range` | `path`・`from`・`to` | 今の系の面 from〜to と物体面・像面だけを系ファイルに書く（範囲の外を参照するピックアップ・ソルブは外れ、絞りが外なら from を絞りに）。`written`・`surfaces` |
| `edit` | `ops`: 操作の並び（下） | 系を編集する。`notes`（外したピックアップなど）・`surfaces`（面の数）・`system_hash`。どれかが失敗すれば系は変えない |
| `surface` | `surface`（番号か名前、省くと全部） | 面の値（`fields`、全部なら `surfaces` の並びと、玉・群の面の範囲 `units`（name・spans・auto、偏心・傾き `motion`（dx dy tx ty tz）・移動 `shift`・媒質の `dn`・`disp`（面ごとに違えば null））） |
| `surface_data` | `surface`（番号か名前） | 面のデータを全部の項目で（`data`。系ファイルの面と同じ形、屈折率 n は除く）と、その面のソルブ `solves`・ピックアップ `pickups` |
| `system_data` | | 系全体の設定: `data` に name・wavelengths（µm）・weights・primary（0 始まり）・fields（x y w vdx vdy vcx vcy van）・field_type・field_norm・aperture・telecentric・aiming・afocal・afocal_unit・floating_clip（浮動開口でけるか、既定 true）・temperature・pressure・adjust_index・glrs・ropd・coating_file。系ファイルと同じ形 |
| `config_table` | | 多重構成の表: 構成の数 `n`・今の構成 `active`・行 `rows`（`param`・構成ごとの `values`・構成のピックアップ `picks`（`k`・`from`・`scale`・`offset`））。.zmx なら、ファイルの多重構成の数 `zmx_cfgs` と読んだ構成 `zmx_cfg` |
| `config_diff` | | .zmx の多重構成を表にする `edit` の操作 `ops`（各構成を読み、構成で違う曲率・コーニック・厚さ・非球面・傾き偏心・dn・disp を行に）と、表にできない違い（硝材・無視・半径・絞り・視野・波長）の `notes`、行の数 `rows`。系は変えない |
| `analyze` | `cmd` と CLI と同じ引数 | CLI の `--json` と同じ結果。干渉（`fringe`）は下の「干渉の解析」。`coatings` は読み込んだ系（今の構成・状態、.zmx でなくても）の膜の境界で、引数は `table`（実測表・モデルを足す）だけ: `coating_file_path`・`wavelengths_um`・`interfaces`（`surface`・`key`・`kind`・`note`）・`needs_table`（実測表が要る境界の KEY）・`auto_coatings` |
| `analysis_args` | `cmd` | 解析のサブコマンドの引数の一覧（GUI の設定欄）: `args`（`name`・`type`・`default`・`unit`・`doc`、選択肢は `choices`、旗を繰り返すものは `repeat`）。`type` は `num`・`count`（0 以上の整数）・`int`・`flag`・`str`・`choice`・`nums`（数のカンマ区切り、JSON では並び）・`path`。系の読み方の引数は除く |
| `plugins` | | 登録された解析・検出器のプラグインの一覧: `plugins`（`name`・`kind`・`doc`・`needs_system`・`config`・`params`・`inputs`・`outputs`）。`params` は設定の項目の型・既定（`analysis_args` の `args` と同じ形、個数の決まった並びは `len`）。CLI の `plugins --json` と同じ |
| `catalog_add` | `catalog`（ユーザーカタログのファイル）、`name`、`blocks`（ブロックの id の並び: 1 つか、最上位の並びの範囲の最初と最後）、`description`・`replace`・`vendor`（省略可） | 読み込んだプロジェクト（状態を当てて）のブロック・範囲（間の空気も）を項目に保存する（CLI の `catalog-add --block`）。面の表の変更（`surface_edits`）は玉の面に焼き込む。ファイルが無ければ作る。結果は `catalog_items` と同じと `added` |
| `catalog_items` | `catalog` | `vendor`・`items`（`name`・`description`・`glass_catalogs`・`blocks`（ブロックの数）） |
| `catalog_edit` | `catalog`、`name`、`rename`・`description`・`delete: true`・`vendor`（どれか） | 項目の名前の変更・説明の変更・削除。結果は `catalog_items` と同じ |
| `catalog_fill` | `catalog`（未完の品のあるカタログ）、`item`、`out`（補足のカタログのファイル。無ければ作る）、`thickness`・`diameter`（mm。径は数か [幅, 高さ]。足りない値の分）、`from`（`page`・`step`・`manual`）、`frame`（省略可。枠付きの品の枠。ユーザーカタログの `mech` と同じ形）・`frame_reversed`（光は枠の z の大きい端から入る）、`spec`（省略可。仕様表の値 `thickness`・`diameter`・`mount_thickness`・`mount_diameter`・`clear_aperture`）・`glass_visible`（STEP のガラスと枠が 1 つの立体で、ガラスの径が見えている径） | 未完の品（`missing` のある項目）に値を足して `out` に同じ型番の普通の品として書く。`frame` は品の `frame` に入れる。`spec` があれば、ガラス・枠の寸法と仕様表の値を比べ、0.05 mm より違う項目（有効径は枠の最小の穴より大きい時）を `notes` に書き、品に有効径が無ければ `spec` の有効径を入れる。`out` は 1 つのメーカーの品だけ。結果は `catalog`・`item`・`vendor`・`specs`・`notes`・`frame`・`frame_reversed` |
| `cad_step` | `catalog`（ユーザーカタログ）、`item`、`cache`（省略可。キャッシュのフォルダ。省略で CLI の cad-step と同じ既定の場所。`--root` のときはその中だけ） | 品の STEP を 1 件だけ取ってキャッシュに置く（キャッシュ → 共有の置き場 → `cad.step` の配布元の順。置き場は docs/project-file.md）。結果は `item`・`url`・`path`（キャッシュのファイル）・`cached`・`source`（`cache`・`store`（共有の置き場から）・`vendor`・`vendor_uploaded`（配布元から取り置き場にも上げた））・`store_notes`（置き場を読めない・上げられない理由）・`measured`（catalog-step と同じ測った値）・`notes`（簡易外形の長さ・外径・外形の形との食い違い。空なら合う）。取れなければ誤り（品ページの URL を添える）。Web 版では使えない |
| `plugin` | `name`、`config`（数・文字列・数の並び）、`inputs`（JSON の数の並び、入れ子で多次元）か bin | プラグインを 1 回呼ぶ。系の要るもの（spot・mtf など）には読み込んだ系（`set`・`edit` の後）と膜を渡す。出力は bin（`<f8`・`<c16`・`|b1`、形つき）、数 1 つの出力は `values` にも入れる |
| `trace` | `px`・`py`（bin か JSON の並び）、`field`（1 始まり）か `hx`・`hy`、`wave`、`last`、`global` | `rays`・`ok`・`hx`・`hy`・`wave`・`last`、bin に `x y z l m n opl`（<f8）・`err vig`（<i4） |
| `layout` | `field`・`wave`（1 始まりの並び、省略は全視野・主波長）、`plane`（見る面 `yz`・`xz`・`3d`、既定 `yz`）、`unfold`（true で光路に沿って展開、2D だけ）、`fan`（`y`・`x`・`cross`・`grid`・`ring`、既定は見る面の向き、3d は `ring`）、`rays`（既定 7、ring は 12）、`first`・`last`、`color`（光線の色分け `field`・`wave`・`cfg`。与えると結果に `color`） | 見る面の断面（[z, y] の点、`xz` なら [z, x]）: `plane`、`solids`（玉・鏡、`surfaces`）、`plates`（像面・物体面）、`rays`（`field`・`wave`・`px`・`py`・`vig`・`err`・`pts`）、`surfaces`（`vertex`・`stop`・`mirror`・`semi`・`t`）、`units`（玉・群の名前と面の範囲 `spans`）、`configs`（重ねた構成の名前）。GUI のレイアウト図。`cfgs`（.zmx の多重構成の番号）・`states`（状態の名前）・`paths`（BS の経路）を並びか `"all"` で与えると、その構成をファイルから読み直して重ねる（CLI の `layout --cfgs/--states/--paths` と同じ。`set` の変更は入らない）。`offset` [dx, dy, dz] で構成 k を k 倍ずらす。光線の `cfg` は構成の番号（0 始まり）。`unfold` は鏡・BS の反射ごとに後ろを鏡の頂点の接平面で鏡映し、折れる前の光軸の延長に並べ直す（`surfaces` の `vertex` も展開した位置、結果に `unfold: true`。座標ブレークの点は光線から除く）。`plane: "3d"` は断面の代わりに `meshes`（立体・面の網: `kind` は `glass`・`mirror`・`image`・`object`・`paraxial`、`surfaces` [前, 後]、`pos` は [x, y, z] を平らに並べた mm、`tris` は 3 つずつの番号）と `rays`（`pts` は [x, y, z]）、`surfaces`（`vertex` [x, y, z]・`stop`）・`units`・`configs`。`mech_detail: true` で、3D の機構部品を取った品の STEP の詳しい形で描く（キャッシュに STEP の無い部品は簡易外形） |
| `layout_export` | `file`（書き出すファイル。形式は拡張子で `svg`・`png`・`dxf`・`gltf`・`glb`・`obj`・`stl`・`step`・`stp`）と `layout` と同じ引数（構成の重ね描き・`offset` も）。2D の形式は `plane` の面（`3d` なら `yz`）、3D の形式は 3D の場面（`unfold` は無視）。`color`（既定は重ね描きなら `cfg`、ほかは `field`）・`width`（PNG の横幅 px、既定 1600）・`segments`（周方向・面の線の分割数、既定 64）・`rings`（3D の半径方向の分割数、既定 16）・`tube`（3D の光線の管の半径 mm）・`mech_detail`（true で機構部品を取った品の STEP の形で書く。3D の形式は三角形、STEP は元の立体を写す）。CLI の `layout -o` と同じ書き出し | `written`（パス）・`format`・`solids`（立体の数）・`rays`（光線の数）。GUI のレイアウトの書き出し |
| `optimize` | `vars`（`"7.t"` か `{"name", "lo", "hi", "configs"}` の並び）、`merit`（`spot`・`wavefront`・`none`）・`rings`・`reference`、`terms`（CLI の `--term` と同じ JSON の項の並び）、`configs`（.zmx の構成番号か状態の名前の並び。省けば読み込んだ系だけ）、`iters`・`jac`（`auto`・`fd`・`forward`）、`apply` | 減衰最小二乗。`names`・`x0`・`x`・`merit0`・`merit`・`history`・`iterations`・`stop`、`terms`（項ごとの `value0`・`value`、既定の評価関数の項は `default: true`）、`applicable`（読み込んだ系の構成に効く変数）。`apply: true` なら効く変数を読み込んだ系に `set` する（`applied`）。他の構成はファイルから読み、読み込んだ後の変更を当てる（`notes`）。GUI の最適化パネル |
| `tolerance` | `lensbench tolerance --spec` と同じ JSON（`trials`・`seed`・`dist`・`preset`・`catalog`・`lenses`・`groups`・`tols`・`compensators`・`merit`・`terms` など、`lensbench_design::tolerance`）に加えて `configs`（optimize と同じ） | モンテカルロ公差。`nominal`・`merit`（統計）・`cdf`（評価関数の昇順）・`terms`（項ごとの統計と `cdf`）・`compensators`（試行ごとの `values` も）・`tilt_magnitudes`（tx・ty の組の大きさ [分]）・`merit_values`・`sensitivity`（公差ごとの lo・hi の評価関数の変化、悪い順）・`rss_estimate`・`correlation`・`failed`・`errors`。読み込んだ系（set・edit の後）を使う。進み具合は `tolerance_progress()`、中止は `tolerance_cancel()` |
| `zoom` | `configs`（状態の名前か .zmx の構成番号の並び、省けばすべて）・`vars`（`"G2.shift"`・`"G2.shift=-5:20"` か `{name, lo, hi}`）・`efl` か `pmag`（状態ごとの目標の並び、`null` は目標なし）・`steps`・`merit`・`reference`・`rings`・`iters`・`tol` | ズーム軌跡（CLI の `zoom --json` と同じ: `quantity`・`vars`・`path`・`warnings`）と `notes`。読み込んだ後の値の変更は各状態の系にも当てる |
| `swap` | `lensbench swap --spec` と同じ設定（`slots`・`vars`・`merit`・`reference`・`rings`・`iters`・`efl`・`state`・`max_count`）、`project`（プロジェクトの JSON。省けば読み込んだファイル。`--allow-remote`・`--root` では使えない）、`top`（50、0 はすべて）・`threads`・`count_only` | カタログレンズの差し替え探索。`count`・`done`・`failed`・`cancelled`・`seconds`・`slots`（所ごとの候補）・`results`（評価関数の小さい順、CLI と同じ行）。`count_only` なら `count`・`slots` だけ。読み込んだ系のガラス・レンズカタログのフォルダを使う。GUI の差し替え探索パネル |
| `configs` | | 重ねて描ける構成: `cfgs`（.zmx の構成の数）、`states`（プロジェクトの状態の名前）、`paths`（今の状態で BS のとれる経路） |
| `echo` | 任意の bin | 同じ bin を返す（通信の時間を測る） |
| `glasses` | `glass_dir`（並び、省略可。LENSBENCH_GLASSCAT も使う） | `catalogs`（名前・パス・数）、`glasses`（`name`・`catalog`・`nd`・`vd`・`formula`・`range_um`）。nd・vd はカタログ温度の分散式から |
| `lenses` | `lens_catalog`（.zmx のファイル・フォルダの並び）、`glass_dir` | `lenses`（`vendor`・`part`・`efl`・`diameter`・`thickness`・`elements`・`pieces`（ガラスの枚数、貼り合わせは 2 枚以上）・`surfaces`・`glasses`・`mirror`・`curv`（最初の玉の前・後ろの面の曲率 [1/mm]）、読めなければ `error`）。efl はカタログの値、無ければ主波長の近軸の値。玉の無い鏡は `mirror: true` と鏡の面の `radius`・`conic`（efl は −radius/2）。`kind` は `lens`・`mirror`・`splitter`。フォルダの中の lensbench 形式のカタログ（ユーザーカタログの .json）の項目も並べ、`source`（`catalog` の絶対パス・`item`）、`description`、鏡・BS の膜の `coat`、BS の中身の `splitter`（`form`・`size`・`glass`・`ratio`・`ar` など。経路・向きは除く）を付ける。`category`（品の種類: lens・mirror・splitter・polarizer・waveplate・window・filter・prism・other）、.json の品の `url`・`specs`・`tags`・`spec_source`・`checked`、同じメーカーの .json の `overlay`・`url_template` で足した値も付ける。面の並びだけの品は `kind: "surfaces"`（`jones` は Jones 面があるか）。未完の品（`missing` のある項目）は `kind: "incomplete"`・`incomplete: true`・`missing`（分かっている `diameter`・`thickness` は仕様から）で、同じメーカー・型番の普通の品があれば並べない |
| `catalog_search` | `lenses` と同じに加えて `kinds`（品の種類の並び）・`vendor`・`text`・`specs`（`名前>=値` などの並び、CLI `catalog-search --spec` と同じ書き方）・`fits`（径 [mm]。その素子が入る機構部品だけ、CLI `--fits`） | `lenses` と同じ形で、条件に合う品だけ |
| `shutdown` | | サーバーを止める |

- パラメータの名前は Python の `get_param`・`set_param` と同じ（`3.t`・`STOP.r`・`lens2.tx`・`G1.shift` など）
- `analyze` で動くのは、解析の登録簿の解析（info・spot・wavefront・zernike・ghost・mtf・mtf-focus・mtf-field・geo-mtf・huygens-mtf・rms・psf・
  rays・pop・mech-check・elements・fringe と、`ext::register` で登録した解析）と `coatings` だけ。
  最適化・公差・差し替え・ズーム・レイアウトの書き出しは専用のメソッド（`optimize`・`tolerance`・`swap`・`zoom`・`layout_export`）で。
  設定の JSON をそのまま渡す。鍵は CLI の旗の `-` を `_` にしたもの（`-` のままでもよい）、値は型のまま（数・`true`・数の並びは配列・
  パスの並びは配列）。前の受け方の数の文字列（`"2"`）・カンマ区切りの文字列（`"10,20"`）・1 つのパスも受ける。`json` は読み飛ばす。
  知らない鍵・読めない値は要求の誤り（−32602）。引数の一覧と既定値は `analysis_args`。
  `{"cmd": "mtf", "field": 2, "freqs": [10, 20]}` は `lensbench mtf <系> --field 2 --freqs 10,20` と同じ。
  系の読み方（`cfg`・`state`・`path`・`glass_dir`・`lens_catalog`）は `load` で選ぶ（`analyze` で与えると誤り）。
  値がオブジェクトの設定（センサの設定など）はオブジェクトのまま渡す（CLI では JSON の文字列で渡す）
- `trace` の結果は最後の面（`last`、省くと像面）の局所座標、`global: true` なら大域座標。`err`・`vig` が 0 の光線が通った光線。
  `opl` は幾何的な光路長。コアの追跡とビット一致する
- `edit` の操作（`op` と引数。面は番号か名前）: `insert_surface` at・surface（面の値: type comment t c r k asph cx material coating semi ignore
  aperture cb cb_order focal jones dn disp）、`delete_surface` surface、`set_surface` surface・fields、`set_surface_type` surface・type、
  `set_material` surface・material、`set_stop` surface、`add_pickup` dst・src・what（c r t k p1〜 dn disp semi before0〜4 after0〜4）・scale・offset・from（元の量、省くと what）、`remove_pickup` dst・what、
  `add_solve` surface・kind（marginal_height など、Python の add_solve と同じ名前）・value・other（別の面）・param、`remove_solve` surface・what（t か c）、`set_param` param・value、`set_config_count` n、`set_config_row` param・values、`add_config_row` param（どの構成も今の値）、
  `remove_config_row` param、`insert_config` at（新しい構成の番号 1〜n+1）・from（値を写す構成、省略は今の構成、pickup なら 1）・pickup（true で新しい構成を from のピックアップに。OpticStudio の Insert Configuration with Pickups）、`remove_config` k（構成が 1 つになると表を消す）、`set_config_pickup` param・k・from・scale（既定 1）・offset（既定 0）（構成 k の値 = 構成 from の値 × scale + offset。ピックアップの構成の値を直接変えると外れる）、`remove_config_pickup` param・k、`set_config` k（表の操作でない操作・`set` で表の行の値を変えると、今の構成の列も変わる）、`set_system_data` data（`system_data` の data と同じ形で、書いた項目だけ変える。波長・視野の数も変えられる）、`set_surface_data` surface・data（`surface_data` の data と同じ形で、書いた項目だけ変える。開口 ap・前後の傾き偏心 before・after・面タイプの形のデータ zern grid bin2 cylf grin もここで。stop・mirror・n は変えない）。開口は `{"type": "circular", "max": 3}` か null、inf は `{"$f": "inf"}`。
  例: `{"ops": [{"op": "insert_surface", "at": 3, "surface": {"r": 50, "t": 5, "material": "N-BK7"}}]}`
- `load` は系ファイルも読める（`format` が `lensbench-system` の .json）
- 数は JSON から丸めずに読む（Python の `float` を送れば同じ f64 が届く）
- 像面距離などの解き（ソルブ）のある系では、値を変えてから元に戻すと、解きを前の値から解き直すので 1e-14 程度の差が残ることがある

`glasses`・`lenses`・`catalog_search` は読み込んだ系と関係なく使える（GUI のカタログブラウザ）。

### 干渉の解析（fringe）

`analyze` の `cmd` に `fringe`（単色の干渉縞）を渡すと、CLI と同じ引数・同じ結果を返す。

```json
{"method": "analyze", "params": {"cmd": "fringe", "beam1": "1", "beam2": "2", "mode": "detector"}}
```

- 2 つの光束は `load` の `cfg`・`state` によらず、`beam1`・`beam2`（構成の番号か状態の名前）で選ぶ。
  光束ごとに系のファイルをその構成・状態で読み、読み込んだ後の変更（`set`・`edit`）を順に当てる
- `set` のパラメータで、その光束のファイルの値が読み込んだ系の変える前の値と違うもの（多重構成の表で構成ごとに値が違うもの）は当てず、
  結果の `notes` に書く。例: 構成 1 を読み込み、試料の腕だけの厚さを `set` すると、参照（構成 2）の厚さは変えない
- `reload`・`load` で変更の記録も消える。変更がなければ CLI と同じ（ビット一致）
- `"grid": true`（CLI の `--grid`）で結果に格子 `grid` も入れる（番号 j·nx + i、行 j は +y の向きで j = 0 が下、データの無い画素は null）。GUI の図はこれを使う
- 拡張の解析（`lensbench_engine::register`）は、系の読み方（`Cx::source`）で別に読んだ系にも読み込んだ後の変更が当たる

### パイプ

`--pipe NAME` で TCP の代わりにパイプで待ち受ける。書式・method は TCP と同じ。

- Windows: 名前付きパイプ `\\.\pipe\NAME`（NAME が `\\.\pipe\` で始まればそのまま）。同じ PC からだけつなげる（リモートの接続は断る）。
  セキュリティは既定（起動した利用者・管理者が読み書きできる）。同じ名前のパイプが既にあれば起動しない
- ほかの OS: NAME のパスの Unix ドメインソケット（0600、起動した利用者だけ）。前に止まったサーバーのソケットが残っていれば作り直す（ソケットでないファイルがあれば起動しない）
- 要求で与えたパスは localhost の TCP と同じに扱う（`--root` が効く）

### 共有メモリ

大きい配列は、ストリームで送る代わりに共有メモリに置ける（Windows・Linux）。クライアントが区画を作り、名前を渡す。

```
{"jsonrpc":"2.0","id":3,"method":"trace","params":{"field":2},"shm":{"name":"psm_1a2b","out":16000000},
 "bin":[{"name":"px","dtype":"<f8","shape":[1000000],"offset":0},{"name":"py","dtype":"<f8","shape":[1000000],"offset":8000000}]}\n
```

- `shm.name` は Python の `multiprocessing.shared_memory.SharedMemory` の `name`（Windows は名前付きのファイルマッピング、
  Linux は `/dev/shm/名前` のファイル）。英数字と `_` `.` `-` だけ
- bin に `offset`（区画の先頭からのバイト位置）があればその配列は共有メモリから読み、ストリームには続けない（`offset` のない bin はストリームから）
- 要求に `shm` があれば、応答の配列も区画に書く（`shm.out` のバイト位置から 64 バイトそろえ、既定 0）。応答の bin に `offset` が付く。
  収まらなければ今までどおりストリームで返す（bin に `offset` がない）。次の要求で上書きするので、クライアントは読んだら写す
- 範囲の外・名前の誤りはエラー（-32602）で、接続は続く。`--allow-remote` では使わない

### 同じプロセスから呼ぶ（GUI）

`lensbench_engine::preview_layout(load, layout)` は `load` と同じ params の系を読んで `layout` の結果を返す（構成の重ね描きも同じ。読み込んだ系は変えず、順番も待たない。GUI のレイアウト図の下書き）。

`lensbench_engine::Session::call(method, &params, threads)`（プロセスで共通の場は `lensbench_engine::call`）は TCP の要求と同じ method・params・result を同じプロセスの中で処理する
（系は場に 1 つ、要求は場ごとに 1 つずつ順に。場は `Session::new()` でいくつでも作れる）。GUI（`crates/lensbench-gui`）は共通の場だけで計算する。bin の配列は使えない
（`trace` の px・py は JSON の並びで渡す）。結果の配列は `call_arrays` で受け取る（名前と f64 の並び）。`shutdown` はプロセスを止めない。
`optimize` を回している間は、別のスレッドから `optimize_progress()`（反復ごとの評価関数の値）で進み具合を読み、
`optimize_cancel()` で止められる（`tolerance` も同じく `tolerance_progress()`・`tolerance_cancel()`）（その反復までの結果を返す）。TCP・パイプでは別の接続から `progress`・`stop` を送る。進み具合・中止は場ごと（`Session::optimize_progress` など。上の関数は共通の場のもの）。
`swap` も同じく `swap_progress()`（終えた組と組の数）・`swap_cancel()`（走っている組は終わらせ、そこまでの結果を返す）。

### Python のクライアント

`clients/python/lensbench_client.py`（標準ライブラリだけで動く。numpy があれば配列は numpy で返す）。

```python
from lensbench_client import Client

with Client.spawn("system.zmx", glass_dir=["catalogs"], threads=1) as lb:   # serve を起動してつなぐ（抜けると止める）
    for t in [10.0, 10.5, 11.0]:
        lb.set({"3.t": t})                          # 外で決めた値を入れる
        print(lb.analyze("spot", field=1)["spot"][0]["rms_centroid"])
    r = lb.trace(px, py, field=2)                   # r["x"], r["y"], r["opl"], r["vig"] は numpy 配列
```

`Client("127.0.0.1", 7010)` で動いている serve につなぐ（パイプは `Client(pipe="lb1")`）。
`Client.spawn(..., pipe=True)` でパイプ、`lb.use_shm(128 << 20)` で共有メモリ（numpy が要る、`use_shm(0)` でやめる）。
長い要求（`optimize`・`tolerance`・`swap`）を回している間は、`o = lb.another()` で別の接続を開いて `o.progress()`・`o.stop("tolerance")`・`o.cancel()` を送る（別のスレッドから）。
`lb.fringe(...)` は `analyze("fringe", ...)` と同じ。`lb.plugin("spot", field=np.array(2.0))` でプラグインを呼ぶ
（numpy の float64・complex128・bool はそのまま送る）。例: `examples/loop/serve_loop.py`（間隔を変えて再解析を繰り返し、往復の時間を出す。CI で回す）。

### 往復の時間

`examples/loop/serve_loop.py`・`crates/lensbench-cli/tests/serve.rs` で測る（`cargo test --release -p lensbench-cli --test it serve:: -- --nocapture`）。
クラウドの Linux（2026-11-08、1 スレッド）の例:

| 往復 | 時間 |
| --- | --- |
| ping | 0.02〜0.04 ms |
| set + get + spot（10 面、視野 1 つ） | 0.6 ms |
| trace 10 万本（入 1.6 MB・出 7.2 MB、Rust のクライアント） | 46 ms |
| echo 4 MB（Python、numpy） | 8〜10 ms |
| trace 100 万本（Python、入 16 MB・出 72 MB） | 410 ms（うちサーバー内 290 ms） |

配列の受け渡しは追跡の時間より短い（100 万本で追跡 290 ms に対して通信 120 ms）。

TCP・パイプ・共有メモリの比較（`examples/loop/serve_loop.py` の最後、trace 100 万本、Python、クラウドの Linux 2026-11-08、5 回の最小）:

| 受け渡し | 往復 | うち通信 |
| --- | --- | --- |
| TCP | 262〜290 ms | 70〜93 ms |
| TCP + 共有メモリ | 257〜260 ms | 64〜66 ms |
| パイプ（Unix ドメインソケット） | 276〜285 ms | 88 ms |
| パイプ + 共有メモリ | 253〜255 ms | 62 ms |

Windows（CI の windows-latest、同じ日）: TCP 351 ms（通信 109 ms）、TCP + 共有メモリ 329 ms（81 ms）、名前付きパイプ 352 ms（107 ms）、
パイプ + 共有メモリ 326 ms（81 ms）。

共有メモリで通信は 2〜3 割減るが、往復全体では 1 割弱。通信の残りは、配列とバイト列の変換と、クライアントが結果を写す分。

## watch

```sh
lensbench watch --status status.json -- spot system.zmx --glass-dir catalogs --json --save out.json
```

- `--` の後は CLI と同じ解析のコマンド。最初に 1 回、その後は系のファイル（と `--also` のファイル）が変わるたびに実行する
- 見張りは `--interval`（既定 0.5 s）ごとに更新時刻と大きさを見る。変わった後、変化が止まるまで 1 間隔待ってから読む（書きかけを読まない）
- 結果（`--save`）と `--status` は一時ファイルに書いてから置き換えるので、読む側が書きかけを見ることはない
- 1 回ごとに 1 行の JSON（`run`・`ok`・`error`・`elapsed_ms`・`written`）を標準出力と `--status` に出す。`run` は 1 からの通し番号で、
  読む側は番号が進んだら結果を読めばよい。解析が失敗しても見張りは続ける
- `--max-runs N` で N 回で終える。`--print` で結果の全体も標準出力に出す（`--save` がなければ常に出す）
