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
    with pytest.raises(TypeError):
        lb.Plugin("rms", rings=True)
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
