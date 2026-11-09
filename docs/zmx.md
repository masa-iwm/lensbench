# .zmx の読込の約束（OpticStudio との違い）

- レンズ単位が mm 以外（`UNIT` の IN・CM・METER）の .zmx は、読込で mm に直す（長さの値・偶数次非球面の係数・厚さのソルブの値・
  入射瞳径・高さの視野・多重構成の THIC・SDIA・CRVT）。結果も mm で出る。換算の決まっていない面タイプ・データは読込で止める
- 浮動開口（`FLAP`）の外の光線をける（OpticStudio と同じ）。機械的な半直径の行（`MEMA`）のない .zmx（OpticStudio 以外が書いたもの）では、
  半直径を固定（`DIAM 値 1`）にした玉の面（前か後ろがガラス）を浮動開口にする（OpticStudio 2026 R1 も読込で `FLAP` を足してける。
  空気どうしの面と `MEMA` のある .zmx には足さない）。浮動開口でけないようにするには `lensbench_core::trace::set_floating_clip(false)` か
  環境変数 `LENSBENCH_FLOATING_CLIP=0`。光線エイミングの追跡は開口を見ない
- 視野がすべて 0（軸上だけ）の系では、正規化視野 (hx, hy) を与えても軸上のまま追う（OpticStudio と同じ）
- 多重構成の `PRAM`（面の PARM）は構成ごとに読む。面の色 `SCOL`・面の熱膨張係数 `TCED`、構成の名前 `MCOM`・重み `CWGT` は読み飛ばす
- Gradium（GRINSUR8）・Gradient 9 の近軸は、小さい実光線の極限（媒質の中の近軸の受け渡し）。OpticStudio の近軸（EFFL、曲率の F 値ソルブ）は
  これと 1% 程度違う系がある（付属のサンプルの Gradium の系で EFL 37.50 と 37.89）。実光線は面の形が同じなら一致する
- 読まないもの: 非順次の部品（`NSOH`）、回折・ホログラム・ユーザー定義・複屈折の面タイプ
- 面タイプ Tilted（`TILTSURF`、z = x tan α + y tan β、局所座標は傾けない）・Odd Asphere（`ODDASPHE`、標準面 + Σ β_i r^i）・
  Irregular（`IRREGULA`、偏心・傾けた標準面 + Zs ρ⁴ + Za ρ_y² + Zc ρ² ρ_y、ρ は半直径で正規化、ρ_y は角度で回した y）を読む。
  物体面が Tilted なら物点はその面の上。実光線と EFFL は OpticStudio と 1e-11 mm 以下で一致（`tests/surftypes_os`）。
  Odd Asphere の r の項（β_1、円錐の頂点）があると、射出瞳（実光線で求める）と OPD が OpticStudio と違う
- 座標ブレークの主光線のソルブ（`PPAR` の元の面が負）は、保存された値で固定する

