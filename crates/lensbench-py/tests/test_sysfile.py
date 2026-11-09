"""系ファイル: 保存・読込・hash・pickle。"""

import json
import pickle
from pathlib import Path

import numpy as np

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]
REFS = ROOT / "tests" / "refs"


def rays(s):
    px = np.linspace(-0.9, 0.9, 7)
    r = s.trace(2, px, px[::-1])
    return np.concatenate([r[k].ravel() for k in ("x", "y", "z", "l", "m", "n", "opl")])


def test_save_load_edited(tmp_path):
    s = lb.System.load(str(REFS / "A0" / "system.zmx"), glass_dirs=GLASS)
    h0 = s.hash
    assert h0.startswith("sha256:")
    s.set_param("2.dn", 1e-4)
    s.insert_surface(1, t=0.0, comment="dummy")
    j = s.image - 1
    s.set_config_count(2)
    t = s.get_param(f"{j}.t")
    s.set_config_row(f"{j}.t", [t, t + 0.5])
    assert s.hash != h0
    p = tmp_path / "a0.json"
    s.save(str(p))
    d = json.loads(p.read_text(encoding="utf-8"))
    assert d["format"] == "lensbench-system" and d["hash"] == s.hash
    assert d["lensbench"] == lb.__version__
    r = lb.System.load(str(p))
    assert r.hash == s.hash
    assert np.array_equal(rays(r), rays(s), equal_nan=True)
    assert np.array_equal(rays(r.config(2)), rays(s.config(2)), equal_nan=True)
    assert r.surface("dummy")["t"] == 0.0
    # 文字列でも
    q = lb.System.from_json(s.to_json())
    assert q.hash == s.hash


def test_pickle_is_system_file():
    s = lb.System.load(str(REFS / "B0" / "system.zmx"), glass_dirs=GLASS)
    s.set_param("3.t", s.get_param("3.t") + 0.1)
    b = pickle.dumps(s)
    assert b"lensbench-system" in b
    r = pickle.loads(b)
    assert r.hash == s.hash
    assert np.array_equal(rays(r), rays(s), equal_nan=True)
