# ビルドとテスト

```
cargo build --release
cargo test --workspace --release        # CI は --profile ci（同じ最適化で LTO なし、ビルドが速い）
python3 scripts/check_public.py --history
```

速さ: `cargo bench -p lensbench-core --bench trace`（criterion。tests/refs の A0・C0・A2・C1 を 2 万本、1 スレッドと全コア）。
ほかの実装と同じシステム・光線数で比べるときは `cargo run --release -p lensbench-core --example bench -- system.zmx --rays 200000 --threads 4 --json`
（瞳の点列は `examples/bench.rs` の xorshift64 で再現できる）。
偏光なしの追跡と実光線エイミングは 16 本ずつまとめて追う（SIMD、x86_64 は AVX を実行時に選ぶ）。
結果は 1 本ずつの追跡とビット単位で同じ（`tests/batch.rs`）。比較用に `lensbench_core::par::with_batch(false, || ..)` の中では切れる（計算の文脈。作業のスレッドにも引き継ぐ）。
実光線エイミングは 1024 本以上のとき、格子の光線から初期値を当てはめて解く。格子は入力の瞳座標の範囲に置くので、同じ瞳座標でも
一度に追う光線の組（1024 本以上か未満か、座標の範囲）で結果がわずかに変わる（絞りでの位置はどちらも収束の許容差の範囲、tests/refs の像面で 2e-11 mm 以下。ビット単位の再現はスレッド数と
まとめ追跡の有無についてで、光線の組についてではない）。

### 試験の環境変数

テストのふるまいを変える環境変数（`LB_*` など）。ほかに `LENSBENCH_LANG` はエラーの文の言語（`.cargo/config.toml` で `en`）。

| 変数 | 効く所 | 意味 |
| --- | --- | --- |
| `LB_REFS_DIR` | 結合テスト（`tests/common.rs`）・照合 | 基準データ（`tests/refs`）のフォルダを差し替える |
| `LB_GLASS_DIR` | 結合テスト・照合 | テスト用ガラス（`tests/glass`）のフォルダを差し替える |
| `LB_REFS` | `lensbench-verify` の照合のテスト | 照合するケースの名前を絞る |
| `LB_PRINT_FWD` | `lensbench-optim` の試験 | 差分のヤコビアンの比べを標準エラーに出す（調べ用） |
| `LB_STEP_FILES` | `lensbench-project` の無視したテスト | 手元の STEP（`;` 区切り）を測って出す（`--ignored` で回す） |
| `LENSBENCH_UPDATE_SCHEMA` | `lensbench-design`・`lensbench-project` の schema の試験 | `docs/schema` の JSON スキーマを型から書き直す |
| `LENSBENCH_UPDATE_SAMPLES` | `lensbench-gui` の `sample_combos` の試験 | `examples/combos` の見本の組合せを作り直す |
| `LB_UPDATE_API` | `lensbench-sdk` の `tests/api.rs` | SDK の約束の型の一覧 `crates/lensbench-sdk/API.txt` を書き直す（変えたら [壊れる変更](changes.md) に書く） |
| `LB_PEEK_ROOT`・`LB_PEEK_GLASS`・`LB_PEEK_DUMP`・`LB_PEEK_UNREAD` | `lensbench-gui` の `readable_files_open_as_combos` | 組合せで開けるかを見るフォルダ・足すガラスのフォルダ・開けないファイルの組合せを書くフォルダ・読めない理由を出す |
| `LENSBENCH_TOUR_SIZE` | `lensbench-gui --tour` | 通し確認の窓の大きさ（`1024x700` など） |

## 第三者のライセンス

配る exe・wheel には第三者のクレート（egui の既定のフォント OFL・UFL を含む）が静的に入る。MIT・Apache-2.0・OFL は
バイナリの配布でも著作権表示とライセンス文の同梱を求めるので、配る zip には一覧を添える:

```
python scripts/third_party_licenses.py --out THIRD-PARTY-LICENSES.txt          # GUI・CLI（Windows 向けの依存）
python scripts/third_party_licenses.py --check                                  # 許すライセンスだけか（コピーレフトの混入を止める）
python scripts/third_party_licenses.py --root lensbench-py --manifest crates/lensbench-py/Cargo.toml --out THIRD-PARTY-LICENSES-py.txt
```

## ドキュメント

このサイトの元は `docs/`（mdBook、目次は `docs/SUMMARY.md`）。`scripts/build_docs.sh` で `target/book` に作る（mdbook と Python の
numpy・matplotlib が要る。GUI の画面は xvfb-run があれば撮る）。CLI・Python API のリファレンス（`scripts/gen_docs.py`）、
CLI の例の出力と図（`scripts/docs_examples.py`）、GUI の画面（`lensbench-gui --tour`）、Rust API（`cargo doc`）はそのたびに作り直すので、
リポジトリには入れない。main への push ごとに GitHub Pages に出る。
