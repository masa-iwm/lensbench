# Web 版

GUI（lensbench-gui）を wasm にしてブラウザで動かす。コードはネイティブと共通で、違うのは次の所だけ
（`cfg(target_arch = "wasm32")` で切り替える）。

| 所 | ネイティブ | Web |
|---|---|---|
| ファイル（`lensbench_core::vfs`） | `std::fs` | プロセスの中の置き場（パス → 中身）。見本は起動時に入れる |
| 計算（`worker.rs`） | 別のスレッド | Web Worker（`worker.js`、中は同じ wasm）。ファイルの置き場は要求・結果と一緒に互いに写す（`wire.rs`） |
| 並列（`lensbench_core::par`） | rayon | ページが cross-origin isolated なら、計算の Web Worker の中で rayon のスレッド（wasm-bindgen-rayon、`mt/` の版）。そうでなければ逐次 |
| SIMD | AVX（実行時に判定） | simd128（両方の版。自動ベクトル化と、Zernike の当てはめの AᵀA は命令を明示した経路。値は同じ） |
| ファイルの窓（`files.rs`） | rfd | 開く: ブラウザのファイル選択（複数選べる）で `/uploads/` に入れる。保存: `/downloads/` に書いてダウンロード。窓に落としても開ける |
| 時計 | `std::time::Instant` | `web_time::Instant`（ネイティブでは std そのもの） |
| アロケータ | mimalloc | 既定 |
| 別の窓（パネルを外に出す） | OS の窓（egui の viewport） | ブラウザの別の窓（`web_tabs.rs`、下の「別の窓のパネル」）。解析は結果を送って描き、ほかのパネルは元のタブが描いた画面を送る |
| 日本語フォント | OS のフォント | Noto Sans JP（SIL OFL 1.1）を字を絞って配る（`scripts/web_font.py`） |

Web では使えないもの: serve・watch・run（TCP・別プロセス）、OpticStudio との連携。
1 スレッドの版（cross-origin isolated でないページ・`?st`）では、解析の途中の中断（系を変えると古い結果は捨てるが、
計算は最後まで回る）と、最適化・公差・差し替え探索の進み具合の表示・停止も効かない（下の「中断と進み具合」）。

## ブラウザに残すもの

| もの | 残す所 | 消すとき |
|---|---|---|
| 読み込んだファイル（`/uploads/`）・ユーザーカタログ（`/user/catalogs/`） | IndexedDB（`lensbench` の `files`）。書いた・消したらすぐ写す（`web_store.rs`） | ファイル → 「ブラウザに残したファイルを消す」 |
| 設定（硝材・レンズのフォルダ、最近のファイル、並び、見た目） | localStorage（eframe。2 秒ごと） | ブラウザのサイトデータの削除 |

ページを開き直すと、残したファイルを起動の前に置き場へ戻し、最近のファイルの先頭がその中にあればそれを開く
（無ければ見本）。ユーザーカタログは `/user/catalogs/` に保存し（カタログに保存の窓の既定）、最初に保存したときに
レンズのフォルダに足す。窓の「⬇」でファイルとしてダウンロードできる。ブラウザが認めれば
`navigator.storage.persist()` で消されにくくする。

## 中断と進み具合

計算の Web Worker が解析を回している間は、その Worker はメッセージを受けられない。スレッドありの版（`mt/`）では、
Worker が同じ wasm・同じ共有メモリの見張りのスレッド（`monitor.js`）を作り、その口（MessagePort）を画面に渡す。
画面は見張りを通して、回している解析の中断（系を変えたとき。中断の印 `lensbench_core::par` を立てる）、
最適化・公差・差し替え探索の停止と進み具合（0.2 秒ごと）を頼む。要求には番号を付け、見張りは計算の側が回している
要求の番号が合うときだけ印を立てる（終わった要求の中断が次の要求に効かない）。

## 別の窓のパネル

パネルのタブを右クリック →「別の窓に出す」（パレットの「別の窓に出す」も同じ）で、そのパネルを
ブラウザの小さな別の窓（ポップアップ、`?panel=<id>&main=<番号>`）で開く。どのパネルも同じ出し方で、窓の上の
「元のタブに戻す」か窓を閉じると、出す前のドックの位置（同じタブの並びの同じ順番、1 枚だけだったときは同じ側・同じ割合）に戻る。

- 系と計算は元のタブが持ったまま。別の窓は計算の Web Worker を作らず、同梱のデータも読まない。元のタブから届く
  系の概要・結果を描き、設定（解析の種類・視野・波長・引数）を変えたら元のタブに送る。元のタブが計算して結果を返す
- 受け渡しは BroadcastChannel `lensbench-panels`。元のタブごとの番号で分けるので、同じページを 2 つ開いても混ざらない
- 別の窓を閉じる（または「元のタブに戻す」）と、パネルは元のタブのドックに戻る。元のタブを閉じると、別の窓に
  その知らせを出す。ページを開き直したときは、別の窓に出していたパネルもドックに戻す（窓はクリックしないと開けない）
- 解析のパネルは、別の窓が結果を受けて自分で描く。解析以外のパネル（系・レンズデータ・カタログ・組合せ・レイアウトなど）は
  （`&remote=1`）、元のタブがそのパネルを別の `egui::Context` で描き、三角形の網と変わったテクスチャを
  `remote_frame.rs` の形で送る。別の窓はそれを描くだけで、クリック・キー・ホイールなどの入力と窓の大きさを
  元のタブに返す。画面が変わったときだけ送る
- 別の窓に出したパネルの制限: そこから開くファイル窓は出ない（ブラウザは元のタブでの操作でないと開かせない。
  ファイルは元のタブで開く）。窓をまたぐドラッグはできない。元のタブが見えていないと画面の更新が遅れる
- ブラウザがポップアップを止めたら、ページの中の窓に出す
- ブラウザは見えていないタブの画面を回さない。元のタブが同じ窓の裏にあると、表に出すまで計算が進まない
  （別の窓にその知らせを出す）

**新しい機能でファイルを読み書きするときは `std::fs` ではなく `lensbench_core::vfs` を通す。** 一時フォルダは
`vfs::temp_dir()`（wasm の `std::env::temp_dir` は panic）。保存先は `crate::files::FileDialog::save_file`、
開くのは `crate::files::open(id, start, ..)`（Web 版は選び終わった後のフレームで返るので、毎フレーム同じ id で呼ぶ）。
スレッドを作るときは wasm を分ける。計算の状態（serve の系など）は Web Worker の中にあるので、画面の側から
`lensbench_cli::serve` を直に呼ばず `Worker` に要求を出す。CI の `web` ジョブ（wasm の clippy と起動確認）で
漏れが分かる。

## ビルド

```sh
rustup target add wasm32-unknown-unknown
rustup toolchain install nightly-2026-10-15 -c rust-src -t wasm32-unknown-unknown  # mt/ の版（THREADS=0 で作らない）
cargo install wasm-bindgen-cli --version <Cargo.lock の wasm-bindgen の版>
python -m pip install fonttools
scripts/build_web.sh target/web      # 配る形のフォルダ
cd target/web && python -m http.server 8000   # http://localhost:8000/ で開く
node scripts/web_smoke.mjs target/web shot.png  # 起動確認（npm の playwright が要る）
```

プロジェクトが参照する .zmx・カタログは、開くときに一緒に選ぶと同じフォルダに入る。

simd128 の経路（Zernike の当てはめの AᵀA）の試験は、wasm32-wasip1 で作って Node の WASI で回す（CI の web ジョブも同じ）:

```sh
rustup target add wasm32-wasip1
RUSTFLAGS="-C target-feature=+simd128" CARGO_TARGET_WASM32_WASIP1_RUNNER="node $PWD/scripts/wasi_run.mjs" \
  cargo test -p lensbench-core --lib --target wasm32-wasip1 column_basis_and_gram_match_the_row_forms
```

同梱する見本は `crates/lensbench-web/build.rs` が集める: 公開用ガラス `tests/glass`、例題のレンズ
`examples/lenses`、例題のプロジェクト `examples/projects`、`tests/refs/*/system.zmx`。
ベンダーのカタログ（AGF・ZMF・膜の表）は同梱しない。

## 公開（GitHub Pages、誰でも見られる）

公開側のビルド（同梱は公開用の見本だけ）は、main への push ごとに `.github/workflows/pages.yml` が
GitHub Pages の `/app/`（https://masa-iwm.github.io/lensbench/app/ ）に置く（根はこのドキュメント）。初回だけ、リポジトリの
Settings → Pages → Build and deployment → Source を「GitHub Actions」にする。

GitHub Pages は COOP/COEP のヘッダーを付けられないので、index.html がヘッダーを付け直す Service Worker
（`coi.js`）を入れて 1 回だけ読み直す。それでも cross-origin isolated にならなければ 1 スレッドの版で動く。

## 2 つの版

`scripts/build_web.sh` は 2 つ作る。どちらを使うかは index.html と worker.js が決める。

- そのまま（stable）: どこでも動く 1 スレッドの版
- `mt/`（nightly、atomics・共有メモリ）: 計算の Web Worker（serve と同じ要求を受ける方）が rayon のスレッドを
  `navigator.hardwareConcurrency`（16 まで）本作る。`mt/` を読むのは計算の Worker だけで、画面のスレッドは
  そのままの版を読む（画面は逐次で足りる）。下書きの Worker も逐次のまま。
  `mt/` を読めないか、スレッドを作れない（`initThreadPool` が失敗する）ときは、Worker がそのままの版に戻して 1 スレッドで回す

URL に `?st` を付けると 1 スレッドの版になる（比べるとき）。4 コアで P9 の MTF が 104 秒 → 31 秒（11/09）。
Web では `with_threads(Some(n))` も大域のプールを使う（スレッドを足せない）。

## 速さ

`scripts/web_bench.mjs` が、同じマシンでネイティブ（`lensbench serve`、1 スレッドと全コア）と Web 版（Chromium の中の
計算の Worker、そのままの版と `mt/`）に同じ要求を渡して、解析 1 回の時間を比べる（受け渡しと描画は含まない）。

```sh
cargo build --release -p lensbench-cli
scripts/build_web.sh target/web
node scripts/web_bench.mjs target/web --systems A0,C0,A2,C1 --cmds spot,wavefront,mtf,psf --rounds 3
```

2026-11-09、Xeon 2.1 GHz 4 コア（Linux、Chromium 153）。時間 [ms] は 3 回の中央値:

| 系 | 解析 | ネイティブ 1T | ネイティブ 4T | Web 1T | Web mt | Web 1T ÷ ネイティブ 1T | Web mt ÷ ネイティブ 4T |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| A0 | spot | 0.60 | 0.85 | 2.25 | 2.21 | 3.71 | 2.59 |
| A0 | wavefront | 2.17 | 2.11 | 6.66 | 9.72 | 3.07 | 4.60 |
| A0 | mtf | 30.5 | 19.2 | 52.4 | 70.6 | 1.72 | 3.68 |
| A0 | psf | 107 | 81.4 | 96.2 | 103 | 0.90 | 1.27 |
| C0 | spot | 1.03 | 0.97 | 1.25 | 1.28 | 1.21 | 1.32 |
| C0 | wavefront | 3.68 | 1.90 | 4.68 | 3.97 | 1.27 | 2.09 |
| C0 | mtf | 2534 | 883 | 4308 | 1381 | 1.70 | 1.56 |
| C0 | psf | 122 | 82.5 | 109 | 104 | 0.89 | 1.27 |
| A2 | spot | 3.04 | 2.01 | 5.56 | 5.28 | 1.83 | 2.62 |
| A2 | wavefront | 7.89 | 5.87 | 12.5 | 8.71 | 1.58 | 1.48 |
| A2 | mtf | 64.5 | 43.8 | 105 | 106 | 1.63 | 2.43 |
| A2 | psf | 123 | 99.7 | 129 | 119 | 1.05 | 1.20 |
| C1 | spot | 2.38 | 3.36 | 4.68 | 4.43 | 1.97 | 1.32 |
| C1 | wavefront | 9.23 | 6.39 | 16.3 | 8.94 | 1.77 | 1.40 |
| C1 | mtf | 2156 | 844 | 3761 | 1192 | 1.74 | 1.41 |
| C1 | psf | 120 | 98.9 | 135 | 127 | 1.13 | 1.28 |

重い解析（C0・C1 の MTF、1 回 1 秒以上）では、Web 版はネイティブの 1 スレッドで約 1.7 倍、全コアで約 1.5 倍の時間。
数 ms〜数十 ms の解析は、スレッドに分ける手間や時計の粗さで比がばらつく（Web の mt がかえって遅いこともある）。

光線のまとめ追跡は、wasm では f64x2（simd128）の命令を名指しして 8 本ずつ追う（LLVM は wasm ではまとめ追跡の
ループをベクトル化しないため。#99）。結果は 1 本ずつの追跡とビットまで同じ（CI で wasm の試験を Node で回して確かめる）。
光線の多い解析（`--cmds spot:rings=100,wavefront:n=512`）の Web 1T は、この変更の前後で次のとおり（2026-11-10、同じマシン、
5 回の中央値 [ms]）:

| 系 | 解析 | 前 | 後 |
| --- | --- | ---: | ---: |
| A0 | spot rings=100 | 91 | 95 |
| A0 | wavefront n=512 | 182 | 173 |
| C0 | spot rings=100 | 114 | 107 |
| C0 | wavefront n=512 | 235 | 216 |
| D0 | spot rings=100 | 63 | 60 |
| D0 | wavefront n=512 | 150 | 103 |
| E0 | spot rings=100 | 136 | 83 |
| E0 | wavefront n=512 | 174 | 114 |

追跡だけ（`cargo run --release --example bench`、1 コア）では wasm（wasmtime・Node）がネイティブの約 0.7〜1.0 倍（前は 0.55〜0.9 倍）。
MTF・PSF は追跡が律速でないので変わらない。

## 公開（Cloudflare Pages ＋ Cloudflare Access、限定公開）

配る形のフォルダはそのまま Cloudflare Pages に置ける。`_headers` で COOP/COEP を付ける（並列化に要る）。
閲覧できる人は Cloudflare Access で限る。限定公開の版（非公開のカタログを同梱した版）の配備と手順は非公開側にある。

## 別の入口（カタログ・プラグインを足した版）

この crate（`lensbench-web`）を依存にした wasm の crate を作り、`#[wasm_bindgen(start)]` でカタログの形式・
プラグインを登録して `lensbench_web::set_extra` で同梱するファイルと硝材・レンズのフォルダを足す。JS の口
（`start`・`worker_*`・`monitor_*`）とページ（`web/` の index.html・worker.js・monitor.js）はこの crate のものを
そのまま使う。wasm-bindgen の出力の名前は `--out-name lensbench_web` にそろえる。
