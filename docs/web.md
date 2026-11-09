# Web 版

GUI（lensbench-gui）を wasm にしてブラウザで動かす。コードはネイティブと共通で、違うのは次の所だけ
（`cfg(target_arch = "wasm32")` で切り替える）。

| 所 | ネイティブ | Web |
|---|---|---|
| ファイル（`lensbench_core::vfs`） | `std::fs` | プロセスの中の置き場（パス → 中身）。見本は起動時に入れる |
| 計算（`worker.rs`） | 別のスレッド | Web Worker（`worker.js`、中は同じ wasm）。ファイルの置き場は要求・結果と一緒に互いに写す（`wire.rs`） |
| 並列（`lensbench_core::par`） | rayon | ページが cross-origin isolated なら、計算の Web Worker の中で rayon のスレッド（wasm-bindgen-rayon、`mt/` の版）。そうでなければ逐次 |
| SIMD | AVX（実行時に判定） | simd128（両方の版） |
| ファイルの窓（`files.rs`） | rfd | 開く: ブラウザのファイル選択（複数選べる）で `/uploads/` に入れる。保存: `/downloads/` に書いてダウンロード。窓に落としても開ける |
| 時計 | `std::time::Instant` | `web_time::Instant`（ネイティブでは std そのもの） |
| アロケータ | mimalloc | 既定 |
| 日本語フォント | OS のフォント | Noto Sans JP（SIL OFL 1.1）を字を絞って配る（`scripts/web_font.py`） |

Web では使えないもの: serve・watch・run（TCP・別プロセス）、OpticStudio との連携。
今のところ、解析の途中の中断（系を変えると古い結果は捨てるが、計算は最後まで回る）と、最適化・公差・差し替え探索の
進み具合の表示・停止も効かない。

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

Web 版で読み込んだファイルはページを閉じると消える（ブラウザに残すのは今後）。プロジェクトが参照する .zmx・
カタログは、開くときに一緒に選ぶと同じフォルダに入る。

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
  `navigator.hardwareConcurrency`（16 まで）本作る。画面のスレッドと下書きの Worker は逐次のまま

URL に `?st` を付けると 1 スレッドの版になる（比べるとき）。4 コアで P9 の MTF が 104 秒 → 31 秒（11/09）。
Web では `with_threads(Some(n))` も大域のプールを使う（スレッドを足せない）。

## 公開（Cloudflare Pages ＋ Cloudflare Access、限定公開）

`target/web` をそのまま Cloudflare Pages に置く。`_headers` で COOP/COEP を付ける（並列化に要る）。
閲覧できる人は Cloudflare Access で限る。配備は非公開側で行う。

所有者が一度だけ行う設定:

1. Cloudflare のアカウントを作る（https://dash.cloudflare.com/sign-up 、無料プランでよい）。
2. Workers & Pages → Create → Pages → 「Upload assets」で、プロジェクト名（例 `lensbench`）を決めて、
   CI の成果物 `lensbench-web` の `target/web` の中身を上げる。`https://lensbench.pages.dev` で開ける。
   （自動の配備を入れた後は GitHub Actions が push ごとに上げる。そのときは API トークンと Account ID を
   GitHub の Secrets に入れる）
3. Zero Trust（左のメニュー）を開き、チーム名を決める（無料プラン、50 人まで）。
4. Zero Trust → Access → Applications → Add an application → Self-hosted:
   - Application domain に `lensbench.pages.dev` と、プレビュー用に `*.lensbench.pages.dev` を入れる
   - Policy: Action は Allow、Include の Emails に見てよい人のメールアドレスを入れる
   - Login methods は One-time PIN（メールに届く番号でログイン）でよい
5. 別のブラウザ（シークレット窓）で `https://lensbench.pages.dev` を開き、ログインを求められることを確かめる。

Pages の設定の「Access policy」からも同じ設定を始められる。
