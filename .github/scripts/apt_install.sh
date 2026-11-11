#!/usr/bin/env bash
# CI の apt を落ちにくくする: 使う: .github/scripts/apt_install.sh 包み...
#
# - 入っていない包みだけ入れる（ランナーの像に入っていれば apt に触らない）
# - apt 自体の再試行は少なめ（Acquire::Retries=2）・短めのタイムアウト。update は取得失敗を失敗にする（--error-on=any）。
#   DNS 失敗でも update は既定で 0 を返し、install が deb ごとに再試行を重ねて数十分止まるため（#169 のレビュー）
# - 失敗したら 30 秒おいて最大 3 回。2 回目からは、ランナーの既定のミラー（mirror+file:/etc/apt/apt-mirrors.txt、
#   azure.archive.ubuntu.com）をやめて archive.ubuntu.com・security.ubuntu.com を直に使う
#   （11/11 に azure.archive.ubuntu.com の名前解決が数分続けて失敗し、gui-tour・web・Pages が落ちた）
set -uo pipefail

missing=()
for p in "$@"; do
  dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q 'install ok installed' || missing+=("$p")
done
if [ ${#missing[@]} -eq 0 ]; then
  echo "apt: すべて入っている（$*）"
  exit 0
fi
echo "apt: 入れる: ${missing[*]}"

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

for attempt in 1 2 3; do
  if [ "$attempt" -ge 2 ]; then
    sleep 30
    use_direct_mirrors
  fi
  # --error-on=any: 取得の失敗（W: Failed to fetch）も update の失敗にして、すぐ次の回（直のミラー）へ
  sudo apt-get "${opts[@]}" update -q --error-on=any && \
    sudo DEBIAN_FRONTEND=noninteractive apt-get "${opts[@]}" install -y -q --no-install-recommends "${missing[@]}" && exit 0
  echo "::warning::apt: ${attempt} 回目が失敗"
done
echo "::error::apt: 3 回とも失敗（${missing[*]}）"
exit 1
