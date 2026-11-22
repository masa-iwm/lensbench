# 使い方（CLI）

視野・波長・コンフィグの番号は 1 始まり。`--json` で JSON 出力。`--threads N` で追跡・解析のスレッド数（1 で逐次。
外側でプロセス並列にするとき用。結果はスレッド数によらず同じビット。Rust からは `lensbench_core::with_threads`）。
ガラスカタログは `--glass-dir`・環境変数 `LENSBENCH_GLASSCAT` の後に、exe の隣の `catalogs/glass` があればそこも探す
（GUI の既定のカタログと同じ配置）。どこにも無いカタログは同梱の SCHOTT・OHARA（屈折率だけ）を使い、注意を出す。

```
lensbench info     system.zmx --glass-dir <AGF のフォルダ>
lensbench spot     system.zmx --field 2 --wave 1
lensbench wavefront system.zmx --n 64
lensbench zernike  system.zmx --kind fringe --terms 37
lensbench mtf      system.zmx --freqs 0,50,100 --wave 2
lensbench mtf      system.zmx --max-freq 200 --freq-steps 40     # 周波数を省いたときの上限と分割数（既定は遮断周波数の 0.95 倍・20 等分）
lensbench mtf-focus system.zmx --field 1 --freqs 50,100 --range 0.05 --points 11
lensbench mtf-field system.zmx --freqs 30,60 --points 11
lensbench rms      system.zmx --vs field --points 11 --reference centroid
lensbench ray-fan  system.zmx --field 3 --rays 20                # 収差図（ほかに opd-fan・pupil-fan・field-curvature・longitudinal・lateral-color・focal-shift・grid-distortion・seidel）
lensbench footprint system.zmx --surface 5                       # 面の上の光束の範囲（ほかに relative-illumination・vignetting）
lensbench gaussian-beam system.zmx --waist 0.5 --surf1-to-waist -20 --m2 1.2   # 近軸ガウスビーム（各面のビーム径・ウエスト・広がり角）
lensbench optimize system.zmx --var 8.t --var "6.c=0:0.05" --merit wavefront --out opt.zmx
lensbench tolerance project.json --set '{"preset":"precision","compensators":["9.t"]}' --trials 500
lensbench zoom     zoom.json --var G2.shift --var G3.shift --efl 95,110,130 --steps 3
lensbench swap     project.json --spec swap.json --lens-catalog lenses --top 10
lensbench psf      system.zmx --field 2 --pupil 64 --image 128 --save psf.npz
lensbench geo-mtf  system.zmx --freqs 0,50,100 --diff-limit
lensbench huygens-mtf system.zmx --field 3 --pupil 128 --image 256
lensbench coatings system.zmx --coatings coat_tables.ctab
lensbench info     project.json --state near
lensbench catalog-add my.json --name "L2" --from system.zmx --lens 2
lensbench catalog-list my.json
lensbench catalog-search lenses --kind polarizer --spec wavelength_nm=633 --spec "extinction_ratio>=1000"
lensbench catalog-search lenses --fits 25.4                    # 径 25.4 mm の素子が入るチューブ・マウント・ケージ板
lensbench catalog-step item.step --json                       # 品の STEP を測る（枠付きの品はガラスと枠、機構部品は穴・長さ・外径・ねじ・ロッドの間隔）
lensbench cad-step --catalog thorlabs.json --item CP02F       # 品の公式の STEP を 1 件だけ取ってキャッシュし、簡易外形と比べる
lensbench layout project.json --mech-detail --out tubes.glb --out tubes.step   # 機構部品を取った STEP の詳しい形で
lensbench mech-check project.json --grid 31                    # 機構部品（チューブ・ケージ板）で切れる光線の割合
lensbench coat-kinks scan.ctab
lensbench coat-model measured.ctab --out coat_models.lbcm --kinks 0.45,0.5
lensbench layout   system.zmx --out layout.svg --out layout.png --out lens.glb --out lens.step
lensbench layout   project.json --beam-paths all --out paths.svg     # BS の経路を重ねて描く（--states・.zmx は --cfgs も）
lensbench zmx      project.json --beam-paths all --out paths.zmx     # 経路ごとに 1 構成の多重構成の .zmx
lensbench zmx-beams paths.zmx --out project.json                    # 腕ごとの構成の .zmx を BS と往路を戻るミラーのプロジェクトに（構成 k が光束 C<k>）
lensbench layout   project.json --beams all --out beams.svg          # プロジェクトの光束（beams）を重ねて描く
lensbench beams    project.json                                      # 光束の表（透過・けられ・主光線の光路長と参照との差）
lensbench rays     system.zmx --fan grid --rays 7 --save rays.npz
lensbench zernike  system.zmx --save zernike.csv
lensbench ghost    system.zmx --coatings coat_models.lbcm --out ghosts
lensbench pop      system.zmx --surf 7 --out-n 64 --out-half 0.02 --save psf.npz
```

**腕ごとの構成の .zmx（zmx-beams）**: キューブの BS に入り、片方の腕のミラーで同じ面を戻ってキューブを出る、を構成ごとに
書いた多重構成の .zmx（戻りの面も書き並べ、通らない腕の面は IGNR で無視したもの。`zmx --beam-paths` の書き出しも同じ形）を、
BS の腕の先に往路を戻るミラー（`back`）を置いたプロジェクトに読み替える。構成 k は光束 `C<k>`（経路は往きと戻りの分ける面の
T・R、状態 `Config k`）。構成ごとの絞りは状態の `surface_ops` の `set_stop`。戻りの面が往路の面と合わない（往路を戻らない）、
ミラーで何度も往復する、などの形は誤り。

**既定が真の旗**: 既定で入っている真偽の設定（`relative-illumination`・`vignetting` の `--remove-vignetting-factors`）は、
`--remove-vignetting-factors=false` で切る（`=` で付ける。値なしと `=true` は既定と同じ）。ほかの真偽の旗は値を取らず、付けると入る。

**負の数**: `--dist -5`・`--pmag -0.5` のように値として渡せる（`--center -1,2` のような数の並びも）。

**設定のファイル**: 解析のサブコマンド（spot・mtf・zernike など、設定を旗で受けるもの）は `--params-file settings.json` で
設定をまとめて渡せる。中身は JSON のオブジェクトで、鍵は旗の名前のハイフンを下線にしたもの（`{"terms": 37, "field": 2}`。
GUI・`serve` の `analyze`・Python の設定と同じ名前）。コマンド行で明示した旗がファイルより優先し、旗の既定値はファイルを
上書きしない。ファイルの中の相対パスはファイルの場所から読む。真偽の旗（`--diff-limit` など）はファイルで `true` に
すると、コマンド行から外せない。知らない鍵はエラー。

**エラーの文の言語**: 英語が元で、日本語の表示では訳す。`LENSBENCH_LANG=ja`（日本語）・`en`（英語）で選び、無ければ OS の言語。

**STEP の測りの規格**: 品の STEP から寸法を測るときのねじ・ロッドの間隔の表は同梱のもの。環境変数 `LENSBENCH_STEP_STANDARDS` に JSON のファイルを与えると足せる（書き方は [プロジェクトファイル](project-file.md) の STEP の測りの段）。

**終了コード**: エラーの種類で分かれる（`lensbench --help` の末尾にも同じ表）。エラーの文の次の行に `hint:`（日本語の表示は `案内:`）で次の手を出すことがある。

| コード | 意味 |
|---|---|
| 0 | 成功 |
| 1 | 失敗（システム・値の不正、計算の失敗、`verify` の不一致、一部の品・コーティングの上げ・取り・読みの失敗。一部の失敗は結果を出したうえで 1） |
| 2 | 使い方のエラー（引数・設定ファイル。`run` の設定のエラーも） |
| 3 | ファイルが無い・読めない・書けない |
| 4 | 未対応の機能（.zmx の未対応の面・項目など、[.zmx の読込の約束](zmx.md)） |
| 127 | `run`: Python が見つからない |
| 130 | 中断した |

`--save` の中身は `--json` の有無によらず同じ（画面の文字の出力だけ要点に絞る）。

この CLI を組み込んだ exe（非公開版など）は、解析を足せる（`lensbench_engine::register`）。足したサブコマンドも
`serve` の `analyze`・`watch`・GUI の解析で同じに使える。

## 例と出力

例のシステムは `tests/refs/A0`（独自の処方のトリプレット、4 視野・3 波長）。コマンド・出力・図は、ドキュメントを作るたびに CLI を実際に走らせて作り直している（`scripts/docs_examples.py`）。

{{#include gen/sys-A0.md}}

### システムの情報（info）

焦点距離・F 値・瞳などのシステムのデータ。

{{#include gen/info.md}}

### スポット（spot）

視野ごとの RMS・GEO 半径。図は同じシステムを `rays --save rays.npz` で書き出した像面の交点を描いたもの。

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

### 波面収差（wavefront）と干渉縞（interference）

主光線基準の OPD の P-V・RMS [波]。

{{#include gen/wavefront.md}}

`interference --beam2 ref` で参照球との干渉縞（瞳の上、OpticStudio の Interferogram に当たる）。
`interference --mode detector --beams all`（名前をカンマで区切っても）でプロジェクトの光束を 3 本以上でも像面で重ねる
（多光束の干渉計・裏面反射の重なり。画素ごとに場を足す。偏光なしでは光束ごとの瞳の平均の透過率（膜・フレネル・分岐比、解析 `beams` と同じ）を強さにかけ（`beam_weight`）、`--polarized` では電場に入る。`--polarized` の 2 本なら `--beam1`・`--beam2` と同じ）。

{{#include gen/interference.md}}

### Zernike 係数（zernike）

Fringe（`--kind fringe`）か Standard の係数 [波]。

{{#include gen/zernike.md}}

### 回折 PSF（psf）

瞳関数の DFT。`--save` の npz を描いたもの。

{{#include gen/psf.md}}

### レイアウト図（layout）

`--out` の拡張子で形式が決まる（2D は svg・png・dxf、3D は gltf・glb・obj・stl、CAD は step）。詳しくは [書き出し](export.md)。

{{#include gen/layout.md}}
