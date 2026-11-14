"""lensbench の Python バインディングのテスト。

CLI（LENSBENCH_CLI、なければ target/release/lensbench）の rays --save の結果とビット一致することも確かめる。
"""

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


def cli():
    p = os.environ.get("LENSBENCH_CLI")
    if p:
        return p
    exe = "lensbench.exe" if sys.platform == "win32" else "lensbench"
    p = ROOT / "target" / "release" / exe
    if not p.exists():
        pytest.skip("CLI がない（cargo build --release -p lensbench-cli）")
    return str(p)


def grid(n):
    g = -1 + 2 * np.arange(n) / (n - 1)
    x, y = np.meshgrid(g, g)
    m = x**2 + y**2 <= 1
    return x[m].copy(), y[m].copy()


def test_system_properties():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    assert len(s.wavelengths) == 3 and s.primary == 2
    assert len(s.fields) == 4
    assert s.image == 9 and 1 <= s.stop <= 9
    assert s.aiming == "off"
    assert s.hxhy(1) == (0.0, 0.0)


@pytest.mark.parametrize("name", ["A0", "A7", "C0"])
def test_matches_cli_bit_for_bit(name, tmp_path):
    s = lb.System.load(zmx(name), glass_dirs=GLASS)
    out = tmp_path / "rays.npz"
    subprocess.run(
        [cli(), "rays", zmx(name), "--glass-dir", GLASS[0], "--all-waves", "--field", "2",
         "--fan", "grid", "--rays", "24", "--surface", str(s.image), "--save", str(out)],
        check=True,
        capture_output=True,
    )
    d = np.load(out)
    nw = len(s.wavelengths)
    m = d["wave"] == 1
    px, py = d["px"][m].copy(), d["py"][m].copy()
    r = s.trace(2, px, py, globals=True)
    assert r["x"].shape == (nw, len(px))
    for k in ["x", "y", "z", "l", "m", "n", "gx", "gy", "gz", "opl"]:
        b = np.stack([d[k][d["wave"] == w + 1] for w in range(nw)])
        assert np.array_equal(r[k].view(np.uint64), b.view(np.uint64)), k
    e = np.stack([d["err"][d["wave"] == w + 1] for w in range(nw)])
    assert np.array_equal(r["err"], e)


def test_threads_do_not_change_the_results():
    s = lb.System.load(zmx("A7"), glass_dirs=GLASS)
    px, py = grid(48)
    a = s.trace(3, px, py, record=True, pol=True, globals=True, threads=1)
    for th in (2, 4, None):
        b = s.trace(3, px, py, record=True, pol=True, globals=True, threads=th)
        assert a.keys() == b.keys()
        for k in a:
            if isinstance(a[k], np.ndarray):
                assert np.array_equal(a[k], b[k], equal_nan=a[k].dtype.kind in "fc"), k


def test_wavelengths_last_and_record():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    px, py = grid(8)
    r = s.trace((0.0, 0.5), px, py, wavelengths=[3, 1], last=5, record=True)
    assert list(r["wavelengths"]) == [3, 1] and r["last"] == 5
    assert (r["hx"], r["hy"]) == (0.0, 0.5)
    assert list(r["rec_surfaces"]) == [1, 2, 3, 4, 5]
    assert r["rec_x"].shape == (2, 5, len(px))
    # 最後に記録した面は last の面
    assert np.array_equal(r["rec_x"][:, -1], r["x"])
    one = s.trace((0.0, 0.5), px, py, wavelengths=[1], last=5)
    assert np.array_equal(one["opl"][0], r["opl"][1])
    assert "gx" not in r and "jones" not in r


def test_record_selected_surfaces():
    # 面を選んだ記録は全面の記録の該当する面と同じ（無視・範囲外の面は除く）。grad と一緒でも同じ
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    px, py = grid(8)
    full = s.trace(2, px, py, record=True)
    part = s.trace(2, px, py, record=[3, 1, 99])
    assert list(part["rec_surfaces"]) == [1, 3]
    assert part["rec_x"].shape == (3, 2, len(px))
    for k, j in enumerate([1, 3]):
        for c in ["rec_x", "rec_n", "rec_opl", "rec_gz"]:
            assert np.array_equal(part[c][:, k], full[c][:, j - 1], equal_nan=True)
    # 面が 1 つもなくても rec_* は (波長, 0, 光線) の空配列
    e = s.trace(2, px, py, record=[99])
    assert e["rec_surfaces"].shape == (0,) and e["rec_x"].shape == (3, 0, len(px))
    g = s.trace(2, px, py, record=[3], grad=["2.t"], wavelengths=[1])
    assert list(g["rec_surfaces"]) == [3]
    gf = s.trace(2, px, py, grad=["2.t"], wavelengths=[1])
    assert np.array_equal(g["grad"], gf["grad"], equal_nan=True)


def test_jones():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    px, py = grid(16)
    r = s.trace(4, px, py, wavelengths=[2], pol=True)
    j = r["jones"]
    assert j.shape == (len(px), 2, 2) and j.dtype == np.complex128
    assert r["jones_wavelength"] == 2
    ok = r["ok"][0]
    # 軸上の主光線: 対角だけで、透過率は 1 未満
    a = s.trace(1, np.zeros(1), np.zeros(1), pol=True)["jones"][0]
    assert 0.5 < abs(a[0, 0]) < 1 and abs(a[0, 0] - a[1, 1]) < 1e-12
    assert abs(a[0, 1]) < 1e-12 and abs(a[1, 0]) < 1e-12
    assert np.all(np.isfinite(j[ok]))
    # 伝搬の位相: E ∝ exp(−ikL)
    p = s.trace(4, px, py, wavelengths=[2], pol=2, pol_phase=True)["jones"]
    k = 2 * np.pi / (s.wavelengths[1] * 1e-3)
    ph = np.exp(-1j * k * r["opl"][0])[:, None, None]
    assert np.allclose(p[ok], (j * ph)[ok], rtol=1e-12, atol=1e-14)


def test_errors():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    z = np.zeros(3)
    with pytest.raises(IndexError):
        s.trace(9, z, z)
    with pytest.raises(IndexError):
        s.trace(1, z, z, wavelengths=[0])
    with pytest.raises(IndexError):
        s.trace(1, z, z, last=99)
    with pytest.raises(ValueError):
        s.trace(1, z, np.zeros(2))
    with pytest.raises(ValueError):
        s.trace(1, z, z, last=3, pol=True)
    with pytest.raises(ValueError):
        lb.System.load(zmx("A0"), glass_dirs=GLASS, cfg=0)
    with pytest.raises(ValueError):
        lb.System.load(str(ROOT / "nope.zmx"))


def test_independent_systems():
    a = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    b = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    px, py = grid(8)
    ra = a.trace(1, px, py)
    b.trace(1, px, py)
    assert np.array_equal(a.trace(1, px, py)["x"], ra["x"])


def test_pupil_arrays_of_any_layout():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    px, py = grid(16)
    ref = s.trace(2, px, py)["x"]
    g = np.stack([px, np.zeros_like(px)], axis=1)[:, 0]  # 飛び飛びの配列
    assert not g.flags.c_contiguous
    assert np.array_equal(s.trace(2, g, py)["x"], ref)
    assert np.array_equal(s.trace(2, list(px), list(py))["x"], ref)
    q = s.trace(2, px.astype(np.float32), py.astype(np.float32))["x"]
    assert np.array_equal(q, s.trace(2, px.astype(np.float32).astype(float), py.astype(np.float32).astype(float))["x"])
    with pytest.raises(TypeError):
        s.trace(True, px, py)


def test_jones_z_component():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    px, py = grid(8)
    r = s.trace(3, px, py, pol=True)
    assert r["jones_z"].shape == (len(px), 2) and r["jones_z"].dtype == np.complex128
    # 電場は光線に垂直: E · k = 0（最後の面の局所座標）
    ok = r["ok"][s.primary - 1]
    w = s.primary - 1
    k = np.stack([r["l"][w], r["m"][w], r["n"][w]], axis=1)
    for c in range(2):
        e = np.stack([r["jones"][:, 0, c], r["jones"][:, 1, c], r["jones_z"][:, c]], axis=1)
        assert np.all(np.abs(np.sum(e * k, axis=1))[ok] < 1e-12)


def test_params():
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    t0 = s.get_param("3.t")
    px, py = grid(8)
    base = s.trace(2, px, py)
    s2 = s.with_params({"3.t": t0 + 0.5, "2.dx": 0.01})
    assert s.get_param("3.t") == t0 and s2.get_param("3.t") == t0 + 0.5
    moved = s2.trace(2, px, py)
    assert not np.array_equal(moved["x"], base["x"])
    s.set_params([("3.t", t0 + 0.5), ("2.dx", 0.01)])
    assert np.array_equal(s.trace(2, px, py)["x"], moved["x"])
    s.set_param("3.t", t0)
    s.set_param("2.dx", 0.0)
    assert np.array_equal(s.trace(2, px, py)["x"], base["x"])
    assert s.find_surface("3") == 3
    with pytest.raises(ValueError):
        s.get_param("3.zz")
    with pytest.raises(ValueError):
        s.set_param("99.t", 1.0)


def test_unit_params():
    # 玉・群の単位のパラメータ: 玉は読込で lens1 …、群は define_unit で
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    names = [u["name"] for u in s.units()]
    assert names == ["lens1", "lens2", "lens3"]
    (a, _), = s.units()[1]["spans"]
    (_, b), = s.units()[2]["spans"]
    s.define_unit("G", [(a, b)], pivot=(0.0, 0.0, 1.0))
    px, py = grid(8)
    base = s.trace(1, px, py)
    t0 = s.get_param(f"{a - 1}.t")
    s2 = s.with_params({"G.shift": 0.3, "lens1.dn": 1e-4})
    assert s2.get_param("G.shift") == 0.3 and s2.get_param("lens1.dn") == 1e-4
    assert s2.get_param(f"{a - 1}.t") == t0 + 0.3
    s3 = s.with_params({"G.tx": 0.2, "lens3.dy": 0.01})
    assert s3.get_param("G.tx") == 0.2
    assert not np.array_equal(s3.trace(1, px, py)["x"], base["x"])
    # 0 に戻すと元と同じ
    s4 = s3.with_params({"G.tx": 0.0, "lens3.dy": 0.0})
    assert np.array_equal(s4.trace(1, px, py)["x"], base["x"])
    s.set_unit_pivot("G", (0.0, 0.0, 0.0))
    assert s.units()[3]["pivot"] == [0.0, 0.0, 0.0]
    with pytest.raises(ValueError):
        s.define_unit("G", [(a, b)])
    with pytest.raises(ValueError):
        s.get_param("G.c")


def test_object_matches_field_without_vignetting():
    for name in ("A0", "B0"):
        s = lb.System.load(zmx(name), glass_dirs=GLASS)
        px, py = grid(8)
        o = s.object_of(3)
        a = s.trace(3, px, py)
        b = s.trace(None, px, py, object=tuple(o))
        assert b["object"] == o
        for k in ("x", "y", "l", "m", "opl"):
            assert np.array_equal(a[k], b[k]), (name, k)
    with pytest.raises(ValueError):
        s.trace(None, px, py, object=(1.0, 0.0, 1.0))  # 有限物体に方向
    with pytest.raises(ValueError):
        s.trace(1, px, py, object=(0.0, 0.0))


def test_grad_matches_finite_difference_with_fixed_end_point():
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    names = ["1.c", "3.t", "2.tx", "8.dx"]
    px, py = np.array([0.1, -0.4]), np.array([0.3, 0.2])
    w = s.primary
    r = s.trace(2, px, py, wavelengths=[w], grad=names, record=True)
    assert r["grad"].shape == (1, 2, len(names)) and r["grad_params"] == names
    obj = tuple(s.object_of(2))
    img = s.image
    for i in range(2):
        tgt = (r["x"][0, i], r["y"][0, i])
        for j, p in enumerate(names):
            h = 1e-6 if p.endswith(".c") else 1e-4
            v = s.get_param(p)
            o = []
            for d in (h, -h):
                t = s.with_params({p: v + d})
                sol = t.solve_ray("pupil", ("position", img, tgt), object=obj, pupil=(px[i], py[i]))
                assert max(map(abs, sol["residual"])) < 1e-9
                q = t.trace(None, [sol["pupil"][0]], [sol["pupil"][1]], wavelengths=[w], object=obj)
                o.append(q["opl"][0, 0])
            fd = (o[0] - o[1]) / (2 * h)
            assert abs(fd - r["grad"][0, i, j]) < 1e-6 * max(1.0, abs(fd)), (p, fd, r["grad"][0, i, j])



@pytest.mark.parametrize("dz", [None, 0.3])
def test_land_grad_and_dz_match_finite_difference(dz):
    """着地点の微分（開始位置・方向を固定）と検出面のずらし"""
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    names = ["1.c", "3.t", "2.tx", "8.dx", "3.dn"]
    px, py = np.array([0.1, -0.4, 0.0]), np.array([0.3, 0.2, -0.6])
    w = s.primary
    r = s.trace(2, px, py, wavelengths=[w], grad=names, land=True, dz=dz, globals=True)
    assert r["grad_land"].shape == (1, 3, len(names), 4)
    # 開始位置・方向を物体の座標系に直す（GLRS が面 9 なので、大域座標はパラメータで動く）
    e = np.vstack([np.zeros(3), np.eye(3)])
    f = s.trace_rays(e, np.tile([0.0, 0.0, 1.0], (4, 1)), wavelengths=[w], globals=True)["p0"][0]
    o, rm = f[0], (f[1:] - f[0]).T
    p0 = (r["p0"][0] - o) @ rm
    k0 = r["k0"][0] @ rm
    for j, p in enumerate(names):
        h = 1e-6 if p.endswith(".c") else 1e-5 if p.endswith(".dn") else 1e-4
        v = s.get_param(p)
        q = [s.with_params({p: v + d}).trace_rays(p0, k0, wavelengths=[w], dz=dz) for d in (h, -h)]
        for c, k in enumerate(["x", "y", "l", "m"]):
            fd = (q[0][k][0] - q[1][k][0]) / (2 * h)
            np.testing.assert_allclose(r["grad_land"][0, :, j, c], fd, rtol=1e-5, atol=1e-8, err_msg=f"{p} {k}")
    # dz: 平面までの延長と、その平面の x・y を固定した dOPL/dp（差分で解き直さず、ずらす前の値との関係で確かめる）
    if dz is not None:
        b = s.trace(2, px, py, wavelengths=[w], grad=names, globals=True)
        tt = (dz - b["z"][0]) / b["n"][0]
        np.testing.assert_allclose(r["z"][0], dz, atol=1e-14)
        np.testing.assert_allclose(r["x"][0], b["x"][0] + tt * b["l"][0], atol=1e-12)
        nimg = 1.0
        np.testing.assert_allclose(r["opl"][0], b["opl"][0] + nimg * tt, atol=1e-11)
        # 像面は平面・空気なので、媒質の微分も形の微分もなく grad は同じ
        np.testing.assert_allclose(r["grad"], b["grad"], atol=1e-12)
    with pytest.raises(ValueError):
        s.trace(2, px, py, land=True)
    with pytest.raises(ValueError):
        s.trace(2, px, py, dz=float("nan"))
    with pytest.raises(ValueError):
        s.trace(2, px, py, dz=0.1, pol=True, pol_phase=True)


def test_land_grad_of_unit_params_matches_finite_difference():
    """玉・群のパラメータの着地点の微分"""
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    s.set_unit_pivot("lens2", (0.0, 0.2, 0.5))
    names = ["lens2.dx", "lens2.dy", "lens2.tx", "lens3.ty", "lens3.tz", "lens1.dn", "lens2.shift"]
    px, py = np.array([0.1, -0.4, 0.0]), np.array([0.3, 0.2, -0.6])
    w = s.primary
    r = s.trace(2, px, py, wavelengths=[w], grad=names, land=True, globals=True)
    e = np.vstack([np.zeros(3), np.eye(3)])
    f = s.trace_rays(e, np.tile([0.0, 0.0, 1.0], (4, 1)), wavelengths=[w], globals=True)["p0"][0]
    o, rm = f[0], (f[1:] - f[0]).T
    p0 = (r["p0"][0] - o) @ rm
    k0 = r["k0"][0] @ rm
    for j, p in enumerate(names):
        h = 1e-5 if p.endswith(".dn") else 1e-4
        v = s.get_param(p)
        q = [s.with_params({p: v + d}).trace_rays(p0, k0, wavelengths=[w]) for d in (h, -h)]
        for c, k in enumerate(["x", "y", "l", "m"]):
            fd = (q[0][k][0] - q[1][k][0]) / (2 * h)
            np.testing.assert_allclose(r["grad_land"][0, :, j, c], fd, rtol=1e-5, atol=1e-8, err_msg=f"{p} {k}")


def test_solve_object_point():
    # 有限物体: 主光線が像面の指定点を通る物点
    s = lb.System.load(zmx("B0"), glass_dirs=GLASS)
    img = s.image
    sol = s.solve_ray("object", ("position", img, (0.0, -1.0)), field=2)
    assert max(map(abs, sol["residual"])) < 1e-9 and sol["iterations"] >= 1
    r = s.trace(None, [0.0], [0.0], object=tuple(sol["object"]))
    assert abs(r["x"][s.primary - 1, 0]) < 1e-9 and abs(r["y"][s.primary - 1, 0] + 1.0) < 1e-9
    with pytest.raises(ValueError):
        s.solve_ray("object", ("bogus", 1, 2))


def test_set_param_while_other_threads_trace():
    # 追跡（GIL を外す）と set_param（GIL を持つ）を別スレッドで同時に回してもデッドロックしない。
    # 止まると GIL ごと固まるので、別プロセスでタイムアウトをかける
    code = f"""
import threading, numpy as np, lensbench as lb
s = lb.System.load({zmx("C1")!r}, glass_dirs={GLASS!r})
g = -1 + 2 * np.arange(60) / 59
px, py = [a.ravel().copy() for a in np.meshgrid(g, g)]
t0 = s.get_param("3.t")
def tracer():
    for _ in range(15):
        s.trace(2, px, py, threads=2)
def setter():
    for i in range(300):
        s.set_param("3.t", t0 + 1e-3 * (i % 7))
ts = [threading.Thread(target=tracer) for _ in range(2)] + [threading.Thread(target=setter)]
[t.start() for t in ts]
[t.join() for t in ts]
s.set_param("3.t", t0)
print("ok")
"""
    r = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, timeout=120)
    assert r.returncode == 0 and r.stdout.strip() == "ok", r.stderr


def test_running_trace_keeps_the_old_system():
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    px, py = grid(8)
    a = s.trace(2, px, py)
    t0 = s.get_param("3.t")
    s.set_param("3.t", t0 + 1.0)
    assert s.get_param("3.t") == t0 + 1.0
    s.set_param("3.t", t0)
    assert np.array_equal(s.trace(2, px, py)["x"], a["x"])


def test_field_type_solve():
    s = lb.System.load(
        str(ROOT / "examples" / "projects" / "P14_tilted_spherical_double_pass.json"),
        glass_dirs=GLASS,
        lens_catalogs=[str(ROOT / "examples" / "lenses" / "LB")],
    )
    assert s.field_type == "object_height"
    s.set_field_type("solve", ("dirsum", 1, "6"), fields=[(0.0, 0.0), (0.002, -0.001)])
    assert s.field_type == ("solve", ("dirsum", 1, 6))
    w = s.primary - 1
    for i, tgt in enumerate([(0.0, 0.0), (0.002, -0.001)]):
        r = s.trace(i + 1, [0.0], [0.0], record=True)
        k = list(r["rec_surfaces"])
        a, b = k.index(1), k.index(6)
        for c, t in zip(("rec_l", "rec_m"), tgt):
            assert abs(r[c][w, a, 0] + r[c][w, b, 0] - t) < 1e-12
    o = s.object_of(1)
    sol = s.solve_ray("object", ("dirsum", 1, 6), object=(0.0, 0.0))
    assert np.allclose(o, sol["object"], atol=1e-9)
    # パラメータを変えると解き直す
    s2 = s.with_params({"3.dx": 0.5})
    assert s2.object_of(1) != o
    with pytest.raises(ValueError):
        s.set_field_type("solve")
    with pytest.raises(ValueError):
        s.set_field_type("angle", ("position", 3))


def test_field_type_image_position_is_real_image_height():
    a = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    h = [(float(a.trace(i, [0.0], [0.0])["x"][1, 0]), float(a.trace(i, [0.0], [0.0])["y"][1, 0])) for i in (1, 2, 3, 4)]
    b = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    a.set_field_type("real_image_height", fields=h)
    b.set_field_type("solve", ("position", b.image), fields=h)
    px, py = grid(8)
    for i in (1, 2, 3, 4):
        assert np.array_equal(a.trace(i, px, py)["x"], b.trace(i, px, py)["x"])



@pytest.mark.parametrize("name", ["A0", "B0", "B1"])
def test_trace_rays_reproduces_pupil_trace(name):
    """開始位置・方向を直接渡す追跡: trace の p0・k0 を渡すと同じ光線"""
    s = lb.System.load(zmx(name), glass_dirs=GLASS)
    px, py = grid(9)
    g = None if s.infinite else ["objy", "2.t"]
    t = s.trace(2, px, py, wavelengths=[1], globals=True, record=True, grad=g)
    r = s.trace_rays(t["p0"][0], t["k0"][0], wavelengths=[1], frame="global", record=True, grad=g)
    ok = t["ok"][0]
    assert ok.sum() > 0
    assert np.array_equal(ok, r["ok"][0])
    for k in ["x", "y", "z", "l", "m", "opl", "rec_x", "rec_opl"]:
        assert np.allclose(t[k][..., ok], r[k][..., ok], rtol=0, atol=1e-10), k
    if g:
        assert np.allclose(t["grad"][0][ok], r["grad"][0][ok], rtol=0, atol=1e-9)
        # 物体の座標系（物体面の局所座標、この系では大域と平行）: 開始位置は物点
        y = s.object_of(2)[1]
        st = np.tile([0.0, y, 0.0], (len(px), 1))
        r2 = s.trace_rays(st, t["k0"][0], wavelengths=[1])
        assert np.allclose(t["x"][0][ok], r2["x"][0][ok], rtol=0, atol=1e-10)
        assert np.allclose(t["opl"][0][ok], r2["opl"][0][ok], rtol=0, atol=1e-10)


@pytest.mark.parametrize("name", ["A0", "A2", "B1", "D2"])
def test_trace_at_stop_points(name):
    """pupil="stop": 絞り面の局所座標 [mm] を通る実光線"""
    s = lb.System.load(zmx(name), glass_dirs=GLASS)
    px, py = grid(7)
    xs, ys = 0.8 * px, 0.8 * py
    r = s.trace(2, xs, ys, wavelengths=[1, 2], last=s.stop, pupil="stop")
    ok = r["ok"]
    assert ok.mean() > 0.9
    assert np.abs(r["x"] - xs)[ok].max() < 1e-9
    assert np.abs(r["y"] - ys)[ok].max() < 1e-9
    with pytest.raises(ValueError):
        s.trace(2, xs, ys, pupil="bad")
    # 目標が NaN の光線は解けない扱い
    r = s.trace(2, [np.nan, 0.0, 0.5], [0.0, np.nan, 0.0], wavelengths=[1], pupil="stop")
    assert r["ok"][0].tolist() == [False, False, True]


def test_trace_rays_errors():
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    with pytest.raises(ValueError):
        s.trace_rays(np.zeros((3, 2)), np.zeros((3, 3)))
    with pytest.raises(ValueError):
        s.trace_rays(np.zeros((3, 3)), np.zeros((2, 3)))
    with pytest.raises(ValueError):
        s.trace_rays(np.zeros((1, 3)), [[0, 0, 1.0]], frame="bad")
    r = s.trace_rays([[0, 0, 0.0]], [[0, 0, 0.0]])
    assert not r["ok"][0, 0]


def test_signed_pupil(tmp_path):
    """pupil="signed": 入射瞳が物体より前の系では normalized と符号が逆、それ以外は同じ"""
    def two_lens(t1):
        p = tmp_path / f"t{t1}.zmx"
        p.write_text(
            "VERS 260327 42\nMODE SEQ\nUNIT MM X W X CM MR CPMM\nFLOA\nFTYP 1 0 2 1 0 0 0\n"
            "XFLN 0 0\nYFLN 0 1\nFWGN 1 1\nWAVM 1 0.5876 1\nPWAV 1\n"
            "SURF 0\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n"
            f"SURF 1\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ {t1}\n"
            "SURF 2\n  STOP\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 50\n  DIAM 2 1 0 0 1 \"\"\n"
            "SURF 3\n  TYPE PARAXIAL\n  PARM 1 50\n  PARM 2 1\n  DISZ 50\n"
            "SURF 4\n  TYPE STANDARD\n  CURV 0.0\n  DISZ 0\n")
        return lb.System.load(str(p))
    for t1, rev in [(100, True), (50.00001, True), (49.99999, False)]:
        s = two_lens(t1)
        assert s.pupil_reversed == rev
        a = s.trace(2, [1.0], [0.0], last=2)
        b = s.trace(2, [1.0], [0.0], last=2, pupil="signed")
        assert abs(b["x"][0, 0] - 2) < 1e-9
        assert abs(a["x"][0, 0] - (-2 if rev else 2)) < 1e-9


def same(a, b):
    assert a.keys() == b.keys()
    for k in a:
        if isinstance(a[k], np.ndarray):
            assert np.array_equal(a[k], b[k], equal_nan=a[k].dtype.kind in "fc"), k
        else:
            assert a[k] == b[k], k


def test_trace_many_matches_each_trace():
    # 多数の系（パラメータの変種）をまとめて追跡しても、系ごとの trace と同じビット
    s = lb.System.load(zmx("C0"), glass_dirs=GLASS)
    t0 = s.get_param("3.t")
    sys_ = [s.with_params({"3.t": t0 + 0.01 * i, "2.dx": 0.001 * i}) for i in range(7)]
    px, py = grid(12)
    kw = dict(record=True, globals=True, grad=["3.t", "2.dx"], land=True, dz=0.2)
    one = [q.trace(2, px, py, threads=1, **kw) for q in sys_]
    for th in (1, 2, 4, None):
        many = lb.trace_many(sys_, 2, px, py, threads=th, **kw)
        assert len(many) == len(sys_)
        for a, b in zip(one, many):
            same(a, b)
    # 瞳の種類・偏光
    many = lb.trace_many(sys_[:3], 1, px, py, pupil="signed")
    for a, q in zip(many, sys_):
        same(a, q.trace(1, px, py, pupil="signed"))
    s7 = lb.System.load(zmx("A7"), glass_dirs=GLASS)
    many = lb.trace_many([s7, s7], 3, px, py, pol=True, threads=4)
    same(many[0], s7.trace(3, px, py, pol=True, threads=1))
    same(many[1], many[0])
    assert lb.trace_many([], 1, px, py) == []
    with pytest.raises(ValueError):
        lb.trace_many([s], 1, px, py[:-1])
    with pytest.raises(ValueError):
        lb.trace_many([s], 1, px, py, grad=["3.zz"])


def test_cardinal_and_system_data():
    """基点: EFFL と一致し、set_param 後の系の値を返す。主点・焦点の関係も確かめる"""
    s = lb.System.load(zmx("A5"), glass_dirs=GLASS)
    c = s.cardinal()
    sd = s.system_data()
    assert c["efl"] == pytest.approx(sd["effl"], rel=1e-14)
    assert sd["effl"] == pytest.approx(52.53408803355001, rel=1e-9)  # OpticStudio の EFFL
    # 空気中: f′ = efl、F′ = P′ + f′、節点 = 主点
    assert c["f_image"] == pytest.approx(c["efl"] * c["n_image"], rel=1e-14)
    assert c["focal_image"] == pytest.approx(c["principal_image"] + c["f_image"], abs=1e-9)
    assert c["focal_object"] == pytest.approx(c["principal_object"] + c["f_object"], abs=1e-9)
    if c["n_image"] == c["n_object"]:
        assert c["nodal_image"] == pytest.approx(c["principal_image"], abs=1e-9)
    # 波長ごと
    c1 = s.cardinal(wave=1)
    assert c1["efl"] != c["efl"]
    with pytest.raises(IndexError):
        s.cardinal(wave=9)
    # パラメータ変更後
    t = s.with_params({"1.c": s.get_param("1.c") * 1.01})
    assert t.cardinal()["efl"] != pytest.approx(c["efl"], rel=1e-6)
    assert t.cardinal()["efl"] == pytest.approx(t.system_data()["effl"], rel=1e-14)


PICKUP_ZMX = """VERS 260327 42
MODE SEQ
UNIT MM X W X CM MR CPMM
ENPD 8
FTYP 0 0 2 1 0 0 0
XFLN 0 1
YFLN 0 2
FWGN 1 1
WAVM 1 0.5876 1
WAVM 2 0.6563 1
PWAV 1
COFN COATING.DAT
SURF 0
  TYPE STANDARD
  CURV 0.0
  DISZ INFINITY
SURF 1
  STOP
  TYPE STANDARD
  CURV 0.02
  COAT AR2
  DISZ 4
  GLAS ___BLANK 1 0 1.5168 0 0 0 0 0 0
SURF 2
  TYPE EVENASPH
  CURV -0.01
  CONI -0.5
  PARM 1 0
  PARM 2 1e-6
  DISZ 3
SURF 3
  TYPE COORDBRK
  PARM 1 0.2
  PARM 4 1.5
  DISZ 0
SURF 4
  TYPE STANDARD
  CURV 0.01 4 1 -1 0 ""
  PKUP 2 -1 0
  PZUP 1 1.5 0.2 0
  DISZ 0
  GLAS ___BLANK 1 0 1.6 0 0 0 0 0 0
SURF 5
  TYPE EVENASPH
  CURV -0.015
  PPAR 2 2 -1 0 0
  PZUP 4 0.5 1 0
  DISZ 0
SURF 6
  TYPE COORDBRK
  PPAR 1 3 -1 0 0
  PPAR 4 3 -1 0 0
  DISZ 20
SURF 7
  TYPE STANDARD
  CURV 0.0
  DISZ 15
  MAZH 0 0
SURF 8
  TYPE STANDARD
  CURV 0.0
  DISZ 0
"""

COATING_DAT = (
    "MATE L\n0.4 1.38 0\n1.2 1.38 0\nMATE H\n0.4 2.1 0\n1.2 2.1 0\n"
    "COAT AR2\nL 0.11 1\nH 0.06 1\n"
)


def same_result(a, b):
    assert a.keys() == b.keys()
    for k in a:
        x, y = a[k], b[k]
        if isinstance(x, np.ndarray):
            assert x.dtype == y.dtype and x.shape == y.shape, k
            assert np.array_equal(x.view(np.uint8), y.view(np.uint8)), k  # ビット一致（NaN も）
        else:
            assert x == y, k


def roundtrip_cases(tmp_path):
    (tmp_path / "COATING.DAT").write_text(COATING_DAT)
    (tmp_path / "p.zmx").write_text(PICKUP_ZMX)
    pk = lb.System.load(str(tmp_path / "p.zmx"), coating_file=str(tmp_path / "COATING.DAT"))
    pk = pk.with_params({"1.t": 4.3, "2.a4": 2e-6, "3.dx": 0.25})  # ピックアップ先も追従した系
    a7 = lb.System.load(zmx("A7"), glass_dirs=GLASS)  # 実光線エイミング
    c0 = lb.System.load(zmx("C0"), glass_dirs=GLASS).with_params({"3.t": 1.0})
    c0.set_field_type("paraxial_image_height", fields=[(0.0, 0.0), (0.0, 1.5)])
    z1 = lb.System.load(zmx("Z1"), glass_dirs=GLASS)  # Zernike Standard Sag
    g1 = lb.System.load(zmx("G1"), glass_dirs=GLASS)  # Grid Sag
    return [pk, a7, c0, z1, g1]


def test_pickle_roundtrip_is_bit_exact(tmp_path):
    """pickle: パラメータ変更・ピックアップ・膜・ガラス・照準・視野の種類を含めて、往復した系の追跡がビット一致"""
    import pickle

    px, py = grid(9)
    cases = roundtrip_cases(tmp_path)
    cases_with_solve = cases[0]
    for s in cases:
        t = pickle.loads(pickle.dumps(s))
        assert repr(t) == repr(s)
        assert t.get_param("1.t") == s.get_param("1.t")
        for f in (1, 2):
            same_result(s.trace(f, px, py, grad=["1.t"]), t.trace(f, px, py, grad=["1.t"]))
            same_result(s.trace(f, px, py, pol=True), t.trace(f, px, py, pol=True))
        assert t.cardinal() == s.cardinal()
        # 作り直した系も変えると、ピックアップ・厚さのソルブ（MAZH）を同じに解き直す
        if s is cases_with_solve:
            a = s.with_params({"1.c": 0.021})
            b = t.with_params({"1.c": 0.021})
            assert b.get_param("7.t") == a.get_param("7.t") != s.get_param("7.t")
            same_result(a.trace(1, px, py), b.trace(1, px, py))
        # 作り直した系も変更できる（元の系は変わらない）
        t.set_param("1.t", t.get_param("1.t") + 0.1)
        assert t.get_param("1.t") != s.get_param("1.t")
    with pytest.raises(ValueError):
        lb._system_from_state(b"\x00\x01", [])


def _trace_in_child(s):
    px, py = grid(9)
    return s.trace(2, px, py)


def test_pickle_across_processes(tmp_path):
    """別プロセス（spawn）に渡して追跡しても同じ結果"""
    import multiprocessing as mp

    cases = roundtrip_cases(tmp_path)
    with mp.get_context("spawn").Pool(2) as pool:
        got = pool.map(_trace_in_child, cases)
    for s, r in zip(cases, got):
        same_result(_trace_in_child(s), r)


RELAY_ZMX = """VERS 260327 42
MODE SEQ
UNIT MM X W X CM MR CPMM
OBNA 0.3 0
FTYP 0 0 1 1 0 0 0
YFLN 0
FWGN 1
WAVM 1 0.6 1
PWAV 1
SURF 0
  TYPE STANDARD
  CURV 0.0
  DISZ 50
SURF 1
  STOP
  TYPE PARAXIAL
  PARM 1 50
  PARM 2 1
  DISZ 100
SURF 2
  TYPE PARAXIAL
  PARM 1 50
  PARM 2 1
  DISZ 50
SURF 3
  TYPE STANDARD
  CURV 0.0
  DISZ 0
"""


def test_fiber_coupling(tmp_path):
    """ファイバ結合: 1:1 の 4f 系でガウスの式と比べる"""
    (tmp_path / "r.zmx").write_text(RELAY_ZMX)
    s = lb.System.load(str(tmp_path / "r.zmx"))
    c = s.fiber_coupling(0.1)
    assert c["eta"] == pytest.approx(1.0, abs=5e-5) and c["system"] == pytest.approx(1.0)
    a, b = 0.1, 0.05
    assert s.fiber_coupling(a, b)["eta"] == pytest.approx((2 * a * b / (a * a + b * b)) ** 2, abs=5e-5)
    # 受け側の横ずれ: exp(−Δ²/w0²)、w0 = λ/(π NA)
    w0 = 0.6e-3 / (np.pi * 0.1)
    e = s.fiber_coupling(0.1, wave=1, receiver=(0.001, 0.0))["eta"]
    assert e == pytest.approx(np.exp(-((0.001 / w0) ** 2)), abs=5e-5)
    p = s.fiber_coupling(0.1, pol=True)
    j = p["jones"]
    assert j.shape == (2, 2) and j.dtype == np.complex128
    assert abs(j[0, 1]) < 1e-6 and abs(abs(j[0, 0]) - 1) < 5e-3
    assert np.sum(np.abs(j[:, 0]) ** 2) == pytest.approx(p["eta"], abs=6e-3)
    with pytest.raises(ValueError):
        s.fiber_coupling(0.1, grid=100)
    with pytest.raises(IndexError):
        s.fiber_coupling(0.1, wave=2)
    with pytest.raises(ValueError):
        lb.System.load(zmx("A0"), glass_dirs=GLASS).fiber_coupling(0.1)  # 無限遠物体


def multi_config_a0(tmp_path):
    # A0 の写しに 3 つの構成を足す（2: 面 4 の厚さ、3: 入射瞳径を半分 = 絞りの切替）
    txt = Path(zmx("A0")).read_bytes().decode("utf-16")
    txt = txt[: txt.index("MNUM")]
    mce = (
        "MNUM 3 1\n"
        "THIC 4 1 2.3 0 0 0 1 1 0\nTHIC 4 2 2.8 0 0 0 1 1 0\nTHIC 4 3 2.3 0 0 0 1 1 0\n"
        "APER 0 1 10 0 0 0 1 1 0\nAPER 0 2 10 0 0 0 1 1 0\nAPER 0 3 5 0 0 0 1 1 0\n"
    )
    p = tmp_path / "mc.zmx"
    p.write_text(txt + mce, encoding="utf-8")
    return str(p)


def test_load_configs_and_trace_them_together(tmp_path):
    # 構成をまとめて読み、同じ物体・同じ瞳の格子で一度に追跡する
    p = multi_config_a0(tmp_path)
    cs = lb.System.load_configs(p, glass_dirs=GLASS)
    assert len(cs) == 3
    two = lb.System.load_configs(p, configs=[3, 1], glass_dirs=GLASS)
    px, py = grid(10)
    obj = (0.01, -0.02, 1.0)
    many = lb.trace_many(cs, None, px, py, object=obj, grad=["4.t"])
    for k, (c, m) in enumerate(zip(cs, many), start=1):
        one = lb.System.load(p, glass_dirs=GLASS, cfg=k)
        same(m, one.trace(None, px, py, object=obj, grad=["4.t"]))
        same(m, c.trace(None, px, py, object=obj, grad=["4.t"]))
    same(lb.trace_many(two, None, px, py, object=obj)[0], cs[2].trace(None, px, py, object=obj))
    assert cs[1].get_param("4.t") == 2.8 and cs[0].get_param("4.t") == 2.3
    # 入射瞳径が半分 → 瞳の縁の光線の高さが半分程度
    assert not np.array_equal(many[0]["x"], many[2]["x"])
    with pytest.raises(ValueError):
        lb.System.load_configs(p, configs=[4], glass_dirs=GLASS)
    with pytest.raises(ValueError):
        lb.System.load_configs(p, configs=[0], glass_dirs=GLASS)
    # 空の list は全部ではなくエラー
    with pytest.raises(ValueError, match="空"):
        lb.System.load_configs(p, configs=[], glass_dirs=GLASS)


def test_pol_grid():
    """偏光の粗い格子＋補間: 全光線の偏光追跡に近く、位相・NaN の約束も同じ"""
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    px, py = grid(41)
    a = s.trace(4, px, py, wavelengths=[2], pol=2)
    b = s.trace(4, px, py, wavelengths=[2], pol=2, pol_grid=25)
    assert b["pol_grid"] == (25, 25)
    assert b["pol_grid_missing"].shape == (0, 2) and not b["pol_grid_bad"].any()
    assert np.abs(a["jones"] - b["jones"]).max() < 1e-6
    assert np.abs(a["jones_z"] - b["jones_z"]).max() < 1e-6
    # 格子点の上では追跡の値そのもの
    g = -1 + 2 * np.arange(25) / 24
    gx, gy = np.meshgrid(g, g)
    m = gx**2 + gy**2 <= 1
    c = s.trace(4, gx[m], gy[m], wavelengths=[2], pol=2)
    d = s.trace(4, gx[m], gy[m], wavelengths=[2], pol=2, pol_grid=25)
    assert np.allclose(c["jones"], d["jones"], rtol=0, atol=1e-14)
    # 伝搬の位相は全光線の光路長で
    p = s.trace(4, px, py, wavelengths=[2], pol=2, pol_phase=True, pol_grid=25)["jones"]
    k = 2 * np.pi / (s.wavelengths[1] * 1e-3)
    ph = np.exp(-1j * k * a["opl"][0])[:, None, None]
    assert np.allclose(p, b["jones"] * ph, rtol=1e-12, atol=1e-14)
    # 偏光の波長が wavelengths に無くても、dz があっても同じ
    e = s.trace(4, px, py, wavelengths=[1], pol=2, pol_grid=25)
    assert np.array_equal(e["jones"], b["jones"])
    f = s.trace(4, px, py, wavelengths=[2], pol=2, pol_grid=25, dz=0.1)
    assert np.array_equal(f["jones"], b["jones"])
    # 止まった光線は NaN のまま
    q = s.trace(4, np.array([0.0, 5.0]), np.array([0.0, 0.0]), pol=True, pol_grid=25)
    r = s.trace(4, np.array([0.0, 5.0]), np.array([0.0, 0.0]), pol=True)
    assert np.array_equal(np.isnan(q["jones"]), np.isnan(r["jones"]))
    with pytest.raises(ValueError):
        s.trace(4, px, py, pol_grid=25)
    with pytest.raises(ValueError):
        s.trace(4, px, py, pol=True, pol_grid=3)
    with pytest.raises(ValueError):
        s.trace(4, px, py, pol=True, pol_grid=25, pupil="stop")


def test_pol_grid_missing(tmp_path):
    """瞳の内側で欠けた格子点は黙って埋めずに返す（絞りに中心遮蔽を付けて作る）"""
    s = lb.System.load(zmx("A0"), glass_dirs=GLASS)
    stop = s.stop
    r0 = s.trace(1, np.array([0.0]), np.array([0.3]), last=stop)["y"][0, 0]
    text = Path(zmx("A0")).read_text(encoding="utf-16")
    lines = text.splitlines()
    k = [i for i, ln in enumerate(lines) if ln.strip() == "STOP"]
    assert len(k) == 1
    lines.insert(k[0] + 1, f"  OBSC 0 {abs(r0)} 0 0 0 0")
    p = tmp_path / "obsc.zmx"
    p.write_text("\r\n".join(lines) + "\r\n", encoding="utf-16")
    o = lb.System.load(str(p), glass_dirs=GLASS)
    px, py = grid(41)
    b = o.trace(1, px, py, pol=True, pol_grid=25)
    m = b["pol_grid_missing"]
    assert len(m) > 0 and np.all(np.hypot(m[:, 0], m[:, 1]) < 0.35)
    bad = b["pol_grid_bad"]
    ok = b["ok"][list(b["wavelengths"]).index(b["jones_wavelength"])]
    assert bad.any() and not (bad & ~ok).any() and np.all(np.isnan(b["jones"][bad]))
    good = ok & ~bad
    assert good.sum() > 0.5 * len(px) and np.all(np.isfinite(b["jones"][good]))
    a = o.trace(1, px, py, pol=True)
    assert np.abs(a["jones"][good] - b["jones"][good]).max() < 1e-5


def test_coatings_reload_after_edit(tmp_path):
    """膜を変える編集の後の偏光追跡は、変えた後の系で膜を読み直す（フォルダの .lbcs は使う膜だけ読む）"""
    (tmp_path / "COATING.DAT").write_text(COATING_DAT)
    (tmp_path / "X.lbcs").write_text("MATE L\n0.4 1.38 0\n1.2 1.38 0\nCOAT X\nL 0.25 1\n")
    (tmp_path / "p.zmx").write_text(PICKUP_ZMX)
    s = lb.System.load(str(tmp_path / "p.zmx"), coating_file=str(tmp_path / "COATING.DAT"))
    a = s.trace(1, [0.0], [0.5], pol=True)
    s.set_surface(1, coating="X")
    b = s.trace(1, [0.0], [0.5], pol=True)
    # 膜 X（AR2 と違う 1 層）で透過が変わる
    assert not np.array_equal(a["jones"], b["jones"])
