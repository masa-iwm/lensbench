# ライブラリ（Rust）

## 入口: 読む・追跡する・解析する

`use lensbench_analysis::prelude::*;` で主な口がそろう（core の prelude に .zmx の読込 `zmx`・`LoadOptions` と解析 `analysis`・`psf` を足したもの）。それ以外の `pub` のモジュールは上の層のクレートと拡張のための口で、
版の間で変わりやすい。

```rust,ignore
use lensbench_analysis::prelude::*;

lensbench_core::init_from_env(); // LENSBENCH_GLASSCAT・LENSBENCH_THREADS など（使わないなら呼ばなくてよい）
lensbench_lang::register(); // 誤りの文・注意の和訳の表（表示の言語が日本語のとき使う。呼ばなければ英語）
let opt = LoadOptions { catalog_dirs: vec!["glass".into()], ..Default::default() };
let sys = zmx::load_with("lens.zmx".as_ref(), &opt)?;
// 視野 1 の瞳座標 (px, py) の光線（Trace.rays の各光線に像面の位置・光路長・けられ）
let t = sys.try_trace(FieldSel::Index(0), &[0.0, 0.5], &[0.0, 0.5], None, None, false)?;
// 回折 MTF（多色）・PSF（2 次元の結果は Grid2: 座標 x()・y() と行優先の data()、psf.i[iy][ix]）
let m = analysis::mtf(&sys, FieldSel::Index(0), Waves::All, None, &[10.0, 50.0])?;
let p = psf::psf(&sys, FieldSel::Index(0), Waves::All, PsfOpts::default())?;
// 近軸量（EFL・瞳など）は report::cardinal、波面は analysis::wavefront、Zernike は analysis::zernike
```

- 計算ごとの設定は文脈で渡す: スレッド数 `with_threads(Some(n), || ..)`、中断 `par::with_cancel(Some(&token), || ..)`、
  拡張の表（カタログの形式・面のモデル・プラグイン）`ext::with(Arc<Ext>, || ..)`、ファイルの口
  `vfs::with(Arc<dyn vfs::Fs>, || ..)`（`vfs::MemFs` に入れたファイルからシステムを読める）。どれも並列の作業のスレッドへ引き継ぐ。
- プロジェクト（玉・群・カタログのレンズを組むシステム）は `lensbench_project::open`、CLI・serve と同じ解析の呼び出しは
  `lensbench_engine::Session`。

## 拡張の約束: 外のクレートから足す

別のリポジトリのクレートから解析・プラグイン・検出器・利用者定義の面と形・材料・膜・カタログの形式を足し、自分の exe・wheel を組める
（DLL は使わない。静的にリンクする）。依存するのは `lensbench-sdk` 1 つ。

- 足すものは `lensbench_sdk::Extensions` 1 つにまとめる: `.plugins(..)`・`.analysis(..)`・`.detector(..)`・
  `.surface_model(..)`・`.shape_model(..)`・`.material_catalog(..)`・`.coating_catalog(..)`・`.coating_source(..)`・`.catalog_source(..)`・`.messages(訳の表)`。GUI の見せ方は `lensbench_sdk::gui::GuiExtensions`
  （`gui_analysis`・`gui_detector`・`gui_translations`）、Python の関数は `lensbench_sdk::py::PyExtensions`（`py_module`）。
- 入口に渡す: CLI は `lensbench_sdk::cli::main(ext)`、GUI は `lensbench_sdk::gui::main(ext)`、Python の拡張モジュールは
  `#[pymodule]` から `lensbench_sdk::py::module(m, ext)`。呼ぶ順の約束は無い。
- 2 つの拡張を `merge` すると、同じ名前のもの（解析・検出器・プラグイン・面と形のモデル・材料のカタログ・膜の一覧と中の膜・カタログと膜の拡張子）は誤りになる
  （置き換えるときは後の方に `.replacing()`）。`Extensions::named(..)` の名前は誤りの文にだけ使い、サブコマンドなど
  利用者に見える名前には入らない。
- 足し忘れは `lensbench_sdk::check(&ext)` で見つかる（GUI の見せ方の無い解析・欄の無い検出器・反転の約束を守らない面・勾配が矢高の差分と合わない形・約束を守らない材料）。
  試験で呼ぶ。
- **約束の型** は `lensbench-sdk` が再輸出するものだけ（一覧は `crates/lensbench-sdk/API.txt`）。各クレートのほかの
  `pub` は中の都合で変わる。一覧の試験（`tests/api.rs`）が見るのは再輸出の名前だけで、型の中身（関数の引数・
  フィールドなど）の変化は公開 CI の雛形のジョブ（`examples/extension` を組んで試す）が止める。0.x の間はマイナー版で約束を壊してよく、壊す変更は [変更点](changes.md) の Rust API に書く。
- 外は git のタグ（`v0.5.0` から）か rev で参照する。egui・pyo3・serde・schemars は SDK の再輸出
  （`lensbench_sdk::gui::egui`・`py::pyo3`・`serde`・`schemars`）を使い、自分の Cargo.toml に直接書かない
  （版がずれると型が合わない）。derive には `#[serde(crate = "lensbench_sdk::serde")]` などを付ける。
- 雛形は `examples/extension`（プラグイン・解析と GUI の見せ方・Python の関数・試験）。写して始める。profile・
  `[patch]` は git 依存から伝わらないので雛形の Cargo.toml に写してある。Cargo.lock は公開のものを写すと、公開の CI で
  試した依存の版になる。
- アロケータは入口の側で決める: SDK の入口（`cli::main`・`gui::main`・`py::module`）は `#[global_allocator]` を
  置かない。公開の exe・wheel は mimalloc を使っていて、外で組む exe・wheel も同じ速さにするには、入口を作るクレートに
  `#[global_allocator] static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;` を 1 つ置く（雛形の `src/lib.rs`。
  wasm では置かない）。置かないと OS のアロケータで動き、値は同じで、光線追跡・カタログの読み込みが遅くなる。
- Web 版: 雛形の `web`（wasm32 だけ）が `#[wasm_bindgen(start)]` で拡張を入れる（wasm-bindgen は
  `lensbench_sdk::web::wasm_bindgen`）。組み立ては同じタグの公開リポジトリで
  `CRATE=<パッケージ名> MANIFEST=<外の Cargo.toml> scripts/build_web.sh <出力先>`。画面用（feature `gui`）・
  計算の Web Worker 用（`worker`・`threads`）の wasm を作り、公開の Web 版のページ・JS と一緒に置く
  （[Web 版](web.md)）。同梱のファイルは `lensbench_sdk::web::set_extra`。

### 拡張の口（プラグイン・面・形・材料）

- `lensbench_core::plugin`: 検出器・解析のプラグインの約束。`Plugin`（名前・種類・`prepare(&Config)`）と
  `Instance`（`run(&Context, &Arrays) -> Arrays`）を実装し、`Registry` に名前で登録する。`run` は状態を持たず、
  同じ `Instance` を複数スレッドから呼んでよい（`run_many`）。システムを追跡するものは `Context` でシステムとコーティングを受け取る。
  多くの方向の微分を流す解析は、任意の `linearize`（`run` と同じ出力と、入力の微分 1 方向から出力の微分を作る `Linear`）を
  実装すると、呼ぶ側は解析を 1 回で済ませられる（既定は None）。
  同梱のプラグインはワークスペース内の crate（`lensbench-analysis`）で、CLI・Python に組み込む（`lensbench plugins`）
- `lensbench_core::usersurf`: 利用者が定義する面の口。形は頂点の平面に限り、`SurfaceModel`（名前・`prepare(&Config)`）の
  `SurfaceInstance::deflect`（交点・入射方向 → 出射方向・OPD に足す光路長）と `paraxial_power`（近軸の屈折力）を実装して
  拡張の表 `lensbench_core::ext::Ext` に `add_surface_model` で足す（`ext::with` で計算ごとに渡す、
  または `ext::set_default`・`update_default` でプロセスの既定にする。カタログの形式・プラグインも同じ表）。面は種類 `USER` に `user = {"model": 名前, "config": {…}}`（システムファイル・`set_surface_data`・GUI の
  「利用者定義の面」）。偏光は理想レンズと同じく方向の変化の最小の回転、dOPL/dp・着地点の微分は未対応、.zmx には書けない、前後は空気。
  同梱のモデル `ideal_mapping`: 画角 θ → 像高 h を与える理想レンズ（`f`、`mapping` = `tan`（f tan θ、理想レンズとビット同じ）・`theta`（f θ）・
  `sin`（f sin θ）・`equisolid`（2f sin(θ/2)）・`stereographic`（2f tan(θ/2)）・`poly`（h = f(θ + c₁θ³ + c₂θ⁵ + …)、`coef`）・
  `table`（`theta_deg`・`height` の表を PCHIP で補間））。同じ方向の光線は焦点面の (h(θ), f) に無収差で集まる（物体は無限遠を想定）
- `lensbench_core::usershape`（SDK では `lensbench_sdk::shape`）: 利用者が定義する形の口。面の形は土台の二次曲面
  （`c`・`k`）にモデルの矢高を足したもので、働きはふつうの屈折・反射（材料・コーティング・偏光・dOPL/dp・着地点の微分・
  玉の反転はほかの面と同じに効く）。`ShapeModel`（名前・`prepare(&Config)`）の `ShapeInstance::sag(x, y)`
  （土台に足す矢高と勾配 (z, ∂z/∂x, ∂z/∂y)）を実装し、変数にしたい設定の項目は `dsag(項目, x, y)` も返す。
  `Extensions::shape_model` か拡張の表の `add_shape_model` で足す。面は種類 `USERSHAPE` で、書き方は 2 通り:
  - 編集の操作（`set_surface_data` の `data`）: `"typ": "UserShape", "shape": {"model": 名前, "config": {…}, "flip_z": …, "mirror_x": …}`
  - システムファイルの面の行: `"shape": {"user": {"c": …, "k": …, "model": 名前, "config": {…}, "flip_z": …, "mirror_x": …}}`

  設定の数の項目（`Num`。数の並びは除く）はパラメータ `3.u_<項目>`（変数・ピックアップ・微分）。`params()` に宣言した項目は拡張を入れた時から読め、宣言していない項目はその項目を持つ形を準備するまで読めない。約束は、クリア半径の少し外まで
  有限で勾配が連続、`prepare` は軽い（値を変えるたびに作り直す）、状態を持たずどのスレッドから呼んでもよい。
  鏡の後・玉の反転の裏返し（z・x の向き）は core が包むので、モデルは何もしなくてよい。近軸の曲率は土台の c に頂点の
  2 階（勾配の差分、幅はクリア半径の 1e-6）を足したもので、YZ 面だけ。.zmx には書けない。`check_shape(モデル, 設定, 半径)` で約束を確かめられる
  （`lensbench_sdk::check` は `example_config`・`example_radius` で呼ぶ）
- `lensbench_core::usermaterial`（SDK では `lensbench_sdk::material`）: 利用者が定義する材料の口。`MaterialModel`
  （名前・`index(波長 µm, 温度 °C, 気圧 atm)`、任意で `tref`・`range`・`transmittance`）を、名前つきのカタログの単位で
  `Extensions::material_catalog(名前, モデルの並び)` か拡張の表の `add_material_catalog` で足す。カタログは AGF の
  カタログと同じに名前で引く（.zmx の GCAT・プロジェクトの `glass_catalogs`・`材料@カタログ`、大文字小文字は問わない。
  フォルダの同じ名前の AGF より先で、システムファイルを読むときは注意を返す。同梱のカタログ SCHOTT・OHARA と同じ名前は
  `Extensions` を入れる時（`install`・`ext`・`check`）の誤り）。屈折率は組み立てた時でなく求める時の拡張の表で決まる（系はモデルを持たない。
  表に無ければ材料とカタログの名前を言う誤り）。系のガラスの表に分散式の枝の 1 つとして入るので、dn・disp・範囲の外の注意・
  内部透過率はガラスと同じに効く。約束は、`index` が系の空気に対する相対屈折率を系の温度・気圧で返す（空気・温度の
  換算はモデルが持つ。屈折率を環境に合わせないときは `tref` と 1 atm で呼ぶ）、状態を持たずどのスレッドから呼んでも
  よい。システムファイルにはカタログの名前だけを書き（hash は空）、カタログを足していないプロセスで読むとカタログが
  無い誤り。.zmx に書くと GCAT にカタログの名前が入るが、OpticStudio には無い（書き出しの注意に挙げる）。`check_material` で約束を確かめられる
- `lensbench_core::usercoating`（SDK では `lensbench_sdk::coating`）: 膜のカタログに外から足す口。膜の中身は今ある型の
  `CoatingSet`（層の並びとその材料 `mate`・`coat`、TABLE の `table`、特性行列のモデル `models`、実測表 `measured`。
  `CoatingSet::parse_dat` で COATING.DAT の形式の文から作れる）。2 つの口がある:
  - **膜の一覧**: `Extensions::coating_catalog(名前, 膜の組)`（拡張の表では `add_coating_catalog`）。中の膜は .zmx の
    面の膜の名前で引く（大文字小文字は問わない）。2 つの一覧に同じ名前の膜があると `Extensions` を入れる時の誤り
  - **膜のファイル形式**: `Extensions::coating_source(..)`（`CoatingSource`: 拡張子と `load(パス) → CoatingSet`）。
    .zmx の膜ファイルと同じフォルダの `<膜の名前>.<拡張子>` を、同梱の `.lbcb` などと同じに膜の名前で読む（同梱の
    形式が先。同梱と同じ拡張子を足すと足した方で読む）。CLI の `--coatings` などで明示して読むファイルも拡張子で選ぶ

  膜の名前を引く順（上ほど先）: 系に書いた表 → 明示して読んだモデル・実測表（`--coatings`）→ 登録した膜の一覧 →
  .zmx の膜ファイル（COATING.DAT）の COAT・TABLE → 膜のフォルダのファイル → 共有の置き場。拡張を入れると、同じ .zmx
  でも COATING.DAT の同じ名前の膜が一覧の膜に替わる。一覧の膜と同じ名前の膜がほかにあれば注意を返す（`Coatings::notes`、
  CLI の `coatings` の `notes`。どの膜を一覧から入れたかは `registered_coatings`）。一覧の層の材料が膜ファイルの同じ名前の
  材料と違えば `<材料>@<一覧の名前>` に替えて入れる。膜の計算は同梱の膜と同じ（利用者が係数を計算する膜はまだ無い）。
  `check_coatings` で約束（膜の名前・材料の行・層の厚さ・表の角度の昇順など）を確かめられる

## パラメータ変更・光路長の微分・光線の解き

- `lensbench_core::param`: システムを作り直さずにパラメータを変える。面は番号か名前（コメント）で、`sys.param("L1.t")`・
  `sys.param("3.c")` のように指定し、`set_param`・`with_params`（軽い複製）で変える。種類は `c` `r` `k` `a2`〜`a16`
  `t` `dx` `dy` `tx` `ty` `tz`（座標ブレークならその値、他の面は前の傾き偏心）、`dn`・`disp`（媒質の屈折率のずれと
  分散の相対変化: n = n0 + dn + disp·(n0 − n0(主波長))）。変えたシステムの追跡は同じ値の .zmx を読み直したシステムとビット一致する。
  ほかの種類（半直径・開口・システムの値・ソルブの条件の値など、多重構成の表の行にするもの）は [パラメータの名前](params.md)
- `lensbench_core::edit`: システムの編集。`insert_surface`・`delete_surface`・`set_surface_fields`（名前と値の組）・
  `set_surface_type`・`set_material`・`add_pickup`・`add_solve`・多重構成の表 `Configs`（`set_config_row`・`config(k)`）。
  システムの欄は書き換えられない（読むのは `sys.surfaces` など、文書は `sys.prescription()`）。値を直すときは
  `sys.edit(|p| { p.surfaces[3].t = 5.0; Ok(()) })`（屈折率・ピックアップ・ソルブ・自動のクリア半径を作り直す）か、
  値をそのまま置く `edit_raw`。文書から作るのは `Prescription::build`・`build_raw`。媒質の約束は OpticStudio と同じ
  （面の後ろは面の材料、ミラー・座標ブレークは前の媒質）
- `lensbench_core::units`: 玉・群（面の範囲の組）の単位のパラメータ `lens2.tx`・`G.shift` など。剛体の偏心・傾きは
  座標系にかけ、`shift`・`dn`・`disp` は面のパラメータの組合せ。`flatten_units` で座標ブレークに展開（.zmx の書き出しで使う）
- `lensbench_core::fermat`: `opl_grad`・`trace_grad` で、1 回の追跡の交点から光線ごとの dOPL/dp を全パラメータ分まとめて
  求める（フェルマーの原理。始点と最後の面の交点を固定した微分）。物点の移動 `objx`・`objy` も
- `lensbench_core::solve`: `solve_ray` で、未知数 2 つ（物点の x・y か瞳座標）を、残差 2 つ（面での位置と目標の差、
  または 2 つの面での方向余弦の和・差）が 0 になるよう解く。ダブルパスで主光線を法線入射させる物点などに使う
