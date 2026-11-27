"""非順次の追跡（nsc）: System.analyze と System.analyze_file。"""

from pathlib import Path

import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]
A0 = str(ROOT / "tests" / "refs" / "A0" / "system.zmx")
MZ = str(ROOT / "examples" / "projects" / "P20_mach_zehnder.json")


def test_budget_closes_and_paths_are_sorted():
    r = lb.System.analyze_file(A0, "nsc", glass_dirs=GLASS, grid=7, pixels=16, paths=5)
    b = r["budget"]
    assert b["source_w"] == pytest.approx(1.0)
    assert abs(b["residual_w"]) < 1e-12
    d = r["detectors"][0]
    assert len(d["irradiance"]) == d["nx"] * d["ny"] == 256
    assert d["power_w"] == pytest.approx(b["detected_w"])
    p = [x["power_w"] for x in r["paths"]]
    assert len(p) == 5 and p == sorted(p, reverse=True)
    # 主な経路は反射なしで全部の面を通る
    assert r["paths"][0]["reflections"] == 0


def test_analyze_on_a_loaded_system_matches_the_file():
    s = lb.System.load(A0, glass_dirs=GLASS)
    a = s.analyze("nsc", grid=5, pixels=8)
    f = lb.System.analyze_file(A0, "nsc", glass_dirs=GLASS, grid=5, pixels=8)
    assert a["budget"] == f["budget"]


def test_project_scene_has_every_beam():
    r = lb.System.analyze_file(MZ, "nsc", glass_dirs=GLASS, grid=5, pixels=8)
    assert len(r["scene"]["systems"]) >= 2
    assert abs(r["budget"]["residual_w"]) < 1e-12


def test_bad_settings_are_errors():
    with pytest.raises(ValueError):
        lb.System.analyze_file(A0, "nsc", glass_dirs=GLASS, half=[1, 2, 3])
    with pytest.raises(ValueError):
        lb.System.analyze_file(A0, "no-such-analysis")
