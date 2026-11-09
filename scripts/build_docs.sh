#!/usr/bin/env bash
# ドキュメントサイトを作る: mdBook の本（docs/）＋自動生成（CLI・Python API のリファレンス、CLI の例の出力と図、
# GUI の画面）＋ Rust API（cargo doc）。
#   scripts/build_docs.sh [出力先]     # 既定 target/book
# 要るもの: mdbook（cargo install mdbook）、Python の numpy・matplotlib。GUI の画面は xvfb-run があれば撮る
# （なければ撮らず、GUI のページの図は抜ける）。PROFILE で cargo のプロファイル（既定 ci）。
# 出力: <出力先>/index.html（本）、<出力先>/api/rust/（cargo doc）
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-target/book}
profile=${PROFILE:-ci}

cargo build --profile "$profile" -p lensbench-cli
cli="target/$profile/lensbench"
python3 scripts/gen_docs.py --cli "$cli"
python3 scripts/docs_examples.py "$cli"

# GUI の画面: 画面の通し確認（--tour）で撮ったものを使う
rm -rf docs/gen/gui
if command -v xvfb-run >/dev/null; then
  cargo build --profile "$profile" -p lensbench-gui
  xvfb-run -a -s "-screen 0 1400x900x24" "target/$profile/lensbench-gui" --tour target/docs-gui-tour \
    --glass-dir tests/glass --lens-catalog examples/lenses/LB examples/projects/P8_catalog_two.json
  mkdir -p docs/gen/gui
  cp target/docs-gui-tour/*.png docs/gen/gui/
else
  echo "xvfb-run がないので GUI の画面は撮らない" >&2
fi

mdbook build -d "$PWD/$out"

cargo doc --no-deps --lib -p lensbench-core -p lensbench-project -p lensbench-export -p lensbench-analysis \
  -p lensbench-cli
mkdir -p "$out/api"
rm -rf "$out/api/rust"
cp -r target/doc "$out/api/rust"
echo "docs: $out"
