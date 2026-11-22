# 解析（MTF・PSF・RMS・収差図）

CLI の MTF・PSF・RMS のまわりの解析と収差図（GUI の解析のパネル・Python・serve の `analyze` も同じ計算と引数）。
スポット・波面・Zernike などほかの解析の引数は [CLI のリファレンス](reference/cli.md)。

## 回折 MTF

`mtf` は回折 MTF（変調度、子午・球欠）。`--wave` を省くと多色（波長の重みで平均）、`--freqs` を省くと 0 から遮断周波数の
0.95 倍までの 21 点（遮断周波数は計算した波長の値、多色は主波長。結果の `cutoff` は `tangential`・`sagittal`）。瞳関数の自己相関を、ずらした 2 つの瞳の重なりで直接求積する（OPD は瞳の格子を 1 度追跡して補間）。求積の点の数は
OPD の傾きから自動で決める（`--n` で指定も可）。OpticStudio の FFT MTF と同じく瞳の座標（主光線に垂直な面）で計算するので、
像面の傾き・回転は考えない。子午・球欠の向きは `--axes`: `field`（既定）は子午が視野の向き（物理の子午面）、`pupil` は子午が瞳の y・球欠が x
（OpticStudio の FFT MTF と同じ）。違うのは x の成分のある視野だけで、そのような視野で OpticStudio と比べるときは `--axes pupil`
（`mtf-focus`・`mtf-field`・`geo-mtf`・`huygens-mtf` も同じ）。
`--polarization` でコーティングの透過とフレネル損失を瞳の振幅に入れる（偏光なしでシステムのコーティングを使う。OpticStudio の Use Polarization に当たる。
`psf`・`huygens-mtf` も）。瞳のアポダイゼーションは常に入る。照合は `crates/lensbench-verify/src/mtf_refs.rs`（OpticStudio の FFT MTF の sampling 512・256 と 24 本で 3e-4〜2e-3）。
`--each-wave` で多色に加えて波長ごとの MTF（`waves`）も出す（波長ごとの計算を使い回すので別々に求めるより約 2 倍速い）。
`--method fft` は瞳関数（直径 512〜4096 点の格子）の自己相関を FFT で全周波数まとめて求める: 時間が周波数の数によらないので、
周波数が多いとき（目安 100 点超）や収差の大きいシステムで速い。求積との差は 24 本で 1e-4 以下（`examples/mtf_fft.rs`）。
`--n` を省いて自動で決めた求積の点の数が 1024 を超えるシステム（収差が大きいシステム。E0・E1 の軸外）で、周波数が 8 個以上のときは、
既定でも FFT に切り替える（E1 の既定の 21 点で約 1.8 倍速い。周波数が少ないときは求積の方が速い）。切り替えずに求積で求めるときは `--method interp`、求積点の OPD をすべて直接追跡して確かめるときは `--method direct`。
時間は `examples/mtf_bench.rs`（全視野、多色＋各波長、21 点）。

## MTF のまわりの解析

既存の MTF・スポット・波面との突き合わせは `tests/mtf_family.rs`、OpticStudio との照合は下の最後の項:

- `mtf-focus`（Through Focus MTF 相当）: 像面を `--range` の ±範囲で `--points` 点ずらした回折 MTF。ずらしは像面の手前の面の厚みに
  足す（厚みのソルブは解き直さない）。
- `mtf-field`（MTF vs Field 相当）: 正規化視野 0〜1 を `--points` 点（`--axis y`・`x`）。OpticStudio と同じくケラレ係数は使わない（`rms --vs field` も）。
- `rms`（RMS vs Field・Focus・Wavelength 相当）: `--vs field|focus|wavelength`。量はスポット半径（`spot`・`spot_x`・`spot_y`）、
  波面収差 `wfe` [波]、ストレール比の近似 `strehl` = exp(−(2π wfe)²)。`--reference centroid`（スポットは重心、波面はピストンと傾きを除く）か
  `chief`（スポットは主光線、波面はピストンだけ除く）。瞳はガウス求積（`--rings`）。波面は各波長の主光線を基準にし、多色は
  分散とストレール比を波長の重みで平均。`wavelength` はシステムの波長の最小〜最大を単色で（その波長を重み 0 で足す。主波長・視野は変えない）。
  `--field-wave`（Python は `field_wave=True`）で、単色の `field`・`wavelength` の掃引の実像高（と条件で定める視野）の視野を解析の波長の主光線で
  解く（OpticStudio の単色の RMS vs Field・vs Wavelength と同じ。既定はスポット・演算子・多色と同じく主波長で解く）。
- `psf`（FFT PSF 相当）: MTF と同じ瞳関数を行列の DFT で、主光線に垂直な面の格子へ（OpticStudio の FFT PSF と同じ。斜めの視野では像面上より 1/cos 細かい）。強度は収差のない同じ瞳の PSF の中心を 1 とする
  （ピークがストレール比の目安）。像の格子の間隔は既定で 1 / (2 × 遮断周波数)（多色は最も短い波長）、格子の幅は瞳の標本の
  周期（既定の間隔なら `--image` ≤ 2 × `--pupil`）まで。
- `geo-mtf`（Geometric MTF 相当）: 像面の光線の位置の分布の特性関数（多色は全波長の光線をまとめて）。`--diff-limit` で回折限界の MTF をかける。
- `psf --huygens`・`huygens-mtf`（Huygens PSF・Huygens MTF 相当）: 射出瞳の参照球の各点を 2 次波源とし、像面（傾き・曲率込み）の
  格子の各点まで距離どおりに足す（瞳から像への写像を近似しない）。座標は像面の局所座標、MTF の周波数も像面の上。傾いた像面・収差の大きいシステム・
  斜めの視野向け。MTF は PSF の格子の調和周波数の値を補間するので、格子が PSF を覆いきれないと低周波が下がる（`--image`・`--pixel` で広げる）。
  OpticStudio の Huygens PSF とは瞳 256 で D0 3e-3・A0 2e-2（OS は瞳の標本を増やすほど lensbench に近づく）、像の間隔を固めた Huygens MTF とは 3e-3
  （`tests/huygens_refs`、許容差と理由は `crates/lensbench-analysis/tests/huygens_refs.rs`）。OS の自動の Huygens MTF は像の格子が狭く、低周波が高く出る。
  既定は速い近似（像の格子の区画ごとに瞳の各標本の位相と振幅を 2 次式で近似し、区画は位相の差が 1e-6 rad 以下になる大きさを自動で選ぶ。
  強度は厳密な和と収差のない中心の値の 1e-7 倍程度で合う）。厳密な和は `--exact`（数倍遅い）。
  GPU（Vulkan・DX12・Metal）があれば、近似の代わりに GPU で全部の点を単精度で足す（`--device auto`、既定。位相が大きくならない書き方で、
  強度は厳密な和と 5e-7 倍程度で合う。RTX 3060 で CPU の既定の 10 倍以上速い）。CPU で動くアダプタ（WARP・llvmpipe）は使わない。
  `--device cpu` で CPU に固定、`--device gpu` は CPU で動くアダプタでも GPU の経路を使う（照合用。使えなければ CPU で計算して `warnings` に書く）。
  `--exact` はいつも CPU。バックエンドは環境変数 `WGPU_BACKEND`（`vulkan`・`dx12`・`metal`）で選べる。Web 版は CPU だけ。
  結果の `device` に和を計算した装置（`cpu` か `gpu: アダプタの名前`、Python の dict も同じ鍵、GUI は計算時間の横）を出す。
  `--device auto` で使う GPU は `lensbench devices` で確かめられる（`gpu` が null なら CPU で計算する）。
- 横軸のある点で計算できない（瞳の縁がけられるなど）ときはその点を空（NaN）にして `warnings` に理由を残す。
- OpticStudio 26.1 との照合は `tests/refs/<システム>/mtf_family.json`・`psf_os.npz`（24 本、許容差と理由は `crates/lensbench-verify/src/mtf_refs.rs`）。
- Python は `System.mtf`（`method="fft"`・`each_wave=True`・`axes=`・`polarization=`）・`mtf_focus`・`mtf_field`・`rms`（`vs=`）・`psf`・`geo_mtf`・`huygens_psf`・`huygens_mtf`。

## 収差図

OpticStudio の収差図に当たる解析。どれも評価関数の演算子と同じ計算から作るので、図の値は演算子の値と同じ
（`crates/lensbench-optim/tests/aber_figs.rs`）。`--wave` を省くと波長ごとに全波長（重み付きの多色ではない）。

- `ray-fan`（Ray Fan 相当）: 視野・波長ごとに、瞳の y 軸に沿った光線の像面の y（子午 ey 対 Py）と x 軸に沿った光線の x（球欠 ex 対 Px）の、
  主波長の主光線との差（TRAY・TRAX）。`--rays n` で瞳の片側 n 本（-1〜1 の 2n+1 点、既定 20）。けられた光線は null（図では線を切る）。
  アフォーカル像空間では向きの角度の差。
- `opd-fan`（OPD Fan 相当）: 同じ並びの光路差 [波]。参照球の中心は主波長の主光線、光路の基準は各波長の主光線（OPDC、波面図の基準と同じ）。
- `pupil-fan`（Pupil Aberration Fan 相当）: 同じ並びの瞳座標の近軸の入射瞳へエイミングなしで向けた実光線が絞りの面を通る点の y（子午）・x（球欠）を、
  近軸の点（瞳座標 × 主波長の近軸の絞りの半径）から実の点を引いた、絞りの半径に対する %。システムのエイミングは使わない（使うと実光線が近軸の点を
  通って 0 になる）。絞りまでにけられた光線は null。
- `field-curvature`（Field Curvature/Distortion 相当）: 正規化視野 0〜1 を `--points` 点（`--axis y`・`x`）で、主光線の近くの光線の子午・球欠の焦点の
  像面からの z（FCGT・FCGS: 瞳を ±0.001 ずらした 2 本の光線が最も近づく z）と、実主光線の歪曲 %。歪曲の理想の像高は `--distortion f-tan`
  （既定、近軸の倍率で f tan θ。軸上を基準にした DISG と同じ）か `f-theta`（f θ、角度の視野だけ）。ケラレ係数は使わない（OpticStudio と同じ）。
  OpticStudio の図の子午の焦点は主光線と +0.001 の片側の差分で、lensbench は ±0.001 の中心差分なので、軸外で 1e-4 mm ほど違う（球欠は一致）。
  アフォーカル像空間では焦点を視度（`unit` が `diopters`。焦点までの主光線に沿った距離 z / cos θ' [m] の逆数で、像面の前の虚の焦点は負）、
  歪曲を主光線の向きの tan（l/n・m/n）と近軸の角倍率 × tan θ（`f-theta` は × θ）の差で出す（OpticStudio と同じ）。
- `longitudinal`（Longitudinal Aberration 相当）: 正規化瞳の高さ 0〜1 を `--points` 帯で、軸上の光線が光軸を横切る点の像面の頂点からの距離（LONA。
  帯 0 は近軸周辺光線。OpticStudio の図の瞳 0 の行は近軸ではなく瞳 0.005 ほどの実光線なので、そこだけ 1e-5 mm ほど違う）。
- `lateral-color`（Lateral Color 相当）: 正規化視野 0〜1 を `--points` 点で、各波長の実主光線の像高（視野の向きの成分）の主波長からの差と、
  主波長の Airy 半径 1.22 λ F/#（作動 F 値）。主光線はケラレ係数をかけた瞳の中心（OpticStudio と同じ）。視野の端での値は近軸の LACL に近い（LACL は近軸主光線）。
- `focal-shift`（Chromatic Focal Shift 相当）: `--min`〜`--max` [µm]（省くとシステムの最短・最長の波長）の `--points` 個の波長で、近軸焦点の
  主波長の近軸焦点からのずれ（帯 0 の LONA の差、AXCL と同じ）。ずれの最大の幅 `range` と回折限界の焦点深度 4 λ (F/#)² も出す。
  アフォーカル像空間では像面からの近軸焦点の距離 [m] の逆数の差 [視度]、焦点深度は 0（OpticStudio と同じ）。
- `grid-distortion`（Grid Distortion 相当）: 最大の視野の円に内接する `--grid`×`--grid` の正方形（既定 9。角度の視野は tan θ の等間隔）の各点で、
  実主光線の像面の点と、近軸の倍率からの理想の点、歪曲 %（DISG）。OpticStudio の Grid Number n は格子 2n+3（9 は 3）。最大の歪曲とその視野も。回転対称のシステムだけ。
  アフォーカル像空間では点を像空間の光線の向きの tan（l/n・m/n、`unit` が `tan`）で、理想の点を近軸の角倍率から出す（OpticStudio と同じ）。
- アフォーカル像空間の像面湾曲・色の焦点ずれ・グリッド歪曲と瞳の収差の扇は、OpticStudio 2026 R1.01 の出力（`tests/aber_os_080`。ケプラー式の
  望遠鏡、A0・E0）と出力の桁で合う（`crates/lensbench-analysis/tests/aber080.rs`。視度は中心差分と片側の差分の違いで 5e-7 ほど）。
- `seidel`（Seidel Coefficients・Seidel Diagram 相当）: 面ごとの 3 次収差の和 S_I〜S_V [レンズ単位]（Welford の定義）、1 次の色収差 C_L（軸上）・
  C_T（倍率、最短と最長の波長の屈折率の差から。符号は OpticStudio の CLA・CTR と同じ）、波面収差の係数 W040・W131・W222・W220・W311 [波]（SPHA・COMA・ASTI・FCUR・DIST）。
  ミラーの無い回転対称のシステムだけ。
- GUI では解析の一覧の「収差」の群。扇は視野ごとに子午・球欠の 2 枚（縦軸は全部の視野でそろえる）、像面湾曲は焦点（実線が子午・破線が球欠）と
  歪曲の 2 枚、ザイデル係数は面ごとの棒（最後の組が和）。Python は `System.analyze("ray-fan", field=1)` など。

## 像の模擬

- `image-sim`（Image Simulation 相当）: 元の画像（`--input` の PNG、グレーか RGB。省くと組み込みの 512 × 512 の格子の図）を物体の視野に置き、
  検出器に写す。画像の中心は光軸、高さ（全体）は `--field-height`（視野の単位。省くと最大の視野の √2 倍で、正方形の画像の角が最大の視野に届く。
  OpticStudio の既定と同じ）。画素は視野の値に線形に対応し、行 0（上）が +y、列 0（左）が −x。
- 主波長の主光線の像点（歪曲）と相対照度は写像の格子（`--map-grid`、既定 17 × 17）で求めて補間し、元の画素の明るさをその像点へ置く。
  `--aberrations diffraction`（既定）は PSF の格子（`--psf-x`・`--psf-y`、既定 5 × 5）の各点の FFT PSF（像面の上、主波長の主光線の像点が中心なので
  倍率色収差を含む）を検出器の画素に積分し、格子点の間は重みで混ぜて FFT で畳み込む。`none` は歪曲と相対照度だけ。`--relative-illumination=false` で相対照度を外す。
- 検出器の画素は既定で元の 1 画素の軸上の像の大きさ（`--pixel` で変える）、画素の数は元の画像と同じ（`--width`・`--height`）。
  像の向きは元の画像と同じ（上が像面の +y）。
- 明るさは、一様な元の画像（値 1）が収差なしで相対照度そのものになる単位（出力 ≈ 入力 × 相対照度、検出器の画素の大きさに依らない。
  OpticStudio と同じ）。元の画素ごとに像の面積の比をかける（相対照度は歪曲による面積の変化を含むので、それ以上は割らない）。
  検出器の画素が元の 1 画素の像より小さいときは元の画素を細かく分けて置くので、OpticStudio の Oversampling は要らない。
- 波長はグレーならウェイトのある全波長を重みで足す（`--wave` で 1 波長）。
- RGB はチャンネルごとに 1 波長（`--rgb-waves`、省くと波長 1〜3 を短い方から B・G・R。OpticStudio も波長 1・2・3 が B・G・R）。
- OpticStudio 2026 R1.01 と違う所（物理的に正しい方を取り、合わせない）: (1) 元の画素の明るさに歪曲による像の面積の比をかける。OpticStudio は
  点の明るさに面積の比をかけないので、一様な画像が相対照度にならず、歪曲の大きい所の点は明るさが違う（A0 の角の点で lensbench が 5 % 明るい）。
  (2) 偶数の画素の画像の中心は画素の境目（512 画素なら 255.5、0 から数えて）で、画像は光軸について対称に置く。OpticStudio は画素 256 に置くので、
  点の位置が 0.5 画素以内でずれる。
- OpticStudio 2026 R1.01 との照合は `tests/refs/<A0・D0>/imagesim_os/`（OS からは 8 bit の PNG しか取れないので、試験の図の点ごとの重心と
  明るさの和を比べる。重心 0.3 px、和は D0 1.5 %・A0 −3〜+5 % で一致。OS は偶数の画像の中心を画素 256 に置く・点の明るさに面積の比をかけない・
  PSF の格子が粗いことによる差。許容差と理由は `crates/lensbench-verify/tests/imagesim_refs.rs`）。
- `--save` で検出器の像を PNG（16 bit、1 で白、それより上は切る）か JSON（`image` に像の値）に書く。結果の `image` は `--grid` のときだけ。
- Python は `System.image_sim(image, ...)`（NumPy の (高さ, 幅) か (高さ, 幅, 3) を受けて同じ形で返す）。GUI は「MTF・PSF」の群の「像の模擬」。

## フットプリント・相対照度・けられ

- `footprint`（Footprint Diagram 相当）: `--surface`（省くと絞りの面）の上で、視野・波長ごとの瞳のグリッド（直径 `--rays` 本、既定 21）の光線の
  ローカル座標の交点と瞳の縁の光線の交点、面の開口の輪郭（`aperture`）。最大の半径 `max_radius` とクリア半径に対する割合 `fill`、像面までけられずに
  通る割合 `through`、この面でけられる割合 `clipped`。面より前でけられた光線は描かない。視野のケラレ係数を使う（OpticStudio と同じ。
  `--no-vignetting-factors` で使わずに、瞳全体を面の開口だけでけらす）。自動の半直径では光線は切られない（開口・固定の半直径の面が切る）。
- `relative-illumination`（Relative Illumination 相当）: 正規化視野 0〜1 を `--points` 点で、主光線の像点に届く光束の射影立体角から求めた軸上に対する
  照度（RELI）と実効 F 値（EFNO）。面の開口でけられる向きは光束の縁を探して入れる。`--polarization` で瞳で平均したコーティング・
  フレネルの透過率の軸上との比を掛け、実効 F 値は透過を含めた F/√T にする（OpticStudio の Use Polarization と同じ）。OpticStudio の図の実効 F 値は
  瞳の標本（既定の Ray Density 10）で数 % 大きく出る（標本を増やすと lensbench の値に近づく）。OpticStudio は物体の 1 点の光束で求めるので、収差がとても大きく像の 1 点に
  広い視野の光が集まる系では違う（超広角の `tests/aim_wide/W2` の視野 0.6 で OpticStudio 0.964・lensbench 0.81。外側の視野から来る光が面 2 に当たらず抜ける分が lensbench だけに入る）。
- ケラレ係数（`relative-illumination`・`vignetting`）: 既定（`remove_vignetting_factors`）では OpticStudio の既定の Remove Vignetting Factors と同じく、
  係数を使った今の半直径（自動の値も）を全部の面で固定にし、前後で媒質が変わる面に浮動開口を付けて、係数を 0 にした系で計算する（係数の無い系はそのまま。半直径を書いていない面には付けない。OpticStudio 2026 R1.01 の
  A6 で、OpticStudio の道具 Replace Vignetting With Apertures と同じ開口）。けられる視野では、OpticStudio の相対照度は瞳の標本が粗く、自分の Vignetting Plot
  とも合わない（A6 の 18° で 4 % 違う）。lensbench の値は、けられない系の相対照度にけられずに通る割合を掛けたものと 0.5 % 以内で合う。
  `remove_vignetting_factors` を切ると（CLI は `--remove-vignetting-factors=false`）、相対照度はケラレ係数を使わず、けられの図は `footprint` と同じ扱い（係数を使う、`no_vignetting_factors` で使わない）。
- `vignetting`（Vignetting Plot 相当）: 正規化視野 0〜1 を `--points` 点で、瞳のグリッド（直径 `--rays` 本、既定 64）の光線のうち像面までけられずに
  届く割合。OpticStudio の図は瞳の約 300 本の光線で数える（1 % ほどの刻み）。
- 相対照度・ケラレ図・グリッド歪曲は、視野を振る向きに軸外の視野が無い系（視野が軸上だけ、長方形の正規化で振る向きの視野が 0）では
  「軸外の視野が要る」と返す（正規化視野に大きさが無く、横軸の視野の値が決まらない）。
- GUI では解析の一覧の「像の質」の群。フットプリントは開口の輪郭（薄い線）と視野ごとの色の点・瞳の縁の線。


## 近軸ガウスビーム

- `gaussian-beam`（Paraxial Gaussian Beam 相当）: 物体空間で与えたビーム（`--waist` 埋め込みのガウスビームのウエストの半径、`--surf1-to-waist`
  面 1 からウエストまでの距離（ウエストが面 1 より後なら正）、`--m2`、`--wave` 省くと主波長）を、近軸光線 2 本から作る系の ABCD 行列で各面へ運ぶ。
  面 1 から像面までの各面の後の空間について、頂点でのビーム半径 `size`（1/e²）・ウエストの半径 `waist`・ウエストから頂点までの距離 `position`
  （負ならウエストはこの先）・波面の曲率半径 `radius`（ウエストの上は null）・漸近線の半角 `divergence`（atan(w₀ / z_R)）・レイリー長 `rayleigh`
  （その媒質の中）を出す。M² の混合モードのビームは `mixed_size`・`mixed_waist`（基本モードの √M² 倍）・`mixed_divergence`
  （atan(√M² w₀ / z_R)）で、曲率半径・位置・レイリー長は基本モードと同じ（OpticStudio と同じ約束）。
- `position`・`radius` の符号は面の局所 z の向き（奇数回の反射の後は逆。OpticStudio と同じ）。`profile` は光軸に沿った基本モードの半径
  （展開した面 1 からの位置。GRIN の媒質の中と無限の間隔は入れない）。
- `--orientation x` は今は回転対称の系だけ（y と同じ値）。トロイダル面などの x の断面は未対応。
- 照合: 薄いレンズ・平板・凹面鏡の解析解と、OpticStudio 26.1 の Paraxial Gaussian Beam（A0・C0（鏡）・D0（近軸レンズ）・E0（非球面）、
  ウエストが面 1 の前・上・後、M² 1 と 1.3、2 波長）と表示の 6 桁で一致する（`tests/refs/<系>/gauss_os.json`、
  `crates/lensbench-analysis/tests/gauss.rs`）。
- GUI では解析の一覧の「干渉・伝搬」の群。光軸に沿ったビーム半径（±、M² > 1 なら混合モードを破線で）と面の位置の縦線。
