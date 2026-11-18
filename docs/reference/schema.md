# JSON スキーマ

lensbench が読み書きする JSON ファイルのスキーマです。ファイルの先頭に `"$schema"` を書くと、エディタの補完・検査が効きます
（[プロジェクトファイル](../project-file.md)）。

| スキーマ | ファイル |
|---|---|
| [project](../schema/project.schema.json) | プロジェクトファイル |
| [system](../schema/system.schema.json) | システムファイル（`lensbench convert` の .json、版 2） |
| [user-catalog](../schema/user-catalog.schema.json) | ユーザーカタログ |
| [swap](../schema/swap.schema.json) | 差し替え探索の `--spec` |
| [terms](../schema/terms.schema.json) | 評価関数の項（`optimize --terms`、1 つか並び） |
| [tolerance](../schema/tolerance.schema.json) | モンテカルロ公差の `--spec` |
| [edit-ops](../schema/edit-ops.schema.json) | システムの編集の操作の並び（serve の `edit` の `ops`。プロジェクトのブロックの `surface_ops` は project の中） |

serde の型から作っているので、型を変えたら `LENSBENCH_UPDATE_SCHEMA=1 cargo test -p lensbench-project -p lensbench-design --test it schema::` で作り直す（swap・terms・tolerance は lensbench-design）。

## システムファイル（版 2）

システムファイルの `system` は、利用者が書いた値（処方）だけを、内部の構造体と切り離して決めた形で書きます。

```json
{"format": "lensbench-system", "format_version": 2, "lensbench": "0.4.0", "hash": "sha256:…", "coatings": [],
 "system": {
  "name": "LB ACH-45",
  "wavelengths": [{"um": 0.4861327}, {"um": 0.5875618}, {"um": 0.6562725}], "primary": 2,
  "aperture": {"type": "epd", "value": 10.0}, "fields": {"points": [[0.0, 0.0]]},
  "surfaces": [
   {"object": true, "t": {"$f": "inf"}},
   {"stop": true, "shape": {"conic": {"c": 0.02702702702702703}}, "semi": 6.35, "t": 5.0, "medium": "N-SK16"},
   {"shape": {"conic": {"c": -0.037037037037037035}}, "semi": 6.35, "t": 2.0, "medium": "F2"},
   {"shape": {"conic": {"c": -0.00909090909090909}}, "semi": 6.35, "t": 57.0},
   {"image": true}],
  "catalogs": [{"name": "SCHOTT", "hash": "sha256:…"}]}}
```

- `surfaces` は 1 行に「面 ＋ その後の間隔」（番号は OpticStudio と同じ）。行は形 `shape`（`conic`・`even_asphere`・`odd_asphere`・
  `toroid`・`zernike_standard`・`zernike_fringe`・`grid_sag`・`tilted`・`irregular`・`cylinder`、種類ごとの値だけ。省くと平面）、
  働き `action`（省くと屈折。`reflect`・`paraxial`・`jones`・`binary2`・`fresnel`・`cylinder_fresnel`・`birefringent_in`・
  `birefringent_out`・`user` など）、`aperture`・`coating`・`stop`・`semi`（数か `"auto"`）・`before`・`after`（傾き・偏心、`"reverse"`）・
  `ignore`・`comment`・厚さ `t`・媒質 `medium`・`dn`・`disp`。座標ブレークは `coordinate_break` の行。既定値の欄は省く。
- 曲率は半径でなく曲率 `c` で書く（往復で値のビットが変わらない）。
- 媒質は省略（空気。ミラー・座標ブレークは前の媒質）・ガラスの名前・モデルガラス `{"model": {"nd": …, "vd": …, "dpgf": …}}`
  （`vd` 0 は分散なし）・屈折率分布 `{"grin": …}`。ガラスの名前は「名前@カタログ」でもよく、そのカタログ（`catalogs` か
  ガラスのフォルダ）のその名前のガラスを引く（`catalogs` の並びで前のカタログにある同じ名前の別のガラスと分ける。
  「名前@カタログ#2」の番号は、同じ名前のカタログが 2 つあって分けたときのもので、引くのはそのカタログ）。
- 書かないもの: 求めた屈折率、自動のクリア半径の値、ガラスの式。ガラスは名前でカタログから引き、`catalogs` に使ったカタログの名前と
  ファイルの SHA-256 を残す。保存では、このプロセスで読んだ AGF・既定のフォルダ（`LENSBENCH_GLASSCAT`）の順にシステムのガラスと
  値が同じファイルを探す。見つからないカタログは `hash` を空にし、注意を出す。
- 読むときは、カタログを渡したフォルダ（`--glass-dir`・`glass_dirs`）・システムファイルのフォルダ・既定のフォルダ・このプロセスで
  読んだ AGF の順に探し、最初に見つかったファイル（今のカタログ）を使う（hash が合うものを選ぶのではない）。hash が保存した
  ときと違えば注意、`hash` が空なら値を確かめられないと注意を出す。どれも値は今のカタログで求める。見つからなければエラー。
- 数の書き方は前と同じ（JSON に書けない数は `{"$f": "inf"}`、実数の欄は整数でもよい、エラーには欄の道が付く）。
- 版 1（内部のシステムをそのまま書いた形）も読める。書くのはいつも版 2。
