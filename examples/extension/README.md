# 拡張の雛形

外のクレートから lensbench に足すものの雛形です。別のリポジトリで始めるときは、このフォルダを写し、`Cargo.toml` の
`lensbench-sdk` を git のタグ（または rev）の参照に替えます。

- `src/lib.rs`: 足すもの（配列のプラグイン `example_gain`、解析 `example-chief` と GUI の見せ方、面の形 `example_saddle`
  （種類 USERSHAPE）、材料のカタログ `EXAMPLE`、Python の関数 `example_double`）を `extensions()` 1 つにまとめる
- `src/bin/cli.rs`・`src/bin/gui.rs`: それを渡す exe の入口
- `src/lib.rs` の `web`: Web 版の入口（wasm32 だけ）
- `tests/check.rs`: 足し忘れの検査（`lensbench_sdk::check`）とプラグインの試験
- 機能 `py` は Python の wheel（cdylib）を組むときだけ付ける。exe（`example-cli`・`example-gui`）は `py` なしで組む
  （`py` は Python の拡張モジュールとして組む設定で、exe には要らない）

## リポジトリの外へ写す

公開リポジトリの中では相対パスで組みますが、写した先ではそのパスが無いので、次の 3 つを同じタグ（または rev）に
そろえます（v0.6.0 の例。v0.6.0 のタグで、写した先の `cargo check --features gui` が通ることを確かめています）。

1. `[dependencies]` と 2 つの `[target.…dependencies]` の `lensbench-sdk` を git の参照に:
   `lensbench-sdk = { git = "https://github.com/masa-iwm/lensbench", tag = "v0.6.0" }`（`features` はそのまま）
2. 末尾の `[patch.crates-io]` の `winit`・`branches` も同じ git の参照に（cargo がリポジトリの中から同じ名前の
   クレートを探します）: `winit = { git = "https://github.com/masa-iwm/lensbench", tag = "v0.6.0" }`
3. 公開リポジトリの同じタグの `rust-toolchain.toml` を、写した先の `Cargo.toml` の隣に写す（SDK は公開の CI と
   同じ Rust の版で組む前提です）

Cargo.lock は公開のものを写してから組むと、公開の CI で試した依存の版になります（`cargo` が雛形の分だけ足す）。

```
cp ../../Cargo.lock .   # 写した先では、公開リポジトリの同じタグの Cargo.lock
cargo run --bin example-cli -- example-chief lens.zmx
cargo run --features gui --bin example-gui
# Web 版（公開リポジトリの同じタグの checkout で。THREADS=0 でスレッドありの版を作らない）
CRATE=lensbench-extension-example MANIFEST=<この Cargo.toml> scripts/build_web.sh <出力先>
```

約束の型と版の決まりは [ライブラリ（Rust）](../../docs/rust.md) の「拡張の約束」。
