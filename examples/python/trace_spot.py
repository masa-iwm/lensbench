"""光線追跡とスポット: 視野ごとに瞳の格子を追い、像面の RMS スポット半径を出す。

    python examples/python/trace_spot.py
    lensbench run --threads 4 examples/python/trace_spot.py
"""

from __future__ import annotations

import numpy as np

import lensbench as lb
from _common import GLASS, pupil_grid, zmx


def main() -> None:
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    px, py = pupil_grid(31)
    w = s.primary  # 番号は 1 始まり（CLI・OpticStudio と同じ）
    print(f"{s.name}: 面 {s.surface_count} 個、絞り 面 {s.stop}、主波長 {s.wavelengths[w - 1]} µm")
    for f in range(1, len(s.fields) + 1):
        r = s.trace(f, px, py, wavelengths=[w])
        # 返り値は numpy 配列の dict。形は (波長, 光線)。届かなかった光線は ok が False
        ok = r["ok"][0]
        x, y = r["x"][0, ok], r["y"][0, ok]
        rms = float(np.sqrt(np.mean((x - x.mean()) ** 2 + (y - y.mean()) ** 2)))
        print(f"視野 {f} {s.fields[f - 1]}: 光線 {ok.sum()}/{ok.size}、RMS 半径 {rms * 1e3:.3f} µm")
        lb.progress(f, len(s.fields), f"視野 {f}")


if __name__ == "__main__":
    main()
