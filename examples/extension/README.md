# 拡張の雛形

外のクレートから lensbench に足すものの雛形です。別のリポジトリで始めるときは、このフォルダを写し、`Cargo.toml` の
`lensbench-sdk` を git のタグ（または rev）の参照に替えます。

- `src/lib.rs`: 足すもの（配列のプラグイン `example_gain`、解析 `example-chief` と GUI の見せ方、Python の関数
  `example_double`）を `extensions()` 1 つにまとめる
- `src/bin/cli.rs`・`src/bin/gui.rs`: それを渡す exe の入口
- `src/lib.rs` の `web`: Web 版の入口（wasm32 だけ）
- `tests/check.rs`: 足し忘れの検査（`lensbench_sdk::check`）とプラグインの試験

Cargo.lock は公開のものを写してから組むと、公開の CI で試した依存の版になります（`cargo` が雛形の分だけ足す）。

```
cp ../../Cargo.lock .   # 写した先では、公開リポジトリの同じタグの Cargo.lock
cargo run --bin example-cli -- example-chief lens.zmx
cargo run --features gui --bin example-gui
# Web 版（公開リポジトリの同じタグの checkout で。THREADS=0 でスレッドありの版を作らない）
CRATE=lensbench-extension-example MANIFEST=<この Cargo.toml> scripts/build_web.sh <出力先>
```

約束の型と版の決まりは [ライブラリ（Rust）](../../docs/rust.md) の「拡張の約束」。
