"""解析の登録簿を Python から使う（System.analyze・analyses・defaults）と、Python の関数の既定値が設定の型の
既定値（CLI・serve・GUI と同じ 1 か所）と合うこと（XCT-02・ANA-03）。"""

import inspect
from pathlib import Path

import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]


def system():
    return lb.System.load(str(ROOT / "tests" / "refs" / "A0" / "system.zmx"), glass_dirs=GLASS)


# Python の関数 → (登録簿の解析か "optimize"・"zoom", {Python の引数: 設定の鍵})
SAME_DEFAULTS = {
    lb.System.psf: ("psf", {"pupil": "pupil", "image": "image", "delta": "delta"}),
    lb.System.huygens_psf: ("psf", {"pupil": "pupil", "image": "image", "delta": "delta"}),
    lb.System.huygens_mtf: ("huygens-mtf", {"pupil": "pupil", "image": "image", "delta": "delta"}),
    lb.System.geometric_mtf: ("geo-mtf", {"n": "n", "diff_limit": "diff_limit"}),
    lb.System.mtf: ("mtf", {"n": "n", "method": "method", "each_wave": "each_wave"}),
    lb.System.through_focus_mtf: ("mtf-focus", {"n": "n"}),
    lb.System.field_mtf: ("mtf-field", {"axis": "axis", "n": "n"}),
    lb.System.rms: ("rms", {"axis": "axis", "reference": "reference", "rings": "rings", "field_wave": "field_wave"}),
    lb.System.mech_check: ("mech-check", {"grid": "grid"}),
    lb.System.optimize: ("optimize", {"reference": "reference", "rings": "rings", "iters": "iters", "jac": "jac",
                                      "qr": "qr", "gtol": "gtol", "active_set": "active_set"}),
    lb.zoom: ("zoom", {"steps": "steps", "merit": "merit", "reference": "reference", "rings": "rings",
                       "iters": "iters", "tol": "tol", "method": "method", "qr": "qr", "gtol": "gtol",
                       "active_set": "active_set"}),
}


@pytest.mark.parametrize("func", list(SAME_DEFAULTS), ids=lambda f: f.__name__)
def test_python_defaults_are_the_definitions(func):
    name, keys = SAME_DEFAULTS[func]
    d = lb.defaults(name)
    sig = inspect.signature(func)
    for arg, key in keys.items():
        assert sig.parameters[arg].default == d[key], f"{func.__name__}({arg}) vs {name}.{key}"


def test_analyses_lists_settings():
    a = {x["name"]: x for x in lb.analyses()}
    for n in ("spot", "psf", "mtf", "rms", "wavefront", "rays", "info"):
        assert n in a
    p = {x["name"]: x for x in a["psf"]["params"]}
    assert p["image"]["default"] == 128 and p["image"]["type"] == "count"
    with pytest.raises(ValueError, match="no analysis"):
        lb.defaults("no-such")


def test_analyze_by_name_matches_the_method():
    s = system()
    r = s.analyze("psf", field=1, image=32)
    m = s.psf(1, image=32)
    assert r["sampling"][0]["peak"] == pytest.approx(m["peak"], rel=1e-12)
    # 省いた鍵は既定値、知らない鍵は誤り
    assert s.analyze("spot", field=1)["unit"] == "mm"
    with pytest.raises(Exception, match="unknown field"):
        s.analyze("spot", ring=3)
    with pytest.raises(ValueError, match="no analysis"):
        s.analyze("no-such")
