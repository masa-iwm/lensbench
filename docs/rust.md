# ライブラリ（Rust）

## 入口: 読む・追跡する・解析する

`use lensbench_core::prelude::*;` で主な口がそろう。それ以外の `pub` のモジュールは上の層のクレートと拡張のための口で、
版の間で変わりやすい。

```rust,ignore
use lensbench_core::prelude::*;

lensbench_core::init_from_env(); // LENSBENCH_GLASSCAT・LENSBENCH_THREADS など（使わないなら呼ばなくてよい）
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

別のリポジトリのクレートから解析・プラグイン・検出器・利用者定義の面・カタログの形式を足し、自分の exe・wheel を組める
（DLL は使わない。静的にリンクする）。依存するのは `lensbench-sdk` 1 つ。

- 足すものは `lensbench_sdk::Extensions` 1 つにまとめる: `.plugins(..)`・`.analysis(..)`・`.detector(..)`・
  `.surface_model(..)`・`.catalog_source(..)`・`.messages(訳の表)`。GUI の見せ方は `lensbench_sdk::gui::GuiExtensions`
  （`gui_analysis`・`gui_detector`・`gui_translations`）、Python の関数は `lensbench_sdk::py::PyExtensions`（`py_module`）。
- 入口に渡す: CLI は `lensbench_sdk::cli::main(ext)`、GUI は `lensbench_sdk::gui::main(ext)`、Python の拡張モジュールは
  `#[pymodule]` から `lensbench_sdk::py::module(m, ext)`。呼ぶ順の約束は無い。
- 2 つの拡張を `merge` すると、同じ名前のもの（解析・検出器・プラグイン・面のモデル・カタログの拡張子）は誤りになる
  （置き換えるときは後の方に `.replacing()`）。`Extensions::named(..)` の名前は誤りの文にだけ使い、サブコマンドなど
  利用者に見える名前には入らない。
- 足し忘れは `lensbench_sdk::check(&ext)` で見つかる（GUI の見せ方の無い解析・欄の無い検出器・反転の約束を守らない面）。
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
- Web 版: 雛形の `web`（wasm32 だけ）が `#[wasm_bindgen(start)]` で拡張を入れる（wasm-bindgen は
  `lensbench_sdk::web::wasm_bindgen`）。組み立ては同じタグの公開リポジトリで
  `CRATE=<パッケージ名> MANIFEST=<外の Cargo.toml> scripts/build_web.sh <出力先>`。画面用（feature `gui`）・
  計算の Web Worker 用（`worker`・`threads`）の wasm を作り、公開の Web 版のページ・JS と一緒に置く
  （[Web 版](web.md)）。同梱のファイルは `lensbench_sdk::web::set_extra`。

## パラメータ変更・光路長の微分・光線の解き

- `lensbench_core::plugin`: 検出器・解析のプラグインの約束。`Plugin`（名前・種類・`prepare(&Config)`）と
  `Instance`（`run(&Context, &Arrays) -> Arrays`）を実装し、`Registry` に名前で登録する。`run` は状態を持たず、
  同じ `Instance` を複数スレッドから呼んでよい（`run_many`）。システムを追跡するものは `Context` でシステムと膜を受け取る。
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

- `lensbench_core::param`: システムを作り直さずにパラメータを変える。面は番号か名前（コメント）で、`sys.param("L1.t")`・
  `sys.param("3.c")` のように指定し、`set_param`・`with_params`（軽い複製）で変える。種類は `c` `r` `k` `a2`〜`a16`
  `t` `dx` `dy` `tx` `ty` `tz`（座標ブレークならその値、他の面は前の傾き偏心）、`dn`・`disp`（媒質の屈折率のずれと
  分散の相対変化: n = n0 + dn + disp·(n0 − n0(主波長))）。変えたシステムの追跡は同じ値の .zmx を読み直したシステムとビット一致する
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
