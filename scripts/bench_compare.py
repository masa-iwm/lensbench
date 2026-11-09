#!/usr/bin/env python3
"""2 つの版の追跡の速さを、同じマシンで交互に測って比べる（.github/workflows/bench.yml から使う）。

    python3 scripts/bench_compare.py --base base/bench --head head/bench \
        --refs head/tests/refs --threads 1,4 --rounds 5 --secs 1 --out result.json

各ラウンドで系ごと・スレッド数ごとに base → head（奇数ラウンドは head → base）の順に 1 回ずつ測り、
Mray/s の中央値で比（head ÷ base）を出す。結果は Markdown の表（標準出力）と JSON（--out）。
読めない系（古い版が未対応の面など）は飛ばして表に書く。
"""
import argparse
import json
import math
import statistics
import subprocess
import sys
from pathlib import Path


def run(exe, zmx, threads, secs, rays):
    cmd = [exe, str(zmx), "--json", "--threads", str(threads), "--secs", str(secs), "--rays", str(rays)]
    p = subprocess.run(cmd, capture_output=True, encoding="utf-8", errors="replace")
    if p.returncode != 0:
        return None, (p.stderr.strip().splitlines() or ["?"])[-1]
    return json.loads(p.stdout.strip().splitlines()[-1]), None


def gmean(xs):
    xs = [x for x in xs if x > 0]
    return math.exp(sum(map(math.log, xs)) / len(xs)) if xs else float("nan")


def main():
    # Windows の既定（cp1252 など）では表の日本語が書けない
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True)
    ap.add_argument("--head", required=True)
    ap.add_argument("--refs", required=True, type=Path)
    ap.add_argument("--systems", default="all", help="all か A0,C0 のような並び")
    ap.add_argument("--threads", default="1,4")
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--secs", type=float, default=1.0)
    ap.add_argument("--rays", type=int, default=200000)
    ap.add_argument("--base-label", default="base")
    ap.add_argument("--head-label", default="head")
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()

    if a.systems == "all":
        names = sorted(p.parent.name for p in a.refs.glob("*/system.zmx"))
    else:
        names = [s.strip() for s in a.systems.split(",") if s.strip()]
    threads = [int(t) for t in a.threads.split(",")]
    exes = {"base": a.base, "head": a.head}

    # samples[(name, threads)][which] = [Mray/s, ...]
    samples, skipped, info = {}, {}, {}
    for r in range(a.rounds):
        order = ["base", "head"] if r % 2 == 0 else ["head", "base"]
        for name in names:
            if name in skipped:
                continue
            zmx = a.refs / name / "system.zmx"
            for t in threads:
                for w in order:
                    res, err = run(exes[w], zmx, t, a.secs, a.rays)
                    if res is None:
                        skipped[name] = f"{w}: {err}"
                        break
                    samples.setdefault((name, t), {"base": [], "head": []})[w].append(res["mray_s"])
                    info.setdefault(name, {})[w + "_ok"] = res["ok"]
                if name in skipped:
                    break
        print(f"round {r + 1}/{a.rounds} done", file=sys.stderr)

    rows = []
    for name in names:
        if name in skipped:
            continue
        ok_b, ok_h = info[name].get("base_ok"), info[name].get("head_ok")
        for t in threads:
            s = samples[(name, t)]
            mb, mh = statistics.median(s["base"]), statistics.median(s["head"])
            rows.append(
                {
                    "name": name,
                    "threads": t,
                    "base_mray_s": mb,
                    "head_mray_s": mh,
                    "ratio": mh / mb,
                    "base_spread": (max(s["base"]) - min(s["base"])) / mb,
                    "head_spread": (max(s["head"]) - min(s["head"])) / mh,
                    "ok_base": ok_b,
                    "ok_head": ok_h,
                }
            )

    out = []
    out.append(f"## 追跡の速さ: {a.head_label} ÷ {a.base_label}\n")
    out.append(f"{a.rounds} ラウンドの中央値（{a.rays} 本、1 回 {a.secs} s、base と head を交互）。ばらつきは (最大−最小)/中央値。\n")
    out.append("| スレッド | 幾何平均 | 最小 | 最大 |")
    out.append("|---|---|---|---|")
    for t in threads:
        rs = [x["ratio"] for x in rows if x["threads"] == t]
        if rs:
            out.append(f"| {t} | {gmean(rs):.3f} | {min(rs):.3f} | {max(rs):.3f} |")
    out.append("")
    out.append("| 系 | T | base Mray/s | head Mray/s | 比 | ばらつき base/head | 通過本数 |")
    out.append("|---|---|---|---|---|---|---|")
    for x in rows:
        okm = "" if x["ok_base"] == x["ok_head"] else f" ≠ {x['ok_base']}"
        out.append(
            f"| {x['name']} | {x['threads']} | {x['base_mray_s']:.3f} | {x['head_mray_s']:.3f} | "
            f"{x['ratio']:.3f} | {x['base_spread']:.0%}/{x['head_spread']:.0%} | {x['ok_head']}{okm} |"
        )
    if skipped:
        out.append("\n飛ばした系:\n")
        for k, v in skipped.items():
            out.append(f"- {k}: {v}")
    print("\n".join(out))

    if a.out:
        a.out.write_text(
            json.dumps(
                {"base": a.base_label, "head": a.head_label, "rounds": a.rounds, "secs": a.secs, "rays": a.rays,
                 "rows": rows, "skipped": skipped},
                ensure_ascii=False,
                indent=1,
            ),
            encoding="utf-8",
        )


if __name__ == "__main__":
    main()
