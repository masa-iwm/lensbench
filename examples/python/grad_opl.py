"""光路長の微分 dOPL/dp: trace(grad=...) の解析的な微分を、パラメータを少し変えた差分と比べる。

微分は「物点と像面上の着地点を固定したときの OPL の変化」。差分の側は solve_ray で同じ着地点に戻してから比べる。
"""

from __future__ import annotations

import numpy as np

import lensbench as lb
from _common import GLASS, zmx


def main() -> None:
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    names = ["1.c", "3.t"]  # 面.種類（面 1 の曲率、面 3 の厚さ）。単位は 1/mm・mm
    px, py = np.array([0.1]), np.array([0.3])
    w = s.primary
    r = s.trace(2, px, py, wavelengths=[w], grad=names)
    obj = tuple(s.object_of(2))
    tgt = (float(r["x"][0, 0]), float(r["y"][0, 0]))
    for j, p in enumerate(names):
        h = 1e-6 if p.endswith(".c") else 1e-4
        v = s.get_param(p)
        o = []
        for d in (h, -h):
            t = s.with_params({p: v + d})  # 元の系は変えない
            sol = t.solve_ray("pupil", ("position", s.image, tgt), object=obj, pupil=(px[0], py[0]))
            q = t.trace(None, [sol["pupil"][0]], [sol["pupil"][1]], wavelengths=[w], object=obj)
            o.append(float(q["opl"][0, 0]))
        fd = (o[0] - o[1]) / (2 * h)
        an = float(r["grad"][0, 0, j])
        print(f"dOPL/d({p}) 解析 {an:.9f}  差分 {fd:.9f}  差 {abs(an - fd):.1e} mm")
        assert abs(an - fd) < 1e-6 * max(1.0, abs(fd))


if __name__ == "__main__":
    main()
