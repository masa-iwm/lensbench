"""照合試験の枠組みを Python から: 例題・Python で足したプラグイン・比べ方だけ"""

import json
import pathlib

import numpy as np

import lensbench as lb

ROOT = pathlib.Path(__file__).resolve().parents[3]


def test_public_samples():
    reps = lb.verify(str(ROOT / "tests/verify"), ci=True)
    assert len(reps) >= 1 and all(r["passed"] for r in reps), reps
    z = next(r for r in reps if r["plugin"] == "zernike")
    assert z["source"]["by"].startswith("OpticStudio")
    assert all(r["threads_identical"] for r in z["runs"])


def test_python_plugin_and_ci_flag(tmp_path):
    lb.register_plugin("twice_py", lambda cfg, a: {"y": 2.0 * a["x"], "ok": a["x"] > 0})
    try:
        np.savez(tmp_path / "out.npz", y=np.array([2.0, np.nan, -4.0]), ok=np.array([True, False, False]))
        m = {
            "format": "lensbench-verify", "version": 1, "name": "twice",
            "source": {"by": "手で書いた値"}, "ci": False, "plugin": "twice_py",
            "checks": {"y": {"abs": 1e-12}, "ok": {"exact": True}},
            "runs": [{"inputs": {"x": [1.0, None, -2.0]}, "expected": "out.npz"}],
        }
        (tmp_path / "a.verify.json").write_text(json.dumps(m))
        reps = lb.verify(tmp_path)
        assert reps[0]["passed"], reps
        # ci: false は ci=True で飛ばす
        assert lb.verify([str(tmp_path)], ci=True) == []
        # NaN の位置が違えば不一致
        m["runs"][0]["inputs"]["x"] = [1.0, 3.0, -2.0]
        (tmp_path / "a.verify.json").write_text(json.dumps(m))
        r = lb.verify(tmp_path)[0]
        assert not r["passed"]
        y = next(c for c in r["runs"][0]["checks"] if c["name"] == "y")
        assert y["mismatched"] == 1
    finally:
        lb.unregister_plugin("twice_py")


def test_compare_only():
    a = {"j": np.array([1 + 0j, 2j + 2e-9])}
    e = {"j": np.array([1 + 0j, 2j])}
    r = lb.verify_compare(a, e, {"j": {"rel": 1.1e-9}})
    assert r[0]["passed"] and abs(r[0]["max_abs"] - 2e-9) < 1e-15
    assert not lb.verify_compare(a, e, {"j": {"abs": 1e-12}})[0]["passed"]
    try:
        lb.verify_compare(a, e, {})
        raise AssertionError("checks の無い基準を受けた")
    except ValueError:
        pass
