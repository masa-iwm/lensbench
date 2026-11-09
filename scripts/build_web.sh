#!/usr/bin/env bash
# Web 版をビルドして配る形のフォルダ（既定 target/web）を作る。docs/web.md
#
# 2 つ作る:
# - そのまま（stable）: どこでも動く 1 スレッドの版
# - mt/（nightly、atomics）: 計算の Web Worker の中で rayon のスレッドを使う版。ページが
#   cross-origin isolated（COOP/COEP のヘッダー）のときだけ使う（index.html・worker.js が選ぶ）
# どちらも simd128 を使う。THREADS=0 で mt/ を作らない
#
# 要るもの: rustup の wasm32-unknown-unknown、mt/ には nightly（$NIGHTLY）と rust-src、
# wasm-bindgen-cli（Cargo.lock の wasm-bindgen と同じ版）、python3 と fonttools（日本語フォントを作る）。
# wasm-opt（binaryen）があれば小さくする
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-target/web}"
PROFILE="${PROFILE:-release}"
THREADS="${THREADS:-1}"
NIGHTLY="${NIGHTLY:-nightly-2026-10-15}"

bindgen() { # wasm 出力先
  wasm-bindgen --target web --no-typescript --out-dir "$2" "$1"
  if command -v wasm-opt >/dev/null; then
    wasm-opt -O2 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
      --enable-mutable-globals --enable-reference-types --enable-simd --enable-threads \
      "$2/lensbench_web_bg.wasm" -o "$2/lensbench_web_bg.wasm"
  fi
}

RUSTFLAGS="-C target-feature=+simd128" \
  cargo build --profile "$PROFILE" --target wasm32-unknown-unknown -p lensbench-web
rm -rf "$OUT"
mkdir -p "$OUT/fonts"
bindgen "target/wasm32-unknown-unknown/$PROFILE/lensbench_web.wasm" "$OUT"
if [ "$THREADS" != 0 ]; then
  # 共有メモリ（外から渡す）とスレッドごとの TLS の口。上限は 4 GiB（既定のままだと足りない）
  L="-C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--max-memory=4294967296"
  L="$L -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size"
  L="$L -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base"
  RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+simd128 $L" \
    cargo "+$NIGHTLY" build --profile "$PROFILE" --target wasm32-unknown-unknown -p lensbench-web \
    --features threads -Z build-std=panic_abort,std --target-dir target/mt
  bindgen "target/mt/wasm32-unknown-unknown/$PROFILE/lensbench_web.wasm" "$OUT/mt"
  # wasm-bindgen-rayon のスレッドの補助は '../../..'（バンドラ向け）で本体を読むので、ファイルを名指しにする
  H=$(find "$OUT/mt/snippets" -name workerHelpers.js)
  grep -q "import('../../..')" "$H"
  sed -i "s#import('../../..')#import('../../../lensbench_web.js')#" "$H"
fi
cp crates/lensbench-web/web/{index.html,_headers,icon.png,worker.js,monitor.js,coi.js} "$OUT/"
python3 scripts/web_font.py "$OUT/fonts/jp.ttf"
ls -la "$OUT" "$OUT/fonts"
