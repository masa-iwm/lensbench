#!/usr/bin/env python3
"""2 つの版の CLI（解析・最適化・公差・swap・起動）の速さを、同じマシンで交互に測って比べる
（.github/workflows/bench.yml から使う。追跡だけの比べは bench_compare.py）。

    python3 scripts/bench_cli.py --base base/lensbench --head head/lensbench \
        --root . --threads 1,4 --rounds 5 --out cli.json

各ラウンドで項目ごと・スレッド数ごとに base → head（奇数ラウンドは head → base）の順に 1 回ずつ測り、
1 回あたりの壁時計の時間の中央値で比（head ÷ base、1 より小さいと head が速い）を出す。
時間はプロセスの起動から終わりまで（系・ガラスの読み込みを含む）。短い項目は reps 回続けて回して割る。
出力（標準出力）が base と head で同じかも見る（数値の桁が変わる直しでは「違う」になる）。
読めない・失敗する項目（古い版に無い旗など）は飛ばして表に書く。

系は tests/refs と examples の公開のもの、swap の候補は examples/lenses/LB（架空のメーカー）の玉を
曲率だけ変えて焦点距離を振った玉を作業用のフォルダに作って使う。
"""
import argparse
import hashlib
import json
import math
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

C0 = "tests/refs/C0/system.zmx"  # トリプレット＋折り返し。視野 4 は x・y とも 10°（x の成分がある視野）
GLASS = ["--glass-dir", "tests/glass"]
C0_VARS = [a for v in ["1.c", "2.c", "3.c", "4.c", "6.c", "7.c", "2.t", "4.t", "7.t"] for a in ("--var", v)]

# name: 表の名前、args: サブコマンドから（{cat} は swap の候補のフォルダ、{spec} は swap の設定）、
# reps: 1 回の測定で続けて回す回数、group: 表の見出し
TASKS = [
    {"name": "version", "group": "起動", "args": ["--version"], "reps": 50},
    {"name": "info A0", "group": "起動", "args": ["info", "tests/refs/A0/system.zmx", *GLASS], "reps": 50},
    {"name": "spot C0", "group": "起動", "args": ["spot", C0, *GLASS], "reps": 10},
    {"name": "mtf C0", "group": "解析", "args": ["mtf", C0, *GLASS]},
    {"name": "mtf C0 f4", "group": "解析", "args": ["mtf", C0, *GLASS, "--field", "4"]},
    {"name": "mtf C0 f2", "group": "解析", "args": ["mtf", C0, *GLASS, "--field", "2"], "reps": 3},
    {"name": "mtf-focus C0", "group": "解析", "args": ["mtf-focus", C0, *GLASS, "--freqs", "20,50"]},
    {"name": "psf C0 f4 w2", "group": "解析", "args": ["psf", C0, *GLASS, "--field", "4", "--wave", "2"]},
    {"name": "huygens-mtf C0 f4 w2", "group": "解析",
     "args": ["huygens-mtf", C0, *GLASS, "--field", "4", "--wave", "2"]},
    {"name": "optimize C0", "group": "設計",
     "args": ["optimize", C0, *GLASS, *C0_VARS, "--merit", "wavefront", "--rings", "12", "--iters", "300"]},
    {"name": "tolerance C0", "group": "設計",
     "args": ["tolerance", C0, *GLASS, "--trials", "150", "--seed", "1", "--set",
              '{"preset":"precision","compensators":["7.t","6.c"],"rings":8,"comp_iters":20}']},
    {"name": "swap P18", "group": "設計",
     "args": ["swap", "examples/projects/P18_swap_two.json", *GLASS, "--lens-catalog", "{cat}",
              "--spec", "{spec}", "--top", "3"]},
]

# swap: 2 枠とも候補から選び、間隔 2 つを合わせる
SWAP_SPEC = {
    "slots": [{"block": "A", "filter": {"efl": [40, 200]}}, {"block": "B", "filter": {"efl": [30, 200]}}],
    "vars": ["gap.t=0.5:20", "bfl.t=1:300"],
    "iters": 15,
}
# 元の玉の焦点距離に掛ける倍率（曲率を 1/倍率 にする。厚さ・径はそのまま）
SWAP_SCALES = [0.8, 0.85, 0.9, 0.95, 1.05, 1.1, 1.2, 1.3, 1.4, 1.6, 1.8, 2.0]


def make_catalog(root, work):
    """examples/lenses/LB の玉と、その曲率を変えた玉を work/LB に書く（候補の数を増やす）"""
    src = root / "examples" / "lenses" / "LB"
    dst = work / "LB"
    dst.mkdir(parents=True, exist_ok=True)
    for f in sorted(src.glob("*.zmx")):
        shutil.copy(f, dst / f.name)
        text = f.read_bytes().decode("utf-16")
        stem, num = f.stem.rsplit("-", 1)
        for s in SWAP_SCALES:

            def curv(m):
                return f"{m.group(1)}{float(m.group(2)) / s!r}"

            out = re.sub(r"^(\s*CURV )(\S+)", curv, text, flags=re.M)
            (dst / f"{stem}-{num}X{int(round(s * 100))}.zmx").write_bytes(out.encode("utf-16"))
    spec = work / "swap.json"
    spec.write_text(json.dumps(SWAP_SPEC), encoding="utf-8")
    return dst, spec


# 出力の比べから外す行（かかった時間）
TIMING = re.compile(rb'^\s*"?(seconds|elapsed\w*|time_s)"?\s*[:=].*$', re.M)


def run(exe, args, threads, reps, root):
    cmd = [exe] + (["--threads", str(threads)] if threads and args[0] != "--version" else []) + args
    digest = None
    t0 = time.perf_counter()
    for _ in range(reps):
        p = subprocess.run(cmd, cwd=root, capture_output=True, env=ENV)
        if p.returncode != 0:
            err = p.stderr.decode("utf-8", "replace").strip().splitlines() or ["?"]
            return None, None, f"exit {p.returncode}: {err[-1]}"
        if digest is None:
            digest = hashlib.sha256(TIMING.sub(b"", p.stdout)).hexdigest()
    return (time.perf_counter() - t0) / reps, digest, None


def gmean(xs):
    xs = [x for x in xs if x > 0]
    return math.exp(sum(map(math.log, xs)) / len(xs)) if xs else float("nan")


def fmt_t(s):
    return f"{s * 1e3:.1f} ms" if s < 1 else f"{s:.2f} s"


# ガラスの探し場所は旗で渡す。環境の LENSBENCH_GLASSCAT・LENSBENCH_THREADS は使わない
ENV = {k: v for k, v in os.environ.items() if k not in ("LENSBENCH_GLASSCAT", "LENSBENCH_THREADS")}


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True, help="base の lensbench の実行ファイル")
    ap.add_argument("--head", required=True, help="head の lensbench の実行ファイル")
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent,
                    help="系・ガラス（tests/refs・tests/glass・examples）のあるリポジトリ")
    ap.add_argument("--tasks", default="all", help="all か、項目の名前（の頭）か group をカンマ区切り")
    ap.add_argument("--threads", default="1,4", help="スレッド数（カンマ区切り、0 は指定しない＝全コア）")
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--base-label", default="base")
    ap.add_argument("--head-label", default="head")
    ap.add_argument("--list", action="store_true", help="項目を並べて終わる")
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()

    if a.list:
        for t in TASKS:
            print(f"{t['group']}\t{t['name']}\tlensbench {' '.join(t['args'])}")
        return
    if a.tasks == "all":
        tasks = TASKS
    else:
        keys = [k.strip() for k in a.tasks.split(",") if k.strip()]
        tasks = [t for t in TASKS if any(t["name"].startswith(k) or t["group"] == k for k in keys)]
        if not tasks:
            sys.exit(f"項目がありません: {a.tasks}（--list で一覧）")
    threads = [int(t) for t in a.threads.split(",")]
    exes = {"base": str(Path(a.base).resolve()), "head": str(Path(a.head).resolve())}
    root = a.root.resolve()
    work = Path(tempfile.mkdtemp(prefix="lb-bench-"))
    cat, spec = make_catalog(root, work)
    subst = {"{cat}": str(cat), "{spec}": str(spec)}

    # 起動は 1 スレッドでも同じなので 1 つだけ（先頭のスレッド数）
    def threads_of(t):
        return threads[:1] if t["group"] == "起動" else threads

    samples, digests, skipped = {}, {}, {}
    try:
        for r in range(a.rounds):
            order = ["base", "head"] if r % 2 == 0 else ["head", "base"]
            for t in tasks:
                name = t["name"]
                if name in skipped:
                    continue
                args = [subst.get(x, x) for x in t["args"]]
                for th in threads_of(t):
                    for w in order:
                        sec, dig, err = run(exes[w], args, th, t.get("reps", 1), root)
                        if sec is None:
                            skipped[name] = f"{w}: {err}"
                            break
                        samples.setdefault((name, th), {"base": [], "head": []})[w].append(sec)
                        digests.setdefault(name, {})[w] = dig
                    if name in skipped:
                        break
            print(f"round {r + 1}/{a.rounds} done", file=sys.stderr)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    rows = []
    for t in tasks:
        name = t["name"]
        if name in skipped:
            continue
        same = digests[name]["base"] == digests[name]["head"]
        for th in threads_of(t):
            s = samples[(name, th)]
            mb, mh = statistics.median(s["base"]), statistics.median(s["head"])
            rows.append({
                "name": name, "group": t["group"], "threads": th,
                "base_s": mb, "head_s": mh, "ratio": mh / mb,
                "base_spread": (max(s["base"]) - min(s["base"])) / mb,
                "head_spread": (max(s["head"]) - min(s["head"])) / mh,
                "same_output": same,
            })

    out = [f"## CLI の時間: {a.head_label} ÷ {a.base_label}\n",
           f"{a.rounds} ラウンドの中央値（1 回あたりの壁時計、base と head を交互）。比は 1 より小さいと head が速い。"
           "ばらつきは (最大−最小)/中央値。出力は標準出力が同じか。\n",
           "| 種類 | スレッド | 幾何平均 | 最小 | 最大 |", "|---|---|---|---|---|"]
    for g in dict.fromkeys(t["group"] for t in tasks):
        for th in threads:
            rs = [x["ratio"] for x in rows if x["group"] == g and x["threads"] == th]
            if rs:
                out.append(f"| {g} | {th or '全'} | {gmean(rs):.3f} | {min(rs):.3f} | {max(rs):.3f} |")
    out += ["", "| 項目 | T | base | head | 比 | ばらつき base/head | 出力 |", "|---|---|---|---|---|---|---|"]
    for x in rows:
        out.append(
            f"| {x['name']} | {x['threads'] or '全'} | {fmt_t(x['base_s'])} | {fmt_t(x['head_s'])} | {x['ratio']:.3f} | "
            f"{x['base_spread']:.0%}/{x['head_spread']:.0%} | {'同じ' if x['same_output'] else '違う'} |")
    if skipped:
        out.append("\n飛ばした項目:\n")
        for k, v in skipped.items():
            out.append(f"- {k}: {v}")
    print("\n".join(out))

    if a.out:
        a.out.write_text(json.dumps({"base": a.base_label, "head": a.head_label, "rounds": a.rounds,
                                     "rows": rows, "skipped": skipped}, ensure_ascii=False, indent=1),
                         encoding="utf-8")


if __name__ == "__main__":
    main()
