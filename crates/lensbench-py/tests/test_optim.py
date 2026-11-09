"""減衰最小二乗の Python API。CLI の --json と同じ結果を返すこと、ヤコビアンの形。"""

import json
import os
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]


def zmx(name):
    return str(ROOT / "tests" / "refs" / name / "system.zmx")


def cli(*args):
    p = os.environ.get("LENSBENCH_CLI")
    if not p:
        exe = "lensbench.exe" if sys.platform == "win32" else "lensbench"
        p = ROOT / "target" / "release" / exe
        if not p.exists():
            pytest.skip("CLI がない（cargo build --release -p lensbench-cli）")
    out = subprocess.run(
        [str(p), *args, "--glass-dir", GLASS[0], "--json"],
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(out.stdout)


def test_optimize_matches_cli():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    base = s.with_params({"8.t": 0.3, "6.tx": 0.2})
    r = base.optimize(["8.t", ("6.tx", -1.0, 1.0)], merit="wavefront")
    assert r["merit"] < 0.5 * r["merit0"]
    assert r["names"] == ["8.t", "6.tx"]
    # 返す系は元の系を変えず、変数の値になっている
    assert base.get_param("8.t") == 0.3
    assert r["system"].get_param("8.t") == r["x"][0]
    assert r["history"][0] == r["merit0"] and r["history"][-1] == r["merit"]


def test_cli_optimize(tmp_path):
    out = tmp_path / "opt.zmx"
    c = cli("optimize", zmx("A0"), "--var", "8.t", "--var", "6.c=0:0.05", "--out", str(out))
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    r = s.optimize(["8.t", ("6.c", 0.0, 0.05)])
    assert c["merit_final"] == pytest.approx(r["merit"], rel=1e-12)
    assert [v["x"] for v in c["vars"]] == pytest.approx(r["x"], rel=1e-12)
    o = lb.System.load(str(out), glass_dirs=GLASS)
    assert o.get_param("8.t") == pytest.approx(r["x"][0], abs=1e-12)


def test_merit_jacobian():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    r, j = s.merit_jacobian(["8.t", "6.c"], merit="spot")
    assert j.shape == (r.shape[0], 2)
    rf, jf = s.merit_jacobian(["8.t", "6.c"], merit="spot", jac="fd")
    np.testing.assert_array_equal(r, rf)
    np.testing.assert_allclose(j, jf, rtol=0, atol=1e-5 * np.abs(jf).max())


# ---------------------------------------------------------------- 項の追加


def test_terms_paraxial_only():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    r = s.optimize(
        ["2.c"], merit=None, terms=[{"type": "paraxial", "q": "efl", "target": 60.0}]
    )
    assert r["term_values"][0] == pytest.approx(60.0, abs=1e-9)
    assert r["term_values0"][0] != pytest.approx(60.0, abs=1e-3)


def test_terms_match_cli():
    t = {"type": "ray", "field": 1, "py": 0.7, "q": "y", "target": 0.0}
    c = cli("optimize", zmx("A0"), "--var", "8.t", "--merit", "none", "--term", json.dumps(t))
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    r = s.optimize(["8.t"], merit=None, terms=[t])
    assert c["merit_final"] == pytest.approx(r["merit"], rel=1e-9, abs=1e-15)
    assert c["terms"][0]["value"] == pytest.approx(r["term_values"][0], abs=1e-12)


def _ray_y(systems):
    out = []
    for s in systems:
        # 波長 1
        t = s.trace(1, [0.0, 0.0], [0.7, -0.7], wavelengths=[1])
        out.append(t["y"][0])
    return np.array(out)


def test_user_term_matches_ray_terms():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    calls = []

    def f(systems):
        calls.append(len(systems))
        return _ray_y(systems)

    rays = [
        {"type": "ray", "field": 1, "wave": 1, "py": py, "q": "y"} for py in (0.7, -0.7)
    ]
    r1, j1 = s.merit_jacobian(["8.t", "6.c"], merit=None, terms=rays)
    r2, j2 = s.merit_jacobian(["8.t", "6.c"], merit=None, terms=[{"type": "user", "fn": f}])
    assert np.allclose(r1, r2, rtol=0, atol=1e-12)
    assert np.allclose(j1, j2, rtol=1e-6, atol=1e-9)
    # 行数を決める 1 回・中心の系 1 回・差分の系 2×2 を 1 回で
    assert calls == [1, 1, 4]


def test_user_term_jac_and_errors():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    seen = []

    def jac(sys, names):
        seen.append(names)
        return np.ones((2, len(names)))

    r, j = s.merit_jacobian(
        ["8.t", "6.c"], merit=None, terms=[{"type": "user", "fn": _ray_y, "jac": jac}]
    )
    assert seen == [["8.t", "6.c"]]
    assert np.all(j == 1.0)

    class Boom(Exception):
        pass

    n = []

    def bad(systems):
        n.append(1)
        if len(n) > 1:
            raise Boom("x")
        return _ray_y(systems)

    with pytest.raises(Boom):
        s.optimize(["8.t"], merit=None, terms=[{"type": "user", "fn": bad}])
    with pytest.raises(ValueError):
        s.optimize(["8.t"], merit=None, terms=[{"type": "ray", "field": 1, "q": "y", "oops": 1}])
    with pytest.raises(ValueError):
        s.optimize(["8.t"], merit=None, terms=[{"type": "user"}])


def test_user_term_optimize():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS).with_params({"8.t": 0.3})
    r = s.optimize(
        ["8.t"], merit=None, terms=[{"type": "user", "fn": lambda ss: _ray_y(ss)[:, :1] - _ray_y(ss)[:, 1:]}]
    )
    assert r["merit"] < 1e-9
