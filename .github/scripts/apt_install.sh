#!/usr/bin/env bash
# CI の apt を落ちにくくする: 使う: .github/scripts/apt_install.sh 包み...
#
# - 入っていない包みだけ入れる（ランナーの像に入っていれば apt に触らない）
# - apt 自体の再試行は少なめ（Acquire::Retries=2）・短めのタイムアウト、update は 4 分・deb を落とす段は 5 分で打ち切る
# - 前に入れた deb を APT_DEB_CACHE（既定 ~/.cache/apt-debs、ワークフローの actions/cache で保存）に置き、
#   あればネットを使わずにそこから入れる（11/11 は archive.ubuntu.com も含め Ubuntu の名前が 2 時間以上引けなかった）
# - 取りに行くときは 2 回まで。2 回目は 30 秒おいて、ランナーの既定のミラー（mirror+file:/etc/apt/apt-mirrors.txt、
#   azure.archive.ubuntu.com）をやめて archive.ubuntu.com・security.ubuntu.com を直に使う
#   （11/11 に azure.archive.ubuntu.com の名前解決が数分続けて失敗し、gui-tour・web・Pages が落ちた）
set -uo pipefail

cache="${APT_DEB_CACHE:-$HOME/.cache/apt-debs}"
installed() { dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q 'install ok installed'; }
missing_of() { local p; for p in "$@"; do installed "$p" || echo "$p"; done; }

mapfile -t missing < <(missing_of "$@")
if [ ${#missing[@]} -eq 0 ]; then
  echo "apt: すべて入っている（$*）"
  exit 0
fi
echo "apt: 入れる: ${missing[*]}"

# 1. 保存した deb。ネットを使わない。まず apt の取得済みの置き場として渡し（像の包みの一覧と版が合えば依存も apt が見る）、
#    だめなら入っていない包みの deb を dpkg で直に入れる。それもだめなら途中の状態を戻して取りに行く
if compgen -G "$cache/*.deb" >/dev/null; then
  mkdir -p "$cache/partial"
  if sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-download --no-install-recommends \
       -o Dir::Cache::archives="$cache" -o DPkg::Lock::Timeout=120 "${missing[@]}" && [ -z "$(missing_of "$@")" ]; then
    echo "apt: 保存した deb から入れた"
    exit 0
  fi
  debs=()
  for d in "$cache"/*.deb; do installed "$(dpkg-deb -f "$d" Package)" || debs+=("$d"); done
  if [ ${#debs[@]} -gt 0 ] && sudo DEBIAN_FRONTEND=noninteractive dpkg -i "${debs[@]}" && [ -z "$(missing_of "$@")" ]; then
    echo "apt: 保存した deb を dpkg で入れた（${#debs[@]} 個）"
    exit 0
  fi
  echo "::warning::apt: 保存した deb では入らなかった（像の更新で版が合わない等）。取りに行く"
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-download -f -o DPkg::Lock::Timeout=120 || true
fi

# 2. 取りに行く。落とした deb は保存用に別の場所へ残す
arch=$(mktemp -d)
mkdir -p "$arch/partial"
chmod 755 "$arch"
sudo chown _apt "$arch/partial" 2>/dev/null || true  # apt は _apt で落とす
opts=(-o Acquire::Retries=2 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 -o DPkg::Lock::Timeout=120)

use_direct_mirrors() {
  echo "::warning::apt: 既定のミラーで失敗したので archive.ubuntu.com を直に使う"
  # deb822 形式（ubuntu.sources）と従来の形式（*.list）の両方で mirror+file: の行を置き換える
  for f in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
    [ -f "$f" ] || continue
    sudo sed -i -E 's#mirror\+file:/etc/apt/apt-mirrors.txt/?#http://archive.ubuntu.com/ubuntu/#g; s#https?://azure\.archive\.ubuntu\.com/ubuntu/?#http://archive.ubuntu.com/ubuntu/#g' "$f"
  done
  if [ -f /etc/apt/apt-mirrors.txt ]; then
    printf 'http://archive.ubuntu.com/ubuntu/\tpriority:1\nhttp://security.ubuntu.com/ubuntu/\tpriority:2\n' | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
  fi
}

for attempt in 1 2; do
  if [ "$attempt" -ge 2 ]; then
    sleep 30
    use_direct_mirrors
  fi
  # update は一部の取得の失敗（W:）でも 0 を返すので続け、落とす段を timeout で区切る（DNS の障害中は deb ごとに
  # 再試行を重ねて数十分止まるため。#169 のレビュー）。落とし終えてから入れる（dpkg の途中で切らない）
  inst=(sudo DEBIAN_FRONTEND=noninteractive apt-get "${opts[@]}" install -y -q --no-install-recommends
        -o Dir::Cache::archives="$arch" -o APT::Keep-Downloaded-Packages=true)
  timeout 240 sudo apt-get "${opts[@]}" update -q
  if timeout 300 "${inst[@]}" --download-only "${missing[@]}" && "${inst[@]}" --no-download "${missing[@]}"; then
    rm -rf "$cache" && mkdir -p "$cache" && cp "$arch"/*.deb "$cache"/ 2>/dev/null
    echo "apt: 入れた。deb $(ls "$cache" | wc -l) 個を $cache に残す"
    exit 0
  fi
  echo "::warning::apt: ${attempt} 回目が失敗"
done
echo "::error::apt: 2 回とも失敗（${missing[*]}）"
exit 1
