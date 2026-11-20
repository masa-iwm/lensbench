# 書き出し

- `layout`: レイアウトを `--out` の拡張子の形式で書き出す（複数可）。
  - 2D（大域の yz 面、`--plane xz` も可。`--unfold` でミラー・BS の折り曲げを伸ばした展開図）: `svg`・`png`（`--width` px）・`dxf`（R12、レイヤー LENS・MIRROR・SURFACE・RAYS）
  - 3D: `gltf`（バッファ埋め込み）・`glb`（m 単位）、`obj`・`stl`（mm）。STL は線を持てないので光線は細い管（`--tube` で半径）
  - CAD 用: `step`・`stp`（AP214）。玉とミラーは B-rep の立体（球面・平面はそのまま、非球面は断面の 3 次の B スプライン曲線を軸のまわりに回した
    SURFACE_OF_REVOLUTION で、断面の区間は中点で面のサグとの差が 1e-6 mm 以下になるまで倍にする。玉の中に偏心・傾きがあれば
    側面は線織面）、光線は折れ線、像面は縁の円。傾いた 2 面の側面は、前の面の x を後ろの面に
    投げた向きから同じ角度の縁の点どうしを結ぶ（3D の網・2D の断面も同じ組）
  - 読み戻しの試験（`tests/readback`、CI の export-readback）: SVG（XML）・glb（trimesh）・STEP（OpenCascade）・
    追跡の CSV・JSON を別の実装で読み、要素の数、面の点が .zmx の形と OpticStudio の面の大域座標（GLRS）の上にあること、
    立体が閉じていて体積が .zmx から求めたものと合うこと（STEP 1e-6、glb は網の粗さの分）、光線の折れ点が追跡の大域座標と
    合うこと、追跡の表が OpticStudio の光線（tests/refs の A0・C0）と 1e-9 mm で合うことを確かめる。
    手元では `pip install -r tests/readback/requirements.txt` の後 `cargo build --profile ci -p lensbench-cli && pytest tests/readback`
  - 光線は `--fields`・`--waves`（番号の並び、`all` も可）・`--fan`（y・x・cross・grid・ring）・`--rays`・`--color`（field・wave）、描く範囲は `--first`・`--last`。
    既定は 2D が y 扇 7 本、3D・STEP が主光線と瞳の縁の円周 12 本。
    ガラスを挟む 2 面ごとに 1 つの立体（外径は 2 面の半径の大きい方）。ダブルパスで同じ玉を 2 回通っても 1 つだけ描く
  - 重ね描き: `--cfgs 1,2,3`（.zmx の多重構成、`all` も可）、`--states 名前,…`・`--beam-paths T,R,…`（プロジェクトの状態・
    BS の経路。`all` で全部、両方指定すると組合せ）で複数の構成を 1 枚に描く。同じ位置・形の玉・ミラー・面は 1 つだけ描き、光線は既定で
    構成ごとの色（`--color cfg`）。`--offset dx,dy,dz` [mm] で構成 k を k 倍ずらして並べる（OpticStudio のレイアウトの構成ごとの
    オフセットと同じ。ずらすと玉も構成ごとに描く）。立体・面・光線の名前の頭に構成の名前（`C2`・状態名・経路）が付く
- `zmx`: システムを .zmx に書き出す（プロジェクトの展開など）。`--beam-paths`・`--states`・`--cfgs` を付けると、構成ごとに 1 構成の多重構成の
  .zmx にまとめる。面は「全部の構成で同じ先頭の面」＋「構成ごとの残りの面」＋像面の順に並べ、ほかの構成の面は多重構成の
  `IGNR`（無視する面。厚さ・座標ブレークも効かない）で外す。先頭の面の厚さの違いは `THIC`、絞りの違いは `STPS`、像面の半径の違いは
  `SDIA`。視野・波長・開口などのシステムの値は全部の構成で同じでなければならない。ピックアップ・ソルブは値にして書く
  - OpticStudio 26.1 との照合（`tests/bs_configs`、`examples/bs_configs.rs`・`make_cfg_refs.py`）: BS の経路（キューブ・平板（表・裏）の T・R、
    Twyman-Green 2 例・Michelson の 4 経路）と状態（P6、玉の抜き差しと物体距離。P7、.zmx の群の移動・偏心・傾き）の 8 本 25 構成で、
    各構成の像面の位置 4e-13 mm・方向 1e-13・光路長 6e-11 mm（けられの有無も一致。無限遠の物体の軸外の視野の光路長は、数え始める面が
    違うので比べない）。多重構成の行は OpticStudio が書く形（値の後に `0 0 0 1 1 倍率 オフセット 0 "" 0`）。
    倍率を落とすと OpticStudio はエラーなしに値を読み違える
- `rays`: 光線の面ごとの交点（局所・大域）・方向余弦・光路長・けられ（1 行 = 1 光線 × 1 面）。`--pupil 0,1,0.5,0`（px,py の組の並び）で
  瞳座標を直接、`--surface` で面を選ぶ。`--all-waves` で全波長。`--save` の拡張子で csv・json・npz（csv・npz は表を全部メモリに作らずに、視野・波長の組ごとに書き足す。途中でエラーになると、csv は書き出し先を消す（前にあった中身も残らない）。npz は書き出し先の隣に列ごとの一時ファイル `<名前>.part<k>` を作り、書き終えたら消す。npz は光線を全部追ってから組むので、追跡中のエラーでは書き出し先を変えず、組む途中のエラーでは書き出し先を消す）
- `--save`（info・spot・wavefront・zernike・mtf・mtf-focus・mtf-field・rms・psf・geo-mtf・elements・mech-check・layout。rays・pop は自分の形で）: 結果を `json`（そのまま）・`csv`・`npz`（表にし、配列は `coef_1`… の列に）で書き出す。
  npz は列ごとの 1 次元配列（`numpy.load` で読める）

MTF・PSF・RMS などの解析の説明は [解析（MTF・PSF・RMS）](analysis.md)。

## 例

A0 のレイアウト図（`--out layout.png`）。

{{#include gen/layout.md}}
