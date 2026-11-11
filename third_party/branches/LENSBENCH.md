# branches 0.3.0（lensbench の修正入り）

crates.io の branches 0.3.0 から `build.rs` を外したもの（他はそのまま）。ルートの `Cargo.toml` の `[patch.crates-io]` で差し替える。

- 不具合: 元の `build.rs` は nightly のコンパイラだと `cfg(unstable)` を立て、`abort()` が `core::intrinsics::abort()` を呼ぶ。
  今の nightly にはこの関数が無いのでコンパイルできない。Web 版のスレッドありの版（mt/）は nightly の build-std で作るので、
  truck（STEP の読み込み、rclite 経由で branches を使う）を入れると止まる。
- 修正: `build.rs` を外して、nightly でも stable と同じ道（`std::process::abort()`）を使う。
- rclite が branches 0.5 以降に移ったら（truck-topology が新しい rclite を使ったら）、このフォルダと `[patch.crates-io]` を外す。
