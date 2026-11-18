#!/usr/bin/env bash
# コミット <sha> の成果物（名前 <name>...、すべて期限内）を持つ成功した CI の run を探して id を出す（無ければ何も出さない）。
# 探す順: <run>（与えたとき）→ <sha> の push の run → <sha> と同じ木の PR の head の run（main の CI が検査を飛ばしたとき）。
#   .github/scripts/ci_artifact_run.sh <sha> <run または空> <name>...
# 要るもの: gh、環境変数 GH_TOKEN・REPO
set -euo pipefail
sha=$1 run=$2
shift 2
# has <run> <name>...: run の成果物にすべての name があるか（先頭の run は名前の並びに入れない）
has() {
  local id=$1 names
  shift
  names=$(gh api "repos/$REPO/actions/runs/$id/artifacts?per_page=100" --jq '[.artifacts[] | select(.expired | not) | .name]')
  for n in "$@"; do grep -q "\"$n\"" <<< "$names" || return 1; done
}
if [ -n "$run" ] && has "$run" "$@"; then echo "$run"; exit 0; fi
for r in $(gh api "repos/$REPO/actions/workflows/ci.yml/runs?event=push&status=success&head_sha=$sha" --jq '.workflow_runs[].id'); do
  if has "$r" "$@"; then echo "$r"; exit 0; fi
done
tree=$(gh api "repos/$REPO/commits/$sha" --jq .commit.tree.sha)
for head in $(gh api "repos/$REPO/commits/$sha/pulls" --jq '.[].head.sha'); do
  [ "$(gh api "repos/$REPO/commits/$head" --jq .commit.tree.sha)" = "$tree" ] || continue
  for r in $(gh api "repos/$REPO/actions/workflows/ci.yml/runs?event=pull_request&status=success&head_sha=$head" --jq '.workflow_runs[].id'); do
    if has "$r" "$@"; then echo "$r"; exit 0; fi
  done
done
