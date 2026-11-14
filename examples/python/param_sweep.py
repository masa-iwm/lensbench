"""パラメータの掃引: 面 3 の厚さを変えながら軸上の RMS スポットを求め、CSV に書く。

`lensbench run --out DIR` から起動すると DIR に書く。進み具合は lb.progress で 1 行ずつ出す。

    lensbench run --out sweep examples/python/param_sweep.py -- --n 11
"""

from __future__ import annotations

import argparse
import csv

import numpy as np

import lensbench as lb
from _common import GLASS, out_dir, pupil_grid, zmx


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=7, help="掃引の点の数")
    ap.add_argument("--span", type=float, default=0.2, help="厚さの変化の幅 [mm]（± の片側）")
    a = ap.parse_args()
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    p = "3.t"
    t0 = s.get_param(p)
    px, py = pupil_grid(21)
    w = s.primary
    rows = []
    for i, dt in enumerate(np.linspace(-a.span, a.span, a.n)):
        t = s.with_params({p: t0 + dt})  # 掃引の 1 点ごとに軽い複製
        r = t.trace(1, px, py, waves=[w])
        ok = r["ok"][0]
        x, y = r["x"][0, ok], r["y"][0, ok]
        rms = float(np.sqrt(np.mean((x - x.mean()) ** 2 + (y - y.mean()) ** 2)))
        rows.append((t0 + dt, rms * 1e3))
        lb.progress(i + 1, a.n, f"{p} = {t0 + dt:.4f}")
    f = out_dir() / "param_sweep.csv"
    with open(f, "w", newline="") as fh:
        wr = csv.writer(fh)
        wr.writerow([f"{p} [mm]", "rms [um]"])
        wr.writerows(rows)
    best = min(rows, key=lambda r: r[1])
    print(f"{f} に {len(rows)} 点。最小 RMS {best[1]:.3f} µm（{p} = {best[0]:.4f} mm）")


if __name__ == "__main__":
    main()
