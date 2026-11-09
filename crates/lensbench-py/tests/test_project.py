"""プロジェクトを JSON の文字列・dict から展開する。"""

import json
import warnings
from pathlib import Path

import numpy as np
import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]
REFS = ROOT / "tests" / "refs"

PROJECT = {
    "glass_catalogs": ["SCHOTT"],
    "wavelengths": [{"um": 0.4861327}, {"um": 0.5875618}, {"um": 0.6562725}],
    "primary": 2,
    "aperture": {"type": "epd", "value": 10},
    "fields": {"type": "angle", "points": [[0, 0], [0, 14]]},
    "blocks": [{"type": "zmx", "path": "A0/system.zmx"}],
}


PX = np.array([0.0, 0.5, -0.4])
PY = np.array([0.3, -0.2, 0.7])


def rays(s):
    return s.trace(2, PX, PY, wavelengths=[2])


def test_dict_and_string_give_the_same_system_as_the_zmx():
    a = lb.System.from_project(PROJECT, base=str(REFS), glass_dirs=GLASS)
    b = lb.System.from_project(json.dumps(PROJECT), base=str(REFS), glass_dirs=GLASS)
    z = lb.System.load(str(REFS / "A0" / "system.zmx"), glass_dirs=GLASS)
    ra, rb, rz = rays(a), rays(b), rays(z)
    for k in ["x", "y", "l", "m"]:
        assert np.array_equal(ra[k], rb[k]), k
        assert np.abs(ra[k] - rz[k]).max() < 1e-12, k
    assert (ra["err"] == 0).all()


def with_surface(surf):
    p = dict(PROJECT)
    p["blocks"] = PROJECT["blocks"] + [{"type": "surfaces", "surfaces": [surf]}]
    return lb.System.from_project(p, base=str(REFS), glass_dirs=GLASS)


def test_jones_and_aperture_surfaces():
    ref = rays(lb.System.from_project(PROJECT, base=str(REFS), glass_dirs=GLASS))
    wide = with_surface({"jones": [1, 0, 0, 0, 0, 0, 1, 0], "aperture": {"type": "rect", "hx": 50, "hy": 50}})
    r = rays(wide)
    assert (r["err"] == 0).all() and (r["vig"] == 0).all()
    assert np.abs(r["x"] - ref["x"]).max() < 1e-12
    assert np.abs(r["y"] - ref["y"]).max() < 1e-12
    s = with_surface({"aperture": {"type": "circular", "max": 1e-3}})
    assert ((rays(s)["vig"]) == s.image - 1).all()


ZMX = """VERS 260327 42
MODE SEQ
UNIT MM X W X CM MR CPMM
ENPD 8
WAVM 1 0.5876 1
PWAV 1
SURF 0
  CURV 0.0
  DISZ INFINITY
SURF 1
  CURV 0.02
  DISZ 4
  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0
SURF 2
  CURV -0.01
  DISZ 3
SURF 3
  CURV 0.01 4 1 0 0 ""
  PZUP 1 1.5 0 0
  GLAS ___BLANK 1 0 1.6 0 0 0 0 0 0
SURF 4
  CURV -0.015
  DISZ 20
SURF 5
  CURV 0.0
  DISZ 0
"""


def test_pickups_outside_the_range_warn(tmp_path):
    (tmp_path / "t.zmx").write_text(ZMX)
    p = {
        "glass_catalogs": [],
        "wavelengths": [{"um": 0.5876}],
        "primary": 1,
        "aperture": {"type": "epd", "value": 8},
        "fields": {"type": "angle", "points": [[0, 0]]},
        "blocks": [{"type": "stop"}, {"type": "air", "t": 2}, {"type": "zmx", "path": "t.zmx", "from": 3}],
    }
    with pytest.warns(UserWarning, match="範囲の外"):
        lb.System.from_project(p, base=str(tmp_path))
    p["blocks"] = [{"type": "zmx", "path": "t.zmx"}]
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        lb.System.from_project(p, base=str(tmp_path))



def test_bad_input():
    with pytest.raises(TypeError):
        lb.System.from_project(3)
    with pytest.raises(ValueError):
        lb.System.from_project("{not json")
