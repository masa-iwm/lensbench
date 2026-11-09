"""機構部品のケラレ: System.mech_check は CLI の mech-check と同じ値を返す。"""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]
P19 = ROOT / "examples" / "projects" / "P19_tube_cage.json"


def cli():
    p = os.environ.get("LENSBENCH_CLI")
    if p:
        return p
    exe = "lensbench.exe" if sys.platform == "win32" else "lensbench"
    p = ROOT / "target" / "release" / exe
    return p if p.exists() else None


def test_p19_matches_cli():
    s = lb.System.load(str(P19), glass_dirs=GLASS)
    r = s.mech_check()
    assert r["pupil_rays"] > 300
    assert [p["name"] for p in r["parts"]] == ["tube", "cage"]
    assert len(r["results"]) == 3
    # 軸上は切れない。各行で、最初に切った部品 + 開口 + 通過 = 100 %
    assert r["results"][0]["passed_pct"] == 100.0
    for row in r["results"]:
        first = sum(p["first_pct"] for p in row["parts"])
        assert abs(first + row["optical_pct"] + row["passed_pct"] - 100.0) < 1e-9
    assert r["results"][2]["parts"][0]["first_pct"] > 0.0
    # 配置の確認: チューブはレンズを保持径に収める
    assert r["fit"] == []
    # 視野・波長・格子の指定
    one = s.mech_check(fields=[3], waves=[1], grid=11)
    assert len(one["results"]) == 1 and one["results"][0]["field"] == 3
    exe = cli()
    if exe is None:
        pytest.skip("CLI がない（cargo build --release -p lensbench-cli）")
    out = subprocess.run(
        [str(exe), "mech-check", str(P19), "--glass-dir", GLASS[0], "--json"],
        check=True, capture_output=True, text=True,
    ).stdout
    assert json.loads(out) == r


def test_errors():
    s = lb.System.load(str(P19), glass_dirs=GLASS)
    with pytest.raises(IndexError):
        s.mech_check(fields=[4])
    with pytest.raises(IndexError):
        s.mech_check(waves=[2])
    with pytest.raises(ValueError):
        s.mech_check(grid=1)
    plain = json.loads(P19.read_text())
    plain.pop("mechanics")
    t = lb.System.from_project(plain, base=str(P19.parent), glass_dirs=GLASS)
    with pytest.raises(ValueError):
        t.mech_check()


def test_fit():
    # 内径 20 の筒にレンズ（半径 12.7）: 前後の面の縁が壁に当たる
    p = json.loads(P19.read_text())
    p["mechanics"] = [{"name": "narrow", "mech": {"bores": [{"z0": 0, "z1": 40, "diameter": 20}], "outer_diameter": 30},
                       "at": {"block": "stop"}}]
    s = lb.System.from_project(p, base=str(P19.parent), glass_dirs=GLASS)
    fit = s.mech_check(grid=5)["fit"]
    assert [f["kind"] for f in fit] == ["collision", "collision"]
    assert all(f["error"] and f["part"] == "narrow" and abs(f["value"] - 2.7) < 1e-9 for f in fit)
