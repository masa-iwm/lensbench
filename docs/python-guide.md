# Python から使う

`crates/lensbench-py`（pyo3・maturin）。同じプロセスの中で追跡し、結果を numpy 配列で返す。wheel は CI の成果物にもある。
版と互換（`lb.API_VERSION`）、単位と座標の約束、型スタブ、例題（`examples/python/`）、画面なしの実行
（`lensbench run script.py`）は [Python API の約束](python.md)。

```sh
pip install maturin numpy
maturin build --release -m crates/lensbench-py/Cargo.toml -o dist
pip install --find-links dist lensbench
```

```python
import numpy as np, lensbench as lb
s = lb.System.load("system.zmx", glass_dirs=["catalogs"], cfg=1)   # .json のプロジェクトファイルなら state=
r = s.trace(2, px, py, wavelengths=[1, 2, 3], threads=1)          # 視野は番号（1 始まり）か (hx, hy)
r["x"], r["y"], r["opl"], r["ok"]                                 # 形は (波長, 光線)
j = s.trace(2, px, py, wavelengths=[1], pol=True)["jones"]       # (光線, 2, 2) complex128
```

- 番号は CLI と同じ（視野・波長は 1 始まり、面は OpticStudio の面番号）。結果は CLI（`rays --save`）とビット一致し、スレッド数にもよらない
- `last=`（途中の面で止める）、`record=True`（全面の交点 `rec_*`、(波長, 面, 光線)）、`globals=True`（大域の位置・方向と開始点）
- `pol=`: True で主波長、または波長の番号。列は入射偏光（物体側の局所 x・y）、行は最後の面の局所座標の出射電場の x・y。
  膜の振幅・位相込み、E ∝ exp(−ikL)。`pol_phase=True` で伝搬の位相 exp(−ik·OPL) も掛ける（既定は掛けない）
- 偏光で使う膜の表・モデルは `load(..., coatings=[...])`、膜ファイルの置き換えは `coating_file=`
- `pol_grid=25`（または `(nx, ny)`）: 偏光追跡を正規化瞳座標 [−1, 1]² の粗い格子で行い、伝搬の位相を除いた Jones を 3 次スプライン
  （not-a-knot）で全光線へ補間する。テスト系 37 系（121² の瞳、主波長、最大視野）で全光線の偏光追跡との差は 25² で 3.4e-6 以下
  （高次の Zernike 面の Z3 だけ 1e-3。瞳の上で急に変わる系では格子を細かくするか使わない）。止まった格子点は最も近い有効な点で埋め、
  瞳の内側で欠けた格子点は `pol_grid_missing` に返し、そこから 2 点の範囲のセルの光線は NaN（`pol_grid_bad`）
- `jones_z`（(光線, 2)）に出射電場の z 成分（最後の面の局所座標）。像面への入射角が大きい系では無視できない
- 物体を直接: `trace(None, px, py, object=(x, y))`（有限物体の物点 [mm]）・`object=(l, m, n)`（無限遠の方向）。`s.object_of(視野)` で視野の物体
- パラメータの変更: `s.get_param("3.t")`、`s.set_param("L1.c", 0.02)`、`s.set_params({"3.t": 10, "5.dx": 0.01})`、
  `s2 = s.with_params({...})`（元の系は変えない）。種類は c r k a2〜a16 t dx dy tx ty tz dn disp
- 玉・群の単位のパラメータ: `"lens2.tx"`・`"G.shift"` のように `名前.種類`（dx dy tx ty tz shift dn disp）。
  玉は読込で `lens1`・`lens2` …（玉モードの番号）、群は `s.define_unit("G", [(3, 7)], pivot=(0, 0, 1))`
  （ダブルパスなら往きと戻りの範囲を 2 つ）、プロジェクトではブロック・群の id。`s.units()` で一覧。偏心・傾きは
  玉・群を剛体として回転の中心 pivot のまわりに動かす（範囲の外は動かない）。`grad=` でも同じ名前
- 系の編集: `s.insert_surface(4, r=50, t=5, material="N-BK7", semi=8, comment="L1")`（面 4 の前に入れる。
  値は `s.surface(4)` の dict と同じ名前: type comment t c r k asph cx material coating semi ignore aperture cb cb_order focal jones dn disp）、
  `s.set_surface(j, ...)`、`s.delete_surface(j)`（外したピックアップ・ソルブの説明を返す）、`s.set_surface_type(j, "COORDBRK")`、
  `s.set_material(j, "MIRROR")`、`s.set_stop(j)`、`s.add_glass_catalogs(["SCHOTT"], dirs=[...])`。
  ピックアップ `s.add_pickup(dst, src, "c", scale=-1)`（面の前・後の傾き偏心も `"before0"`〜`"before4"`・`"after0"`〜`"after4"` で。
  面の後を元の面の前から引くなら `src_what="before0"`）・`remove_pickup`・`pickups()`、厚さのソルブ `s.add_solve(j, height=0)`・
  `remove_solve`・`solves()`。多重構成の表 `s.set_config_count(3)`・`s.set_config_row("5.t", [10, 12, 14])`・`s.config(2)`（新しい系）・
  `s.set_config(2)`・`s.configs()`。どの編集も失敗すれば系は元のまま。編集の後はピックアップとソルブを解き直し、面番号は付け替える
- 光路長の微分: `trace(..., grad=["3.t", "objx"])` で `grad`（(波長, 光線, パラメータ)）
- 多数の系をまとめて: `lb.trace_many([s1, s2, ...], 1, px, py, grad=[...])` → 系ごとの dict の list。
  引数は `trace` と同じで、結果は系ごとの `trace` とビット一致。系の単位でも並列にするので、系ごとの光線が少なくても
  コアを使い切る（4 コア・256 系・32〜1716 本で、系ごとの 1 スレッドの 2.2〜4.1 倍、系ごとに 4 スレッドで呼ぶときの 3.3〜5.9 倍）
- 近軸量: `s.cardinal(wave=None)` で基点（OpticStudio の Cardinal Points 相当: 焦点・主点・反主点・節点・反節点、物体空間は面 1 から・像空間は像面から、媒質の中の焦点距離 `f_object`・`f_image`、空気換算の `efl`、`bfl`・`ffl`）、`s.system_data()` でシステムデータ（`effl`・`enpp`・`expp`・`wfno` など OpticStudio のオペランド名の小文字）。変更後の系の値
- ファイバ結合: `s.fiber_coupling(na_source, na_receiver=None, wave=None, source=(0, 0), receiver=(0, 0), grid=101, pol=False, clip_stop=True)` → `eta`・`system`・`receiver`（pol=True なら膜込みの `jones` (2, 2)、入射偏光 e の結合は |jones @ e|²）。送り側は物点、受け側は像面の点の、遠視野 1/e² の NA のガウスモードで、受け側の方向空間で重なり積分をとる（有限物体の系）。系の絞りの外を通る光は既定で系の損失 `system` に数える（`clip_stop=False` で無視）
- 系ファイル: `s.save("x.json")` で lensbench 独自の JSON に欠けなく保存し（面・ガラスのデータ・パラメータの変更・ピックアップ・
  ソルブ・玉と群・多重構成の表・dn と disp・`coatings=` のパス）、`lb.System.load("x.json")` で読み直す（追跡はビット一致）。
  `s.to_json()`・`lb.System.from_json(txt)`、`s.hash`（系の SHA-256、同じ系なら同じ値。結果に刻んで後から確かめる）。
  CLI は `lensbench convert x.zmx -o x.json`（.zmx・プロジェクトから）、どの解析も系ファイルを読め、`--save` の JSON と `info` に
  `lensbench`（版）と `system_hash` を書く。serve は `save`・`edit`・`surface`（docs/serve.md）
- 一括実行: 公差・モンテカルロ・掃引を、スレッドで並列に回して JSONL に逐次書き、止まっても続きから。
  ```python
  plan = lb.Sampler(s, {"3.t": 0.02, "5.dx": ("uniform", -0.01, 0.01), "G.tx": ("trunc", 0.05, 2.0)}, corr=None, seed=1)
  # 掃引なら lb.Grid(s, {"3.t": [10, 11, 12], "5.t": [3, 4]})（直積）
  def f(i, sys, seed):                    # sys は i 番目の組の値を入れた系、seed はタスクごとの種
      return {"rms": float(sys.rms(field=1)["rms"][0])}
  res = lb.run_batch(f, plan, n=1000, out="mc.jsonl", threads=8)   # 進み具合と残り時間を標準エラーに
  head, rows = lb.read_batch("mc.jsonl")  # 頭（版・系の hash・種・組の作り方）と {"i", "seed", "result"}
  ```
  i 番目の組は (種, i) だけで決まる（スレッド数・再開・順序によらない）。相関はガウスのコピュラ（周辺分布はそのまま）。
  同じファイルに同じ計算（頭が同じ）なら終わった組を飛ばす。func の中の lensbench の追跡・解析は GIL を外すので並列に進む
  （Python の計算ばかりの func は並列にならない）。Rust からは `lensbench_core::runs`（`Sampler`・`Grid`・`run`）
- pickle: `pickle.dumps(s)` で系を系ファイルの JSON のバイト列にし、別プロセスで作り直せる（パラメータ変更・ピックアップ・ガラス・照準・視野の種類を含め、追跡はビット一致）。膜ファイルと `coatings=` の表は同じパスから読み直す（同じ計算機向け）
- 多重構成をまとめて: `cs = lb.System.load_configs("x.zmx", configs=None)`（省くと全構成、ファイルは 1 回だけ読む）→
  `lb.trace_many(cs, None, px, py, object=物点)` で同じ物点・同じ瞳の格子で全構成を一度に追跡（絞りの径の切替 APMX・APER なども構成どおり）
- 視野の種類: `s.field_type`、`s.set_field_type(kind, cond=None, fields=None)`（条件で定める視野はプロジェクトファイルの節を参照）
- 物点・瞳座標の解き: `s.solve_ray("object", ("dirsum", 12, 30), object=(0.1, 0.0))`、
  `s.solve_ray("pupil", ("position", 40, (0.0, 0.0)), field=1)` → object・pupil・residual・iterations
- プラグイン（検出器・解析）: 名前で呼ぶ。設定は数・文字列・数の並びの辞書、入出力は名前つきの配列
  ```python
  lb.plugins()                                   # [("rms", "analysis"), ...]。lb.plugin_spec(name) で入出力の説明
  p = lb.Plugin("rms", rings=4)                  # 設定を確かめて準備（知らない設定名は誤り）
  out = p.run(system=s, field=1)                 # 系を追跡するプラグインには系を渡す → {"wfe", "spot", ...}
  outs = p.run_many([dict(field=1), dict(field=2)], system=s, threads=8)  # 入力ごとに並列、結果は入力の順

  def fringe(config, inputs, system=None):       # 試作用: Python の関数を登録（配列はまとめて渡す）
      return {"tilt": ...}
  lb.register_plugin("fringe", fringe, kind="analysis")
  lb.Plugin("fringe", deg=4).run(img=img4)
  ```
- px・py は list・float32・飛び飛びの配列も受け付ける（float64 に直す）
- ゴースト: `s.ghost_pairs()` → [(a, b), ...]、`g = s.ghost(a, b)` で 2 回反射の経路を展開した系（`trace`・`pol` など
  そのまま）。`s.ghost_origin(a, b)` で展開した系の面ごとの元の面と反射の面の番号
- 系はオブジェクトごとに独立（プロセス並列にしてよい）。追跡の間は GIL を外す
