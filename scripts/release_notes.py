#!/usr/bin/env python3
"""リリースの本文を CHANGELOG.md の版の節から作る（release.yml）。

    python scripts/release_notes.py v0.5.0 [CHANGELOG.md]   # 本文を標準出力へ

タグ vX.Y.Z（またはリリース候補 vX.Y.Z-rc.N）に対し、CHANGELOG の `## X.Y.Z` の節（見出しの後ろの「（開発中）」などは
問わない）を出す。リリース候補には頭にその旨を足す。節が無いときは CHANGELOG へのリンクだけにする（リリースは止めない）。
"""
import re
import sys
from pathlib import Path

CHANGELOG_URL = "https://github.com/masa-iwm/lensbench/blob/main/CHANGELOG.md"


def section(text: str, version: str) -> str | None:
    lines = text.splitlines()
    head = re.compile(r"^## " + re.escape(version) + r"(?![\w.\-])")
    for i, line in enumerate(lines):
        if head.match(line):
            body = []
            for rest in lines[i + 1:]:
                if rest.startswith("## "):
                    break
                body.append(rest)
            return "\n".join(body).strip()
    return None


def notes(tag: str, text: str) -> str:
    m = re.fullmatch(r"v(\d+\.\d+\.\d+)(-rc\.\d+)?", tag)
    if not m:
        return f"変更の記録: {CHANGELOG_URL}"
    version, rc = m.group(1), m.group(2)
    body = section(text, version)
    out = []
    if rc:
        out.append(f"{version} のリリース候補（pre-release）。下は {version} に向けた変更のまとめ（途中のもの）。")
    out.append(body if body else f"変更の記録: {CHANGELOG_URL}")
    return "\n\n".join(out) + "\n"


def main() -> int:
    # Windows の Python は標準出力が cp1252 のことがあり、日本語の出力で落ちる
    sys.stdout.reconfigure(encoding="utf-8")
    if len(sys.argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    path = Path(sys.argv[2]) if len(sys.argv) == 3 else Path(__file__).resolve().parents[1] / "CHANGELOG.md"
    sys.stdout.write(notes(sys.argv[1], path.read_text(encoding="utf-8")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
