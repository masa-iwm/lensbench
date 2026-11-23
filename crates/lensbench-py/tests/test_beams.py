"""プロジェクトの光束（beams）: 一覧・光束ごとの系・光束の表。"""

import json
from pathlib import Path

import numpy as np
import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]
TG = ROOT / "examples" / "projects" / "P16_twyman_green.json"

BEAMS = [
    {"name": "test", "path": "TR", "role": "sample"},
    {"name": "refb", "path": "RT", "role": "reference"},
    {"name": "g0", "path": "TR", "turn": {"block": "TEST", "surface": 0}, "role": "stray", "off": True},
]


@pytest.fixture
def project(tmp_path):
    v = json.loads(TG.read_text(encoding="utf-8"))
    v["beams"] = BEAMS
    p = tmp_path / "tg.json"
    p.write_text(json.dumps(v), encoding="utf-8")
    return str(p)


def test_beams_lists_the_beams_as_written(project):
    b = lb.System.beams(project)
    assert [x["name"] for x in b] == ["test", "refb", "g0"]
    assert b[0] == {"name": "test", "path": "TR", "state": None, "role": "sample", "turn": None, "off": False}
    assert b[2]["turn"] == {"block": "TEST", "surface": 0} and b[2]["off"] is True
    assert lb.System.beams(str(TG)) == []


def test_load_beams_matches_load_with_the_beam_name(project):
    d = lb.System.load_beams(project, glass_dirs=GLASS)
    assert list(d) == ["test", "refb"]
    for name, s in d.items():
        one = lb.System.load(project, glass_dirs=GLASS, state=name)
        assert s.hash == one.hash, name
    assert d["refb"].hash == lb.System.load(project, glass_dirs=GLASS, beam_path="RT").hash
    every = lb.System.load_beams(project, all=True, glass_dirs=GLASS)
    assert list(every) == ["test", "refb", "g0"]
    two = lb.System.load_beams(project, beams=["refb", "test"], glass_dirs=GLASS)
    assert list(two) == ["refb", "test"]
    # 光束を一緒に追跡する
    r = lb.trace_many(list(two.values()), 1, np.array([0.0]), np.array([0.0]))
    assert len(r) == 2
    with pytest.raises(lb.InvalidError, match="no beam x"):
        lb.System.load_beams(project, beams=["x"], glass_dirs=GLASS)
    with pytest.raises(lb.InvalidError, match="empty"):
        lb.System.load_beams(project, beams=[], glass_dirs=GLASS)
    with pytest.raises(lb.InvalidError, match="no beams"):
        lb.System.load_beams(str(TG), glass_dirs=GLASS)


def test_beam_table_is_the_same_as_the_analysis(project):
    t = lb.System.beam_table(project, glass_dirs=GLASS)
    assert t["reference"] == "refb"
    rows = t["beams"]
    assert [r["name"] for r in rows] == ["test", "refb"]
    assert rows[1]["opl_difference_mm"] == 0.0
    assert abs(rows[1]["transmission"] - 0.25) < 1e-9
    d = rows[0]["chief_opl_mm"] - rows[1]["chief_opl_mm"]
    assert rows[0]["opl_difference_mm"] == pytest.approx(d, abs=1e-12) and d != 0.0
    every = lb.System.beam_table(project, all=True, reference="test", grid=5, glass_dirs=GLASS)
    assert [r["name"] for r in every["beams"]] == ["test", "refb", "g0"]
    assert every["reference"] == "test"
    with pytest.raises(lb.InvalidError, match="no beam x"):
        lb.System.beam_table(project, reference="x", glass_dirs=GLASS)
