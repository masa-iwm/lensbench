"""プラグインの Python の口のテスト: 同梱の解析を名前で呼ぶ、Python の関数の登録、一括の呼び出し。"""

from pathlib import Path

import numpy as np
import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]


def a0():
    return lb.System.load(str(ROOT / "tests" / "refs" / "A0" / "system.zmx"), glass_dirs=GLASS)


def test_builtin_analysis_listed():
    assert ("rms", "analysis") in lb.plugins()
    s = lb.plugin_spec("rms")
    assert s["kind"] == "analysis" and s["needs_system"]
    assert "field" in [k for k, _ in s["inputs"]]
    # 設定の項目は型・既定つき
    cfg = {p["name"]: p for p in s["config"]}
    assert cfg["rings"]["type"] == "count" and cfg["rings"]["default"] == 6
    assert cfg["reference"]["choices"] == ["centroid", "chief"]
    assert lb.plugin_spec("wavefront")["config"][2]["default"] is False
    # 出力は名前・型・形・単位つき
    out = {o["name"]: o for o in lb.plugin_spec("psf")["outputs"]}
    assert out["i"]["dtype"] == "float64" and out["i"]["shape"] == "(image, image)"
    assert out["center"]["unit"] == "mm"
    # 真偽は True・False（0・1 も）
    w = lb.Plugin("wavefront", ref_primary=True, n=16).run(system=a0(), field=1)
    w0 = lb.Plugin("wavefront", ref_primary=1, n=16).run(system=a0(), field=1)
    assert w["rms"] == w0["rms"]


def test_run_many_ordered_and_thread_independent():
    s = a0()
    inputs = [dict(field=[0.0, 0.1 * k]) for k in range(10)]
    p = lb.Plugin("rms", rings=4)
    a = p.run_many(inputs, system=s, threads=1)
    b = p.run_many(inputs, system=s, threads=4)
    for x, y, inp in zip(a, b, inputs):
        np.testing.assert_array_equal(x["wfe"], y["wfe"])
        np.testing.assert_array_equal(x["wfe"], p.run(system=s, **inp)["wfe"])


def test_python_callback_plugin():
    def tilt(config, inputs):
        """平均と、重みつきの和"""
        w = inputs["mask"].astype(float) * config["k"]
        return {"mean": inputs["img"].mean(axis=(1, 2)), "s": np.sum(w), "ok": inputs["mask"]}

    lb.register_plugin("test_tilt", tilt)
    try:
        assert ("test_tilt", "analysis") in lb.plugins()
        assert lb.plugin_spec("test_tilt")["doc"].startswith("平均")
        p = lb.Plugin("test_tilt", k=2.0, mode="x", deg=[1, 2])
        img = np.arange(4 * 3 * 3, dtype=float).reshape(4, 3, 3)
        mask = np.ones((3, 3), bool)
        out = p.run(img=img, mask=mask)
        np.testing.assert_array_equal(out["mean"], img.mean(axis=(1, 2)))
        assert out["s"].shape == () and float(out["s"]) == 18.0
        assert out["ok"].dtype == bool
        many = p.run_many([dict(img=img, mask=mask)] * 5, threads=3)
        assert len(many) == 5 and all(float(m["s"]) == 18.0 for m in many)
        # 関数の中の誤り・dict 以外の戻り値は ValueError
        with pytest.raises(ValueError):
            p.run(img=img)
        lb.register_plugin("test_tilt", lambda c, i: 1.0)
        with pytest.raises(ValueError):
            lb.Plugin("test_tilt").run()
    finally:
        assert lb.unregister_plugin("test_tilt")
    assert ("test_tilt", "analysis") not in lb.plugins()


def test_python_plugin_with_system():
    """系を受け取って中で追跡する形（MTF などと同じ呼ばれ方）"""
    s = lb.System.load(str(ROOT / "tests" / "refs" / "A0" / "system.zmx"), glass_dirs=GLASS)

    def chief_y(config, inputs, system=None):
        if system is None:
            raise ValueError("系が要る")
        r = system.trace(field=int(inputs["field"]), px=[0.0], py=[0.0])
        return {"y": np.asarray(r["y"])[-1:]}

    lb.register_plugin("test_chief", chief_y, needs_system=True)
    try:
        assert lb.plugin_spec("test_chief")["needs_system"]
        p = lb.Plugin("test_chief")
        with pytest.raises(ValueError):
            p.run(field=1)
        want = np.asarray(s.trace(field=2, px=[0.0], py=[0.0])["y"])[-1:]
        np.testing.assert_array_equal(p.run(system=s, field=2)["y"], want)
        outs = p.run_many([dict(field=1), dict(field=2)], system=s, threads=2)
        np.testing.assert_array_equal(outs[1]["y"], want)
    finally:
        lb.unregister_plugin("test_chief")


def test_errors():
    with pytest.raises(ValueError):
        lb.Plugin("no_such_plugin")
    with pytest.raises(ValueError):
        lb.Plugin("rms", ring=4)  # 綴りの誤り
    with pytest.raises(ValueError):
        lb.Plugin("rms", rings=True)  # 整数の項目に真偽
    with pytest.raises(TypeError):
        lb.Plugin("rms", rings={"a": 1})
    with pytest.raises(ValueError):
        lb.register_plugin("x", print, kind="surface")
    p = lb.Plugin("rms")
    with pytest.raises(ValueError):
        p.run(system=a0())  # field が無い
    with pytest.raises(ValueError):
        p.run_many([dict(field=1)])  # 系が無い


def test_builtin_analysis_plugins_match_methods():
    """同梱の解析（mtf・psf）は System のメソッドと同じビット。視野をまとめて呼べる"""
    s = lb.System.load(str(ROOT / "tests" / "refs" / "A0" / "system.zmx"), glass_dirs=GLASS)
    names = [n for n, k in lb.plugins() if k == "analysis"]
    assert {"mtf", "psf", "spot", "wavefront", "zernike", "rms", "geo_mtf", "pop", "ghost"} <= set(names)
    fs = [0.0, 10.0, 30.0]
    want = s.mtf(2, fs, method="fft")
    p = lb.Plugin("mtf", freqs=fs, method="fft")
    got = p.run(system=s, field=2)
    np.testing.assert_array_equal(got["tan"], want["tangential"])
    np.testing.assert_array_equal(got["sag"], want["sagittal"])
    outs = p.run_many([dict(field=1), dict(field=2), dict(field=[0.0, 0.5])], system=s, threads=3)
    np.testing.assert_array_equal(outs[1]["tan"], want["tangential"])
    want = s.psf(1, pupil=32, image=64)
    got = lb.Plugin("psf", pupil=32, image=64).run(system=s, field=1)
    np.testing.assert_array_equal(got["i"], want["i"])
    # ゴーストは視野を使わない（入力なし）
    g = lb.Plugin("ghost").run(system=s)
    assert g["a"].shape == g["intensity"].shape and len(g["a"]) > 0


def test_registered_coating_catalog(tmp_path):
    """Python から登録した膜の一覧を、面の膜の名前で引く（読む前に登録する。同じ名前の膜が別の一覧にあれば誤り）"""
    def traced():
        s = a0()
        s.set_surface(1, coating="PY_EXT_AR")
        return s.trace(1, [0.0], [0.5], pol=True)

    with pytest.raises(ValueError):
        traced()
    (tmp_path / "x.dat").write_text("MATE PYL\n0.4 1.38 0\n1.2 1.38 0\n")
    try:
        n = lb.register_coating_catalog("PYTEST", files=[str(tmp_path / "x.dat")], dat="COAT PY_EXT_AR\nPYL 0.25\n")
        assert n == 1
        assert lb.coating_catalogs()["PYTEST"] == ["PY_EXT_AR"]
        assert np.all(np.isfinite(traced()["jones"]))
        # 膜ファイルにも同じ名前の膜があれば、一覧の膜を使い注意を Python の warnings に出す
        (tmp_path / "COATING.DAT").write_text("MATE PYL\n0.4 1.38 0\n1.2 1.38 0\nCOAT PY_EXT_AR\nPYL 0.1\n")
        s = lb.System.load(str(ROOT / "tests" / "refs" / "A0" / "system.zmx"), glass_dirs=GLASS,
                           coating_file=str(tmp_path / "COATING.DAT"))
        s.set_surface(1, coating="PY_EXT_AR")
        with pytest.warns(UserWarning, match="PY_EXT_AR"):
            s.trace(1, [0.0], [0.5], pol=True)
        with pytest.warns(UserWarning, match="PY_EXT_AR"):
            lb.Plugin("ghost").run(system=s)
        with pytest.raises(ValueError):
            lb.register_coating_catalog("OTHER", dat="COAT PY_EXT_AR\nPYL 0.1\n")
    finally:
        assert lb.unregister_coating_catalog("PYTEST")
    assert "PYTEST" not in lb.coating_catalogs()
    with pytest.raises(ValueError):
        traced()


def test_source_models_without_an_extension():
    # 光源は拡張が足す（同梱の光源は無い）。知らない名前・frame は誤り
    s = a0()
    assert lb.source_models() == {}
    with pytest.raises(lb.InvalidError, match="no user-defined source model"):
        lb.source_rays(s, "nothing")
    with pytest.raises(ValueError, match="frame"):
        lb.source_rays(s, "nothing", frame="pupil")


def test_python_source_traces_like_trace_rays():
    """Python の関数の光源: 波長ごとに 1 回呼び、物体の座標系の光線を trace_rays と同じに追う"""
    calls = []

    def fan(config, wl):
        """fan of rays"""
        calls.append(wl)
        n = int(config.get("n", 5))
        k = np.zeros((n, 3))
        k[:, 1] = np.linspace(-0.05, 0.05, n)
        k[:, 2] = 1.0
        return {"start": np.zeros((n, 3)), "direction": k, "weight": np.ones(n)}

    s = a0()
    lb.register_source("py_fan", fan)
    try:
        assert lb.source_models()["py_fan"] == "fan of rays"
        r = lb.source_rays(s, "py_fan", {"n": 7}, wave=1)
        assert r["start"].shape == (7, 3) and r["weight"].shape == (7,)
        assert len(calls) == 1 and calls[0] == pytest.approx(s.wavelengths[0])
        t = s.trace_rays(r["start"], r["direction"], waves=[1])
        u = s.trace_rays(np.zeros((7, 3)), r["direction"], waves=[1])
        assert np.array_equal(t["x"], u["x"])
        g = lb.source_rays(s, "py_fan", {"n": 7}, wave=1, frame="global")
        assert np.array_equal(s.trace_rays(g["start"], g["direction"], waves=[1], frame="global")["x"], t["x"])
        lb.register_source("py_bad", lambda c, wl: {"start": np.zeros((2, 3)), "direction": np.zeros((2, 3)), "weight": np.ones(2)})
        with pytest.raises(lb.InvalidError, match="not finite or is zero"):
            lb.source_rays(s, "py_bad")
    finally:
        assert lb.unregister_source("py_fan")
        lb.unregister_source("py_bad")
    assert "py_fan" not in lb.source_models()


def test_python_coating_model():
    """係数を Python の関数で計算する膜: 境界の条件の組ごとに cos1 の格子で 1 回呼ぶ。T = 50% の膜の振幅は T = 100% の膜の √0.5 倍"""
    calls = []

    def nd(t):
        def f(wl, n1, n2, cos1, path, from_substrate):
            calls.append(path)
            z = np.zeros_like(cos1, dtype=complex)
            return {"rs": z, "rp": z, "ts": np.sqrt(t) + z, "tp": np.sqrt(t)}
        return f

    def traced(coat):
        s = a0()
        s.set_surface(1, coating=coat)
        return s.trace(1, [0.0, 0.3], [0.5, -0.2], pol=True)

    try:
        n = lb.register_coating_catalog("PYMODELS", models={"PY_ND50": nd(0.5), "PY_T1": nd(1.0)})
        assert n == 2
        a = traced("PY_ND50")["jones"]
        b = traced("PY_T1")["jones"]
        assert np.allclose(a, np.sqrt(0.5) * b, rtol=1e-12, atol=1e-15)
        k = len(calls)
        traced("PY_ND50")
        assert len(calls) == k  # 表を覚えている
        with pytest.raises(TypeError):
            lb.register_coating_catalog("PYBAD", models={"X": 1})
    finally:
        lb.unregister_coating_catalog("PYMODELS")


def test_python_shape_traces_like_the_sphere():
    """Python の関数の形: 平面の土台に球の矢高を足した USERSHAPE は、同じ曲率の球面とほぼ同じに追う（格子の補間の誤差）"""
    import json

    def sphere(config, x, y):
        """sphere sag"""
        c = config["c"]
        r2 = x * x + y * y
        q = np.sqrt(1 - c * c * r2)
        z = c * r2 / (1 + q)
        return z, c * x / q, c * y / q

    s = a0()
    j = json.loads(s.to_json())
    c = j["system"]["surfaces"][1]["shape"]["conic"]["c"]
    lb.register_shape("py_sphere", sphere, params={"c": 0.0})
    try:
        j["system"]["surfaces"][1]["shape"] = {"user": {"c": 0.0, "k": 0.0, "model": "py_sphere",
                                                        "config": {"c": c, "radius": 8.0, "grid": 257}}}
        j.pop("hash", None)
        u = lb.System.from_json(json.dumps(j), glass_dirs=GLASS)
        # 同じ始点・方向（瞳の位置は近軸の曲率の差分で少し動くので、瞳座標でなく光線で比べる）
        start = np.array([[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [1.5, -3.0, 0.0]])
        k = np.array([[0.0, 0.0, 1.0], [0.0, -0.05, 1.0], [0.02, 0.03, 1.0]])
        a, b = s.trace_rays(start, k), u.trace_rays(start, k)
        for key in ("x", "y", "l", "m"):
            assert np.allclose(a[key], b[key], atol=1e-8), key
        with pytest.raises(ValueError):
            lb.register_shape("py_bad", sphere, params={"radius": 1.0})
    finally:
        assert lb.unregister_shape("py_sphere")
