"""Python から系を編集する: 面の挿入・削除・種類・材料、ピックアップ・ソルブ、多重構成の表。"""

import warnings
from pathlib import Path

import numpy as np
import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]
REFS = ROOT / "tests" / "refs"

EMPTY = {
    "glass_catalogs": ["SCHOTT"],
    "wavelengths": [{"um": 0.4861327}, {"um": 0.5875618}, {"um": 0.6562725}],
    "primary": 2,
    "aperture": {"type": "epd", "value": 10},
    "fields": {"type": "angle", "points": [[0, 0], [0, 3]]},
    "blocks": [{"type": "stop"}, {"type": "air", "t": 10}],
}


def empty():
    return lb.System.from_project(EMPTY, glass_dirs=GLASS)


def image_y(s, py=0.5):
    r = s.trace(1, np.array([0.0]), np.array([py]))
    return float(r["y"][1, 0])


def test_build_singlet_and_solve():
    s = empty()
    n0 = s.surface_count
    st = s.stop
    s.insert_surface(st + 1, r=50.0, t=5.0, material="N-BK7", semi=8.0, comment="L1 front")
    s.insert_surface(st + 2, r=-50.0, t=40.0, comment="L1 back")
    assert s.surface_count == n0 + 2
    d = s.surface("L1 front")
    assert d["type"] == "STANDARD" and d["material"] == "N-BK7"
    assert abs(d["n"][1] - 1.5168) < 1e-3
    assert s.surface("L1 back")["n"] == [1.0, 1.0, 1.0]
    # 像面の手前の厚さを近軸の焦点に解く
    back = s.find_surface("L1 back")
    last = s.image - 1
    if last != back:
        s.set_surface(last, t=0.0)
    s.add_solve(back, value=0.0)
    assert s.solves()[0]["surf"] == back and s.solves()[0]["kind"] == "marginal_height"
    assert abs(image_y(s, 0.1)) < 1e-3
    with pytest.raises(ValueError):
        s.set_param(f"{back}.t", 30.0)
    s.remove_solve(back)
    s.set_param(f"{back}.t", 30.0)


def test_pickup_and_delete():
    s = empty()
    st = s.stop
    s.insert_surface(st + 1, c=0.02, t=5.0, material="1.5,0")
    s.insert_surface(st + 2, t=40.0)
    s.add_pickup(st + 2, st + 1, "c", scale=-1.0)
    assert s.surface(st + 2)["c"] == -0.02
    s.set_param(f"{st + 1}.c", 0.03)
    assert s.surface(st + 2)["c"] == -0.03
    assert s.pickups() == [
        {"dst": st + 2, "src": st + 1, "what": "c", "scale": -1.0, "offset": 0.0}
    ]
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always")
        notes = s.delete_surface(st + 1)
    assert len(notes) == 1 and len(w) == 1
    assert s.pickups() == []
    with pytest.raises(ValueError):
        s.delete_surface(s.stop)


def test_type_material_aperture():
    s = empty()
    st = s.stop
    s.insert_surface(st + 1, t=5.0, material="1.5,0")
    s.insert_surface(st + 2, type="COORDBRK", cb=[0.0, 0.5, 0.0, 0.0, 0.0])
    d = s.surface(st + 2)
    assert d["type"] == "COORDBRK" and d["n"] == [1.5, 1.5, 1.5]
    assert d["cb"][1] == 0.5
    s.set_surface_type(st + 2, "standard")
    assert s.surface(st + 2)["n"] == [1.0, 1.0, 1.0]
    s.set_material(st + 2, "MIRROR")
    assert s.surface(st + 2)["mirror"]
    s.set_material(st + 2, "")
    s.set_surface(st + 1, aperture={"type": "circular", "max": 2.0})
    assert s.surface(st + 1)["aperture"] == {
        "type": "circular", "min": 0.0, "max": 2.0, "dx": 0.0, "dy": 0.0
    }
    r = s.trace(1, np.array([0.0, 0.0]), np.array([0.0, 1.0]))
    assert r["ok"][1, 0] and not r["ok"][1, 1]
    s.set_surface(st + 1, aperture=None)
    assert s.surface(st + 1)["aperture"] is None
    with pytest.raises(ValueError):
        s.set_material(st + 1, "NO-SUCH-GLASS")
    # 失敗しても系は元のまま
    assert s.surface(st + 1)["material"] == "1.5,0"


def test_glass_catalog():
    s = empty()
    st = s.stop
    s.insert_surface(st + 1, t=5.0)
    assert s.add_glass_catalogs(["NOPE"], dirs=GLASS) == ["NOPE"]
    s.set_material(st + 1, "N-BK7")


def test_configs_and_original_unchanged():
    s = lb.System.load(str(REFS / "A0" / "system.zmx"), glass_dirs=GLASS)
    base = s.trace(2, np.array([0.0]), np.array([0.7]))
    t = s.with_params({})
    j = t.image - 1
    th = t.get_param(f"{j}.t")
    t.set_config_count(2)
    t.set_config_row(f"{j}.t", [th, th + 1.0])
    assert t.config_count == 2 and t.configs() == [(f"{j}.t", [th, th + 1.0])]
    c2 = t.config(2)
    assert c2.active_config == 2 and c2.get_param(f"{j}.t") == th + 1.0
    t.set_config(2)
    assert t.get_param(f"{j}.t") == th + 1.0
    t.set_config(1)
    r = t.trace(2, np.array([0.0]), np.array([0.7]))
    assert np.array_equal(r["x"], base["x"]) and np.array_equal(r["y"], base["y"])
    # 前に面を入れると表の面番号が付け替わる
    t.insert_surface(1, t=0.0)
    assert t.configs() == [(f"{j + 1}.t", [th, th + 1.0])]
    # 元の系は変わらない
    assert s.config_count == 1 and s.surface_count + 1 == t.surface_count
    t.set_config_count(1)
    assert t.configs() == []


def test_curvature_solve():
    s = empty()
    st = s.stop
    s.insert_surface(st + 1, c=0.02, t=5.0, material="1.5,0")
    s.insert_surface(st + 2, t=40.0)
    s.add_solve(st + 2, kind="concentric_surface", other=st + 1)
    assert abs(s.surface(st + 2)["c"] + 0.2) < 1e-12
    q = s.solves()[0]
    assert (q["what"], q["kind"], q["other"]) == ("c", "concentric_surface", st + 1)
    with pytest.raises(ValueError):
        s.set_surface(st + 2, c=0.1)
    s.insert_surface(1, t=0.0)
    assert s.solves()[0]["other"] == st + 2
    s.remove_solve(st + 3, what="c")
    assert s.solves() == []
    with pytest.raises(ValueError):
        s.add_solve(st + 2, kind="nope")
