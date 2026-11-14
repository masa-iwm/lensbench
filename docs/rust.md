# ライブラリ（Rust）: パラメータ変更・光路長の微分・光線の解き

- `lensbench_core::plugin`: 検出器・解析のプラグインの約束。`Plugin`（名前・種類・`prepare(&Config)`）と
  `Instance`（`run(&Context, &Arrays) -> Arrays`）を実装し、`Registry` に名前で登録する。`run` は状態を持たず、
  同じ `Instance` を複数スレッドから呼んでよい（`run_many`）。系を追跡するものは `Context` で系と膜を受け取る。
  多くの方向の微分を流す解析は、任意の `linearize`（`run` と同じ出力と、入力の微分 1 方向から出力の微分を作る `Linear`）を
  実装すると、呼ぶ側は解析を 1 回で済ませられる（既定は None）。
  同梱のプラグインはワークスペース内の crate（`lensbench-analysis`）で、CLI・Python に組み込む（`lensbench plugins`）
- `lensbench_core::usersurf`: 利用者が定義する面の口。形は頂点の平面に限り、`SurfaceModel`（名前・`prepare(&Config)`）の
  `SurfaceInstance::deflect`（交点・入射方向 → 出射方向・OPD に足す光路長）と `paraxial_power`（近軸の屈折力）を実装して
  `usersurf::register` で登録する。面は種類 `USER` に `user = {"model": 名前, "config": {…}}`（系ファイル・`set_surface_data`・GUI の
  「利用者定義の面」）。偏光は理想レンズと同じく方向の変化の最小の回転、dOPL/dp・着地点の微分は未対応、.zmx には書けない、前後は空気。
  同梱のモデル `ideal_mapping`: 画角 θ → 像高 h を与える理想レンズ（`f`、`mapping` = `tan`（f tan θ、理想レンズとビット同じ）・`theta`（f θ）・
  `sin`（f sin θ）・`equisolid`（2f sin(θ/2)）・`stereographic`（2f tan(θ/2)）・`poly`（h = f(θ + c₁θ³ + c₂θ⁵ + …)、`coef`）・
  `table`（`theta_deg`・`height` の表を PCHIP で補間））。同じ方向の光線は焦点面の (h(θ), f) に無収差で集まる（物体は無限遠を想定）

- `lensbench_core::param`: 系を作り直さずにパラメータを変える。面は番号か名前（コメント）で、`sys.param("L1.t")`・
  `sys.param("3.c")` のように指定し、`set_param`・`with_params`（軽い複製）で変える。種類は `c` `r` `k` `a2`〜`a16`
  `t` `dx` `dy` `tx` `ty` `tz`（座標ブレークならその値、他の面は前の傾き偏心）、`dn`・`disp`（媒質の屈折率のずれと
  分散の相対変化: n = n0 + dn + disp·(n0 − n0(主波長))）。変えた系の追跡は同じ値の .zmx を読み直した系とビット一致する
- `lensbench_core::edit`: 系の編集。`insert_surface`・`delete_surface`・`set_surface_fields`（名前と値の組）・
  `set_surface_type`・`set_material`・`add_pickup`・`add_solve`・多重構成の表 `Configs`（`set_config_row`・`config(k)`）。
  面を直接書き換えたときは `refresh()` で屈折率・ピックアップ・ソルブを作り直す。媒質の約束は OpticStudio と同じ
  （面の後ろは面の材料、鏡・座標ブレークは前の媒質）
- `lensbench_core::units`: 玉・群（面の範囲の組）の単位のパラメータ `lens2.tx`・`G.shift` など。剛体の偏心・傾きは
  座標系にかけ、`shift`・`dn`・`disp` は面のパラメータの組合せ。`flatten_units` で座標ブレークに展開（.zmx の書き出しで使う）
- `lensbench_core::fermat`: `opl_grad`・`trace_grad` で、1 回の追跡の交点から光線ごとの dOPL/dp を全パラメータ分まとめて
  求める（フェルマーの原理。始点と最後の面の交点を固定した微分）。物点の移動 `objx`・`objy` も
- `lensbench_core::solve`: `solve_ray` で、未知数 2 つ（物点の x・y か瞳座標）を、残差 2 つ（面での位置と目標の差、
  または 2 つの面での方向余弦の和・差）が 0 になるよう解く。ダブルパスで主光線を法線入射させる物点などに使う
