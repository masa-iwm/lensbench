"""lensbench serve の Python クライアント。LENSBENCH_EXE か target/release/lensbench を使う。"""

import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "clients" / "python"))

from lensbench_client import Client, RpcError  # noqa: E402

ZMX = ROOT / "tests" / "refs" / "A0" / "system.zmx"
GLASS = ROOT / "tests" / "glass"


def _exe():
    e = os.environ.get("LENSBENCH_EXE")
    if e:
        return e
    for name in ("lensbench", "lensbench.exe"):
        p = ROOT / "target" / "release" / name
        if p.exists():
            return str(p)
    w = shutil.which("lensbench")
    if w:
        return w
    pytest.skip("lensbench の実行ファイルがない（cargo build --release -p lensbench-cli）")


@pytest.fixture
def lb():
    with Client.spawn(ZMX, exe=_exe(), glass_dir=[GLASS], threads=1) as c:
        yield c


def test_loop_and_restore(lb):
    t0 = lb.get("3.t")
    first = lb.analyze("spot", field=3)
    seen = []
    for k in range(5):
        lb.set({"3.t": t0 + 0.1 * k})
        assert lb.get("3.t") == t0 + 0.1 * k  # 値は丸めずに届く
        seen.append(lb.analyze("spot", field=3)["spot"][0]["rms_centroid"])
    assert len(set(seen)) == 5
    lb.set({"3.t": t0})
    # 像面距離の解き（ソルブ）は前の値から解き直すので、戻しても 1e-14 程度の差は残りうる
    again = lb.analyze("spot", field=3)["spot"][0]["rms_centroid"]
    assert again == pytest.approx(first["spot"][0]["rms_centroid"], rel=1e-12)
    assert lb.get("3.t", "5.t")[0] == t0


def test_analyze_args_like_cli(lb):
    r = lb.analyze("mtf", field=1, wave=2, freqs=[10, 20])
    exe = _exe()
    out = subprocess.run(
        [exe, "mtf", str(ZMX), "--glass-dir", str(GLASS), "--field", "1", "--wave", "2", "--freqs", "10,20", "--json"],
        capture_output=True,
        check=True,
        text=True,
    ).stdout
    import json

    assert r == json.loads(out)


def test_trace_arrays(lb):
    rng = np.random.default_rng(1)
    px = rng.uniform(-0.7, 0.7, 20000)
    py = rng.uniform(-0.7, 0.7, 20000)
    r = lb.trace(px, py, field=2)
    assert r["rays"] == 20000
    assert r["x"].dtype == np.float64 and r["x"].shape == (20000,)
    assert r["vig"].dtype == np.int32
    ok = (r["err"] == 0) & (r["vig"] == 0)
    assert ok.sum() == r["ok"]
    assert np.allclose(r["l"] ** 2 + r["m"] ** 2 + r["n"] ** 2, 1.0)
    # 主光線（瞳の中心）は 2 回追っても同じ
    a = lb.trace([0.0], [0.0], field=2)
    b = lb.trace(np.zeros(1), np.zeros(1), hx=r["hx"], hy=r["hy"])
    assert a["y"][0] == b["y"][0]


def test_echo_and_errors(lb):
    a = np.arange(1_000_000, dtype=np.float64)
    i = np.arange(10, dtype=np.int32)
    r = lb.echo(a=a, i=i)
    assert np.array_equal(r["a"], a) and np.array_equal(r["i"], i)
    with pytest.raises(RpcError) as e:
        lb.set({"99.t": 1.0})
    assert e.value.kind == "invalid"  # 計算の誤りの種類（error.data.kind）
    with pytest.raises(RpcError) as e:
        lb.call("nothing")
    assert e.value.kind is None
    assert lb.ping()["version"]


def test_reload_after_external_write(tmp_path):
    p = tmp_path / "sys.zmx"
    with Client.spawn(ZMX, exe=_exe(), glass_dir=[GLASS]) as lb:
        lb.write(p)
    # 別アプリが書いた系を読み、変えて書き戻す → reload で取り込む
    with Client.spawn(p, exe=_exe(), glass_dir=[GLASS]) as lb:
        t0 = lb.get("3.t")
        lb.set({"3.t": t0 + 0.5})
        lb.write(p)
        lb.set({"3.t": t0})
        lb.reload()
        assert abs(lb.get("3.t") - (t0 + 0.5)) < 1e-12


def test_watch(tmp_path):
    exe = _exe()
    p = tmp_path / "sys.zmx"
    shutil.copy(ZMX, p)
    out = tmp_path / "out.json"
    st = tmp_path / "status.json"
    w = subprocess.Popen(
        [exe, "watch", "--interval", "0.05", "--max-runs", "2", "--status", str(st), "--",
         "spot", str(p), "--glass-dir", str(GLASS), "--save", str(out)],
        stdout=subprocess.PIPE,
    )
    import json

    def run_no():
        try:
            return json.loads(st.read_text())["run"]
        except (OSError, ValueError):
            return 0

    t = time.time()
    while run_no() < 1:
        assert time.time() - t < 60
        time.sleep(0.02)
    a = json.loads(out.read_text())
    time.sleep(0.05)
    with Client.spawn(p, exe=exe, glass_dir=[GLASS]) as lb:
        lb.set({"3.t": lb.get("3.t") + 0.3})
        lb.write(p)
    assert w.wait(timeout=60) == 0
    w.stdout.close()
    assert run_no() == 2
    b = json.loads(out.read_text())
    assert a != b


def test_edit_and_save(lb, tmp_path):
    n0 = len(lb.surface()["surfaces"])
    r = lb.edit({"op": "insert_surface", "at": 1, "surface": {"t": 0.0, "comment": "D"}})
    assert r["surfaces"] == n0 + 1 and r["system_hash"].startswith("sha256:")
    assert lb.surface("D")["surface"] == 1
    with pytest.raises(RpcError):
        lb.edit({"op": "delete_surface", "surface": "D"}, {"op": "nope"})
    assert lb.surface("D")["surface"] == 1
    a = lb.analyze("spot", field=2)
    w = lb.save(tmp_path / "s.json")
    assert w["system_hash"] == r["system_hash"]
    lb.load(tmp_path / "s.json")
    assert lb.analyze("spot", field=2) == a


TWO_CFG = (ROOT / "crates" / "lensbench-cli" / "tests" / "serve_ipc.rs").read_text(encoding="utf-8")
TWO_CFG = TWO_CFG[TWO_CFG.index('const TWO_CFG: &str = "') + len('const TWO_CFG: &str = "') :]
TWO_CFG = TWO_CFG[: TWO_CFG.index('";')]


def test_fringe_over_serve(tmp_path):
    p = tmp_path / "two_cfg.zmx"
    p.write_text(TWO_CFG)
    with Client.spawn(p, exe=_exe(), threads=1) as lb:
        f = lb.fringe(n=16)
        assert f["nx"] == 16 and f["pv_waves"] > 0 and f["notes"] == []
        lb.set({"1.t": 100.5})  # 構成 1 だけの値
        f = lb.fringe(n=16)
        assert len(f["notes"]) == 1


@pytest.mark.parametrize("pipe", [False, True])
def test_pipe_and_shared_memory(pipe):
    px = np.linspace(-1, 1, 200_001)
    py = 0.3 * px
    with Client.spawn(ZMX, exe=_exe(), glass_dir=[GLASS], threads=1, pipe=pipe) as lb:
        assert lb.ping()["version"]
        want = lb.trace(px, py, field=2)
        if sys.platform not in ("win32", "linux"):
            return
        lb.use_shm(32 << 20)  # 入 3.2 MB・出 14 MB が収まる
        got = lb.trace(px, py, field=2)
        for k in ("x", "y", "z", "l", "m", "n", "opl", "err", "vig"):
            assert got[k].dtype == want[k].dtype and np.array_equal(got[k], want[k]), k
        # 収まらない配列は今までどおり送る
        lb.use_shm(1 << 16)
        got = lb.trace(px, py, field=2)
        assert np.array_equal(got["opl"], want["opl"])
        lb.use_shm(0)
        assert np.array_equal(lb.echo(v=px)["v"], px)


def test_plugin_complex_and_bool(lb):
    n = 7
    rng = np.random.default_rng(1)
    es = rng.normal(size=(n, 2)) + 1j * rng.normal(size=(n, 2))
    s = lb.plugin("spot", field=np.array(2.0))
    assert s["rms_centroid"] == pytest.approx(lb.analyze("spot", field=2)["spot"][0]["rms_centroid"], rel=0, abs=0)
    assert any(p["name"] == "spot" for p in lb.plugins())
    # bool・complex の echo
    m = np.array([True, False, True])
    e = lb.echo(m=m, z=es)
    assert e["m"].dtype == np.bool_ and np.array_equal(e["m"], m)
    assert e["z"].dtype == np.complex128 and np.array_equal(e["z"], es)


def test_progress_and_stop_from_another_connection(lb):
    """長い公差を回している間に、別の接続から進み具合を読み、止める（そこまでの結果が返る）"""
    import threading

    with lb.another() as o:
        assert o.progress()["running"] is False
        assert o.cancel() is False
        out = {}
        t = threading.Thread(
            target=lambda: out.setdefault("r", lb.call("tolerance", {"trials": 100000, "seed": 2, "sensitivity": False})[0])
        )
        t.start()
        t0 = time.time()
        while o.progress()["tolerance"] is None:
            assert time.time() - t0 < 60, "進み具合が出ない"
            time.sleep(0.01)
        assert o.progress()["running"] is True
        assert o.stop("tolerance") is True
        t.join(60)
        assert out["r"]["cancelled"] is True
        assert o.progress()["running"] is False
        with pytest.raises(RpcError):
            o.stop("foo")
