"""例題で共通に使う小物: リポジトリの中のテスト用の系・ガラス、出力先（`lensbench run --out` を優先）"""

from __future__ import annotations

from pathlib import Path

import numpy as np
from numpy.typing import NDArray

import lensbench as lb

ROOT = Path(__file__).resolve().parents[2]
#: 公開のテスト用ガラス（`lensbench run --glass-dir` で足したフォルダも System.load は自分で探す）
GLASS = [str(ROOT / "tests" / "glass")]


def zmx(name: str) -> str:
    """tests/refs/<name>/system.zmx（公開のテスト用の系）"""
    return str(ROOT / "tests" / "refs" / name / "system.zmx")


def out_dir() -> Path:
    """結果を書くフォルダ: `lensbench run --out` で渡された場所、なければこのファイルの隣の out/"""
    o = lb.run_settings()["out"]
    p = Path(o) if o else Path(__file__).resolve().parent / "out"
    p.mkdir(parents=True, exist_ok=True)
    return p


def pupil_grid(n: int) -> tuple[NDArray[np.float64], NDArray[np.float64]]:
    """正規化瞳の単位円の中の n×n 格子の点 (px, py)"""
    g = np.linspace(-1.0, 1.0, n)
    x, y = np.meshgrid(g, g)
    m = x**2 + y**2 <= 1.0
    return x[m].copy(), y[m].copy()
