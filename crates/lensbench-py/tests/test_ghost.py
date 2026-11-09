"""ゴースト（2 回反射の経路を展開した系）の Python からの使い方。"""

from pathlib import Path

import numpy as np

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]


def test_ghost_system_traces_like_any_system():
    s = lb.System.load(str(ROOT / "tests" / "refs" / "A0" / "system.zmx"), glass_dirs=GLASS)
    pairs = s.ghost_pairs()
    assert (2, 1) in pairs and all(a > b for a, b in pairs)
    a, b = pairs[-1]
    g = s.ghost(a, b)
    origin, at = s.ghost_origin(a, b)
    assert len(origin) == g.image + 1
    assert origin[at[0]] == a and origin[at[1]] == b
    px = np.array([0.0, 0.3])
    py = np.array([0.0, 0.2])
    m = s.trace(1, px, py, pol=True)
    r = g.trace(1, px, py, pol=True)
    assert r["ok"].all()
    # ゴーストは反射 2 回ぶん弱い
    im = np.sum(np.abs(m["jones"]) ** 2)
    ig = np.sum(np.abs(r["jones"]) ** 2)
    assert 0 < ig < 0.01 * im
