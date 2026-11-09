"""光線の解き: 像面の指定点を主光線が通る物点を求め、その物点から追い直して確かめる。"""

from __future__ import annotations

import lensbench as lb
from _common import GLASS, zmx


def main() -> None:
    s = lb.System.load(zmx("B0"), glass_dirs=GLASS)  # 有限物体の系
    target = (0.0, -1.0)  # 像面の (x, y) [mm]（像面の局所座標）
    sol = s.solve_ray("object", ("position", s.image, target), field=2)
    print(f"物点 {sol['object']} mm、反復 {sol['iterations']} 回、残差 {max(map(abs, sol['residual'])):.1e}")
    r = s.trace(None, [0.0], [0.0], object=tuple(sol["object"]))
    w = s.primary - 1
    print(f"追い直した像面の点 ({r['x'][w, 0]:.12f}, {r['y'][w, 0]:.12f})")
    assert abs(r["y"][w, 0] - target[1]) < 1e-9


if __name__ == "__main__":
    main()
