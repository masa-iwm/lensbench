# 番号・単位・省いたときの約束

CLI・serve・Python・GUI・プラグインで共通の約束。解析ごとの違いは各解析の説明（`lensbench <解析> --help`、serve の `analysis_args`）に書く。

## 番号

| もの | 始まり | 例 |
| --- | --- | --- |
| 視野・波長 | 1 | `--field 2`、`{"wave": 1}`、Python の `field=2` |
| 面 | 0（物体面が 0。OpticStudio と同じ） | `3.t`（面 3 の厚さ）、`surface: 5` |
| 玉・構成（多重構成） | 1 | `lens: 2`、optimize の `vars[].configs: [1, 3]` |

範囲の外の番号は「視野 9 はありません（1〜3）」の形の誤り（`data.kind = "invalid"`、Python は `lensbench.InvalidError`）。
Rust の `lensbench_core` の中（`FieldSel::Index`・`Waves::One`・`System::fields` の添え字）だけは 0 始まり。

プラグインの `field` は、数 1 つなら視野の番号（1 始まり）、2 つの並びなら正規化視野 (hx, hy)。

## 省いたとき

- `field` を省くと全部の視野（視野ごとに 1 行）
- `wave` を省くと:
  - スポット・RMS・回折 MTF・幾何 MTF・PSF（FFT・Huygens）・スルーフォーカス・MTF vs 視野は、全部の波長の重み付き（多色）
  - 波面・Zernike・OPD は主波長（単色の解析で、基準はその波長の主光線）
- 遮断周波数（`cutoff`）は計算した波長、多色なら主波長で求める

## 単位

| 量 | 単位 | 鍵・名札 |
| --- | --- | --- |
| 長さ（面の値・像の位置・スポット・PSF の格子） | mm（.zmx の単位は読込で mm に直す） | 結果の `unit` は `"mm"` |
| 波長 | µm | `wavelengths_um`（serve の概要）、`System::wavelengths` |
| 波面・OPD | 波（その波長） | `pv`・`rms` |
| 周波数 | cycles/mm | `unit` は `"cycles/mm"` |
| 角度（視野・傾き） | 度 | 解析の引数の説明に [deg] |

アフォーカル像空間（`afocal`）では、像の位置は角度の単位（系の設定、既定は mrad）、周波数は cycles/角度の単位で、`unit` にその名前が入る。

前の版はスポット・PSF の `unit` を `"lens"`（意味は mm）と書いていた（[壊れる変更](changes.md)）。
