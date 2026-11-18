# 最適化（減衰最小二乗）

- `optimize`: 変数（`--var`、`with_params` と同じ書き方。範囲は `"7.t=0.5:3"`、片側は `"7.t=0.5:"`）を減衰最小二乗
  （Levenberg–Marquardt）で最適化する。評価関数（`--merit`）は全視野（視野の重み）・多色の RMS スポット（`spot`）か
  RMS 波面（`wavefront`）。値は √(Σ 重み × RMS² / Σ 重み)（`rms` と同じ光線・重み・基準 `--reference`）。`--out` で最適化したシステムを .zmx に。
- ヤコビアンは、スポット・波面では解析的な微分（着地点の微分と光路長の微分）。変数で光線の始め方が変わる列
  （絞りより前の面・エイミング・実像高の視野など。変数ごとに確かめる）と MTF の項は中心差分。解析的な微分は列の数によらない手間が
  大きいので、項ごとに解析的に求められる列が 16 個より少なければ中心差分にする（そのほうが速い。Rust では `Problem::analytic_min`、0 で常に解析的）。`--jac fd` ですべて中心差分、`--jac forward` ですべて前進差分（差分の追跡が半分で済むが、誤差は刻みの 1 次）。
- 反復の設定（既定はどれも切ってある）: `--qr` は一歩を正規方程式でなく、列の大きさで無次元にした [J D⁻¹; √μ I] の QR で解く
  （条件数を 2 乗にしない。数学的には同じ一歩だが、条件の悪い問題では丸めの差で反復の道筋が変わる）。`--gtol g` は残差と変数ごとの
  列のなす角の余弦の最大が g 未満で止める（`small_gradient`。0 は判定しない）。`--active-set` は一歩で範囲を出る変数を境界に固定して
  残りを解き直す（既定は成分ごとに範囲へ切り詰める）。serve の `optimize` は `qr`・`gtol`・`active_set`、Python は同じ名前の引数、
  Rust は `DlsOpts`。
- 止まった光線（全反射・面に届かない）は、既定では項全体が評価できないエラーになる（最適化はその一歩を捨てて減衰を強める）。
  `--stopped-as-vignetted`（serve・Python は `stopped_as_vignetted`、Rust は `Problem::stopped_as_vignetted`）で、けられた光線と
  同じく除いて残りの重みで割り直す。どちらの場合も、瞳の光線を並べる項の結果の `rays` にけられた・止まった光線の数が出る。
- 回転対称でないシステムの波面は、射出瞳を実光線の外挿で求める（OpticStudio と同じ）ので評価関数に 1e-8 程度の揺れがあり、
  差分の刻みを大きめ（相対 1e-3）にする。偏心・傾きが 0 の点では射出瞳の求め方が切り替わるので、評価関数はわずかに不連続。
- Rust の `optim::Problem` は複数の構成（多重構成・ズームの状態）と構成ごとの変数、MTF の項も扱う。`Problem::jacobian` は
  残差とヤコビアンだけを返す（反復を自前で回す使い方）。Python は `System.optimize(vars, merit=...)`（最適化したシステムは `"system"`）と
  `System.merit_jacobian(vars, ...)`。
- 項の追加: `--term '{"type":"paraxial","q":"efl","target":50,"weight":10}'`（複数可）か `--terms 項の配列.json` で
  既定の評価関数に項を足す（`--merit none` なら足した項だけ）。種類は `spot`・`wavefront`・`mtf`・`paraxial`（`q`: efl・pmag・
  enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・obsn・isna。値は System Data と同じ。efl・pimh は `wave` でその波長を主波長にした値、OpticStudio の EFFL・PIMH の Wave）・`ray`（1 本の実光線の面 `surf` の
  局所座標での `q`: x・y・z・l・m・n、OpticStudio の REAX〜REAC 相当。`trax`・`tray`・`trar` は主波長の主光線からのずれ、TRAX・TRAY・TRAR。`opth` は面 `surf` までの光路長、OPTH。無限遠物体では同じ波長の主光線が面 1 を通る点を通り光線に垂直な面から測る。`gx`・`gy`・`gz`・`gl`・`gm`・`gn` はグローバル座標の交点と面の後の方向余弦、RAGX〜RAGC。`r` は交点の √(x² + y²)、REAR。`ang` は面の後の方向と z 軸の角 [rad]、RANG。`tanx`・`tany` は l/n・m/n、RETX・RETY。`nl`・`nm`・`nn` は交点での面の法線（−z 寄り）、RENA〜RENC。`aoi`・`aoe` は面の法線との入射角・射出角 [度]、RAID・RAED。`cosi`・`cose` はその cos、RAIN・RAEN。`to` を書くと面 `surf` から `to` まで: `opth` は面の間の光路長（PLEN。`surf` 0 は OPTH と同じ基準）、`aoi`・`aoe` は `min`・`max` と面ごとに比べてはみ出しを足す（MNRI・MXRI・MNRE・MXRE と同じ値））・`opd`（1 本の光線の OPD [波]、OPDC 相当）・
  `zernike`（係数 [波]）・`coupling`（ファイバ結合効率）・`thickness`（面 `surf` から `to` までの中心の厚みの和。OpticStudio の CTVA・TTHI、`min`・`max` で CTGT・CTLT。`edge: true` で縁の厚み（面と次の面の矢高を、それぞれの半直径 × `zone`（既定 1）の高さで測る。ETVA・ETGT・ETLT）。`each` は和でなく面ごとに `min`・`max` と比べてはみ出しを足す（`all`・`air`・`glass` の面だけ。MNCT・MNCA・MNCG・MNET・MNEA・MNEG・MX* と同じ値））。`calc`（`op`: sum・diff・prod・div・abs・sqrt（√|a|）・reci・max・min・qsum（√Σa²）・sin・cos・tan・asin・acos・atan（ラジアン）を、`args` の数か 1 つの値の項（weight・config・min・max は書かない）に。OpticStudio の SUMM・OSUM・DIFF・PROD・DIVI・ABSO・SQRT・RECI・MAXX・MINN・QSUM・SINE〜ATAN。0 で割る・asin や acos の定義域の外は項の誤り）。`surface`（面 `surf` の量 `q`: `curv` 曲率・`conic` コーニック・`sag` 点 (`x`, `y`) の矢高・`sagx`・`sagy` 半直径での矢高・`thick` 点 (`x`, `y`) での次の面までの厚み・`diam` 直径・`mech_diam` 玉（ガラスでつながる面）の最大の直径・`gx`・`gy`・`gz` 頂点のグローバル座標。OpticStudio の CVVA・COVA・SSAG・SAGX・SAGY・STHI・DMVA（Mode 1・0）・GLCX〜GLCZ、`min`・`max` で CVGT・CVLT・COGT・COLT・DMGT・DMLT。半直径を読む `diam`・`mech_diam`・`sagx`・`sagy` と縁の厚みの項があれば、自動の半直径を評価のたびに求め直す）。視野・波長は 1 始まり（視野は `[hx, hy]` も可）、目標 `target` の
  既定は 0（MTF・結合効率は 1）。目標が 1 つの値の項（mtf の tan・sag、paraxial・ray・opd・zernike・coupling）は `target` の代わりに
  片側の条件 `min`（値 ≥ min なら残差 0）・`max`（値 ≤ max なら残差 0）を書ける。キーの一覧は `lensbench_design::merit`、スキーマは [terms](schema/terms.schema.json)（種類ごとに使えるキーだけを受け、知らないキーはエラー）。結果の `terms` に足した項の前後の値。
- 光線（z を除く）・OPD の項はスポット・波面と同じく解析的に微分し、近軸量・Zernike・結合効率は差分。
- Zernike の項は `fit` 項で当てはめた係数（既定は `term` と 37 の大きい方。格子の上の最小二乗では当てはめる項の数で低い次の
  係数が動くので、既定は項を多くとった値に寄せる）。OpticStudio 26.1 の ZERN は Fringe がいつも 37 項、Standard が隣り合う
  ZERN の行（Term のほかが同じ）の一番大きい Term と 11 の大きい方で当てはめる。`--zmx-merit` で読んだ ZERN の行は `fit` を
  それに合わせるので OpticStudio と同じ値になる（Double Gauss 28 の視野 2 の Standard Z11 は 11 項で 0.04221、37 項で 0.04100 波）。
- .zmx のメリットファンクション: `--zmx-merit` で .zmx に保存したメリットファンクション（面の後の行）を項にして使う
  （`--merit` のデフォルトのメリットファンクションは足さない。`--term`・`--terms` の項はその後に足す）。写すオペランドは
  EFFL・ENPP・EPDI・EXPD・EXPP・ISFN・ISNA・PIMH・PMAG・WFNO・OBSN・TOTR（波長は主波長だけ）、RSCE・RSCH・RWCE・RWCH（目標 0 だけ）、
  OPDC、ZERN（Fringe・Standard）、MTFT・MTFS（FFT の変調）、REAX〜REAC、CENX・CENY（像面）、面の量（CVGT〜COVA・DMGT〜DMVA・SAGX・SAGY・SSAG・STHI・GLCX〜GLCZ。192 行で相対 1.8e-15 一致）、厚み（CTGT・CTLT・CTVA・TTHI・ETGT・ETLT・ETVA と範囲の境界 MNCT〜MNEG。Double Gauss 28 の 943 行で 6e-14 mm 一致。物体面からの TTHI と Surf1 > Surf2 の範囲は飛ばす）、計算の行（SUMM・OSUM・DIFF・PROD・DIVI・ABSO・SQRT・RECI・CONS・MAXX・MINN・QSUM・SINE〜ATAN・OPVA を `calc` の項に、ABGT・ABLT は abs の `calc` の項の `min`・`max` に。参照する行は引数に入れ子にし、行の番号 0 は行 1、範囲の外・後ろの行・BLNK は 0 と OpticStudio に合わせる。片側の条件の行（CTGT・OPGT など）を参照する行と、レンズ単位が mm でない .zmx の計算の行は飛ばす）と、それらを参照する OPGT・OPLT
  （`min`・`max`）。CONF n の後の行は構成 n の項で、使う構成をまとめて最適化する。重み 0 の行は飛ばし、写せない行・負の重み
  （OpticStudio のラグランジュの拘束）は行の番号つきの警告にして残りで最適化する。長さの目標はレンズ単位から mm に直す。
  RSCE・RSCH・RWCE・RWCH は `quadrature: "opticstudio"` の項にし、瞳の求積を OpticStudio 26.1 と同じにする（腕 2·Ring 本を
  90° 回して置く。Double Gauss 28 の Ring 2〜6・6 視野・各波長で 1e-10 で一致。Ring 1 の Hx = 0・Hy ≠ 0 の RSCE だけは
  OpticStudio の置き方が分からず数 % 違う。多色の RWCE は波長ごとの傾きの除き方が違い、Ring 2 以上で 1.4e-5、Ring 1 では大きく
  違う）。lensbench の項の既定（`quadrature: "lensbench"`、腕 4·rings + 2 本）は少ない rings でも収束に近く、Ring が小さいと
  OpticStudio の値と違う（DG28 の RSCH の Ring 2 で 14%）。評価関数の Wizard の行（DMFS の後の「Operands for field N.」の
  OPDX・TRAR・TRAC・TRAD・TRAE・TRCX・TRCY）は、構成・視野ごとにまとめてスポット・波面の項 1 つ（x+y の行は `axis` が x と y の
  2 つ）にし、重みを行の重みの和にする。瞳の点は DMFS の次の BLNK の設定文から、GQ は `quadrature: "opticstudio"` と `arms`
  （腕を 0° から等間隔に）、RA n × n は `quadrature: "grid"` と `n`（n × n の升の中心で瞳の中の点を等しい重みで）。Ignore
  Lateral Color（PRIM の行）は波長ごとの単色の項にする。境界の行（MNCA など）も項にするので、評価関数の値は OpticStudio と
  同じになる（DG28 の Wizard の 7 つの .zmx で、スポットは 3e-13、多色の波面は 7.7e-6。Wizard の設定は .zmx にこの文しか残らない）。Assume Axial Symmetry の Wizard は瞳の半分だけの行を
  並べるが、項は瞳全体の点を使う（回転対称な系では同じ値）。PTV など RMS でない Wizard の行は 1 行ずつ読む。MTF の標本は渡さず、
  lensbench が収束する求積点の数を選ぶ。`--out` の .zmx には、読んだメリットファンクションの行（写せなかった行も含めそのまま）と、
  `--term`・`--terms` の項を写した行を面の後に書く（OpticStudio で開くとメリットファンクションに入る）。項を写せないもの
  （coupling・plugin・名前で指定した zplm・pwfn、正規化視野で書いた mtf・zernike・centroid）は警告にして書かない。`min`・`max`
  は重み 0 の行とそれを参照する OPGT・OPLT の 2 行に、MTF の both は MTFT・MTFS の 2 行にする。読んだ .zmx のレンズ単位が mm
  でなければ読んだ行は書かない（警告を出す）。読めなかった行は番号を保つため BLNK にして元の文字をコメントに残す。構成が 2 つ以上のときは構成 1 の系を書き、多重構成の表は読んだ値のまま（表を書けない系では構成 2 以降の行を書かない）。近軸量の行の波長は主波長の番号で書く。Rust は `lensbench_core::zmx::merit`（行）、`lensbench_design::merit_zmx`（項と行の写し）、
  `lensbench_formats::zmx_write::save_with_merit`。
- Python で定義する項（UDOP 相当）: `System.optimize(vars, merit=None, terms=[{"type": "user", "fn": f}])`。`f(systems)` は
  System の list を受け取り、システムごとの残差を (システムの数, 残差の数) の配列で返す。差分のヤコビアンではずらしたシステムをまとめて
  1 回で渡す。`"jac": g`（`g(system, names)` が (残差の数, 変数の数) を返す）を与えると微分はそれを使う。
  `merit_jacobian` も `terms=` を受け付ける（Gauss–Newton などを自前で回す使い方）。Rust は `optim::UserTerm`。
- ZPL マクロの項（ZPLM 相当）: `{"type": "zplm", "macro": 17, "data": 0, "target": 0}` は ZPL17.ZPL を走らせ、
  `OPTRETURN 0, 値` の値 − target を残差にする（`macro` は名前でも、`file` で .ZPL のパスでも）。マクロはシステムを読む関数・命令
  （CURV・THIC・RAYTRACE・RAYX など、Macro Solve と同じ解釈系）と、項の `hx`・`hy`・`px`・`py` を返す PVHX()・PVHY()・
  PVPX()・PVPY() を使える。探す順は `file`、`macro_dirs`、システムに読み込んだマクロ、LENSBENCH_MACROS、今のフォルダ。serve を `--root`・`--allow-remote` で
  起動したときは、`file`・`macro_dirs` も `--root` の下だけで、LENSBENCH_MACROS・今のフォルダは探さない。JSON で書けるので
  CLI・serve・GUI（最適化・公差のパネル）・Python のどこでも使える。止まったときなどは OpticStudio 26.1.1 の実測に合わせる:
  `OPTRETURN data` を呼ばなかった番号は 0、範囲外の RAYTRACE はそこで黙って終わりそれまでの値、実行時のエラー（未定義の変数など）・
  値が有限でないときは評価できない（OpticStudio は評価関数の全体を 9.0E+09 にする。公差では失敗の試行）。lensbench が対応していない
  命令・関数と文法のエラーはエラーにする（OpticStudio は知らない命令で黙って止まり、文法のエラーでは何も走らせず 0）。
- プラグインの項: `{"type": "plugin", "name": "rms", "field": 1, "output": "wfe", "args": {"rings": 4}, "target": 0}` は
  登録したプラグイン（同梱の解析、非公開版や Python で足した分）にシステムを渡し、出力 `output` の配列（`index` でその要素だけ）
  − target を残差にする。`args` は設定、`inputs` は入力の配列（`field` は `inputs` の field の略）。残差の数は読込のときに 1 回評価して決める。

## 例

カタログレンズ 3 枚の組（examples/projects/P9_catalog_three.json）の 2 つの空気間隔を RMS スポットで最適化する。

システム:

{{#include gen/sys-P9.md}}

{{#include gen/optimize.md}}
