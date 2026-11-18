# 拡張の雛形

外のクレートから lensbench に足すものの雛形です。別のリポジトリで始めるときは、このフォルダを写し、`Cargo.toml` の
`lensbench-sdk` を git のタグ（または rev）の参照に替えます。

- `src/lib.rs`: 足すもの（配列のプラグイン `example_gain`、解析 `example-chief` と GUI の見せ方、Python の関数
  `example_double`）を `extensions()` 1 つにまとめる
- `src/bin/cli.rs`・`src/bin/gui.rs`: それを渡す exe の入口
- `tests/check.rs`: 足し忘れの検査（`lensbench_sdk::check`）とプラグインの試験

```
cargo run --bin example-cli -- example-chief lens.zmx
cargo run --features gui --bin example-gui
```

約束の型と版の決まりは [ライブラリ（Rust）](../../docs/rust.md) の「拡張の約束」。
