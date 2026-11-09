# 書き出し

- `layout`: レイアウトを `--out` の拡張子の形式で書き出す（複数可）。
  - 2D（大域の yz 面、`--plane xz` も可。`--unfold` で鏡・BS の折り曲げを伸ばした展開図）: `svg`・`png`（`--width` px）・`dxf`（R12、レイヤー LENS・MIRROR・SURFACE・RAYS）
  - 3D: `gltf`（バッファ埋め込み）・`glb`（m 単位）、`obj`・`stl`（mm）。STL は線を持てないので光線は細い管（`--tube` で半径）
  - CAD 用: `step`・`stp`（AP214）。玉と鏡は B-rep の立体（球面・平面はそのまま、非球面は回転した B スプライン面、
    玉の中に偏心・傾きがあれば側面は線織面）、光線は折れ線、像面は縁の円。傾いた 2 面の側面は、前の面の x を後ろの面に
    投げた向きから同じ角度の縁の点どうしを結ぶ（3D の網・2D の断面も同じ組）
  - 読み戻しの試験（`tests/readback`、CI の export-readback）: SVG（XML）・glb（trimesh）・STEP（OpenCascade）・
    追跡の CSV・JSON を別の実装で読み、要素の数、面の点が .zmx の形と OpticStudio の面の大域座標（GLRS）の上にあること、
    立体が閉じていて体積が .zmx から求めたものと合うこと（STEP 1e-6、glb は網の粗さの分）、光線の折れ点が追跡の大域座標と
    合うこと、追跡の表が OpticStudio の光線（tests/refs の A0・C0）と 1e-9 mm で合うことを確かめる。
    手元では `pip install -r tests/readback/requirements.txt` の後 `cargo build --profile ci -p lensbench-cli && pytest tests/readback`
  - 光線は `--field`・`--wave`（`all` も可）・`--fan`（y・x・cross・grid・ring）・`--rays`・`--color`（field・wave）、描く範囲は `--first`・`--last`。
    既定は 2D が y 扇 7 本、3D・STEP が主光線と瞳の縁の円周 12 本。
    ガラスを挟む 2 面ごとに 1 つの立体（外径は 2 面の半径の大きい方）。ダブルパスで同じ玉を 2 回通っても 1 つだけ描く
  - 重ね描き: `--cfgs 1,2,3`（.zmx の多重構成、`all` も可）、`--states 名前,…`・`--paths T,R,…`（プロジェクトの状態・
    BS の経路。`all` で全部、両方指定すると組合せ）で複数の構成を 1 枚に描く。同じ位置・形の玉・鏡・面は 1 つだけ描き、光線は既定で
    構成ごとの色（`--color cfg`）。`--offset dx,dy,dz` [mm] で構成 k を k 倍ずらして並べる（OpticStudio のレイアウトの構成ごとの
    オフセットと同じ。ずらすと玉も構成ごとに描く）。立体・面・光線の名前の頭に構成の名前（`C2`・状態名・経路）が付く
- `zmx`: 系を .zmx に書き出す（プロジェクトの展開など）。`--paths`・`--states`・`--cfgs` を付けると、構成ごとに 1 構成の多重構成の
  .zmx にまとめる。面は「全部の構成で同じ先頭の面」＋「構成ごとの残りの面」＋像面の順に並べ、ほかの構成の面は多重構成の
  `IGNR`（無視する面。厚さ・座標ブレークも効かない）で外す。先頭の面の厚さの違いは `THIC`、絞りの違いは `STPS`、像面の半径の違いは
  `SDIA`。視野・波長・開口などの系の値は全部の構成で同じでなければならない。ピックアップ・ソルブは値にして書く
  - OpticStudio 26.1 との照合（`tests/bs_configs`、`examples/bs_configs.rs`・`make_cfg_refs.py`）: BS の経路（キューブ・平板（表・裏）の T・R、
    Twyman-Green 2 例・Michelson の 4 経路）と状態（P6、玉の抜き差しと物体距離。P7、.zmx の群の移動・偏心・傾き）の 8 系 25 構成で、
    各構成の像面の位置 4e-13 mm・方向 1e-13・光路長 6e-11 mm（けられの有無も一致。無限遠の物体の軸外の視野の光路長は、数え始める面が
    違うので比べない）。多重構成の行は OpticStudio が書く形（値の後に `0 0 0 1 1 倍率 オフセット 0 "" 0`）。
    倍率を落とすと OpticStudio はエラーなしに値を読み違える
- `rays`: 光線の面ごとの交点（局所・大域）・方向余弦・光路長・けられ（1 行 = 1 光線 × 1 面）。`--pupil "0,1;0.5,0"` で瞳座標を直接、
  `--surface` で面を選ぶ。`--save` を省くと CSV を標準出力へ
- `--save`（info・spot・wavefront・zernike・mtf・mtf-focus・mtf-field・rms・psf・geo-mtf・elements・layout）: 結果を `json`（そのまま）・`csv`・`npz`（表にし、配列は `coef_1`… の列に）で書き出す。
  npz は列ごとの 1 次元配列（`numpy.load` で読める）

`mtf` は回折 MTF（変調度、子午・球欠）。`--wave` を省くと多色（波長の重みで平均）、`--freqs` を省くと 0 から遮断周波数の
0.95 倍までの 21 点。瞳関数の自己相関を、ずらした 2 つの瞳の重なりで直接求積する（OPD は瞳の格子を 1 度追跡して補間）。求積の点の数は
OPD の傾きから自動で決める（`--n` で指定も可）。OpticStudio の FFT MTF と同じく瞳の座標（主光線に垂直な面）で計算するので、
像面の傾き・回転は考えない。照合は `tests/mtf_refs.rs`（OpticStudio の FFT MTF の sampling 512・256 と 24 系で 3e-4〜2e-3）。
`--each-wave` で多色に加えて波長ごとの MTF（`waves`）も出す（波長ごとの計算を使い回すので別々に求めるより約 2 倍速い）。
`--fft` は瞳関数（直径 512〜4096 点の格子）の自己相関を FFT で全周波数まとめて求める: 時間が周波数の数によらないので、
周波数が多いとき（目安 100 点超）や収差の大きい系で速い。求積との差は 24 系で 1e-4 以下（`examples/mtf_fft.rs`）。
`--n` を省いて自動で決めた求積の点の数が 1024 を超える系（収差が大きい系。E0・E1 の軸外）で、周波数が 8 個以上のときは、
既定でも FFT に切り替える（E1 の既定の 21 点で約 1.8 倍速い。周波数が少ないときは求積の方が速い）。切り替えずに求積で求めるときは `--interp`。
時間は `examples/mtf_bench.rs`（全視野、多色＋各波長、21 点）。

MTF のまわりの解析（`tests/mtf_family.rs` で既存の MTF・スポット・波面と突き合わせ済み。OpticStudio との照合はこれから）:

- `mtf-focus`（Through Focus MTF 相当）: 像面を `--range` の ±範囲で `--steps` 点ずらした回折 MTF。ずらしは像面の手前の面の厚さに
  足す（厚さのソルブは解き直さない）。
- `mtf-field`（MTF vs Field 相当）: 正規化視野 0〜1 を `--points` 点（`--axis y`・`x`）。OpticStudio と同じくケラレ係数は使わない（`rms --vs field` も）。
- `rms`（RMS vs Field・Focus・Wavelength 相当）: `--vs field|focus|wavelength`。量はスポット半径（`spot`・`spot_x`・`spot_y`）、
  波面収差 `wfe` [波]、ストレール比の近似 `strehl` = exp(−(2π wfe)²)。`--ref centroid`（スポットは重心、波面はピストンと傾きを除く）か
  `chief`（スポットは主光線、波面はピストンだけ除く）。瞳はガウス求積（`--rings`）。波面は各波長の主光線を基準にし、多色は
  分散とストレール比を波長の重みで平均。`wavelength` は系の波長の最小〜最大を単色で（その波長を重み 0 で足す。主波長・視野は変えない）。
  `--field-wave`（Python は `field_wave=True`）で、単色の `field`・`wavelength` の掃引の実像高（と条件で定める視野）の視野を解析の波長の主光線で
  解く（OpticStudio の単色の RMS vs Field・vs Wavelength と同じ。既定はスポット・演算子・多色と同じく主波長で解く）。
- `psf`（FFT PSF 相当）: MTF と同じ瞳関数を行列の DFT で、主光線に垂直な面の格子へ（OpticStudio の FFT PSF と同じ。斜めの視野では像面上より 1/cos 細かい）。強度は収差のない同じ瞳の PSF の中心を 1 とする
  （ピークがストレール比の目安）。像の格子の間隔は既定で 1 / (2 × 遮断周波数)（多色は最も短い波長）、格子の幅は瞳の標本の
  周期（既定の間隔なら `--image` ≤ 2 × `--pupil`）まで。
- `geo-mtf`（Geometric MTF 相当）: 像面の光線の位置の分布の特性関数（多色は全波長の光線をまとめて）。`--diff-limit` で回折限界の MTF をかける。
- `psf --huygens`・`huygens-mtf`（Huygens PSF・Huygens MTF 相当）: 射出瞳の参照球の各点を 2 次波源とし、像面（傾き・曲率込み）の
  格子の各点まで距離どおりに足す（瞳から像への写像を近似しない）。座標は像面の局所座標、MTF の周波数も像面の上。傾いた像面・収差の大きい系・
  斜めの視野向け。MTF は PSF の格子の調和周波数の値を補間するので、格子が PSF を覆いきれないと低周波が下がる（`--image`・`--delta` で広げる）。
  OpticStudio の Huygens PSF とは瞳 256 で D0 3e-3・A0 2e-2（OS は瞳の標本を増やすほど lensbench に近づく）、像の間隔を固めた Huygens MTF とは 3e-3
  （`tests/huygens_refs`、許容差と理由は `crates/lensbench-core/tests/huygens_refs.rs`）。OS の自動の Huygens MTF は像の格子が狭く、低周波が高く出る。
- 横軸のある点で計算できない（瞳の縁がけられるなど）ときはその点を空（NaN）にして `warnings` に理由を残す。
- OpticStudio 26.1 との照合は `tests/refs/<系>/mtf_family.json`・`psf_os.npz`（24 系、許容差と理由は `crates/lensbench-core/tests/mtf_family_refs.rs`）。
- Python は `System.mtf`（`method="fft"`・`each_wave=True`）・`through_focus_mtf`・`field_mtf`・`rms`（`vs=`）・`psf`・`geometric_mtf`・`huygens_psf`・`huygens_mtf`。

## 例

A0 のレイアウト図（`--out layout.png`）。

{{#include gen/layout.md}}
