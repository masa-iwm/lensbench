"""型スタブ・版・例題・lensbench run の Python 側"""

import os
import subprocess
import sys
from pathlib import Path

import pytest

import lensbench as lb

ROOT = Path(__file__).resolve().parents[3]
EXAMPLES = sorted((ROOT / "examples" / "python").glob("[!_]*.py"))


def test_stub_matches_the_module():
    # 関数・引数を足したり変えたりしたら gen_stubs.py で作り直す
    r = subprocess.run(
        [sys.executable, str(ROOT / "crates" / "lensbench-py" / "tools" / "gen_stubs.py"), "--check"],
        capture_output=True, text=True, encoding="utf-8", errors="replace",
    )
    assert r.returncode == 0, r.stderr


def test_api_version():
    assert isinstance(lb.API_VERSION, tuple) and len(lb.API_VERSION) == 2
    assert lb.API_VERSION[0] >= 1
    assert isinstance(lb.__version__, str)


def test_run_settings_reads_the_environment(tmp_path):
    code = "import json, lensbench as lb; print(json.dumps(lb.run_settings()))"
    env = dict(os.environ, LENSBENCH_OUT=str(tmp_path), LENSBENCH_THREADS="2", LENSBENCH_RUN="1",
               LENSBENCH_COATINGS=os.pathsep.join(["a.lbcm", "b.lbct"]))
    r = subprocess.run([sys.executable, "-c", code, "x", "-y"], env=env, capture_output=True, text=True, encoding="utf-8", errors="replace", check=True)
    import json

    v = json.loads(r.stdout)
    assert v["out"] == str(tmp_path) and v["threads"] == 2 and v["under_run"]
    assert v["coatings"] == ["a.lbcm", "b.lbct"]
    assert v["args"] == ["x", "-y"]


def test_progress_line(capfd):
    lb.progress(3, 10, "視野 3")
    lb.progress(4, 10)
    err = capfd.readouterr().err
    assert "lensbench:progress 3/10 視野 3\n" in err and "lensbench:progress 4/10\n" in err


def test_surface_label_in_errors():
    # 膜の表がない面の偏光追跡: エラーに面の番号・種類・膜の名前が入る
    s = lb.System.load(str(ROOT / "tests" / "refs" / "A0" / "system.zmx"), glass_dirs=[str(ROOT / "tests" / "glass")])
    j = 1
    mat = s.surface(j)["material"]
    s.set_surface(j, coating="NOSUCHCOAT")
    with pytest.raises(ValueError) as e:
        s.trace(1, [0.0], [0.0], pol=True)
    assert f"面 {j}(STANDARD, {mat}, 膜 NOSUCHCOAT)" in str(e.value), str(e.value)


@pytest.mark.parametrize("path", EXAMPLES, ids=lambda p: p.name)
def test_example_runs(path, tmp_path):
    # Windows の既定の標準出力（cp1252 など）では日本語を書けないので、lensbench run と同じく UTF-8 にする
    env = dict(os.environ, LENSBENCH_OUT=str(tmp_path), PYTHONIOENCODING="utf-8")
    r = subprocess.run([sys.executable, str(path)], env=env, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300)
    assert r.returncode == 0, r.stdout + r.stderr


def cli():
    p = os.environ.get("LENSBENCH_CLI")
    if p:
        return p
    exe = "lensbench.exe" if sys.platform == "win32" else "lensbench"
    p = ROOT / "target" / "release" / exe
    if not p.exists():
        pytest.skip("CLI がない（cargo build --release -p lensbench-cli）")
    return str(p)


def test_lensbench_run_example(tmp_path):
    out = tmp_path / "res"
    r = subprocess.run(
        [cli(), "run", "--python", sys.executable, "--threads", "2", "--out", str(out),
         str(ROOT / "examples" / "python" / "param_sweep.py"), "--n", "3"],
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=300,
    )
    assert r.returncode == 0, r.stdout + r.stderr
    rows = (out / "param_sweep.csv").read_text().splitlines()
    assert len(rows) == 4
    log = (out / "run.log").read_text(encoding="utf-8")
    assert "lensbench:progress 3/3" in log and "終了コード 0" in log
    # スクリプトが失敗すれば終了コードで分かる
    bad = tmp_path / "bad.py"
    bad.write_text("raise SystemExit(5)\n")
    r = subprocess.run([cli(), "run", "--python", sys.executable, str(bad)], capture_output=True, text=True, encoding="utf-8", errors="replace")
    assert r.returncode == 5


def test_error_classes_type_check(tmp_path):
    """型スタブの例外は BaseException の子として型検査が通る（except・raise）"""
    pytest.importorskip("mypy")
    p = tmp_path / "use_errors.py"
    p.write_text(
        "import lensbench as lb\n"
        "def f() -> None:\n"
        "    try:\n"
        "        raise lb.ReadError('x')\n"
        "    except lb.InvalidError as e:\n"
        "        print(e)\n"
        "    except lb.Error:\n"
        "        pass\n",
        encoding="utf-8",
    )
    r = subprocess.run([sys.executable, "-m", "mypy", "--strict", str(p)], capture_output=True, text=True, encoding="utf-8", errors="replace")
    assert r.returncode == 0, r.stdout + r.stderr
