#!/usr/bin/env python3
"""変更の記録の断片（changelog.d/）を扱う（docs/versioning.md の「変更の記録」）。

利用者に見える変更は、PR ごとに `changelog.d/<名前>.<節>.md` を 1 つ足す（CHANGELOG.md は版上げのときだけ書き換える）。
ほかの PR と同じファイルに触らないので、並行の PR が衝突しない。

  python3 scripts/changelog.py check     [--dir changelog.d]                        # 断片の名前と中身の検査（CI）
  python3 scripts/changelog.py preview   [--dir changelog.d]                        # まとめた節を標準出力へ
  python3 scripts/changelog.py collect X.Y.Z [--date YYYY-MM-DD] [--changelog CHANGELOG.md] [--dir changelog.d]
      # 断片を CHANGELOG.md の `## X.Y.Z` の節へまとめて断片を消す（版上げの PR で）。--date で見出しを「X.Y.Z（日付）」にする

名前は英小文字・数字・ハイフン（Issue の番号から始めると並びが分かりやすい。例 `1083-coating-notes.fixed.md`）。
節は下の KINDS のどれか。中身は CHANGELOG にそのまま入る「- 」で始まる項目（複数行・複数の項目でもよい）。
同じ節の中は名前の順（頭の数は数として比べる）。
"""
import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# 節の鍵と見出し（CHANGELOG の版の節の中の並び順）
KINDS = {
    "breaking": "互換が切れる所",
    "results": "結果が変わる所",
    "changed": "変えたもの",
    "added": "足したもの",
    "perf": "速さ",
    "fixed": "直した所",
    "known": "既知の問題",
}
NAME = re.compile(r"^([a-z0-9][a-z0-9-]*)\.(" + "|".join(KINDS) + r")\.md$")
# 断片でないファイル（説明）
IGNORED = {"README.md", ".gitkeep"}


def _sort_key(name: str):
    m = re.match(r"(\d+)(.*)", name)
    return (0, int(m.group(1)), m.group(2)) if m else (1, 0, name)


def fragments(d: Path) -> tuple[dict[str, list[tuple[Path, str]]], list[str]]:
    """節ごとの (ファイル, 中身) と、問題の一覧"""
    out: dict[str, list[tuple[Path, str]]] = {k: [] for k in KINDS}
    errors = []
    if not d.is_dir():
        return out, errors
    for p in sorted(d.iterdir(), key=lambda p: _sort_key(p.name)):
        if p.name in IGNORED:
            continue
        m = NAME.match(p.name)
        if not p.is_file() or not m:
            errors.append(f"{p.name}: 名前は <英小文字・数字・ハイフン>.<{'|'.join(KINDS)}>.md")
            continue
        text = p.read_text(encoding="utf-8").strip("\n")
        lines = text.splitlines()
        if not lines or not lines[0].startswith("- "):
            errors.append(f"{p.name}: 中身は「- 」で始まる項目にする")
            continue
        if any(l.startswith("#") for l in lines):
            errors.append(f"{p.name}: 見出し（#）は書かない（節はファイル名で決まる）")
            continue
        if text.rstrip() != text or any(l.rstrip() != l for l in lines):
            errors.append(f"{p.name}: 行末の空白を消す")
            continue
        out[m.group(2)].append((p, text))
    return out, errors


def render(frags: dict[str, list[tuple[Path, str]]]) -> str:
    """断片だけの節の中身（### 見出しと項目）"""
    parts = []
    for kind, head in KINDS.items():
        if frags[kind]:
            parts.append(f"### {head}\n\n" + "\n".join(t for _, t in frags[kind]))
    return "\n\n".join(parts)


def merge_body(body: str, frags: dict[str, list[tuple[Path, str]]]) -> str:
    """版の節の中身 body（### の小節を含む）に断片を足す。小節の末尾に足し、無い小節は決まった並びの所に作る"""
    lines = body.strip("\n").splitlines() if body.strip() else []
    # 前書き（最初の ### より前）と小節に分ける
    pre, subs, cur = [], [], None
    for line in lines:
        if line.startswith("### "):
            cur = [line.rstrip(), []]
            subs.append(cur)
        elif cur is None:
            pre.append(line)
        else:
            cur[1].append(line)
    order = list(KINDS.values())
    for kind, head in KINDS.items():
        if not frags[kind]:
            continue
        add = "\n".join(t for _, t in frags[kind]).splitlines()
        hit = next((s for s in subs if s[0] == f"### {head}"), None)
        if hit is None:
            hit = [f"### {head}", []]
            rank = order.index(head)
            pos = len(subs)
            for i, s in enumerate(subs):
                h = s[0][4:]
                if h in order and order.index(h) > rank:
                    pos = i
                    break
            subs.insert(pos, hit)
        while hit[1] and not hit[1][-1].strip():
            hit[1].pop()
        while hit[1] and not hit[1][0].strip():
            hit[1].pop(0)
        hit[1].extend(add)
    out = [l for l in pre]
    while out and not out[-1].strip():
        out.pop()
    for head, content in subs:
        while content and not content[-1].strip():
            content.pop()
        while content and not content[0].strip():
            content.pop(0)
        if out:
            out.append("")
        out.append(head)
        out.append("")
        out.extend(content)
    return "\n".join(out)


def _find_section(lines: list[str], version: str) -> tuple[int, int] | None:
    head = re.compile(r"^## " + re.escape(version) + r"(?![\w.\-])")
    for i, line in enumerate(lines):
        if head.match(line):
            j = i + 1
            while j < len(lines) and not lines[j].startswith("## "):
                j += 1
            return i, j
    return None


def collect(changelog: Path, d: Path, version: str, date: str | None) -> int:
    frags, errors = fragments(d)
    if errors:
        print("断片に問題があります:\n  " + "\n  ".join(errors), file=sys.stderr)
        return 1
    text = changelog.read_text(encoding="utf-8")
    lines = text.splitlines()
    found = _find_section(lines, version)
    if found is None:
        print(f"{changelog} に `## {version}` の節がありません（開発版への PR で先に作る）", file=sys.stderr)
        return 1
    i, j = found
    body = merge_body("\n".join(lines[i + 1:j]), frags)
    title = f"## {version}（{date}）" if date else lines[i]
    new = lines[:i] + [title, ""] + body.splitlines() + [""] + lines[j:]
    changelog.write_text("\n".join(new).rstrip("\n") + "\n", encoding="utf-8")
    n = 0
    for items in frags.values():
        for p, _ in items:
            p.unlink()
            n += 1
    print(f"{n} 個の断片を {changelog} の {version} の節へまとめ、断片を消した")
    return 0


def main() -> int:
    # Windows の Python は標準出力が cp1252 のことがあり、日本語の出力で落ちる
    sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["check", "preview", "collect"])
    ap.add_argument("version", nargs="?")
    ap.add_argument("--dir", type=Path, default=ROOT / "changelog.d")
    ap.add_argument("--changelog", type=Path, default=ROOT / "CHANGELOG.md")
    ap.add_argument("--date")
    a = ap.parse_args()
    if a.cmd == "collect":
        if not a.version:
            ap.error("collect には版（X.Y.Z）が要る")
        return collect(a.changelog, a.dir, a.version, a.date)
    frags, errors = fragments(a.dir)
    if errors:
        print("断片に問題があります:\n  " + "\n  ".join(errors), file=sys.stderr)
        return 1
    if a.cmd == "preview":
        sys.stdout.write(render(frags) + "\n")
    else:
        print(f"OK: 断片 {sum(len(v) for v in frags.values())} 個")
    return 0


if __name__ == "__main__":
    sys.exit(main())
