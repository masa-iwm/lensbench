# Rust API リファレンス

`cargo doc` で作った各 crate の説明です（main の最新）。

| crate | 中身 |
|---|---|
| [lensbench_core](../api/rust/lensbench_core/index.html) | 光線追跡・解析のライブラリ（系、追跡、パラメータ、編集、微分、解き、膜、プラグインの約束） |
| [lensbench_project](../api/rust/lensbench_project/index.html) | ブロックで組むプロジェクトファイルと面への展開、JSON スキーマ |
| [lensbench_export](../api/rust/lensbench_export/index.html) | 書き出し（レイアウトの 2D・3D・STEP、追跡結果の CSV・JSON・npz） |
| [lensbench_analysis](../api/rust/lensbench_analysis/index.html) | 同梱のプラグイン: 系を追跡する解析（スポット・MTF・PSF など） |
| [lensbench_engine](../api/rust/lensbench_engine/index.html) | 解析の登録簿・系の読み方・serve の場（`Session::call`） |
| [lensbench_cli](../api/rust/lensbench_cli/index.html) | CLI（引数の解釈・serve の要求を運ぶ道） |

手元で見るときは `cargo doc --no-deps --open -p lensbench-core`。よく使う入口の説明は [ライブラリ（Rust）](../rust.md)。
