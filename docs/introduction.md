# lensbench

カタログレンズを組み合わせた光学系の評価・最適化のための、順次光線追跡ツール（開発中）。
A sequential ray-tracing tool for evaluating and optimizing systems built from stock lenses (work in progress).

**ブラウザで試す: [Web 版](app/)**（GUI をそのままブラウザで動かす。見本のシステムだけ同梱）

- Windows の GUI（インストール不要の単一 exe）、Linux・Windows の CLI、Python、ブラウザの Web 版
- .zmx（面のまま、または玉ごと）と AGF のガラスカタログを読み、カタログのレンズをブロックで組む
- スポット・波面・Zernike・MTF・PSF・簡易 POP・ゴースト・干渉縞・膜と偏光、最適化・モンテカルロ公差・ズーム・カタログレンズの差し替え探索
- 計算結果は OpticStudio の出力と数値で照合している（照合に使った基準データは `tests/` に置く）

Zemax および OpticStudio は Ansys, Inc. の商標です。lensbench は Ansys とは関係がなく、その製品の互換品ではありません。

## 構成

| ディレクトリ | 中身 |
|---|---|
| `crates/lensbench-core` | 光線追跡・解析のライブラリ |
| `crates/lensbench-formats` | core の上のファイルの形式（.zmx の読込と書き出し） |
| `crates/lensbench-step` | STEP（ISO 10303-21）の読み取り。依存なし |
| `crates/lensbench-catalog` | カタログの検索・絞り込み・品の項目の補完、STEP からの寸法の測定（project の上） |
| `crates/lensbench-project` | ブロックで組むプロジェクトファイルと面への展開 |
| `crates/lensbench-runs` | 一括実行（公差・モンテカルロ・掃引を並列に回し、JSONL に書いて途中から再開）、共通の乱数と検出器の雑音 |
| `crates/lensbench-optim` | 減衰最小二乗の最適化（評価関数の項・ZPL マクロの項 ZPLM）とズーム軌跡 |
| `crates/lensbench-design` | 評価関数の項・モンテカルロ公差・差し替え探索（設定と結果の JSON、公差の計算）、膜のモデルの当てはめと設計 |
| `crates/lensbench-export` | 書き出し（レイアウトの 2D・3D・STEP、追跡結果の CSV・JSON・npz） |
| `crates/lensbench-cad` | メーカーの品の CAD データ（公式の STEP）の取得・置き場・形の測定、膜のモデルの取得 |
| `crates/lensbench-analysis` | 同梱のプラグイン: システムを追跡する解析（スポット・MTF・PSF など）、Huygens PSF・MTF・ゴーストの経路・単色の干渉縞・偏光の格子の計算 |
| `crates/lensbench-verify` | 照合試験の枠組み（`lensbench verify`） |
| `crates/lensbench-engine` | 実行の層: 解析の登録簿・システムの読み方・serve の場（`Session::call`）。CLI・GUI・Web 版が使う |
| `crates/lensbench-cli` | コマンドライン（`lensbench`）。引数の解釈と serve の要求を運ぶ道（TCP・パイプ） |
| `crates/lensbench-gui` | GUI（`lensbench-gui`、eframe・egui）。計算は serve と同じ API をプロセス内で呼ぶ |
| `crates/lensbench-web` | Web 版（GUI を wasm にしたもの） |
| `crates/lensbench-py` | Python バインディング（pyo3・maturin、ワークスペースのメンバー） |
| `clients/python` | `lensbench serve` の Python クライアントの試験（クライアントは `crates/lensbench-py/lensbench_client.py`、wheel に同梱） |
| `examples` | 見本のプロジェクトファイル・組み合わせ・ユーザーカタログのレンズ・Python の例・serve のループの例 |
| `tests` | 照合とテストのデータ（下の「テスト用データ」） |
| `third_party` | 手を入れた依存の写し（winit の混在 DPI の修正など、`[patch]` で差し替え） |
| `docs` | このドキュメント（mdBook の元）と JSON スキーマ（`docs/schema`） |
| `scripts/check_public.py` | 公開できないファイル（メーカーのカタログなど）が入っていないかの検査。CI で全履歴に対して実行 |

メーカーのレンズ・ガラス・膜のカタログはこのリポジトリに含めない。利用者が手元のファイルを指定して読む。

## テスト用データ

- `tests/glass/`: refractiveindex.info のデータベース（CC0）から `scripts/make_test_glass.py` で作った AGF。メーカー配布のカタログではない
- `tests/verify/`: 照合試験の例題
- `tests/refs/`: lensbench 独自の処方のテスト光学系（トリプレット、傾き偏心、理想レンズ、非球面など）について OpticStudio で求めた基準値

## ライセンス

MIT または Apache-2.0 のどちらか（`LICENSE-MIT`、`LICENSE-APACHE`）。
