# lensbench

カタログレンズを組み合わせた光学系の評価・最適化のための、順次光線追跡ツール（開発中）。
A sequential ray-tracing tool for evaluating and optimizing systems built from stock lenses (work in progress).

**ドキュメント: https://masa-iwm.github.io/lensbench/** （使い方・CLI・Python・Rust API）、**Web 版: https://masa-iwm.github.io/lensbench/app/**

- Windows の GUI（インストール不要の単一 exe）、Linux・Windows の CLI、Python、ブラウザの Web 版
- .zmx（面のまま、または玉ごと）と AGF のガラスカタログを読み、カタログのレンズをブロックで組む
- スポット・波面・Zernike・MTF・PSF・簡易 POP・ゴースト・干渉縞・膜と偏光、最適化・モンテカルロ公差・ズーム・カタログレンズの差し替え探索
- 計算結果は OpticStudio の出力と数値で照合している（照合に使った基準データは `tests/` に置く）
- 言語: エラー・CLI の文言は英語が元で、日本語の訳を出来るだけ付ける（`LENSBENCH_LANG=ja`・`en`、無ければ OS の言語）。GUI は日本語と英語を選べる。ドキュメントは日本語。エラーの文の英語への移しかえは途中（まだ日本語だけの文がある）

Zemax および OpticStudio は Ansys, Inc. の商標です。lensbench は Ansys とは関係がなく、その製品の互換品ではありません。

## すぐ試す

CLI だけなら速くできる（GUI・Web 版・Python は作らない）:

```sh
cargo build --release -p lensbench-cli
target/release/lensbench spot tests/refs/A0/system.zmx --glass-dir tests/glass
# examples のプロジェクトはレンズカタログを渡す（ガラスは同梱の SCHOTT・OHARA で足りる）
target/release/lensbench info examples/projects/P8_catalog_two.json --lens-catalog examples/lenses
```

GUI は `cargo build --release -p lensbench-gui` のあと `target/release/lensbench-gui examples/projects/P8_catalog_two.json`。
Windows の GUI の exe と Python の wheel は CI の成果物（`lensbench-gui-windows` など、7 日で消える）からも取れる。

## ドキュメント

使い方・CLI・Python・Rust API・構成（ディレクトリごとの中身）は **https://masa-iwm.github.io/lensbench/** 。
元は `docs/`（mdBook）で、ビルドとテストの手順は [ビルドとテスト](https://masa-iwm.github.io/lensbench/build.html)。

メーカーのレンズ・ガラス・膜のカタログはこのリポジトリに含めない。利用者が手元のファイルを指定して読む。

開発は非公開側でまとめて行い、このリポジトリの main へは公開してよい部分を書き出しています。このリポジトリでは Pull Request を受け付けていません（Issue は受け付けます）。
コードの書き方の約束は [CONTRIBUTING.md](CONTRIBUTING.md)。

## ライセンス

MIT または Apache-2.0 のどちらか（`LICENSE-MIT`、`LICENSE-APACHE`）。
