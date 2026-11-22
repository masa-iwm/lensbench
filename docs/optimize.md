# 最適化（減衰最小二乗）

## 変数と評価関数

- `optimize`: 変数（`--var`、`with_params` と同じ書き方。範囲は `"7.t=0.5:3"`、片側は `"7.t=0.5:"`）を減衰最小二乗
  （Levenberg–Marquardt）で最適化する。評価関数（`--merit`）は全視野（視野の重み）・多色の RMS スポット（`spot`）か
  RMS 波面（`wavefront`）。値は √(Σ 重み × RMS² / Σ 重み)（`rms` と同じ光線・重み・基準 `--reference`）。`--out` で最適化したシステムを .zmx に。

## 反復と微分

- ヤコビアンは、スポット・波面では解析的な微分（着地点の微分と光路長の微分）。変数で光線の始め方が変わる列
  （絞りより前の面・エイミング・実像高の視野など。変数ごとに確かめる）と MTF の項は中心差分。解析的な微分は列の数によらない手間が
  大きいので、項ごとに解析的に求められる列が 16 個より少なければ中心差分にする（そのほうが速い。Rust では `Problem::analytic_min`、0 で常に解析的）。`--jac fd` ですべて中心差分、`--jac forward` ですべて前進差分（差分の追跡が半分で済むが、誤差は刻みの 1 次）。
- 反復の設定（既定はどれも切ってある）: `--qr` は一歩を正規方程式でなく、列の大きさで無次元にした [J D⁻¹; √μ I] の QR で解く
  （条件数を 2 乗にしない。数学的には同じ一歩だが、条件の悪い問題では丸めの差で反復の道筋が変わる）。`--gtol g` は残差と変数ごとの
  列のなす角の余弦の最大が g 未満で止める（`small_gradient`。0 は判定しない）。`--active-set` は一歩で範囲を出る変数を境界に固定して
  残りを解き直す（既定は成分ごとに範囲へ切り詰める）。serve の `optimize` は `qr`・`gtol`・`active_set`、Python は同じ名前の引数、
  Rust は `DlsOpts`。

## 止まった光線・回転対称でないシステム

- 止まった光線（全反射・面に届かない）は、既定では項全体が評価できないエラーになる（最適化はその一歩を捨てて減衰を強める）。
  `--stopped-as-vignetted`（serve・Python は `stopped_as_vignetted`、Rust は `Problem::stopped_as_vignetted`）で、けられた光線と
  同じく除いて残りの重みで割り直す。どちらの場合も、瞳の光線を並べる項の結果の `rays` にけられた・止まった光線の数が出る。
- 回転対称でないシステムの波面は、射出瞳を実光線の外挿で求める（OpticStudio と同じ）ので評価関数に 1e-8 程度の揺れがあり、
  差分の刻みを大きめ（相対 1e-3）にする。偏心・傾きが 0 の点では射出瞳の求め方が切り替わるので、評価関数はわずかに不連続。

## 複数の構成と、反復を自前で回す口

- Rust の `optim::Problem` は複数の構成（多重構成・ズームの状態）と構成ごとの変数、MTF の項も扱う。`Problem::jacobian` は
  残差とヤコビアンだけを返す（反復を自前で回す使い方）。Python は `System.optimize(vars, merit=...)`（最適化したシステムは `"system"`）と
  `System.merit_jacobian(vars, ...)`。

## 大域の探索

減衰最小二乗は始点の近くの谷に落ちる。`--starts N`（serve・Python は `starts`、Rust は `optim::global_search`）は始点を変えて
最適化を N 回繰り返し、評価関数の良い解を残す（OpticStudio の Global Search・Hammer の、始点を変えて回す部分に当たる）。

- 始点 1 は今の値（範囲に切り詰めたもの）。始点 2 以降は、両側に範囲のある変数は範囲の中で一様に、範囲の無い側のある変数は
  今の値 ± `--spread`（既定 0.5）× max(|今の値|, 種類ごとの目安の大きさ) の中で一様に振り、範囲で切る。目安の大きさは曲率 0.01、
  半径 10、コーニック 1、dn 0.001、disp 0.01、ほかは 1。範囲を書いた変数のほうが振り方の意味がはっきりするので、変数に範囲を付けて使う。
- 乱数は (`--seed`, 始点の番号) で決まる列なので、同じ種なら並列の数・終わる順によらず同じ結果になる。
- 始点ごとの局所の最適化は並列に回す。進み具合は始点が終わるたびに、それまでの最良の評価関数の値を出す。止めると新しい始点を
  始めず、そこまでの最良の解を返す（`stop` は `cancelled`）。光線が通らない始点（面に届かないなど）は数えて捨てる
  （`--stopped-as-vignetted` を付けると、けられたとみなして評価を続ける）。
- 結果は最良の解（`x`・`merit` とシステム）と、`global_search`: 回した始点の数 `ran`・捨てた数 `failed`・最良の解の始点 `best_start`
  （0 は今の値）・残した解 `solutions`（良い順に `--keep` 個まで、既定 5。変数の差がどれも目安の大きさの 1e-4 倍以下の解は同じ解として 1 つに）。
  `merit0` は今の値の評価関数。
- 例（トリプレット A0 の曲率 6 つ、範囲 ±0.08）: 局所の最適化が RMS スポット 13.3 µm の谷に止まる始点から、始点 32 で 7.1 µm の谷を見つける。

```
lensbench optimize lens.zmx --var 1.c=-0.08:0.08 --var 2.c=-0.08:0.08 ... --starts 32 --seed 1 --stopped-as-vignetted
```

## ガラスの置き換え

`--glass-surfaces 1,3,6`（serve・Python は `glass: {"surfaces": [1, 3, 6], ...}`、Rust は `lensbench_design::glass_sub`）は、
最適化（大域の探索）の後に、並べた面の硝材をカタログの実在の硝材から選ぶ（OpticStudio のガラス置換、Hammer のガラスの入れ替えに当たる）。

- 面ごとに、今の媒質の実効の nd・vd（d・F・C 線、システムの温度・気圧で）に近い硝材を `--glass-candidates`（既定 8）個選び、
  それぞれに入れ替えたシステムで残りの変数を最適化し直して、評価関数の最も良い硝材を残す。近さは (Δnd / 0.01)² + (Δvd)²
  （nd の 0.01 を vd の 1 と同じに数える）。システムの波長が硝材の分散式の範囲の外になる硝材は候補にしない。
- 2 つの使い方を同じ設定で扱う:
  - **モデルガラスで設計してから実在の硝材へ寄せる**: モデルガラス（`"1.62,36"`）・屈折率だけの媒質・dn か disp が 0 でない面は、
    必ず候補のどれかに置き換える。硝材の屈折率のずれ `N.dn` と分散の変化 `N.disp` を変数にして最適化すれば、硝材の nd・vd を
    動かす設計（OpticStudio のモデルガラスの変数に当たる）になる。置き換えた面の dn・disp は 0 にし、その変数は以後の最適化から外す
    （結果の `x` では 0）。
  - **実在の硝材を入れ替えて比べる**: 硝材の面は今の硝材も候補に入れ、評価関数が良くなるときだけ入れ替える。
- 面を順に 1 つずつ置き換え（前の面の置き換えの結果から次を始める）、どの面も変わらなくなるまで `--glass-passes`（既定 2）周まで繰り返す。
- 候補のカタログは `--glass-catalog SCHOTT`（複数可、serve・Python は `catalogs`）。カタログは `--glass-dir`（serve は読込の
  `glass_dirs`）、`LENSBENCH_GLASSCAT`、同梱の SCHOTT・OHARA の順に探す。省けばシステムに読み込んだ硝材から選ぶ。
  価格・入手性での絞り込みはまだ無い。多重構成の表の材料の行は見ない（どの構成にも同じ硝材を入れる）。
- 結果の `glass`: 置き換えの記録 `changes`（面・前の材料・選んだ硝材・前の実効の nd・vd・選んだ硝材の nd・vd・置き換えた後の評価関数）、
  面ごとの最後の材料 `materials`、評価した候補の数 `tried`、置き換える前の評価関数 `merit_before`。
- serve の `apply` と GUI の「システムに反映」は、置き換えた材料も当てる。選んだ硝材のカタログがシステムの硝材カタログに無いときは、
  serve は何も当てず `notes` に書く（先にカタログをシステムに足す）。
- 例（トリプレットの硝材をモデルガラス 1.64/58・1.60/38・1.64/58 にして最適化し、SCHOTT から選ぶ）: 凸の 2 枚は低分散の
  クラウン系、凹は高分散のフリント系の硝材になり、置き換えて最適化し直した RMS スポットはモデルガラスのときより小さくなった（9.9 → 5.3 µm）。

## 評価関数の項

`--term '{"type":"paraxial","q":"efl","target":50,"weight":10}'`（複数可）か `--terms 項の配列.json` で既定の評価関数に項を足す（`--merit none` なら足した項だけ）。種類:

- `spot`: アフォーカル像空間の系は解析のスポットと同じ角度
- `wavefront`
- `mtf`: `dir`: both（既定、残差 2 つ）・tan・sag・avg（タンジェンシャルとサジタルの平均、MTFA）。`kind`: diffraction（既定、回折 MTF）・square（矩形波の縞の応答。回折 MTF の奇数倍の周波数の級数 (4/π) Σ (−1)^k M((2k+1)ν)/(2k+1) を、使う波長の最大の遮断周波数まで足す。MSW*）・geometric（幾何 MTF、GMT*。`diff_limit`（既定 true）で視野・波長ごとの回折限界の MTF をかける）・huygens（像面の Huygens PSF の MTF、MTH*）
- `paraxial`: `q`: efl・pmag・enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・obsn・isna・amag・linv・petc・petz。値は System Data と同じ。efl・pimh は `wave` でその波長を主波長にした値、OpticStudio の EFFL・PIMH の Wave。amag（角倍率、AMAG）・linv（ラグランジュ不変量、絞りの後の n · 近軸主光線の傾き · 近軸周辺光線の高さ、LINV）・petc（ペッツバール曲率 −n′ Σ (n′ − n) c / (n n′)、PETC）・petz（その逆数、PETZ）・wlen（波長 [µm]、WLEN。.zmx の Wave 0 は波長 1）は `wave` の波長で近軸光線を追う（入射瞳は主波長のまま）。ミラーのある系の petc・petz と、絞りの前にミラーのある系の linv は項の誤り（OpticStudio で確かめていない）
- `ray`: 1 本の実光線の面 `surf` の局所座標での `q`: x・y・z・l・m・n、OpticStudio の REAX〜REAC 相当。`trax`・`tray`・`trar` は主波長の主光線からのずれ、TRAX・TRAY・TRAR。`anax`・`anay`・`anar` は方向余弦 l・m の主波長の主光線との差と √(Δl² + Δm²)、ANAX・ANAY・ANAR。`dxdx`・`dxdy`・`dydx`・`dydy` は交点 x・y の正規化瞳座標 px・py での微分（刻み 0.001 の中心差分）、DXDX〜DYDY。`fcgt`・`fcgs` は正規化瞳座標を y・x の向きに ±0.001 ずらした 2 本の光線が最も近づく点の、面からの z の距離（像面湾曲、FCGT・FCGS。OpticStudio は主光線と +0.001 の片側の差分で、ずらす向きが視野の向きと垂直でなければ 1e-4 ほど違う）。`opth` は面 `surf` までの光路長、OPTH。無限遠物体では同じ波長の主光線が面 1 を通る点を通り光線に垂直な面から測る。`gx`・`gy`・`gz`・`gl`・`gm`・`gn` はグローバル座標の交点と面の後の方向余弦、RAGX〜RAGC。`r` は交点の √(x² + y²)、REAR。`ang` は面の後の方向と z 軸の角 [rad]、RANG。`tanx`・`tany` は l/n・m/n、RETX・RETY。`nl`・`nm`・`nn` は交点での面の法線（−z 寄り）、RENA〜RENC。`aoi`・`aoe` は面の法線との入射角・射出角 [度]、RAID・RAED。`cosi`・`cose` はその cos、RAIN・RAEN。`to` を書くと面 `surf` から `to` まで: `opth` は面の間の光路長（PLEN。`surf` 0 は OPTH と同じ基準）、`aoi`・`aoe` は `min`・`max` と面ごとに比べてはみ出しを足す（MNRI・MXRI・MNRE・MXRE と同じ値）。`paraxial: true` は近軸光線（x・y を別々に近軸で追い、位置は頂点面、方向は傾き (u_x, u_y, 1) の向き、法線はその位置での面の法線。`q` は x・y・z・l・m・n・r・nl・nm・nn・tanx・tany、PARX〜PATY。角度の視野の無限遠物体か物体高の視野）
- `opd`: 1 本の光線の OPD [波]、OPDC 相当
- `zernike`: 係数 [波]
- `coupling`: ファイバ結合効率
- `thickness`: 面 `surf` から `to` までの中心の厚みの和。OpticStudio の CTVA・TTHI、`min`・`max` で CTGT・CTLT。`edge: true` で縁の厚み（面と次の面の矢高を、それぞれの面の形が終わる半径（クリア半径 + チップゾーン）× `zone`（既定 1）の高さで測る。`clear: true` はクリア半径 × `zone` で測る。`axis` は矢高を測る向き（面の局所座標の `"+y"`（既定）・`"+x"`・`"-y"`・`"-x"`）。ETVA・ETGT・ETLT（Mode 0・1、Code 0〜3））。`each` は和でなく面ごとに `min`・`max` と比べてはみ出しを足す（`all`・`air`・`glass` の面だけ。MNCT・MNCA・MNCG・MNET・MNEA・MNEG・MX* と同じ値）
- `calc`: `op`: sum・diff・prod・div・abs・sqrt（√|a|）・reci・max・min・qsum（√Σa²）・sin・cos・tan・asin・acos・atan（ラジアン）を、`args` の数か 1 つの値の項（weight・config・min・max は書かない）に。OpticStudio の SUMM・OSUM・DIFF・PROD・DIVI・ABSO・SQRT・RECI・MAXX・MINN・QSUM・SINE〜ATAN。0 で割る・asin や acos の定義域の外は項の誤り。OpticStudio は 0 で割ると 1e13 や 1e20 の値を返す
- `surface`: 面 `surf` の量 `q`: `curv` 曲率・`conic` コーニック・`sag` 点 (`x`, `y`) の矢高・`sagx`・`sagy` 半直径での矢高・`thick` 点 (`x`, `y`) での次の面までの厚み・`total` 包む厚み（中心の厚み + max(0, 次の面の矢高) − min(0, 面の矢高)。矢高はそれぞれの面の形が終わる半径（クリア半径 + チップゾーン。その外で面は平ら）の点で、(`x`, `y`) は測る向き（既定 +y））・`total_clear` 矢高をクリア半径の点で測る `total`・`param` 面タイプのパラメータ（`param` に .zmx の PARM の番号。値を持たない番号は 0）・`diam` 直径・`mech_diam` 玉の直径（ガラスでつながる面の「縁 + チップゾーン」と固定した機械半径の最大の 2 倍。縁は円形の開口の面なら開口の最大の半径、ほかは半直径。無視した面は飛ばす。固定した機械半径が自動より小さければ使わない）・`gx`・`gy`・`gz` 頂点のグローバル座標・`ga`・`gb`・`gc` 面の局所座標の z 軸のグローバル座標での向き・`nd`・`vd`・`dpgf`・`cost`・`tce` 面の後の硝材の d 線の屈折率・アッベ数（カタログの温度の分散式から）・ΔPgF・相対価格・熱膨張係数（カタログの ED・OD の行。空気は 0。カタログに価格の無い硝材（OD の最初の値が −1 など）はその値のまま）・`surf_tce` 面の熱膨張係数（レンズデータの TCE の列、.zmx の TCED。硝材の面でも面の値で、設定しなければ 0。光線追跡には効かない。TCVA・TCGT・TCLT）・`power` 面のパワー (n′ − n) c・`ynip` 近軸周辺光線の y · n · i・`index` 面の後の屈折率（この 3 つは `wave` で波長を選ぶ。POWR・YNIP・INDX。ミラーの面の `power` と、その面までにミラーのある `ynip` は項の誤り）。OpticStudio の CVVA・COVA・SSAG・SAGX・SAGY・STHI・DMVA（Mode 1・0）・GLCX〜GLCZ・GLCA〜GLCC・GCOS・GTCE・TTVA（Code 0〜3 = +y・+x・−y・−x、Mode 0 は `total`・1 は `total_clear`）・PMVA、`min`・`max` で CVGT・CVLT・COGT・COLT・DMGT・DMLT・TTGT・TTLT・PMGT・PMLT。`to` を書くと面 `surf` から `to` までを面ごとに `min` か `max` と比べてはみ出しを足す（`semi`・`nd`・`vd`・`dpgf`・`cost`・`tce` だけ。硝材の量は硝材の面だけ数える。値は目標 ± Σ はみ出しで、OpticStudio の MNSD・MXSD・MNIN・MXIN・MNAB・MXAB・MNPD・MXPD）。半直径を読む `diam`・`mech_diam`・`sagx`・`sagy`・`total`・`total_clear` と縁の厚み・体積の項があれば、自動の半直径を評価のたびに求め直す
- `volume`: 面 `surf` から `to`（既定 `surf`）の要素の体積 [cm³]。面 i の要素は面 i と i + 1 の間で、空気の間も含む。`mass: true` で硝材の要素だけの質量 [g]（密度はカタログの ED の行）。外径は 2 面の半径の大きい方（既定は玉の半径 = `mech_diam` の半分で、面はクリア半径 + チップゾーンの外で縁の高さのまま平ら。`clear: true` は外径をクリア半径にし、面はクリア半径の外で平ら）。面の局所座標で数値積分する。`cylinder: true` は要素の形でなく、面 `surf` から `to` + 1 を包む最小の円柱（長さは頂点の間の厚みの和、半径は面の半径の最大。矢高は入れない）の体積 [cm³]。OpticStudio の VOLU・TMAS（Mode 0・1）・CVOL（面 Surf1〜Surf2 は `surf` = Surf1、`to` = Surf2 − 1。値はレンズ単位³）。ミラーのある系と、無視する面・座標ブレークに接する要素は項の誤り
- `tsag`: 面 `surf` の局所座標の点 `point`（[x, y, z]、既定 0）から、z 軸を `tilt`（x・y・z 軸まわりの角 [deg]、既定 0。向きは Rz(−tz) Ry(−ty) Rx(−tx) ẑ で、tx が正なら +y へ、ty が正なら −x へ傾く）だけ回した向きに面まで測った距離 [レンズ単位]（面が向きの先なら正）。`flat: true` は玉の半径がクリア半径より大きい面を、クリア半径 + チップゾーンの外で縁の矢高のまま平らにする。OpticStudio の TSAG（Mode 0・1）。線が面に当たらなければ項の誤り（傾き 0 の線が球に当たらない点では OpticStudio は曲率半径を返す）
- `bfs`: 面 `surf` の y の向きの断面に、半径 `rmin`（既定 0）から `rmax`（既定 0 はクリア半径）で当てはめた最適球の量 `q`: `curv` 曲率・`radius` 曲率半径（平面は 0。`min`・`max` では小さい値として効く）・`vertex` 球の頂点の z・`depth` 削る深さの最大・`volume` 削る体積 [レンズ単位³]・`slope` 面と球の傾き dz/dr の差の最大・`depth_rms`・`slope_rms` 深さ・傾きの差の面積の重みの RMS。球は面の空気の側に置き（面の後がガラスなら前）、面との差が 0 以上で削る体積が最小になる曲率を 4001 点の台形で探す。OpticStudio の BFSD（Data 0〜7）。曲率は 1.6e-5、ほかの量は 2.7e-3 の相対差で合う
- `cardinal`: 面 `from` から `to`（既定は面 1 から像面の手前の面まで）の近軸の系の量 `q`: `efl` 空気換算の焦点距離 1/パワー・`f_object`・`f_image` 媒質の中の焦点距離・`focal_*` 焦点・`principal_*` 主点・`anti_principal_*` 反主点・`nodal_*` 節点・`anti_nodal_*` 反節点（`*` は `object`・`image`。物体空間の点は面 `from` の頂点から、像空間の点は面 `to` の頂点から測る）。`wave` で波長。OpticStudio の CARD（Data 0〜11）・EFLX・EFLY。.zmx の EFLA は面 Surf と Surf + 1 が空気中の単レンズのときだけ読む（ほかは OpticStudio が 1e8 を返す）。X の向き（EFLX・CARD の Orientation 1）は回転対称の系だけ読む
- `fnum`: 視野 `field`・波長 `wave` の実光線の F ナンバー 1/(n′(sin θ₊ + sin θ₋))。θ± は主光線と瞳の縁の 2 本の光線の像空間での角度で、`dir` が `tan`（既定）なら瞳の y の縁 (0, ±1)、`sag` なら x の縁 (±1, 0)。OpticStudio の TFNO・SFNO
- `seidel`: 面 `surf`（0・省略は全部の面の和）のザイデル収差係数 [波]: `q` が `spha` W040 = S_I/8λ・`coma` W131 = S_II/2λ・`asti` W222 = S_III/2λ・`fcur` W220 = S_IV/4λ・`dist` W311 = S_V/2λ。近軸周辺光線と最大の視野へ合わせた近軸主光線から（主光線は主波長の入射瞳の中心を通す）。`wave` で波長。OpticStudio の SPHA・COMA・ASTI・FCUR・DIST（面の寄与）。不変量 H は物体空間の n (ū y − u ȳ) で、OpticStudio は絞りの n ū y を使うので、主波長でない FCUR・DIST は少し違う（Double Gauss で最大 0.9%）。標準面・偶数次非球面（r⁴ の項まで）だけで、ミラーのある系は項の誤り
- `distortion`: 視野 `field`（既定は正規化視野 [0, 1]）の主光線の像面の点 p と、基準の視野 `reference` から線形に予測した点の差 d: |d|（符号は d と予測の変位の内積の符号で、像が予測より内側なら負。近軸像高が負の倒立した像でも同じ）[レンズ単位] を `absolute: true` で、既定は予測の変位の長さに対する %。予測は基準の視野の主光線の点 p_r + J (t − t_r) で、t は角度の視野なら各向きの角の tan、ほかは視野の座標、J は基準の視野での ∂p/∂t（グリッド歪曲の A・B・C・D）。基準が省略・軸上の視野なら近軸主光線の倍率で予測する（回転対称な系だけ）。`peak: true` は軸上から `field` までの線を 100 等分した点の絶対値の最大、`ftheta: true` は y の向きの視野を 100 等分した角 θ の主光線の高さ y に原点を通る y = cθ を最小二乗で当てたずれの最大（|y − cθ|、% は |y − cθ| / |cθ|。角度の視野だけ）。`wave` で波長。`coef`（`a`〜`d`）は基準の視野での J の成分 [[a, b], [c, d]]（ABCD）。`abcd: [A, B, C, D]` は J の代わりに使う係数で、`component` が `x`・`y` なら差と予測の変位のその成分で測る（既定 `radial`。DISA）。OpticStudio の DIST の Surf 0 と Absolute、DISG（Wave が負で長さ）、DIMX、DISC、ABCD、DISA。Double Gauss 28 で 1e-10 で一致
- `longitudinal`: 軸上の光線が光軸を横切る点の像面の頂点からの距離 [レンズ単位]。`zone` 0（既定）は近軸周辺光線、0 < `zone` ≤ 1 は正規化瞳座標 (0, `zone`) の実光線。`wave` で波長、`wave2` があればその波長の距離 − `wave` の距離（軸上色収差）。OpticStudio の LONA・AXCL
- `lateral_color`: 最大の視野の近軸主光線の像面での高さの `wave2` − `wave` [レンズ単位]。OpticStudio の LACL
- `semi`: 面 `surf` から `to`（既定 `surf`）の半直径を面ごとに `min` か `max` と比べ、はみ出しを足す。OpticStudio の MNSD・MXSD。自動の半直径は評価のたびに求め直す
- `energy`: 視野 `field`・波長 `wave`（省略は重みのある全波長）の包囲エネルギー。`size_at` は光量のその割合が入る大きさ [レンズ単位]、`fraction_at` はその大きさに入る光量の割合。`shape` は `circle`（既定、半径）・`x`・`y`（帯の半幅）・`square`（正方形の半幅）、`center` は `chief`（既定、主波長の主光線）・`centroid`・`vertex`（像面の原点）・`middle`（点の広がりの中心。geometric だけ）。`method` は `geometric`（既定、瞳の `n` × `n`（既定 256）の升の中心の光線。波長ごとの光量は波長の重み）・`fft`・`huygens`（像面の PSF の画素の光量を画素の中に一様に広げて数える）。`airy: true` は geometric の円で、点ごとに視野・波長ごとの回折限界のエアリー像の半径 r の円の割合をかける。OpticStudio の GENC・GENF・DENC・DENF
- `strehl`: FFT の PSF のピークの、無収差の PSF のピークに対する比。`wave` 省略は多色。STRH
- `param`: 系のパラメータ `name`（変数と同じ書き方: `6.t`・`3.c`・`wave1`・`aper` など）の、項の構成 `config` の系での値。.zmx の MCOV・MCOG・MCOL は多重構成の行 Op# のパラメータの構成 Cfg# の項にする（行は THIC・SDIA・CRVT・CONN・PRAM・WAVE・WLWT・PRWV・APER・XFIE・YFIE・FLWT・FVDX・FVDY・FVCX・FVCY・FVAN・FTAN・SATP・STPS・IGNR・APMX・APMN・APDX・APDY・APTP・CBDX〜CBTZ・CBOR・CADX〜CATZ・CAOR・MIND・MABB・MDPG・TCEX・MCSD・CHZN・CSP1・CSP2・TSP1〜TSP3・PSP1〜PSP3・TEMP・PRES・AFOC・APDT・APDF・RAAM・OPDR）。書き出しはしない
- `spread`: 1 つの値の項 `term`（weight・config・min・max は書かない）の、問題のすべての構成での値の最大 − 最小。`max` を付けた厚みの項の `spread` は OpticStudio の ZTHI（Surf2 が像面なら像面の手前まで）。微分は差分で、変数ごとにすべての構成の系をずらす
- `illum`: 視野 `field`・波長 `wave`（既定は主波長）の主光線が像面に当たる点に届く光の、像空間の向きの射影立体角 Ωp から。`q` が `relative` なら軸上の視野に対する相対照度（RELI）、`fno` なら実効 F ナンバー 1/(2 √(Ωp n′² / π))（EFNO）。瞳の縁の各点を通って像点に着く光線を視野を変えて解き、その向きの像面の接平面への成分が囲む面積を Ωp にする。けられ係数は使わず、面の開口でけられる向きは瞳の半径を縮めて縁を探す。`n` は縁の点の数（既定 128）。アフォーカルの系は項の誤り
- `comment`: `text` の文だけのコメントの行。OpticStudio の BLNK。項の並びを読むときに飛ばし（誤りの「term N」の番号は数える）、.zmx に書き出すと BLNK の行になる

視野・波長は 1 始まり（視野は `[hx, hy]` も可）、目標 `target` の既定は 0（MTF・結合効率は 1）。目標が 1 つの値の項（mtf の tan・sag・avg、paraxial・ray・opd・zernike・coupling・energy・strehl・illum など）は `target` の代わりに片側の条件 `min`（値 ≥ min なら残差 0）・`max`（値 ≤ max なら残差 0）を書ける。キーの一覧は `lensbench_design::merit`、スキーマは [terms](schema/terms.schema.json)（種類ごとに使えるキーだけを受け、知らないキーはエラー）。結果の `terms` に足した項の前後の値。

- 光線（z を除く）・OPD の項はスポット・波面と同じく解析的に微分し、近軸量・Zernike・結合効率は差分。
- Zernike の項は `fit` 項で当てはめた係数（既定は `term` と 37 の大きい方。格子の上の最小二乗では当てはめる項の数で低い次の係数が動くので、既定は項を多くとった値に寄せる）。OpticStudio 26.1 の ZERN は Fringe がいつも 37 項、Standard が隣り合う ZERN の行（Term のほかが同じ）の一番大きい Term と 11 の大きい方で当てはめる。`--zmx-merit` で読んだ ZERN の行は `fit` をそれに合わせるので OpticStudio と同じ値になる（Double Gauss 28 の視野 2 の Standard Z11 は 11 項で 0.04221、37 項で 0.04100 波）。

## .zmx のメリットファンクション

- .zmx のメリットファンクション: `--zmx-merit` で .zmx に保存したメリットファンクション（面の後の行）を項にして使う（`--merit` のデフォルトのメリットファンクションは足さない。`--term`・`--terms` の項はその後に足す）。
- 写すオペランドは EFFL・ENPP・EPDI・EXPD・EXPP・ISFN・ISNA・PIMH・PMAG・WFNO・OBSN・TOTR（波長は主波長だけ）・WLEN、RSCE・RSCH・RWCE・RWCH（目標 0 だけ）、 OPDC、ZERN（Fringe・Standard）、MTFT・MTFS・MTFA（FFT の変調）・MSWT・MSWS・MSWA・GMTT・GMTS・GMTA・MTHT・MTHS・MTHA（Grid 0・1 を同じ量として読み、Data Type 0 だけ）、GENC・GENF・DENC・DENF（大きさは µm を mm に）・STRH・RELI・EFNO（標本は読まず lensbench の既定）、TCVA・TCGT・TCLT、MCOV・MCOG・MCOL、ZTHI、TSAG・BFSD、REAX〜REAC、CENX・CENY（像面）、面の量（CVGT〜COVA・DMGT〜DMVA・SAGX・SAGY・SSAG・STHI・GLCX〜GLCZ。192 行で相対 1.8e-15 一致）、厚み（CTGT・CTLT・CTVA・TTHI・ETGT・ETLT・ETVA と範囲の境界 MNCT〜MNEG。Double Gauss 28 の 943 行で 6e-14 mm 一致。物体面からの TTHI と Surf1 > Surf2 の範囲は飛ばす）、計算の行（SUMM・OSUM・DIFF・PROD・DIVI・ABSO・SQRT・RECI・CONS・MAXX・MINN・QSUM・SINE〜ATAN・OPVA を `calc` の項に、ABGT・ABLT は abs の `calc` の項の `min`・`max` に。参照する行は引数に入れ子にし、行の番号 0 は行 1、範囲の外・後ろの行・BLNK は 0 と OpticStudio に合わせる。片側の条件の行（CTGT・OPGT など）を参照する行と、レンズ単位が mm でない .zmx の計算の行は飛ばす）と、それらを参照する OPGT・OPLT （`min`・`max`）。
- CONF n の後の行は構成 n の項で、使う構成をまとめて最適化する。
- 重み 0 の行は飛ばし、写せない行・負の重み（OpticStudio のラグランジュの拘束）は行の番号つきの警告にして残りで最適化する。
- 長さの目標はレンズ単位から mm に直す。
- RSCE・RSCH・RWCE・RWCH は `quadrature: "opticstudio"` の項にし、瞳の求積を OpticStudio 26.1 と同じにする（腕 2·Ring 本を 90° 回して置く。Double Gauss 28 の Ring 2〜6・6 視野・各波長で 1e-10 で一致。Ring 1 の Hx = 0 の RSCE だけは数 % 違う（OpticStudio 2026 R1.01 はこのときだけ重心を引かず、RSCH と同じ値を返す。Hx を 1e-6 ずらすと重心を引いた値になる）。アフォーカル系のちょうど軸上の RSCE も OpticStudio だけ別の計算で Ring ごとに値が揃わない（Hy を 1e-4 ずらすと Ring 3〜6 が揃い lensbench と一致）。どちらも OpticStudio 側の例外の扱いと見て、lensbench は合わせない。多色の RWCE は波長ごとの傾きの除き方が違い、Ring 2 以上で 1.4e-5、Ring 1 では大きく違う。OpticStudio の RSCE などは面の開口でけらないが、lensbench の項はけった光線を除く。けられのある系では値が違う）。
- RSRE・RSRH・RWRE・RWRH（目標 0 だけ）は `quadrature: "grid"` の項（`n` は 2·Samp）にし、Double Gauss 28 などで 1e-11 で一致する（Hx ≠ 0 の視野の RSRH と、多色の RWRE の軸外 1e-5 は違う。前者は読むと警告を出す。OpticStudio 2026 R1.01 の RSRH は格子の Px > 0 の半分の瞳だけで計算していて（左右対称を仮定した近道）、x にずれた視野では対称でなくなるので値が狂う。回転対称の系で視野 (0.6, 0)・(0, 0.6)・(−0.6, 0) が 0.00797・0.00699・0.00585 と違い、瞳全体なら 0.00724 で lensbench と同じ。lensbench は合わせない）。
- TRAI は面 `surf` の `trar`、ANAX〜ANAR・DXDX〜DYDY は像面の `anax`〜`dydy`（Wave 0 の ANAX・ANAY は OpticStudio では 0 だが、主波長として読み、警告を出す）、SPCH は帯 Zone と帯 0 の `longitudinal` の差の `calc` の項にする。
- DIMX・DISG・DISC・ABCD・DISA は `distortion` の `peak`・`reference`・`ftheta`・`coef`・`abcd` の項、BSER は軸上の視野の像面の `r` を近軸の `efl` で割る `calc` の項、FCGS・FCGT は像面の `fcgs`・`fcgt` にする（DISG の Px・Py は OpticStudio でも値を変えないので読まない）。
- 像の質の行は OpticStudio の標本を上げると lensbench の値に近づく（Double Gauss 28 の 3 視野・3 波長と多色で、MTF* は Samp 6 で 3.1e-4、MSW* は Samp 5 で 1.8e-3（標本を 1 段上げるごとに差が半分）、GMT* は 1.2e-4、STRH は OpticStudio が Samp 4 でもまだ動いていて 6e-3、RELI は 1e-5〜1.8e-4、EFNO は OpticStudio の標本の 1/Samp の誤りを外挿して 3.6e-5、GENC・GENF は 1.5e-3）。
- `--out` の .zmx には STRH を Samp 5、RELI・EFNO を Samp 100 で書く（それでも OpticStudio の EFNO は 1/Samp の誤りで 0.5% ほど違う）。
- MTH* は OpticStudio の Samp 1〜3 の値が大きくずれる（像の格子が小さい）ので、lensbench の回折 MTF との差（8.6e-3）で確かめた。
- DENC・DENF は OpticStudio が主光線に垂直な面で測り、lensbench は像面で測るので軸外で違う（視野 3 で 1.2%。主光線に垂直な面の PSF で計算すると 3e-3〜6.2e-3 で一致）。
- 原点が基準の GENC・GENF（Refp 2）は OpticStudio の値が 0.6% 大きく、理由が分からない（読むと警告を出す）。OpticStudio 2026 R1.01 では、像の広がりが 11 µm の系で Frac を動かすと値が約 240 µm の幅で変わり、GENF が 1 を超える（最大 1.108）ので、OpticStudio 側の誤りと見て合わせない。
- XENC・XENF・MECS などの MEC*・ERFP は読まない。
- 行の流れを変える行も OpticStudio 26.1 の実測どおりに読む: ENDX の後の行は項にしない。
- GOTO n は自分の行から行 n の手前までを飛ばす（n が自分の行以下なら何も飛ばさない）。
- FDMO は FDRE（または最後）まで、視野の番号で指す行の視野を Hx・Hy（正規化視野）に変える（Hx・Hy を直接書いた行は変えない。けられ係数は使わず、0 でなければ警告を出す）。
- lensbench の項の既定（`quadrature: "lensbench"`、腕 4·rings + 2 本）は少ない rings でも収束に近く、Ring が小さいと OpticStudio の値と違う（DG28 の RSCH の Ring 2 で 14%）。
- 評価関数の Wizard の行（DMFS の後の「Operands for field N.」の OPDX・TRAR・TRAC・TRAD・TRAE・TRCX・TRCY）は、構成・視野ごとにまとめてスポット・波面の項 1 つ（x+y の行は `axis` が x と y の 2 つ）にし、重みを行の重みの和にする。
- 瞳の点は DMFS の次の BLNK の設定文から、GQ は `quadrature: "opticstudio"` と `arms` （腕を 0° から等間隔に）、RA n × n は `quadrature: "grid"` と `n`（n × n の升の中心で瞳の中の点を等しい重みで）。
- Ignore Lateral Color（PRIM の行）は波長ごとの単色の項にする。
- 境界の行（MNCA など）も項にするので、評価関数の値は OpticStudio と同じになる（DG28 の Wizard の 7 つの .zmx で、スポットは 3e-13、多色の波面は 7.7e-6。Wizard の設定は .zmx にこの文しか残らない）。
- Assume Axial Symmetry の Wizard は瞳の半分だけの行を並べるが、項は瞳全体の点を使う（回転対称な系では同じ値）。
- PTV など RMS でない Wizard の行は 1 行ずつ読む。
- MTF の標本は渡さず、 lensbench が収束する求積点の数を選ぶ。
- `--out` の .zmx には、読んだメリットファンクションの行（写せなかった行も含めそのまま）と、 `--term`・`--terms` の項を写した行を面の後に書く（OpticStudio で開くとメリットファンクションに入る）。
- 項を写せないもの（coupling・plugin・名前で指定した zplm・pwfn、正規化視野で書いた mtf・zernike・centroid）は警告にして書かない。
- `min`・`max` は重み 0 の行とそれを参照する OPGT・OPLT の 2 行に、MTF の both は MTFT・MTFS の 2 行にする。
- 読んだ .zmx のレンズ単位が mm でなければ読んだ行は書かない（警告を出す）。
- 読めなかった行は番号を保つため BLNK にして元の文字をコメントに残す。
- 構成が 2 つ以上のときは構成 1 の系を書き、多重構成の表は読んだ値のまま（表を書けない系では構成 2 以降の行を書かない）。
- 近軸量の行の波長は主波長の番号で書く。
- Rust は `lensbench_formats::zmx::merit`（行）、`lensbench_design::merit_zmx`（項と行の写し）、 `lensbench_formats::zmx_write::save_with_merit`。

## 自前の項（Python・ZPL マクロ・プラグイン）

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
