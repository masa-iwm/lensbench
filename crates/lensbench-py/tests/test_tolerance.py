"""モンテカルロ公差の Python API。CLI の --json と同じ結果を返すこと、Python の関数の項。"""

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
        # 結果に日本語のラベルがある（Windows の既定のコードページでは読めない）
        encoding="utf-8",
    )
    return json.loads(out.stdout)


SPEC = {"trials": 8, "seed": 5, "preset": "precision", "compensators": ["6.t"]}


def test_tolerance_matches_cli():
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    r = s.tolerance(SPEC, threads=2)
    c = cli("tolerance", zmx("C0"), "--set", json.dumps(SPEC))
    assert r["trials"] == 8 and r["failed"] == 0
    assert r["cdf"] == c["cdf"]
    assert r["nominal"] == c["nominal"]
    assert [x["label"] for x in r["sensitivity"]] == [x["label"] for x in c["sensitivity"]]
    assert r["merit"]["n"] == 8
    assert r["cdf"] == sorted(r["cdf"])


def test_tolerance_user_term():
    # Python の関数の項（計測を模した値などに使う）: ここでは像面の主光線の y を残差にする
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)

    def f(systems):
        out = []
        for x in systems:
            t = x.trace(field=1, px=[0.0], py=[0.0])
            out.append([float(np.asarray(t["y"]).ravel()[-1])])
        return np.array(out)

    spec = {"trials": 4, "merit": "none", "sensitivity": False, "samples": True,
            "tols": [{"param": "lens1.dy", "pm": 0.05}], "preset": "none"}
    r = s.tolerance(spec, terms=[{"type": "user", "fn": f, "name": "chief_y"}])
    assert r["trials"] == 4
    assert len(r["terms"]) == 1
    assert len(r["samples"]) == 4
    # 偏心があると主光線がずれる
    assert any(abs(x) > 0 for x in r["terms"][0]["cdf"])


def test_tolerance_bad_spec():
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    with pytest.raises(ValueError):
        s.tolerance({"trails": 3})
    with pytest.raises(ValueError):
        s.tolerance({"lenses": {"nolens": None}})


@pytest.mark.skipif(sys.platform == "win32", reason="SIGINT を子に送るのは POSIX で")
def test_tolerance_ctrl_c_interrupts():
    # 長い公差の計算も Ctrl-C（SIGINT）で KeyboardInterrupt になって止まる
    import signal
    import time

    code = (
        "import lensbench as lb, sys\n"
        f"s = lb.System.load({zmx('C0')!r}, glass_dirs={GLASS!r})\n"
        "print('start', flush=True)\n"
        "try:\n"
        "    s.tolerance({'trials': 200000, 'seed': 1, 'preset': 'precision', 'compensators': ['6.t']}, threads=1)\n"
        "except KeyboardInterrupt:\n"
        "    print('interrupted', flush=True)\n"
        "    sys.exit(3)\n"
    )
    p = subprocess.Popen([sys.executable, "-c", code], stdout=subprocess.PIPE, text=True)
    assert p.stdout.readline().strip() == "start"
    time.sleep(1.0)
    t0 = time.monotonic()
    p.send_signal(signal.SIGINT)
    out, _ = p.communicate(timeout=60)
    assert p.returncode == 3, out
    assert "interrupted" in out
    assert time.monotonic() - t0 < 10
