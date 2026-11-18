#!/usr/bin/env python3
"""版ごとのドキュメントの置き場（GitHub Pages、.github/workflows/pages.yml から呼ぶ）。

  /         最新（main）のドキュメント
  /vX.Y/    リリースしたときのドキュメント（使い方のページとスキーマ。/api/・/app/ は入れない）。版は小数第 1 位まで、
            同じ版の直し（0.5.1 など）は同じ置き場を上書きする。リリースのドキュメントはリリースの添付
            （lensbench-docs-<タグ>.tar.gz）に固めて残し、Pages を組むたびにそこから置く（後から書き換えない）
  /schema/vX.Y/<名前>.schema.json   その版のスキーマ（lensbench が書くファイルの "$schema"）。リリースのスキーマは、タグの木の
            docs/schema をリリースの添付（lensbench-schema-<タグ>.tar.gz）に固めて置く（CI の成果物が無くても置けるように）。
            リリースしていない main の版（0.5.0-dev など）は main のスキーマを置く（リリースの後は main の版を次へ上げる）
  /versions.json   版の一覧（docs/theme/versions.js が版の切り替えに使う）

  docs_versions.py snapshot <サイト> <出力.tar.gz>
  docs_versions.py pack-schema <リポジトリの tarball> <出力.tar.gz>   tarball の docs/schema/*.schema.json だけを固める
  docs_versions.py assemble <サイト> --snapshots <添付のフォルダ> --dev <main の版>
"""
import argparse
import json
import re
import shutil
import sys
import tarfile
from pathlib import Path

# 固めないもの（サイトの根の直下）: Rust API・Web 版
SKIP = {"api", "app"}
ASSET = re.compile(r"^lensbench-(docs|schema)-v(\d+)\.(\d+)\.(\d+)\.tar\.gz$")


def minor(version: str) -> str:
    m = re.match(r"^(\d+)\.(\d+)", version)
    if not m:
        raise SystemExit(f"版の形でない: {version}")
    return f"{m[1]}.{m[2]}"


def snapshot(site: Path, out: Path) -> None:
    if not (site / "index.html").is_file():
        raise SystemExit(f"{site}: index.html が無い")
    out.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(out, "w:gz") as t:
        for p in sorted(site.iterdir()):
            if p.name not in SKIP:
                t.add(p, arcname=p.name)
    print(f"{out}: {site}（{', '.join(sorted(SKIP))} を除く）")


def pack_schema(tarball: Path, out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    n = 0
    with tarfile.open(tarball) as t, tarfile.open(out, "w:gz") as o:
        for m in t.getmembers():
            parts = m.name.split("/")
            # GitHub の tarball は <リポジトリ>-<コミット>/ の下
            if m.isfile() and len(parts) == 4 and parts[1:3] == ["docs", "schema"] and parts[3].endswith(".schema.json"):
                m2 = tarfile.TarInfo(parts[3])
                m2.size, m2.mtime, m2.mode = m.size, m.mtime, 0o644
                o.addfile(m2, t.extractfile(m))
                n += 1
    if n == 0:
        out.unlink()
        raise SystemExit(f"{tarball}: docs/schema/*.schema.json が無い")
    print(f"{out}: スキーマ {n} 個")


def assemble(site: Path, snaps: Path, dev: str) -> dict:
    # 添付を版（X.Y）ごと・種類ごとに、直しの番号の順で
    found: dict[str, dict[tuple[int, int], dict[int, Path]]] = {"docs": {}, "schema": {}}
    for p in sorted(snaps.iterdir()) if snaps.is_dir() else []:
        m = ASSET.match(p.name)
        if m:
            found[m[1]].setdefault((int(m[2]), int(m[3])), {})[int(m[4])] = p
    versions = []
    for (ma, mi), by_patch in sorted(found["docs"].items(), reverse=True):
        v, patch = f"{ma}.{mi}", max(by_patch)
        dest = site / f"v{v}"
        if dest.exists():
            shutil.rmtree(dest)
        dest.mkdir(parents=True)
        with tarfile.open(by_patch[patch]) as t:
            t.extractall(dest, filter="data")
        versions.append({"version": v, "tag": f"v{v}.{patch}", "path": f"v{v}/"})
        print(f"/v{v}/: {by_patch[patch].name}")
    # スキーマ: 版ごとに直しの番号のいちばん大きいタグの、スキーマの添付（無ければドキュメントの添付の schema/）
    released = set()
    for key in sorted(set(found["docs"]) | set(found["schema"]), reverse=True):
        v = f"{key[0]}.{key[1]}"
        sch, doc = found["schema"].get(key, {}), found["docs"].get(key, {})
        patch = max(set(sch) | set(doc))
        dest = site / "schema" / f"v{v}"
        if patch in sch:
            if dest.exists():
                shutil.rmtree(dest)
            dest.mkdir(parents=True)
            with tarfile.open(sch[patch]) as t:
                t.extractall(dest, filter="data")
            src = sch[patch].name
        else:
            # patch はドキュメントの添付のいちばん大きい直し（/vX.Y/ に置いたもの）
            copy_schemas(site / f"v{v}" / "schema", dest)
            src = doc[patch].name
        if not list(dest.glob("*.schema.json")):
            raise SystemExit(f"{src}: スキーマが無い")
        released.add(v)
        print(f"/schema/v{v}/: {src}")
    dv = minor(dev)
    if dv not in released:
        n = copy_schemas(site / "schema", site / "schema" / f"v{dv}")
        print(f"/schema/v{dv}/: main（{dev}、リリース前）のスキーマ {n} 個")
    index = {"dev": dev, "dev_released": dv in released, "versions": versions}
    (site / "versions.json").write_text(json.dumps(index, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return index


def copy_schemas(src: Path, dest: Path) -> int:
    files = sorted(src.glob("*.schema.json"))
    if not files:
        raise SystemExit(f"{src}: スキーマが無い")
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    for f in files:
        shutil.copy2(f, dest / f.name)
    return len(files)


def main(argv: list[str]) -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("snapshot")
    s.add_argument("site", type=Path)
    s.add_argument("out", type=Path)
    k = sub.add_parser("pack-schema")
    k.add_argument("tarball", type=Path)
    k.add_argument("out", type=Path)
    a = sub.add_parser("assemble")
    a.add_argument("site", type=Path)
    a.add_argument("--snapshots", type=Path, required=True)
    a.add_argument("--dev", required=True)
    o = ap.parse_args(argv)
    if o.cmd == "snapshot":
        snapshot(o.site, o.out)
    elif o.cmd == "pack-schema":
        pack_schema(o.tarball, o.out)
    else:
        assemble(o.site, o.snapshots, o.dev)


if __name__ == "__main__":
    main(sys.argv[1:])
