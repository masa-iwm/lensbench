"""MTF 系の解析の Python API。CLI の --json と同じ値を返すことを確かめる。"""

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
    ).stdout
    return json.loads(out)


@pytest.fixture(scope="module")
def b0():
    return lb.System.load(zmx("B0"), glass_dirs=GLASS)


def test_mtf_and_through_focus(b0):
    f = [20.0, 50.0]
    m = b0.mtf(2, f)
    t = b0.through_focus_mtf(2, f, [-0.01, 0.0, 0.01])
    assert t["tangential"].shape == (2, 3)
    assert np.array_equal(t["tangential"][:, 1], m["tangential"])
    assert np.array_equal(t["sagittal"][:, 1], m["sagittal"])
    c = cli("mtf-focus", zmx("B0"), "--field", "2", "--freqs", "20,50", "--range", "0.01", "--points", "3")
    rows = c["through_focus"]
    got = np.array([r["tangential"] for r in rows]).reshape(2, 3)
    assert np.array_equal(got, t["tangential"])


def test_mtf_each_wave_and_fft(b0):
    f = [20.0, 50.0]
    m = b0.mtf(2, f, each_wave=True)
    assert np.array_equal(m["tangential"], b0.mtf(2, f)["tangential"])
    assert len(m["waves"]) == len(b0.wavelengths)
    w1 = b0.mtf(2, f, wave=1)
    assert np.array_equal(m["waves"][0]["sagittal"], w1["sagittal"])
    c = cli("mtf", zmx("B0"), "--field", "2", "--freqs", "20,50", "--each-wave")["mtf"][0]
    assert np.array_equal(c["waves"][0]["sagittal"], w1["sagittal"])
    ff = b0.mtf(2, f, method="fft")
    assert np.allclose(ff["tangential"], m["tangential"], atol=5e-4)
    with pytest.raises(ValueError):
        b0.mtf(2, f, wave=1, each_wave=True)
    with pytest.raises(ValueError):
        b0.mtf(2, f, method="x")


def test_field_mtf(b0):
    h = np.linspace(0, 1, 3)
    m = b0.field_mtf([50.0], h)
    c = cli("mtf-field", zmx("B0"), "--freqs", "50", "--points", "3")
    assert np.array_equal(m["sagittal"][0], [r["sagittal"] for r in c["field_mtf"]])
    assert np.array_equal(m["field_value"], [r["field_value"] for r in c["field_mtf"]])
    assert m["warnings"] == []


def test_rms(b0):
    one = b0.rms(2)
    assert set(one) >= {"spot", "spot_x", "spot_y", "wfe", "strehl"}
    r = b0.rms(vs="field", x=np.linspace(0, 1, 3), reference="chief")
    c = cli("rms", zmx("B0"), "--vs", "field", "--points", "3", "--reference", "chief")
    assert np.array_equal(r["wfe"], [q["wfe"] for q in c["rms"]])
    assert np.array_equal(r["spot"], [q["spot"] for q in c["rms"]])
    w = b0.rms(2, vs="wavelength", x=b0.wavelengths)
    for k in range(len(b0.wavelengths)):
        assert w["wfe"][k] == b0.rms(2, wave=k + 1)["wfe"]
    z = b0.rms(1, vs="focus", x=[-0.1, 0.0, 0.1], wave=2)
    assert z["wfe"][1] == b0.rms(1, wave=2)["wfe"]
    with pytest.raises(ValueError):
        b0.rms(vs="focus", x=[0.0])
    with pytest.raises(ValueError):
        b0.rms(2, vs="field", x=[0.0])


def test_psf(b0):
    p = b0.psf(1, wave=2, pupil=32, image=64)
    assert p["i"].shape == (64, 64)
    assert p["peak"] == p["i"].max()
    assert 0 < p["peak"] <= 1
    c = cli("psf", zmx("B0"), "--field", "1", "--wave", "2", "--pupil", "32", "--image", "64")
    assert np.array_equal(p["i"].ravel(), [q["i"] for q in c["psf"]])
    with pytest.raises(ValueError):
        b0.psf(1, pupil=16, image=64)


def test_geometric_mtf(b0):
    g = b0.geometric_mtf(2, [0.0, 50.0], diff_limit=True)
    assert g["tangential"][0] == pytest.approx(1.0, abs=1e-14)
    c = cli("geo-mtf", zmx("B0"), "--field", "2", "--freqs", "0,50", "--diff-limit")
    assert np.array_equal(g["tangential"], c["geometric_mtf"][0]["tangential"])


def test_huygens_psf_mtf(b0):
    p = b0.huygens_psf(1, wave=2, pupil=32, image=64)
    assert p["i"].shape == (64, 64)
    assert 0 < p["peak"] <= 1
    c = cli("psf", zmx("B0"), "--huygens", "--field", "1", "--wave", "2", "--pupil", "32", "--image", "64")
    assert c["method"] == "huygens"
    assert np.array_equal(p["i"].ravel(), [q["i"] for q in c["psf"]])
    m = b0.huygens_mtf(2, [0.0, 50.0], wave=2, pupil=32, image=64)
    assert m["tangential"][0] == pytest.approx(1.0, abs=1e-12)
    c = cli("huygens-mtf", zmx("B0"), "--field", "2", "--wave", "2", "--freqs", "0,50", "--pupil", "32", "--image", "64")
    assert c["method"] == "huygens"
    assert np.array_equal(m["tangential"], c["mtf"][0]["tangential"])
