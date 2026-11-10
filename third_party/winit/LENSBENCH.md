# winit 0.30.13（lensbench の修正入り）

crates.io の winit 0.30.13 に、Windows で拡大率の違うモニターの間を引きずったときの修正を 1 か所当てたもの（差分は `LENSBENCH_PATCH.diff`）。
ルートの `Cargo.toml` の `[patch.crates-io]` で差し替える。

- 不具合: 引きずり中の WM_DPICHANGED で、winit が Windows の勧める位置ではなく自前の位置に窓を置くため、窓が元のモニター側に戻って拡大率の変更が繰り返され、変わる前の大きさを新しい拡大率で換算して窓が 1/1.5・1.5 倍に飛ぶ（公開 #84）。
- 修正: 引きずり中は Windows の勧める位置（suggested rect）をそのまま使う。winit 本家では 0.31 系に同じ修正（rust-windowing/winit#4341）が入っている。iced-rs/winit#20 と同じ差分。
- eframe が winit 0.31 以降に移ったら、このフォルダと `[patch.crates-io]` を外す。
