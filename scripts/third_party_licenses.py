#!/usr/bin/env python3
"""配る exe・wheel に静的に入る第三者のクレートのライセンスの一覧（THIRD-PARTY-LICENSES.txt）を作る。

MIT・Apache-2.0・OFL などは、バイナリの配布でも著作権表示とライセンス文の同梱を求める。
cargo metadata で根（既定は lensbench-gui と lensbench-cli）から通常の依存（build・dev を除く）をたどり、
クレートごとに名前・版・ライセンスの式と、ソースにある LICENSE*・COPYING*・NOTICE*（フォントのライセンス
OFL・UFL など、fonts/ の下の .txt も）の本文を書く。同じ本文は 1 回だけ書き、ほかは参照にする。

    python scripts/third_party_licenses.py --out THIRD-PARTY-LICENSES.txt
    python scripts/third_party_licenses.py --root lensbench-py --manifest crates/lensbench-py/Cargo.toml

--check はライセンスの式が許すもの（--allow）だけかを確かめ、外れたクレートがあれば終了コード 1。
"""

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

# 配る物に入れてよいライセンス（コピーレフトの混入を止める）。式の中の語がどれかに当たれば可
DEFAULT_ALLOW = [
    "MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause", "ISC",
    "Zlib", "Unicode-3.0", "Unicode-DFS-2016", "BSL-1.0", "0BSD", "CC0-1.0", "MIT-0", "OFL-1.1",
    "Ubuntu-font-1.0", "LicenseRef-UFL-1.0", "MPL-2.0", "Unlicense", "CDLA-Permissive-2.0", "NCSA",
]

NAMES = re.compile(r"^(licen[cs]e|copying|notice|authors|copyright)", re.I)


def metadata(manifest, target):
    cmd = ["cargo", "metadata", "--format-version", "1", "--locked"]
    if manifest:
        cmd += ["--manifest-path", manifest]
    if target:
        cmd += ["--filter-platform", target]
    return json.loads(subprocess.run(cmd, check=True, capture_output=True, text=True).stdout)


def closure(m, roots):
    pk = {p["id"]: p for p in m["packages"]}
    nodes = {n["id"]: n for n in m["resolve"]["nodes"]}
    start = [p["id"] for p in m["packages"] if p["name"] in roots and p["source"] is None]
    if not start:
        sys.exit(f"根のクレートが無い: {roots}")
    seen, stack = set(), list(start)
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            # 通常の依存だけ（build スクリプト・試験の依存は配る物に入らない）
            if any(k["kind"] is None for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    return [pk[i] for i in sorted(seen, key=lambda i: (pk[i]["name"], pk[i]["version"]))]


def license_files(p):
    d = Path(p["manifest_path"]).parent
    out = []
    for f in sorted(d.iterdir()):
        if f.is_file() and NAMES.match(f.name):
            out.append(f)
    fonts = d / "fonts"
    if fonts.is_dir():
        out += sorted(f for f in fonts.rglob("*.txt") if f.is_file())
    if p.get("license_file"):
        f = d / p["license_file"]
        if f.is_file() and f not in out:
            out.append(f)
    return out


def allowed(expr, allow):
    words = [w for w in re.split(r"\s+OR\s+|\s+AND\s+|[()/]", expr or "") if w.strip()]
    return bool(words) and any(w.strip() in allow for w in words)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", action="append", help="根のクレート（複数可。既定 lensbench-gui・lensbench-cli）")
    ap.add_argument("--manifest", help="Cargo.toml（ワークスペースの外のクレート用）")
    ap.add_argument("--target", default="x86_64-pc-windows-msvc", help="依存を絞る対象（空ですべて）")
    ap.add_argument("--out", help="書くファイル（省略で標準出力）")
    ap.add_argument("--check", action="store_true", help="許すライセンスだけかを確かめる")
    ap.add_argument("--allow", action="append", default=[], help="許すライセンスを足す")
    a = ap.parse_args()
    roots = a.root or ["lensbench-gui", "lensbench-cli"]
    m = metadata(a.manifest, a.target or None)
    pkgs = [p for p in closure(m, roots) if p["source"] is not None]
    allow = set(DEFAULT_ALLOW + a.allow)
    bad = [p for p in pkgs if not allowed(p.get("license"), allow)]
    if a.check:
        for p in bad:
            print(f"許さないライセンス: {p['name']} {p['version']}: {p.get('license')}", file=sys.stderr)
        sys.exit(1 if bad else 0)
    texts = {}
    lines = [
        "lensbench に入る第三者のソフトウェアのライセンス",
        "=" * 40,
        "",
        f"根: {', '.join(roots)}（対象 {a.target or 'すべて'}）。クレート {len(pkgs)} 個。",
        "",
    ]
    body = []
    for p in pkgs:
        files = license_files(p)
        lines.append(f"{p['name']} {p['version']}  ({p.get('license') or p.get('license_file') or '不明'})")
        refs = []
        for f in files:
            t = f.read_text(encoding="utf-8", errors="replace").strip()
            h = hashlib.sha256(t.encode()).hexdigest()[:12]
            if h not in texts:
                texts[h] = t
                body.append(f"\n{'-' * 72}\n[{h}] {p['name']} {p['version']}: {f.name}\n{'-' * 72}\n{t}\n")
            refs.append(f"{f.name} [{h}]")
        if refs:
            lines.append("    " + ", ".join(refs))
        elif p.get("license"):
            lines.append("    （ソースにライセンスの本文が無い。式のライセンスの標準の本文による）")
    out = "\n".join(lines) + "\n" + "".join(body)
    if a.out:
        Path(a.out).write_text(out, encoding="utf-8")
        print(f"{a.out}: クレート {len(pkgs)} 個、本文 {len(texts)} 種", file=sys.stderr)
    else:
        sys.stdout.write(out)
    if bad:
        for p in bad:
            print(f"注意: 許すライセンスの一覧に無い: {p['name']} {p['version']}: {p.get('license')}", file=sys.stderr)


if __name__ == "__main__":
    main()
