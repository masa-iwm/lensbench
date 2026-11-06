"""照合試験の枠組みの例題を作る。lensbench を使わない（別の実装の基準）。

    python tests/verify/make_samples.py

- a0_zernike.verify.json: tests/refs/A0/zernike.json（OpticStudio 26.1 の Zernike 係数）を基準に、プラグイン zernike を照合
"""

import json
import pathlib
import subprocess

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def commit():
    try:
        return subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True,
                              check=True).stdout.strip()
    except Exception:
        return ""


def write(name, d):
    (HERE / name).write_text(json.dumps(d, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")


def zernike():
    src = json.loads((ROOT / "tests/refs/A0/zernike.json").read_text())
    ops = json.loads((ROOT / "tests/refs/A0/ops.json").read_text())
    runs = []
    for key, v in src.items():
        kind, f, w = key.split("|")
        runs.append({
            "label": key,
            "config": {"kind": kind, "wave": int(w) + 1, "terms": len(v["coef"])},
            "inputs": {"field": int(f) + 1},
            "expected": {"coef": v["coef"]},
        })
    write("a0_zernike.verify.json", {
        "format": "lensbench-verify", "version": 1,
        "name": "A0 の Zernike 係数（OpticStudio）",
        "about": "Zernike Fringe・Standard の係数（37 項、瞳の格子 64）を OpticStudio と比べる",
        "source": {"by": f"OpticStudio {ops['version']}（ZOS-API）", "commit": "",
                   "how": "ZOS-API で書き出した tests/refs/A0/zernike.json を tests/verify/make_samples.py で写した",
                   "date": "2026-11-07"},
        "ci": True,
        "system": {"path": "../refs/A0/system.zmx", "cfg": 1, "glass_dirs": ["../glass"]},
        "plugin": "zernike",
        "config": {"n": 64, "ref_primary": 0},
        "threads": [1, 3],
        "checks": {"coef": {"abs": 2e-8, "note": "OpticStudio の係数は小数 8 桁（丸めで 5e-9）"}},
        "runs": runs,
    })


if __name__ == "__main__":
    zernike()
