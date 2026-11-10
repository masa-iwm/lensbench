# 最適化（減衰最小二乗）

- `optimize`: 変数（`--var`、`with_params` と同じ書き方。範囲は `"7.t=0.5:3"`、片側は `"7.t=0.5:"`）を減衰最小二乗
  （Levenberg–Marquardt）で最適化する。評価関数（`--merit`）は全視野（視野の重み）・多色の RMS スポット（`spot`）か
  RMS 波面（`wavefront`）。値は √(Σ 重み × RMS² / Σ 重み)（`rms` と同じ光線・重み・基準 `--ref`）。`--out` で最適化した系を .zmx に。
- ヤコビアンは、スポット・波面では解析的な微分（着地点の微分と光路長の微分）。変数で光線の始め方が変わる列
  （絞りより前の面・エイミング・実像高の視野など。変数ごとに確かめる）と MTF の項は中心差分。`--jac fd` ですべて中心差分、`--jac forward` ですべて前進差分（差分の追跡が半分で済むが、誤差は刻みの 1 次）。
- 回転対称でない系の波面は、射出瞳を実光線の外挿で求める（OpticStudio と同じ）ので評価関数に 1e-8 程度の揺れがあり、
  差分の刻みを大きめ（相対 1e-3）にする。偏心・傾きが 0 の点では射出瞳の求め方が切り替わるので、評価関数はわずかに不連続。
- Rust の `optim::Problem` は複数の構成（多重構成・ズームの状態）と構成ごとの変数、MTF の項も扱う。`Problem::jacobian` は
  残差とヤコビアンだけを返す（反復を自前で回す使い方）。Python は `System.optimize(vars, merit=...)`（最適化した系は `"system"`）と
  `System.merit_jacobian(vars, ...)`。
- 項の追加: `--term '{"type":"paraxial","q":"efl","target":50,"weight":10}'`（複数可）か `--terms 項の配列.json` で
  既定の評価関数に項を足す（`--merit none` なら足した項だけ）。種類は `spot`・`wavefront`・`mtf`・`paraxial`（`q`: efl・pmag・
  enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・obna・isna。値は System Data と同じ）・`ray`（1 本の実光線の面 `surf` の
  局所座標での `q`: x・y・z・l・m・n、OpticStudio の REAX〜REAC 相当）・`opd`（1 本の光線の OPD [波]、OPDC 相当）・
  `zernike`（係数 [波]）・`coupling`（ファイバ結合効率）。視野・波長は 1 始まり（視野は `[hx, hy]` も可）、目標 `target` の
  既定は 0（MTF・結合効率は 1）。目標が 1 つの値の項（mtf の tan・sag、paraxial・ray・opd・zernike・coupling）は `target` の代わりに
  片側の条件 `min`（値 ≥ min なら残差 0）・`max`（値 ≤ max なら残差 0）を書ける。キーの一覧は `lensbench_project::merit`。結果の `terms` に足した項の前後の値。
- 光線（z を除く）・OPD の項はスポット・波面と同じく解析的に微分し、近軸量・Zernike・結合効率は差分。
- Python で定義する項（UDOP 相当）: `System.optimize(vars, merit=None, terms=[{"type": "user", "fn": f}])`。`f(systems)` は
  System の list を受け取り、系ごとの残差を (系の数, 残差の数) の配列で返す。差分のヤコビアンではずらした系をまとめて
  1 回で渡す。`"jac": g`（`g(system, names)` が (残差の数, 変数の数) を返す）を与えると微分はそれを使う。
  `merit_jacobian` も `terms=` を受け付ける（Gauss–Newton などを自前で回す使い方）。Rust は `optim::UserTerm`。
- ZPL マクロの項（ZPLM 相当）: `{"type": "zplm", "macro": 17, "data": 0, "target": 0}` は ZPL17.ZPL を走らせ、
  `OPTRETURN 0, 値` の値 − target を残差にする（`macro` は名前でも、`file` で .ZPL のパスでも）。マクロは系を読む関数・命令
  （CURV・THIC・RAYTRACE・RAYX など、Macro Solve と同じ解釈系）と、項の `hx`・`hy`・`px`・`py` を返す PVHX()・PVHY()・
  PVPX()・PVPY() を使える。探す順は `file`、`macro_dirs`、系に読み込んだマクロ、LENSBENCH_MACROS、今のフォルダ。serve を `--root`・`--allow-remote` で
  起動したときは、`file`・`macro_dirs` も `--root` の下だけで、LENSBENCH_MACROS・今のフォルダは探さない。JSON で書けるので
  CLI・serve・GUI（最適化・公差のパネル）・Python のどこでも使える。止まったときなどは OpticStudio 26.1.1 の実測に合わせる:
  `OPTRETURN data` を呼ばなかった番号は 0、範囲外の RAYTRACE はそこで黙って終わりそれまでの値、実行時のエラー（未定義の変数など）・
  値が有限でないときは評価できない（OpticStudio は評価関数の全体を 9.0E+09 にする。公差では失敗の試行）。lensbench が対応していない
  命令・関数と文法のエラーはエラーにする（OpticStudio は知らない命令で黙って止まり、文法のエラーでは何も走らせず 0）。
- プラグインの項: `{"type": "plugin", "name": "rms", "field": 1, "output": "wfe", "args": {"rings": 4}, "target": 0}` は
  登録したプラグイン（同梱の解析、非公開版や Python で足した分）に系を渡し、出力 `output` の配列（`index` でその要素だけ）
  − target を残差にする。`args` は設定、`inputs` は入力の配列（`field` は `inputs` の field の略）。残差の数は読込のときに 1 回評価して決める。

## 例

カタログレンズ 3 枚の組（examples/projects/P9_catalog_three.json）の 2 つの空気間隔を RMS スポットで最適化する。

系:

{{#include gen/sys-P9.md}}

{{#include gen/optimize.md}}
