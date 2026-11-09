"""ズーム軌跡・カタログレンズの差し替え探索の Python API。CLI の --json と同じ結果を返すこと。"""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]
ZOOM = str(ROOT / "examples" / "projects" / "P17_zoom_three_groups.json")
SWAP = str(ROOT / "examples" / "projects" / "P18_swap_two.json")
LB = [str(ROOT / "examples" / "lenses" / "LB")]


def cli(*args):
    p = os.environ.get("LENSBENCH_CLI")
    if not p:
        exe = "lensbench.exe" if sys.platform == "win32" else "lensbench"
        p = ROOT / "target" / "release" / exe
        if not p.exists():
            pytest.skip("CLI がない（cargo build --release -p lensbench-cli）")
    out = subprocess.run(
        [str(p), *args, "--glass-dir", GLASS[0], "--json"],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(out.stdout)


def states():
    return [lb.System.load(ZOOM, glass_dirs=GLASS, state=n) for n in ("wide", "mid", "tele")]


def test_zoom_reaches_targets_and_matches_cli():
    r = lb.zoom(states(), ["G2.shift", ("G3.shift", -10.0, 30.0)], efl=[95.0, None, 130.0], steps=1)
    assert r["quantity"] == "efl"
    assert r["names"] == ["G2.shift", "G3.shift"]
    assert len(r["path"]) == 5
    assert [p["state"] for p in r["path"]] == [0, None, 1, None, 2]
    assert r["path"][2]["target"] is None
    for p in (r["path"][0], r["path"][4]):
        assert abs(p["value"] - p["target"]) < 1e-8 * p["target"]
    assert len(r["systems"]) == 3
    assert r["systems"][2].system_data()["effl"] == r["path"][4]["value"]
    c = cli("zoom", ZOOM, "--var", "G2.shift", "--var", "G3.shift=-10:30", "--efl", "95,-,130", "--steps", "1")
    for a, b in zip(r["path"], c["path"]):
        assert a["value"] == b["efl"]
        assert a["x"] == [b["G2.shift"], b["G3.shift"]]
        assert a["merit"] == b["merit"]


def test_zoom_errors():
    with pytest.raises(ValueError):
        lb.zoom(states(), ["G2.shift"], efl=[95.0])
    with pytest.raises(ValueError):
        lb.zoom(states(), ["G2.shift"], efl=[1, 2, 3], pmag=[1, 2, 3])
    with pytest.raises(ValueError):
        lb.zoom([], ["G2.shift"])


def test_swap_search_matches_cli(tmp_path):
    spec = {
        "slots": [
            {"block": "A", "filter": {"efl": [40, None]}, "flip": True},
            {"block": "B", "parts": [{"vendor": "LB", "part": "ACH-45"}, {"vendor": "LB", "part": "BCX-50"}]},
        ],
        "vars": ["gap.t=0.5:20", "bfl.t=1:200"],
        "iters": 15,
    }
    r = lb.swap_search(SWAP, spec, glass_dirs=GLASS, lens_catalogs=LB, top=5)
    assert len(r) == 5
    m = [x["merit"] for x in r]
    assert m == sorted(m)
    assert r[0]["lenses"][0]["block"] == "A"
    f = tmp_path / "spec.json"
    f.write_text(json.dumps(spec))
    c = cli("swap", SWAP, "--lens-catalog", LB[0], "--spec", str(f), "--top", "5")
    assert c["results"] == r
    # JSON の文字列でも
    assert lb.swap_search(SWAP, json.dumps(spec), glass_dirs=GLASS, lens_catalogs=LB, top=5, threads=1) == r
    # out の JSONL に書き出し、続きから（全部書いてある）でも同じ
    out = tmp_path / "r.jsonl"
    assert lb.swap_search(SWAP, spec, glass_dirs=GLASS, lens_catalogs=LB, top=5, out=str(out)) == r
    assert lb.swap_search(SWAP, spec, glass_dirs=GLASS, lens_catalogs=LB, top=5, out=str(out)) == r
    with pytest.raises(ValueError):
        lb.swap_search(SWAP, {"slots": [{"block": "X"}]}, glass_dirs=GLASS, lens_catalogs=LB)
