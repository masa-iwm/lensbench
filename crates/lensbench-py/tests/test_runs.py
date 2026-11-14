"""一括実行: 抽選・掃引、スレッド並列、逐次書き出しと再開、進み具合。"""

import json
from pathlib import Path

import numpy as np
import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
GLASS = [str(ROOT / "tests" / "glass")]
REFS = ROOT / "tests" / "refs"


def a0():
    return lb.System.load(str(REFS / "A0" / "system.zmx"), glass_dirs=GLASS)


def spot(i, s, seed):
    r = s.trace(2, np.linspace(-0.9, 0.9, 9), np.zeros(9))
    return {"i": i, "seed": seed, "x": float(np.nanmean(r["x"][1])), "t": s.get_param("3.t")}


def test_sampler_reproducible():
    s = a0()
    t0 = s.get_param("3.t")
    p = lb.Sampler(s, {"3.t": 0.02, "4.dx": ("uniform", -0.01, 0.01)}, seed=7)
    assert p.draw(3) == p.draw(3) and p.draw(3) != p.draw(4)
    d = p.draw(5)
    assert -0.01 <= d["4.dx"] - s.get_param("4.dx") <= 0.01
    assert p.system(5).get_param("3.t") == d["3.t"]
    v = np.array([p.draw(i)["3.t"] - t0 for i in range(4000)])
    assert abs(v.std() - 0.02) < 0.002
    with pytest.raises(ValueError):
        lb.Sampler(s, {"3.t": ("nope", 1)})


def test_run_threads_resume(tmp_path):
    s = a0()
    p = lb.Sampler(s, {"3.t": 0.05}, seed=3)
    seq = lb.run_batch(spot, p, n=24, threads=1, progress=False)
    assert [r["i"] for r in seq] == list(range(24))
    out = tmp_path / "mc.jsonl"
    seen = []

    def stop_at_10(i, s, seed):
        if i == 10:
            raise RuntimeError("止める")
        return spot(i, s, seed)

    with pytest.raises(RuntimeError):
        lb.run_batch(stop_at_10, p, n=24, out=str(out), threads=1, progress=False)
    head, rows = lb.read_batch(str(out))
    assert head["seed"] == 3 and head["system_hash"] == s.hash
    assert len(rows) == 10

    def counted(i, s, seed):
        seen.append(i)
        return spot(i, s, seed)

    prog = []
    par = lb.run_batch(counted, p, n=24, out=str(out), threads=4, progress=prog.append)
    assert sorted(seen) == list(range(10, 24))
    assert prog and prog[-1]["done"] == 24 and prog[-1]["total"] == 24
    # スレッド数・再開によらず同じ結果
    assert par == seq
    # 別の種のファイルには続けない
    with pytest.raises(ValueError):
        lb.run_batch(spot, lb.Sampler(s, {"3.t": 0.05}, seed=4), n=24, out=str(out), progress=False)


def test_grid():
    s = a0()
    g = lb.Grid(s, {"3.t": [1.0, 2.0], "5.t": [3.0, 4.0, 5.0]})
    assert len(g) == 6 and g.draw(4) == {"3.t": 2.0, "5.t": 4.0}
    r = lb.run_batch(lambda i, s, seed: [s.get_param("3.t"), s.get_param("5.t")], g, threads=2, progress=False)
    assert r == [[1.0, 3.0], [1.0, 4.0], [1.0, 5.0], [2.0, 3.0], [2.0, 4.0], [2.0, 5.0]]


def test_numpy_values_and_nan_in_results(tmp_path):
    # 結果に numpy の配列・スカラーを返せる（json モジュールを往復しない、CLI-20）。NaN は None になる
    s = a0()
    p = lb.Sampler(s, {"3.t": 0.01}, seed=1)

    def f(i, s, seed):
        return {"i": np.int64(i), "v": np.arange(3, dtype=np.float32), "bad": float("nan")}

    r = lb.run_batch(f, p, n=3, out=str(tmp_path / "r.jsonl"), threads=1, progress=False,
                     header={"b": np.int32(2), "a": [np.float64(1.5)]})
    assert r[1] == {"i": 1, "v": [0.0, 1.0, 2.0], "bad": None}
    h, rows = lb.read_batch(str(tmp_path / "r.jsonl"))
    assert rows[2]["result"]["i"] == 2


def test_old_nan_lines_and_non_string_keys(tmp_path):
    # 旧版が書いた NaN・Infinity（JSON でない）の行も read_batch・再開で読める。dict のキーは json.dumps と同じく
    # 文字列にする（int・float・bool・None）
    s = a0()
    p = lb.Sampler(s, {"3.t": 0.01}, seed=2)
    out = tmp_path / "old.jsonl"

    def f(i, s, seed):
        return {"v": i, 3: "a", 1.5: "b", True: "c", None: "d"}

    r = lb.run_batch(f, p, n=2, out=str(out), threads=1, progress=False)
    assert r[0] == {"v": 0, "3": "a", "1.5": "b", "true": "c", "null": "d"}
    lines = out.read_text(encoding="utf-8").splitlines()
    lines[1] = lines[1].replace('"v":0', '"v":NaN').replace('"v": 0', '"v": NaN')
    assert "NaN" in lines[1]
    # Windows のエディタで保存し直したような CRLF で書く（どの OS でも同じ試験になるよう改行を指定）
    out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\r\n")
    h, rows = lb.read_batch(str(out))
    v = {x["i"]: x["result"]["v"] for x in rows}
    assert v[0] != v[0] and v[1] == 1
    r = lb.run_batch(f, p, n=3, out=str(out), threads=1, progress=False)
    assert len(r) == 3 and r[2]["v"] == 2 and r[0]["v"] != r[0]["v"]
