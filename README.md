# lensbench

カタログレンズを組み合わせた光学系の評価・最適化のための、順次光線追跡ツール（開発中）。
A sequential ray-tracing tool for evaluating and optimizing systems built from stock lenses (work in progress).

- Windows の GUI（インストール不要の単一 exe）と、Linux・Windows の CLI
- .zmx の読込（面のまま、または玉ごと）、ガラスカタログ（AGF 形式）
- スポット、波面・Zernike、MTF、干渉（低コヒーレンス含む）、SH 波面計測の模擬、公差モンテカルロ、最適化
- 計算結果は OpticStudio の出力と数値で照合している（照合に使った基準データは `tests/` に置く）

Zemax および OpticStudio は Ansys, Inc. の商標です。lensbench は Ansys とは関係がなく、その製品の互換品ではありません。

## 構成

| ディレクトリ | 中身 |
|---|---|
| `crates/lensbench-core` | 光線追跡・解析のライブラリ |
| `crates/lensbench-cli` | コマンドライン（`lensbench`） |
| `scripts/check_public.py` | 公開できないファイル（メーカーのカタログなど）が入っていないかの検査。CI で全履歴に対して実行 |

メーカーのレンズ・ガラス・膜のカタログはこのリポジトリに含めない。利用者が手元のファイルを指定して読む。

## ビルドとテスト

```
cargo build --release
cargo test --workspace --release
python3 scripts/check_public.py --history
```

## 使い方（CLI）

視野・波長・コンフィグの番号は 1 始まり。`--json` で JSON 出力。

```
lensbench info     system.zmx --glass-dir <AGF のフォルダ>
lensbench spot     system.zmx --field 2 --wave 1
lensbench wavefront system.zmx --n 64
lensbench zernike  system.zmx --kind fringe --terms 37
```

ガラスカタログは `--glass-dir`、環境変数 `LENSBENCH_GLASSCAT`（パス区切りで複数）、.zmx と同じフォルダの順に探す。

## テスト用データ

- `tests/glass/`: refractiveindex.info のデータベース（CC0）から `scripts/make_test_glass.py` で作った AGF。メーカー配布のカタログではない
- `tests/refs/`: lensbench 独自の処方のテスト光学系（トリプレット、傾き偏心、理想レンズ、非球面など）について OpticStudio で求めた基準値

## ライセンス

MIT または Apache-2.0 のどちらか（`LICENSE-MIT`、`LICENSE-APACHE`）。
