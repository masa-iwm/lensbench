# Rust API リファレンス

`cargo doc` で作った各 crate の説明です（main の最新）。外のクレートから拡張を足すときに依存するのは `lensbench_sdk` だけで、
約束になるのはそこが再輸出する型です（[ライブラリ（Rust）](../rust.md) の拡張の約束）。ほかの crate の `pub` は中の都合で変わります。

| crate | 中身 |
|---|---|
| [lensbench_sdk](../api/rust/lensbench_sdk/index.html) | 拡張の SDK: 外のクレートが解析・プラグイン・検出器・面・形・材料を足し、自分の exe・wheel を組む入口 |
| [lensbench_core](../api/rust/lensbench_core/index.html) | 順次光線追跡の中心（システム、追跡、パラメータ、編集、微分、解き、OPD、コーティング、プラグインの約束） |
| [lensbench_formats](../api/rust/lensbench_formats/index.html) | core の上のファイルの形式（.zmx の読込と書き出し） |
| [lensbench_analysis](../api/rust/lensbench_analysis/index.html) | 実光線の解析（スポット・波面・Zernike・MTF・PSF・POP など、解析ごとの型つきの定義）と同梱のプラグイン |
| [lensbench_project](../api/rust/lensbench_project/index.html) | ブロックで組むプロジェクトファイルと面への展開、JSON スキーマ |
| [lensbench_catalog](../api/rust/lensbench_catalog/index.html) | カタログの品の検索・絞り込み・品の項目の補完、STEP からの寸法の測定 |
| [lensbench_step](../api/rust/lensbench_step/index.html) | STEP（ISO 10303-21）の読み取り |
| [lensbench_design](../api/rust/lensbench_design/index.html) | 評価関数の項・モンテカルロ公差・差し替え探索（設定と結果の JSON）、コーティングのモデルの当てはめ |
| [lensbench_optim](../api/rust/lensbench_optim/index.html) | 減衰最小二乗の最適化（評価関数の項・ZPL マクロの項）とズーム軌跡 |
| [lensbench_runs](../api/rust/lensbench_runs/index.html) | 一括実行（並列・JSONL・途中から再開）、共通の乱数と検出器の雑音 |
| [lensbench_export](../api/rust/lensbench_export/index.html) | 書き出し（レイアウトの 2D・3D・STEP、追跡結果の CSV・JSON・npz） |
| [lensbench_engine](../api/rust/lensbench_engine/index.html) | 実行の層: 解析の登録簿・システムの読み方・serve の場（`Session::call`）、型で渡す計算の要求（`Request`） |
| [lensbench_cli](../api/rust/lensbench_cli/index.html) | CLI（引数の解釈・serve の要求を運ぶ道） |

手元で見るときは `cargo doc --no-deps --open -p lensbench-core`。よく使う入口の説明は [ライブラリ（Rust）](../rust.md)。
