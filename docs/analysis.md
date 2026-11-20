# 解析（MTF・PSF・RMS）

CLI の MTF・PSF・RMS のまわりの解析（GUI の解析のパネル・Python・serve の `analyze` も同じ計算と引数）。
スポット・波面・Zernike などほかの解析の引数は [CLI のリファレンス](reference/cli.md)。

## 回折 MTF

`mtf` は回折 MTF（変調度、子午・球欠）。`--wave` を省くと多色（波長の重みで平均）、`--freqs` を省くと 0 から遮断周波数の
0.95 倍までの 21 点（遮断周波数は計算した波長の値、多色は主波長。結果の `cutoff` は `tangential`・`sagittal`）。瞳関数の自己相関を、ずらした 2 つの瞳の重なりで直接求積する（OPD は瞳の格子を 1 度追跡して補間）。求積の点の数は
OPD の傾きから自動で決める（`--n` で指定も可）。OpticStudio の FFT MTF と同じく瞳の座標（主光線に垂直な面）で計算するので、
像面の傾き・回転は考えない。子午・球欠の向きは `--axes`: `field`（既定）は子午が視野の向き（物理の子午面）、`pupil` は子午が瞳の y・球欠が x
（OpticStudio の FFT MTF と同じ）。違うのは x の成分のある視野だけで、そのような視野で OpticStudio と比べるときは `--axes pupil`
（`mtf-focus`・`mtf-field`・`geo-mtf`・`huygens-mtf` も同じ）。
`--polarization` で膜の透過とフレネル損失を瞳の振幅に入れる（偏光なしでシステムの膜を使う。OpticStudio の Use Polarization に当たる。
`psf`・`huygens-mtf` も）。瞳のアポダイゼーションは常に入る。照合は `crates/lensbench-verify/src/mtf_refs.rs`（OpticStudio の FFT MTF の sampling 512・256 と 24 本で 3e-4〜2e-3）。
`--each-wave` で多色に加えて波長ごとの MTF（`waves`）も出す（波長ごとの計算を使い回すので別々に求めるより約 2 倍速い）。
`--method fft` は瞳関数（直径 512〜4096 点の格子）の自己相関を FFT で全周波数まとめて求める: 時間が周波数の数によらないので、
周波数が多いとき（目安 100 点超）や収差の大きいシステムで速い。求積との差は 24 本で 1e-4 以下（`examples/mtf_fft.rs`）。
`--n` を省いて自動で決めた求積の点の数が 1024 を超えるシステム（収差が大きいシステム。E0・E1 の軸外）で、周波数が 8 個以上のときは、
既定でも FFT に切り替える（E1 の既定の 21 点で約 1.8 倍速い。周波数が少ないときは求積の方が速い）。切り替えずに求積で求めるときは `--method interp`、求積点の OPD をすべて直接追跡して確かめるときは `--method direct`。
時間は `examples/mtf_bench.rs`（全視野、多色＋各波長、21 点）。

## MTF のまわりの解析

既存の MTF・スポット・波面との突き合わせは `tests/mtf_family.rs`、OpticStudio との照合は下の最後の項:

- `mtf-focus`（Through Focus MTF 相当）: 像面を `--range` の ±範囲で `--points` 点ずらした回折 MTF。ずらしは像面の手前の面の厚さに
  足す（厚さのソルブは解き直さない）。
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
- 横軸のある点で計算できない（瞳の縁がけられるなど）ときはその点を空（NaN）にして `warnings` に理由を残す。
- OpticStudio 26.1 との照合は `tests/refs/<システム>/mtf_family.json`・`psf_os.npz`（24 本、許容差と理由は `crates/lensbench-verify/src/mtf_refs.rs`）。
- Python は `System.mtf`（`method="fft"`・`each_wave=True`・`axes=`・`polarization=`）・`mtf_focus`・`mtf_field`・`rms`（`vs=`）・`psf`・`geo_mtf`・`huygens_psf`・`huygens_mtf`。
