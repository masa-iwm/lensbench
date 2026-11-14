# lensbench

カタログレンズを組み合わせた光学系の評価・最適化のための、順次光線追跡ツール（開発中）。
A sequential ray-tracing tool for evaluating and optimizing systems built from stock lenses (work in progress).

**ドキュメント: https://masa-iwm.github.io/lensbench/** （使い方・CLI・Python・Rust API）、**Web 版: https://masa-iwm.github.io/lensbench/app/**

- Windows の GUI（インストール不要の単一 exe）と、Linux・Windows の CLI
- .zmx の読込（面のまま、または玉ごと）、ガラスカタログ（AGF 形式）。曲率・厚さのソルブ（OPD の厚さのソルブも）とピックアップ（別の列から、半直径・材料・面の後の傾き偏心も）は読込とパラメータの変更で解き直す
- スポット、波面・Zernike、MTF、干渉縞、公差モンテカルロ、最適化
- 計算結果は OpticStudio の出力と数値で照合している（照合に使った基準データは `tests/` に置く）

Zemax および OpticStudio は Ansys, Inc. の商標です。lensbench は Ansys とは関係がなく、その製品の互換品ではありません。

## すぐ試す

```sh
cargo build --release
target/release/lensbench spot tests/refs/A0/system.zmx --glass-dir tests/glass     # CLI
target/release/lensbench-gui examples/projects/P8_catalog_two.json                 # GUI
```

Windows の GUI の exe は CI の成果物 `lensbench-gui-windows` から取れる（Python の wheel も CI の成果物にある）。

## ドキュメント

| ページ | 中身 |
|---|---|
| [GUI](https://masa-iwm.github.io/lensbench/gui.html) | 画面の使い方（図つき） |
| [CLI](https://masa-iwm.github.io/lensbench/cli.html) | コマンドの例と出力の図。最適化・公差・ズーム・差し替え探索・書き出しなどは各ページ |
| [プロジェクトファイル](https://masa-iwm.github.io/lensbench/project-file.html) | ブロックで系を組む JSON（状態・群・BS） |
| [serve・watch](https://masa-iwm.github.io/lensbench/serve.html) | 別アプリとの解析ループ |
| [Python](https://masa-iwm.github.io/lensbench/python-guide.html) | Python バインディング（`import lensbench`） |
| [リファレンス](https://masa-iwm.github.io/lensbench/reference/cli.html) | CLI・Python API・Rust API・JSON スキーマ（自動生成） |

ドキュメントの元は `docs/`（mdBook）。手元では `scripts/build_docs.sh` で `target/book` に作る。

## 構成

| ディレクトリ | 中身 |
|---|---|
| `crates/lensbench-core` | 光線追跡・解析のライブラリ |
| `crates/lensbench-project` | ブロックで組むプロジェクトファイルと面への展開 |
| `crates/lensbench-design` | 評価関数の項・モンテカルロ公差・差し替え探索の設定と結果の JSON |
| `crates/lensbench-export` | 書き出し（レイアウトの 2D・3D・STEP、追跡結果の CSV・JSON・npz） |
| `crates/lensbench-analysis` | 同梱のプラグイン（系を追跡する解析） |
| `crates/lensbench-verify` | 照合試験の枠組み（`lensbench verify`） |
| `crates/lensbench-engine` | 実行の層: 解析の登録簿・系の読み方・serve の場（`Session::call`）。CLI・GUI・Web 版が使う |
| `crates/lensbench-cli` | コマンドライン（`lensbench`）。引数の解釈と serve の要求を運ぶ道（TCP・パイプ） |
| `crates/lensbench-gui` | GUI（`lensbench-gui`、eframe・egui）。計算は serve と同じ API をプロセス内で呼ぶ |
| `crates/lensbench-py` | Python バインディング（pyo3・maturin、ワークスペースのメンバー） |
| `clients/python` | `lensbench serve` の Python クライアント（標準ライブラリだけ） |
| `docs` | ドキュメント（mdBook の元）と JSON スキーマ（`docs/schema`） |
| `scripts/check_public.py` | 公開できないファイル（メーカーのカタログなど）が入っていないかの検査。CI で全履歴に対して実行 |

メーカーのレンズ・ガラス・膜のカタログはこのリポジトリに含めない。利用者が手元のファイルを指定して読む。

## ビルドとテスト

```sh
cargo build --release
cargo test --workspace --release        # CI は --profile ci（同じ最適化で LTO なし、ビルドが速い）
python3 scripts/check_public.py --history
```

速さの測り方・テスト用データは [ビルドとテスト](https://masa-iwm.github.io/lensbench/build.html)。

開発は非公開側でまとめて行い、このリポジトリの main へは公開してよい部分を書き出しています。このリポジトリでは Pull Request を受け付けていません（Issue は受け付けます）。

## ライセンス

MIT または Apache-2.0 のどちらか（`LICENSE-MIT`、`LICENSE-APACHE`）。
