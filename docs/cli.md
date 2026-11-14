# 使い方（CLI）

視野・波長・コンフィグの番号は 1 始まり。`--json` で JSON 出力。`--threads N` で追跡・解析のスレッド数（1 で逐次。
外側でプロセス並列にするとき用。結果はスレッド数によらず同じビット。Rust からは `lensbench_core::with_threads`）。

```
lensbench info     system.zmx --glass-dir <AGF のフォルダ>
lensbench spot     system.zmx --field 2 --wave 1
lensbench wavefront system.zmx --n 64
lensbench zernike  system.zmx --kind fringe --terms 37
lensbench mtf      system.zmx --freqs 0,50,100 --wave 2
lensbench mtf      system.zmx --max-freq 200 --freq-steps 40     # 周波数を省いたときの上限と分割数（既定は遮断周波数の 0.95 倍・20 等分）
lensbench mtf-focus system.zmx --field 1 --freqs 50,100 --range 0.05 --steps 11
lensbench mtf-field system.zmx --freqs 30,60 --points 11
lensbench rms      system.zmx --vs field --points 11 --ref centroid
lensbench optimize system.zmx --var 8.t --var "6.c=0:0.05" --merit wavefront --out opt.zmx
lensbench tolerance project.json --set '{"preset":"precision","compensators":["9.t"]}' --trials 500
lensbench zoom     zoom.json --var G2.shift --var G3.shift --efl 95,110,130 --steps 3
lensbench swap     project.json --spec swap.json --lens-catalog lenses --top 10
lensbench psf      system.zmx --field 2 --pupil 64 --image 128 --save psf.npz
lensbench geo-mtf  system.zmx --freqs 0,50,100 --diff-limit
lensbench huygens-mtf system.zmx --field 3 --pupil 128 --image 256
lensbench coatings system.zmx --table coat_tables.ctab
lensbench info     project.json --state near
lensbench catalog-add my.json --name "L2" --from system.zmx --lens 2
lensbench catalog-list my.json
lensbench catalog-search lenses --kind polarizer --spec wavelength_nm=633 --spec "extinction_ratio>=1000"
lensbench catalog-search lenses --fits 25.4                    # 径 25.4 mm の素子が入るチューブ・マウント・ケージ板
lensbench catalog-step item.step --json                       # 品の STEP を測る（枠付きの品はガラスと枠、機構部品は穴・長さ・外径・ねじ・ロッドの間隔）
lensbench cad-step --catalog thorlabs.json --item CP02F       # 品の公式の STEP を 1 件だけ取ってキャッシュし、簡易外形と比べる
lensbench cad-upload --catalog thorlabs.json --dir step/      # 手元の STEP をファイル名で品に当てて共有の置き場に上げる（--cache はキャッシュをそのまま）
lensbench layout project.json --mech-detail --out tubes.glb --out tubes.step   # 機構部品を取った STEP の詳しい形で
lensbench mech-check project.json --grid 31                    # 機構部品（チューブ・ケージ板）で切れる光線の割合
lensbench coat-upload catalogs/coating                      # 膜のモデル（.lbcb）の目録を作り共有の置き場に上げる（coat-fetch で取る）
lensbench coat-kinks scan.ctab
lensbench coat-model measured.ctab --out coat_models.lbcm --kinks 0.45,0.5
lensbench layout   system.zmx --out layout.svg --out layout.png --out lens.glb --out lens.step
lensbench layout   project.json --paths all --out paths.svg          # BS の経路を重ねて描く（--states・.zmx は --cfgs も）
lensbench zmx      project.json --paths all --out paths.zmx          # 経路ごとに 1 構成の多重構成の .zmx
lensbench rays     system.zmx --fan grid --rays 7 --save rays.npz
lensbench zernike  system.zmx --save zernike.csv
lensbench ghost    system.zmx --coatings coat_models.lbcm --out-dir ghosts
lensbench pop      system.zmx --surf 7 --out-n 64 --out-half 0.02 --save psf.npz
```

**負の数**: `--dist -5`・`--pmag -0.5` のように値として渡せる（`--center -1,2` のような数の並びも）。

**終了コード**: 0 は成功、1 は実行の失敗（一部の品・膜の上げ・取り・読みの失敗も。結果は出したうえで 1）、
2 は引数の誤り。`verify` の不一致も 1。`--save` の中身は `--json` の有無によらず同じ（画面の文字の出力だけ要点に絞る）。

この CLI を組み込んだ exe（非公開版など）は、解析を足せる（`lensbench_engine::register`）。足したサブコマンドも
`serve` の `analyze`・`watch`・GUI の解析で同じに使える。

## 例と出力

例の系は `tests/refs/A0`（独自の処方のトリプレット、4 視野・3 波長）。コマンド・出力・図は、ドキュメントを作るたびに CLI を実際に走らせて作り直している（`scripts/docs_examples.py`）。

{{#include gen/sys-A0.md}}

### 系の情報（info）

焦点距離・F 値・瞳などの系のデータ。

{{#include gen/info.md}}

### スポット（spot）

視野ごとの RMS・GEO 半径。図は同じ系を `rays --save rays.npz` で書き出した像面の交点を描いたもの。

{{#include gen/spot.md}}

### 回折 MTF（mtf）

子午（T）・球欠（S）の MTF。`--json` で全部の値が出る（図はそれを描いたもの）。

{{#include gen/mtf.md}}

### フィールド MTF（mtf-field）

正規化視野に対する MTF。

{{#include gen/mtf-field.md}}

### スルーフォーカス MTF（mtf-focus）

像面をずらしたときの MTF。

{{#include gen/mtf-focus.md}}

### RMS の視野依存（rms）

RMS スポット半径・RMS 波面収差を視野・フォーカス・波長に対して。

{{#include gen/rms.md}}

### 波面収差（wavefront）と干渉縞（fringe）

主光線基準の OPD の P-V・RMS [波]。

{{#include gen/wavefront.md}}

`fringe --beam2 ref` で参照球との干渉縞（瞳の上、OpticStudio の Interferogram に当たる）。

{{#include gen/fringe.md}}

### Zernike 係数（zernike）

Fringe（`--kind fringe`）か Standard の係数 [波]。

{{#include gen/zernike.md}}

### 回折 PSF（psf）

瞳関数の DFT。`--save` の npz を描いたもの。

{{#include gen/psf.md}}

### レイアウト図（layout）

`--out` の拡張子で形式が決まる（2D は svg・png・dxf、3D は gltf・glb・obj・stl、CAD は step）。詳しくは [書き出し](export.md)。

{{#include gen/layout.md}}
