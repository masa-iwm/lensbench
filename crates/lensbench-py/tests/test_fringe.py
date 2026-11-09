"""単色の干渉縞の Python バインディングのテスト。入射瞳径 10・f 50 の理想レンズの 2 つの構成
（構成 2 は像面を 0.05 mm 後ろへ）。"""

import numpy as np

import lensbench as lb

ZMX = """VERS 260327 42
MODE SEQ
UNIT MM X W X CM MR CPMM
ENPD 10
FTYP 0 0 2 1 0 0 0
XFLN 0 0
YFLN 0 1
FWGN 1 1
WAVM 1 0.55 1
PWAV 1
SURF 0
  TYPE STANDARD
  CURV 0.0
  DISZ INFINITY
SURF 1
  STOP
  TYPE PARAXIAL
  PARM 1 50
  PARM 2 1
  DISZ 50
SURF 2
  TYPE STANDARD
  CURV 0.0
  DISZ 0
MNUM 2 1
THIC 1 1 50 0 0 0 1 1 0
THIC 1 2 50.05 0 0 0 1 1 0
"""


def systems(tmp_path):
    p = tmp_path / "mc.zmx"
    p.write_text(ZMX)
    return lb.System.load(str(p), cfg=1), lb.System.load(str(p), cfg=2)


def test_pupil_interferogram_is_the_difference_of_wavefronts(tmp_path):
    a, b = systems(tmp_path)
    r = lb.interferogram(a, b, field=1, n=32, scale=2.0, xtilt=1.0)
    assert r["delta"].shape == (32, 32) and r["p"].shape == (32,)
    m = ~np.isnan(r["delta"])
    assert m.sum() > 700
    # 焦点ずれの波面 n (1 − cos θ) dz の 2 倍 + 傾き 1 本（直径あたり）
    sin2 = 25.0 / (2500.0 + 25.0)
    pv = 2 * 0.05 * (1 - np.sqrt(1 - sin2)) / 0.55e-3
    np.testing.assert_allclose(r["intensity"][m], 0.5 * (1 + np.cos(2 * np.pi * r["delta"][m])), atol=1e-12)
    d = r["delta"] - 0.5 * r["p"][None, :]
    assert abs((np.nanmax(d) - np.nanmin(d)) - pv) < 2e-3 * pv
    # 参照球との干渉（光束 2 なし）: 焦点の理想レンズは平ら
    r0 = lb.interferogram(a, None, n=32)
    assert r0["pv"] < 1e-9


def test_detector_fringe_same_path_is_bright_and_delay_darkens(tmp_path):
    _, b = systems(tmp_path)
    r = lb.fringe_detector(b, b, npx=16, center=(0.0, 0.0), width=0.008, pupil=33)
    both = r["ok_sample"] & r["ok_reference"]
    assert both.sum() > 20
    np.testing.assert_allclose(r["visibility"][both], 1.0, atol=1e-12)
    np.testing.assert_allclose(r["intensity"][both], 4 * r["i_sample"][both], rtol=1e-9)
    r2 = lb.fringe_detector(b, b, npx=16, center=(0.0, 0.0), width=0.008, pupil=33, delay=0.55e-3 / 2)
    assert np.all(np.abs(r2["intensity"][both]) < 1e-9 * r2["i_sample"][both])
    assert r["x"].shape == (16,) and abs(r["wavelength"] - 0.55) < 1e-15
