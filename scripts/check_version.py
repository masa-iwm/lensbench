#!/usr/bin/env python3
"""版の検査（docs/versioning.md）: Cargo.toml の workspace.package.version が、最後のリリースのタグ（v*）より新しいこと。
HEAD がタグのコミットなら、版はそのタグの名前（v を除く）と同じこと。タグが 1 つも無ければ検査しない。

  python3 scripts/check_version.py          # git のタグを読む（CI は fetch-depth: 0 で）
"""
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$")


def key(v: str):
    """semver の順の鍵（リリース候補・開発中はその版より前。接尾辞の各部は数なら数で比べる）"""
    m = SEMVER.match(v)
    if not m:
        raise ValueError(f"版の形でない: {v}")
    pre = m[4]
    if pre is None:
        tail = (1,)
    else:
        tail = (0, tuple((0, int(p), "") if p.isdigit() else (1, 0, p) for p in pre.split(".")))
    return (int(m[1]), int(m[2]), int(m[3]), tail)


def check(version: str, tags: list[str], head_tags: list[str]) -> list[str]:
    errs = []
    vt = [t[1:] for t in tags if t.startswith("v") and SEMVER.match(t[1:])]
    here = [t[1:] for t in head_tags if t.startswith("v") and SEMVER.match(t[1:])]
    try:
        k = key(version)
    except ValueError as e:
        return [str(e)]
    if here:
        if version not in here:
            errs.append(f"版 {version} が HEAD のタグ {', '.join('v' + t for t in here)} と違う")
        return errs
    if vt:
        last = max(vt, key=key)
        if k <= key(last):
            errs.append(f"版 {version} が最後のリリースのタグ v{last} より新しくない（タグの後は main を次の開発中の版、例 "
                        f"{key(last)[0]}.{key(last)[1] + 1}.0-dev に上げる）")
    return errs


def git(*args: str) -> list[str]:
    out = subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout
    return out.split()


def main() -> int:
    version = tomllib.load(open(ROOT / "Cargo.toml", "rb"))["workspace"]["package"]["version"]
    errs = check(version, git("tag", "-l", "v*"), git("tag", "--points-at", "HEAD", "-l", "v*"))
    for e in errs:
        print(f"誤り: {e}", file=sys.stderr)
    if not errs:
        print(f"版 {version}: OK")
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main())
