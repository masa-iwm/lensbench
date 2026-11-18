#!/usr/bin/env python3
"""版の検査（docs/versioning.md）: Cargo.toml の workspace.package.version が、最後のリリースのタグ（v*、接尾辞の無いもの）より
新しいこと。リリース候補のタグ（v0.5.0-rc.1 など）は比べる相手にしない（その後の main は 0.5.0-dev に戻る）。
HEAD がタグのコミットなら、版はそのタグの名前（v を除く）と同じこと（開発中の版 0.5.0-dev には、同じ版のリリース候補のタグ
v0.5.0-rc.N も合う。リリース候補は版を上げずに打つ）。リリースのタグが 1 つも無ければ、タグとの新しさは検査しない。

  python3 scripts/check_version.py              # git のタグを読む（CI は fetch-depth: 0 で）
  python3 scripts/check_version.py --tag v0.5.0  # タグが今の版に合うかだけ（リリースのビルドの前）
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


def tag_matches(tag: str, version: str) -> bool:
    """タグ（v を除いた名前）が版に合うか: 同じ名前か、開発中の版 X.Y.Z-dev に対するリリース候補 X.Y.Z-rc.N"""
    if tag == version:
        return True
    return version.endswith("-dev") and re.fullmatch(re.escape(version[: -len("-dev")]) + r"-rc\.\d+", tag) is not None


def check(version: str, tags: list[str], head_tags: list[str]) -> list[str]:
    errs = []
    # リリースのタグだけ（接尾辞つきのリリース候補は除く）
    vt = [t[1:] for t in tags if t.startswith("v") and (m := SEMVER.match(t[1:])) and m[4] is None]
    here = [t[1:] for t in head_tags if t.startswith("v") and SEMVER.match(t[1:])]
    try:
        k = key(version)
    except ValueError as e:
        return [str(e)]
    if here:
        if not any(tag_matches(t, version) for t in here):
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
    # Windows の Python は標準出力が cp1252 のことがあり、日本語の print で落ちる
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8")
    version = tomllib.load(open(ROOT / "Cargo.toml", "rb"))["workspace"]["package"]["version"]
    if sys.argv[1:2] == ["--tag"] and len(sys.argv) == 3:
        tag = sys.argv[2]
        if tag.startswith("v") and tag_matches(tag[1:], version):
            print(f"タグ {tag}: 版 {version} に合う")
            return 0
        print(f"誤り: タグ {tag} が Cargo.toml の版 {version} に合わない（タグは v＋版。開発中の版 X.Y.Z-dev では "
              f"リリース候補 vX.Y.Z-rc.N も打てる。docs/versioning.md）", file=sys.stderr)
        return 1
    errs = check(version, git("tag", "-l", "v*"), git("tag", "--points-at", "HEAD", "-l", "v*"))
    for e in errs:
        print(f"誤り: {e}", file=sys.stderr)
    if not errs:
        print(f"版 {version}: OK")
    return 1 if errs else 0


if __name__ == "__main__":
    sys.exit(main())
