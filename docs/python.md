# Python API の約束（版・単位・座標）

`import lensbench as lb` で使う、lensbench の Python バインディングです。計算は Rust のコアが同じプロセスの中で行い、
結果は numpy の配列で返ります。GUI・CLI 本体は Python がなくても動きます。

- 作り方: `maturin build --release -m crates/lensbench-py/Cargo.toml -o dist` → `pip install dist/lensbench-*.whl`
- 型スタブ: wheel に `lensbench.pyi` と `py.typed` が入っています。エディタの補完・mypy・pyright がそのまま使えます
- 例題: [`examples/python/`](https://github.com/masa-iwm/lensbench/tree/main/examples/python)（下の「例題」）
- 画面なしの実行: `lensbench run script.py`（下の「lensbench run」）

## 版と互換

| 名前 | 意味 |
| --- | --- |
| `lb.__version__` | lensbench 本体の版（CLI の `--version` と同じ） |
| `lb.API_VERSION` | Python API の版 `(major, minor)` |

- **minor** は足すだけの変更で上がります（関数・引数・返り値の dict のキーを足す）。今までのスクリプトはそのまま動きます。
- **major** は互換を壊す変更で上がります（名前・引数の意味・返り値の形を変える、消す）。
- 互換を壊す前に、まず古い使い方を**非推奨**にします。非推奨の間は古い使い方も動き、呼ぶと `DeprecationWarning` で
  新しい使い方を知らせます。非推奨の期間は、少なくとも 1 つの minor 版、かつ 30 日です。その後の major 版で消します。
- 変更はこの文書の「変更履歴」に書きます。スクリプトの頭で `assert lb.API_VERSION[0] == 1` のように確かめられます。
- 返り値の dict に**キーが増える**のは互換の範囲です（dict を丸ごと比べるテストは、使うキーだけ比べてください）。

### 変更履歴

- **2.12**: `System.analyze_file(path, name, glass_dirs=None, lens_catalogs=None, threads=None, **settings)`（ファイルの系で登録簿の解析を走らせる。
  CLI の同じ名前のサブコマンドと同じで、プロジェクトのほかの光束・経路・構成も読む。例: 非順次の追跡 `nsc`）を追加
- **2.11**: 機構部品で光線をけるかの設定 `System.mech_clip`（既定 True。プロジェクト・系ファイルの `mech_clip`）を追加。
  True なら光線の追跡（すべての解析）で機構部品の穴の外に出た光線をける。機構部品の無い系では何も変わらない
- **2.10**: プロジェクトの光束（`beams`）: `System.beams(path)`（光束の一覧の dict の list）・`System.load_beams(path, beams=None, all=False, ...)`
  （光束の名前 → System の dict。それぞれ `load(path, state=名前)` と同じ）・`System.beam_table(path, field=1, wave=None, grid=21, reference=None, all=False, ...)`
  （光束の表。CLI の `beams`・serve の解析 `beams` と同じ）
- **2.9**: Python の関数を光源・膜・形として登録する口（試作用。Python のプロセスの中だけで、CLI・GUI からは使えない）: `register_source(name, func)`・
  `unregister_source(name)`（`func(config, wl) -> {"start", "direction", "weight"}`、波長ごとに 1 回）と、`register_coating_catalog` の `models={膜の名前: func}`
  （`func(wl, n1, n2, cos1, path, from_substrate) -> {"rs", "rp", "ts", "tp"}`。波長・屈折率・経路・向きの組ごとに cos1 の 2001 点の格子で 1 回呼び、cos1 で線形に補間する）、
  `register_shape(name, func, params=None)`・`unregister_shape(name)`（面の種類 USERSHAPE の形。`func(config, x, y) -> (z, dzdx, dzdy)` を形の準備ごとに設定 `radius`・`grid` の格子で
  1 回呼び、値と勾配の双 3 次 Hermite で引く。設定は変数にできない）。
- **2.8**: 拡張が足した利用者定義の光源の一覧 `source_models()`（{名前: 説明}）と、光源の光線 `source_rays(system, name, config=None, wave=None, frame="object")`
  （始点・方向・重みの配列の dict。start・direction をそのまま `trace_rays` に渡す）を追加（[ライブラリ（Rust）](rust.md) の `lensbench_core::usersource`）。
- **2.7**: 像の模擬 `System.image_sim(image, ...)`（OpticStudio の Image Simulation 相当。NumPy の (高さ, 幅) か (高さ, 幅, 3) を受けて同じ形で返す）を追加。
- **2.6**: 膜の一覧を登録する `register_coating_catalog(name, files=[], dat=None)`・`unregister_coating_catalog(name)`・
  `coating_catalogs()` を追加。登録した膜は面の膜の名前で、.zmx の膜ファイル・膜のフォルダより先に引く（拡張の
  `Extensions::coating_catalog` と同じ表。[ライブラリ（Rust）](rust.md) の `lensbench_core::usercoating`）。系は膜を一度だけ読むので、使う系を読む前に登録する。
- **2.5**: 熱の構成を作る `System.make_thermal(temps, pressures=None)`（OpticStudio の Make Thermal。新しいシステムと注意の list を返す）を追加。
- **2.4**: `System.optimize` に大域の探索（`starts`・`seed`・`keep`・`spread`）とガラスの置き換え（`glass`）の引数を足した。
  返り値の dict に、回したときだけ `global_search`・`glass` のキーが付く（[最適化](optimize.md)）。`glass` の dict は候補の絞り込み
  （`max_cost`・`max_cr` …・`status`・`exclude`・`exclude_incomplete`）と近さの刻み（`nd_step`・`vd_step`・`dpgf_step`）も受ける
- **2.3**: 多重構成の表の材料・コーティングの行を名前で書く `System.set_config_names(spec, names)`（spec は `N.material` か `N.coating`、
  コーティングの名前の空はコーティングなし）と、構成ごとの名前を読む `System.config_names(spec)` を追加。`configs()` の材料・コーティングの行の値は名前の番号。
- **2.2**: `huygens_psf`・`huygens_mtf` の既定の計算を速い近似（像のグリッドの区画ごとに位相と振幅を 2 次式で）にした。強度は前と
  1e-7 程度で合う。前と同じ厳密な和は、足した引数 `exact=True` で。
- **2.1**: MTF の `mtf`・`mtf_focus`・`mtf_field`・`geo_mtf`・`huygens_mtf` に子午・球欠の向き `axes`（`"field"` が既定で子午が視野の向き、
  `"pupil"` は前と同じく子午が瞳の y の向き）を足した。既定の値は x の成分がある視野で前と変わる。
- **2.0**: 引数の名前を CLI・serve とそろえた（用語の表は下の「引数の名前」）。互換を壊す変更で、旧名は受けない
  （全コードの見直しの破壊的変更として、非推奨の期間を置かずに変えた）:
  `load`・`from_project` の `split_path` → `beam_path`、`System.split_paths` → `System.beam_paths`、
  `trace`・`trace_rays`・`trace_many` の `wavelengths` → `waves`、`solve_ray` の `wavelength` → `wave`。
  serve のクライアント（`lensbench_client`）は `glass_dir`・`lens_catalog` → `glass_dirs`・`lens_catalogs`、`trace` の `global_` → `globals`。
- **1.1**: `verify()`・`verify_compare()`（照合試験の枠組み）を追加。
- **1.0**: 今の API を 1.0 とする。`API_VERSION`・`run_settings()`・`progress()` を追加。
  `System.load` などの `coatings=` を省くと環境変数 `LENSBENCH_COATINGS` を使う。`threads=` を省くと
  環境変数 `LENSBENCH_THREADS` があればその数（import したときに読む。CLI は起動したときに読む）。

## 単位と座標の約束

この節が約束の正本です。各関数の説明に書いた単位はここと同じです。

### 単位

| 量 | 単位 |
| --- | --- |
| 長さ（位置・厚み・半径・偏心・光路長 OPL・`opl`・`grad` の分子） | mm |
| 曲率 `c` | 1/mm |
| 波長 | µm（名前に `_nm` が付く引数だけ nm） |
| 角度（面の傾き tx・ty・tz、視野角、入射角、光線のエイミングの角度） | deg（OpticStudio と同じ） |
| 方向 | 方向余弦 (l, m, n)（単位ベクトル） |
| 波面・OPD・Zernike の係数 | 波（その波長の波長単位）。`[mm]` と書いたものは mm |
| 空間周波数（MTF） | cycles/mm |
| NA | 無次元（n sin θ） |

### 番号

- **視野・波長は 1 始まり**（OpticStudio・CLI と同じ）。`s.fields[f - 1]`・`s.wavelengths[w - 1]` のように Python の
  並びを引くときだけ 1 を引きます。
- **面は OpticStudio の面番号**（物体面 0、像面 `s.image`）。多くの引数は面の名前（コメント）でも指定できます（`s.find_surface`）。
- 返り値の配列の軸は `(波長, 光線)`、面の記録は `(波長, 面, 光線)` です（波長の軸は指定した `waves` の順）。

### 引数の名前

Python・CLI・serve は同じ名前を使います（CLI は下線をハイフンにした旗）。並びを取る引数は Python・serve では複数形、
CLI では単数形の旗を繰り返します。

| 名前 | 意味 | Python | CLI | serve |
| --- | --- | --- | --- | --- |
| `wave` | 波長の番号 1 つ（1 始まり） | `wave=` | `--wave` | `wave` |
| `waves` | 波長の番号の並び | `waves=` | — | `wave`（並び） |
| `wavelengths` | 波長の値 [µm] の並び（システムの波長を置き換える） | `load(wavelengths=)` | — | — |
| `beam_path` | プロジェクトのビームスプリッターの経路（T・R） | `beam_path=` | `--beam-path` | `beam_path` |
| `beam_paths` | 重ねる経路の並び（`all` で全部） | `System.beam_paths()` は一覧 | `--beam-paths` | `beam_paths`（layout） |
| `glass_dirs` | ガラスカタログを探すフォルダ | `glass_dirs=` | `--glass-dir`（繰り返す） | `glass_dirs` |
| `lens_catalogs` | レンズカタログ | `lens_catalogs=` | `--lens-catalog`（繰り返す） | `lens_catalogs` |
| `reference` | RMS の基準（centroid・chief） | `reference=` | `--reference` | `reference` |

旧名（`split_path`・`--path`・`path`、`--paths`・`paths`（layout）、`wavelengths=`（番号）・`wavelength=`、
`glass_dir`・`lens_catalog`（serve）、`--ref`・`ref`（rms・公差の設定））は受けません（2.0 で変えた）。
解析のメソッドの名前は CLI のサブコマンド・登録簿の名前の `-` を `_` にしたもの（`geo_mtf`・`mtf_focus`・`mtf_field`。
旧名 `geometric_mtf`・`through_focus_mtf`・`field_mtf` は受けません）。`trace` の結果の波長の番号は `waves`（引数と同じ名前）。

### 座標

- **局所座標**（既定）: 各面の頂点を原点とし、その面の傾き・偏心を当てた後の座標。`trace` の `x, y, z, l, m, n` は
  最後の面（`last`、省くと像面）の局所座標です。
- **大域座標**: `globals=True` で `gx, gy, gz, gl, gm, gn` も返します。基準の面はシステムの大域座標の基準面（OpticStudio の
  Global Coordinate Reference Surface）。書き出し（STEP・glTF）も大域座標です。
- 物体: 有限物体なら物体面の局所座標の物点 (x, y) [mm]、無限遠なら方向 (l, m, n)（`object=`、`object_of(field)`）。

### 瞳座標（`pupil=`）

| 値 | 意味 |
| --- | --- |
| `"normalized"`（既定） | 正規化瞳座標（OpticStudio と同じ）。入射瞳が物体より前の有限物体系では px = 1 が絞りの −x 側を通る |
| `"signed"` | システムによって符号を揃えた正規化瞳座標（px = 1 が常に絞りの同じ側。`System.pupil_reversed` が True のシステムで normalized と逆） |
| `"stop"` | 絞り面の局所座標 [mm]。その点を通る実光線を光線ごとに解く（エイミングによらない） |

## エラー

- 計算のエラーは `lensbench.Error`（`ValueError` の子）です。種類ごとに `InvalidError`（値・構成が不正）・`UnsupportedError`（対応していない機能）・
  `ReadError`（ファイルの読込）・`CancelledError`（中断した）に分かれます（どれも `lensbench.Error` の子）。範囲外の番号は `IndexError`、型のエラーは `TypeError` です。
  `System.load` はファイルが無い・読めないとき `ReadError`、引数の組み合わせや中身のエラーは `InvalidError` です。
  例外の `code` は機械で見分ける符号です（例 `e.code == "glass.not_found"`。文は表示の言語で変わるので、判定には `code` を使う）。
  今ある符号: `file.not_found`（ファイルが無い）・`file.is_folder`（フォルダを渡した）・`glass.not_found`（ガラス・ガラスカタログが無い）・
`project.state_not_found`（無い状態の名前）・`project.block_not_found`（無いブロックの id）・`catalog.lens_not_found`（カタログに無い品）・
`argument.invalid`（構成・状態・経路の指定のエラー）・`system.pupil_vignetted`（瞳の光線が像面に届かず、MTF などで瞳の大きさを決められない）・`system.all_vignetted`（視野の光線が全部けられ、像面に届く光線が無い。全視野の解析ではその視野を注意 `field N: …` にして飛ばす）。符号の無いエラーの `code` は種類と同じ。システムの保存や編集などの一部のエラーは、まだ種類のない `lensbench.Error` になります（順に種類を付けていきます）。
- 文は「種類: 内容」の形です（`不正:`・`未対応:`・`読込:`）。**面に関わるエラーには面の名札が付きます**:
  `面 番号(種類, 材料, コーティング 名前, 「コメント」)`（空の項は省く）。例:

  ```text
  未対応: 面 1(STANDARD, N-SK16, コーティング MYCOAT): コーティング MYCOAT（境界 MYCOAT|AIR|N-SK16）: コーティングファイルになく、実測表もモデルもありません（暗号化されたコーティング？）
  ```

- 警告（分散式の範囲外、プロジェクトの読み替え、コーティングを使う計算（偏光の `trace`・`interference` の `polarized`・システムを渡したプラグインなど）での同じ名前のコーティングの注意など）は Python の `warnings` で出ます。

## 解析の登録簿

CLI のサブコマンド・serve の `analyze`・GUI の解析パネルと同じ解析は、名前で回せます。設定・既定値・結果の形は 1 か所の定義
（解析の設定の型）から作るので、どの入口でも同じです。

```python
import lensbench as lb
s = lb.System.load("system.zmx", glass_dirs=["glass"])
[a["name"] for a in lb.analyses()]        # 解析の名前（各要素の params は設定の項目: name・type・default・unit・doc）
lb.defaults("psf")                         # 既定の設定 {"image": 128, ...}
r = s.analyze("mtf", field=1, max_freq=100)  # 結果は CLI の --json と同じ dict（配列は NumPy でなくリスト）
```

`System.psf`・`mtf`・`rms`・`optimize`・`lensbench.zoom` などの関数の引数の既定値も同じ定義の値です（`lb.defaults("optimize")`・
`lb.defaults("zoom")`。試験で突き合わせています）。

## 結果の型

結果は dict です。主な結果（`trace`・`trace_rays`・`mtf`・`geo_mtf`・`huygens_mtf`・`mtf_focus`・`mtf_field`・`psf`・
`huygens_psf`・`cardinal`・`optimize`）は型スタブに鍵と型を書いてあり（`lensbench.pyi` の `_TraceResult` などの `TypedDict`）、
エディタの補完と mypy の検査が効きます。実行時の型は dict のままです。オプションで付く鍵（`record=True` の `rec_x` など）は
`total=False` の側にあります。

## 例題

[`examples/python/`](https://github.com/masa-iwm/lensbench/tree/main/examples/python) にあります。リポジトリの中の公開のテスト用のシステム（`tests/refs/`）とガラス（`tests/glass/`）を使い、
どれも数秒で終わります。CI でも毎回実行し、mypy で型も確かめています。

| ファイル | 内容 |
| --- | --- |
| `trace_spot.py` | 視野ごとに瞳の格子を追い、RMS スポット半径を出す |
| `grad_opl.py` | 光路長の微分 `trace(grad=...)` を差分と比べる |
| `solve_ray.py` | 像面の指定点を主光線が通る物点を解く（`solve_ray`） |
| `param_sweep.py` | 厚みを掃引して CSV に書く（`with_params`・`progress`・`run_settings`） |

```sh
python examples/python/trace_spot.py
lensbench run --threads 4 --out sweep examples/python/param_sweep.py -- --n 11
```

## lensbench run（画面なしの実行）

```sh
lensbench run [--config run.json] [--python PY] [--threads N] [--glass-dir DIR]... [--coatings FILE]...
              [--out DIR] [--log FILE | --no-log] script.py [スクリプトの引数...]
```

exe に Python は入っていません。外の Python を見つけてスクリプトを起動し、設定を環境変数で渡す薄い入口です。

- **Python の探し方**: `--python`（`"py -3"` のように引数つきも可）→ 設定ファイルの `python` → 環境変数 `LENSBENCH_PYTHON`
  → 仮想環境（`VIRTUAL_ENV`）→ PATH の `python3`・`python`（Windows は `python`・`py -3`）。その Python に lensbench の
  wheel が入っている必要があります（入っていなければそう知らせます）。
- **渡す設定**（スクリプトは `lb.run_settings()` で dict として受け取れます）:

  | 引数 | 環境変数 | 効き方 |
  | --- | --- | --- |
  | `--glass-dir` | `LENSBENCH_GLASSCAT`（前に足す） | `System.load` などがガラスを探す |
  | `--coatings` | `LENSBENCH_COATINGS` | `coatings=` を省いたときのコーティングの実測表・モデル |
  | `--threads` | `LENSBENCH_THREADS`・`RAYON_NUM_THREADS` | `threads=` を省いた計算のスレッド数 |
  | `--out` | `LENSBENCH_OUT` | 出力先（無ければ作る）。スクリプトが `run_settings()["out"]` で使う |
  | `--log` | `LENSBENCH_LOG` | ログのファイル |

  パスは絶対パスにして渡します。ほかに `LENSBENCH_RUN=1`・`PYTHONUNBUFFERED=1`・`PYTHONIOENCODING=utf-8` を付けます。
- **設定ファイル**（`--config`、JSON。相対パスはこのファイルのフォルダから。引数が優先）:

  ```json
  {
    "python": "C:/Python312/python.exe",
    "glass_dirs": ["glass"],
    "coatings": ["coat/models.lbcm"],
    "threads": 8,
    "out": "results",
    "log": "results/run.log",
    "env": {"MY_SETTING": "1"}
  }
  ```

- **ログと進み具合**: スクリプトの標準出力・標準エラーはそのまま画面に出し、ログのファイル（`--log`、省くと
  `--out` があれば `<out>/run.log`）にも経過時間と出どころ（out・err・run）つきで書きます。`lb.progress(i, n, "文")` は
  `lensbench:progress i/n 文` の 1 行を標準エラーに出します（ログに残り、将来の GUI が進捗の表示に使います）。
- **終了コード**: スクリプトの終了コードをそのまま返します（例外で終われば 1）。Python を起動できなければ 127、
  設定・スクリプトのパスのエラーは 2。CI やスケジュール実行で成否を判断できます。
- GUI からも同じものを呼べるよう、本体は `lensbench_cli::run::run_script`（行ごとの受け取り口つき）です。

## 型スタブの更新

スタブは実行中のモジュールと Rust のソースから自動で作ります。API を変えたら作り直してください（CI が食い違いを検査します）。

```sh
maturin build --release -m crates/lensbench-py/Cargo.toml -o dist && pip install --force-reinstall dist/lensbench-*.whl
python crates/lensbench-py/tools/gen_stubs.py          # crates/lensbench-py/lensbench.pyi を書き直す
python crates/lensbench-py/tools/gen_stubs.py --check  # 食い違えば 1
```

Rust の型から決まらない引数・返り値は `Any` になります。意味の決まったものは `tools/gen_stubs.py` の `OVERRIDES` で与えます。
結果の dict の鍵は `RESULTS` に書き、`tests/test_stubs_examples.py` が実際の結果と突き合わせます。
