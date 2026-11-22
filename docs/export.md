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
    BS の経路。`all` で全部、両方指定すると組合せ）・`--beams 名前,…`（プロジェクトの光束 `beams`。`all` で `off` でない光束の全部。
    `--states`・`--beam-paths` とは一緒に使えない）で複数の構成を 1 枚に描く。同じ位置・形の玉・ミラー・面は 1 つだけ描き、光線は既定で
    構成ごとの色（`--color cfg`）。`--offset dx,dy,dz` [mm] で構成 k を k 倍ずらして並べる（OpticStudio のレイアウトの構成ごとの
    オフセットと同じ。ずらすと玉も構成ごとに描く）。立体・面・光線の名前の頭に構成の名前（`C2`・状態名・経路）が付く
- `zmx`: システムを .zmx に書き出す（プロジェクトの展開など）。`--beam-paths`・`--states`・`--beams`・`--cfgs` を付けると、構成ごとに 1 構成の多重構成の
  .zmx にまとめる。面は「全部の構成で同じ先頭の面」＋「構成ごとの残りの面」＋像面の順に並べ、ほかの構成の面は多重構成の
  `IGNR`（無視する面。厚み・座標ブレークも効かない）で外す。先頭の面の厚みの違いは `THIC`、絞りの違いは `STPS`、像面の半径の違いは
  `SDIA`。視野・波長・開口などのシステムの値は全部の構成で同じでなければならない。ピックアップ・ソルブは値にして書く
  - OpticStudio 26.1 との照合（`tests/bs_configs`、`examples/bs_configs.rs`・`make_cfg_refs.py`）: BS の経路（キューブ・平板（表・裏）の T・R、
    Twyman-Green 2 例・Michelson の 4 経路）と状態（P6、玉の抜き差しと物体距離。P7、.zmx の群の移動・偏心・傾き）の 8 本 25 構成で、
    各構成の像面の位置 4e-13 mm・方向 1e-13・光路長 6e-11 mm（けられの有無も一致。無限遠の物体の軸外の視野の光路長は、数え始める面が
    違うので比べない）。多重構成の行は OpticStudio が書く形（値の後に `0 0 0 1 1 倍率 オフセット 0 "" 0`）。
    倍率を落とすと OpticStudio はエラーなしに値を読み違える
- `make-thermal`: 熱の構成を作る（OpticStudio の Make Thermal）。`--temps -20,60` の温度ごとに構成を足し、構成 1 を今のシステム
  （今の構成、温度・気圧はシステムの値）にした多重構成のシステムを `--out`（.json か .zmx）に書き出す。足す構成の気圧は `--pressures`
  （温度と同じ数、省くとシステムの気圧）。表には温度・気圧と、温度ごとの曲率・厚さ・面タイプのパラメータ・半直径・機械半径・
  チップゾーンの行を書く。屈折率は構成の温度・気圧から硝材の式で決まる（屈折率を環境に合わせる設定が切れていれば入れる。
  OpticStudio と同じ）。線膨張係数 α と伸び縮みの規則:
  - 硝材の面は硝材カタログの TCE（面の後ろが硝材ならその硝材、空気なら前の硝材）、ミラーと前後とも空気の面は面の TCE の列。
    温度差 ΔT は元のシステムの温度から
  - 曲率は c / (1 + αΔT)、r^(2n) の非球面係数は × (1 + αΔT)^(1 − 2n)、円錐係数はそのまま。半直径・機械半径・チップゾーンと
    硝材の厚さは × (1 + αΔT)
  - 面タイプのパラメータ（OpticStudio 2026 R1.01 と同じ。形の長さが全部 (1 + αΔT) 倍になる）: 偶数次非球面・Binary 2・Fresnel・
    Zernike Sag・Grid Sag の r^(2n) の係数は上と同じ、Odd Asphere の r^n の係数は × (1 + αΔT)^(1 − n)、Irregular の偏心と Zs・Za・Zc は
    × (1 + αΔT)（傾き・角度はそのまま）。Tilted の tan、Binary 2 の位相の係数・回折次数、Paraxial の焦点距離は変えない
  - OpticStudio は伸び縮みさせるが lensbench の多重構成の表に置けないので変えずに注意を出すもの: トロイダル面の曲率・回転半径・係数、
    Binary 2 と Fresnel の曲率・係数、Zernike の項・正規化半径・偏心、Binary 2 の正規化半径
  - 空気の間隔は、前後の面の半直径の縁を結ぶ薄い円筒のスペーサー（α は間隔の面の TCE の列）とみる。縁から縁の長さが
    × (1 + α_m ΔT)、縁の高さは半直径の小さい方の面（像面までの間隔は前の面）はスペーサーの α_m で、大きい方はその面の硝材の α で
    変わり、中心の厚さは縁の長さと温度の後の矢高から決める
  - 表に書く曲率・厚さのソルブは外す（値は元のシステムのまま。OpticStudio も熱の解析の前に外すよう求める）。座標ブレークの
    厚さは変えない。OpticStudio の熱のピックアップ（元のシステムを変えると追いかける）は作らず、作った時の値を書く
  - OpticStudio 2026 R1.01 との照合（`tests/refs/A0`・`E0` の `thermal_os.json`、`lensbench-analysis/tests/thermal.rs`）:
    A0（ダブレット）と E0（偶数次非球面の玉）を −20 °C・60 °C にした表の全部の行が 1e-12、構成ごとの屈折率が 1e-9、焦点距離が 1e-9 で一致。
    面タイプ 10 種（Standard・Odd Asphere・Toroidal・Zernike Standard/Fringe Sag・Binary 2・Tilted・Irregular・Fresnel・Paraxial）の単玉を
    −30 °C・70 °C にした表も、上の表に置けないもののほかは 1e-12 で一致（像面までの空気の厚さは変化量の 3 % 以内。自動の半直径が
    OpticStudio と少し違う Paraxial の系の分）
  - GUI は「ファイル > 熱の構成を作る…」（開いているシステムは変えずに .zmx に書き出す）、Python は `s.make_thermal([-20, 60])`、
    serve は `write` の `thermal` か `edit` の `make_thermal`
- `rays`: 光線の面ごとの交点（局所・大域）・方向余弦・光路長・けられ（1 行 = 1 光線 × 1 面）。`--pupil 0,1,0.5,0`（px,py の組の並び）で
  瞳座標を直接、`--surface` で面を選ぶ。`--all-waves` で全波長。`--save` の拡張子で csv・json・npz（csv・npz は表を全部メモリに作らずに、視野・波長の組ごとに書き足す。途中でエラーになると、csv は書き出し先を消す（前にあった中身も残らない）。npz は書き出し先の隣に列ごとの一時ファイル `<名前>.part<k>` を作り、書き終えたら消す。npz は光線を全部追ってから組むので、追跡中のエラーでは書き出し先を変えず、組む途中のエラーでは書き出し先を消す）
- `--save`（info・spot・wavefront・zernike・mtf・mtf-focus・mtf-field・rms・psf・geo-mtf・elements・mech-check・layout。rays・pop は自分の形で）: 結果を `json`（そのまま）・`csv`・`npz`（表にし、配列は `coef_1`… の列に）で書き出す。
  npz は列ごとの 1 次元配列（`numpy.load` で読める）

MTF・PSF・RMS などの解析の説明は [解析（MTF・PSF・RMS）](analysis.md)。

## 例

A0 のレイアウト図（`--out layout.png`）。

{{#include gen/layout.md}}
