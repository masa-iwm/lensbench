# lensbench

カタログレンズを組み合わせた光学系の評価・最適化のための、順次光線追跡ツール（開発中）。
A sequential ray-tracing tool for evaluating and optimizing systems built from stock lenses (work in progress).

**ブラウザで試す: [Web 版](app/)**（GUI をそのままブラウザで動かす。見本の系だけ同梱）

- Windows の GUI（インストール不要の単一 exe）と、Linux・Windows の CLI
- .zmx の読込（面のまま、または玉ごと）、ガラスカタログ（AGF 形式）。曲率・厚さのソルブ（OPD の厚さのソルブも）とピックアップ（別の列から、半直径・材料・面の後の傾き偏心も）は読込とパラメータの変更で解き直す
- スポット、波面・Zernike、MTF、干渉縞、公差モンテカルロ、最適化
- 計算結果は OpticStudio の出力と数値で照合している（照合に使った基準データは `tests/` に置く）

Zemax および OpticStudio は Ansys, Inc. の商標です。lensbench は Ansys とは関係がなく、その製品の互換品ではありません。


## 構成

| ディレクトリ | 中身 |
|---|---|
| `crates/lensbench-core` | 光線追跡・解析のライブラリ |
| `crates/lensbench-project` | ブロックで組むプロジェクトファイルと面への展開 |
| `crates/lensbench-export` | 書き出し（レイアウトの 2D・3D・STEP、追跡結果の CSV・JSON・npz） |
| `crates/lensbench-analysis` | 同梱のプラグイン: 系を追跡する解析（スポット・MTF・PSF など） |
| `crates/lensbench-verify` | 照合試験の枠組み（`lensbench verify`） |
| `crates/lensbench-engine` | 実行の層: 解析の登録簿・系の読み方・serve の場（`Session::call`）。CLI・GUI・Web 版が使う |
| `crates/lensbench-cli` | コマンドライン（`lensbench`）。引数の解釈と serve の要求を運ぶ道（TCP・パイプ） |
| `crates/lensbench-gui` | GUI（`lensbench-gui`、eframe・egui）。計算は serve と同じ API をプロセス内で呼ぶ |
| `crates/lensbench-py` | Python バインディング（pyo3・maturin、ワークスペースのメンバー） |
| `clients/python` | `lensbench serve` の Python クライアント（標準ライブラリだけ） |
| `docs` | このドキュメント（mdBook の元）と JSON スキーマ（`docs/schema`） |
| `scripts/check_public.py` | 公開できないファイル（メーカーのカタログなど）が入っていないかの検査。CI で全履歴に対して実行 |

メーカーのレンズ・ガラス・膜のカタログはこのリポジトリに含めない。利用者が手元のファイルを指定して読む。

## テスト用データ

- `tests/glass/`: refractiveindex.info のデータベース（CC0）から `scripts/make_test_glass.py` で作った AGF。メーカー配布のカタログではない
- `tests/verify/`: 照合試験の例題
- `tests/refs/`: lensbench 独自の処方のテスト光学系（トリプレット、傾き偏心、理想レンズ、非球面など）について OpticStudio で求めた基準値

## ライセンス

MIT または Apache-2.0 のどちらか（`LICENSE-MIT`、`LICENSE-APACHE`）。
