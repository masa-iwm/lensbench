"""書き出したファイルを lensbench とは別の実装で読み戻して確かめる。

- SVG: 標準の XML 読み込み器。要素の数と、断面・光線の座標
- glTF（.glb）: trimesh。網の数・頂点・面、閉じた立体の体積、光線の点
- STEP: OpenCascade（cadquery-ocp）。立体の数・妥当性・体積・境界箱、面の点、光線の点
- 追跡結果（CSV・JSON）: csv・json。OpticStudio の基準（tests/refs/*/rays.npz）と照合

面の位置と形は、.zmx をこのファイルの中で読んだ曲率・非球面・半径と、OpticStudio が書いた各面の大域座標
（rays.npz の gmat、.zmx の大域基準面 = GLRS）から求める。lensbench の計算は使わないので、書き出しの座標が
GLRS 基準で、形が系と合っていることを独立に確かめられる。

lensbench の CLI は環境変数 LENSBENCH_BIN（省略で target/ci/lensbench）。
"""

from __future__ import annotations

import csv
import json
import math
import os
import re
import subprocess
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import pytest
import trimesh

ROOT = Path(__file__).resolve().parents[2]
EXE = ".exe" if os.name == "nt" else ""
BIN = Path(os.environ.get("LENSBENCH_BIN", ROOT / "target" / "ci" / f"lensbench{EXE}"))
GLASS = ROOT / "tests" / "glass"

# A0: 共軸の 3 枚玉（球面）。C0: 偏心・傾き・非球面・鏡があり、大域基準面が像の手前の面（GLRS の確かめ）
SYSTEMS = ["A0", "C0"]


# ---------------------------------------------------------------- .zmx と基準


@dataclass
class Surf:
    kind: str = "STANDARD"
    curv: float = 0.0
    conic: float = 0.0
    parm: dict[int, float] = field(default_factory=dict)
    semi: float = 0.0
    glass: str = ""

    def sag(self, r: np.ndarray) -> np.ndarray:
        c, k = self.curv, self.conic
        r2 = r * r
        z = c * r2 / (1.0 + np.sqrt(np.maximum(1.0 - (1.0 + k) * c * c * r2, 0.0)))
        if self.kind == "EVENASPH":
            for i in range(1, 9):
                z = z + self.parm.get(i, 0.0) * r ** (2 * i)
        elif self.kind != "STANDARD":
            raise ValueError(f"面の形 {self.kind} はこの試験では扱わない")
        return z


def read_zmx(path: Path) -> list[Surf]:
    raw = path.read_bytes()
    txt = raw.decode("utf-16") if raw[:2] in (b"\xff\xfe", b"\xfe\xff") else raw.decode()
    out: list[Surf] = []
    for line in txt.splitlines():
        t = line.split()
        if not t:
            continue
        if t[0] == "SURF":
            out.append(Surf())
        elif not out:
            continue
        elif t[0] == "TYPE":
            out[-1].kind = t[1]
        elif t[0] == "CURV":
            out[-1].curv = float(t[1])
        elif t[0] == "CONI":
            out[-1].conic = float(t[1])
        elif t[0] == "PARM":
            out[-1].parm[int(t[1])] = float(t[2])
        elif t[0] == "DIAM":
            out[-1].semi = float(t[1])
        elif t[0] == "GLAS":
            out[-1].glass = t[1]
    return out


@dataclass
class Ref:
    name: str
    zmx: Path
    surfs: list[Surf]
    # OpticStudio の各面の大域座標: 回転（列が面の x・y・z 軸）と原点
    rot: np.ndarray
    org: np.ndarray
    hx_hy: np.ndarray
    px_py: np.ndarray
    rays: np.ndarray  # [視野, 光線, 面, (x y z l m n opl err vig)]
    primary: int  # 1 始まり

    def to_local(self, j: int, p: np.ndarray) -> np.ndarray:
        return (p - self.org[j]) @ self.rot[j]

    def to_global(self, j: int, q: np.ndarray) -> np.ndarray:
        return q @ self.rot[j].T + self.org[j]


def load_ref(name: str) -> Ref:
    d = ROOT / "tests" / "refs" / name
    z = np.load(d / "rays.npz")
    g = z["gmat"]
    ops = json.loads((d / "ops.json").read_text())
    return Ref(
        name=name,
        zmx=d / "system.zmx",
        surfs=read_zmx(d / "system.zmx"),
        rot=g[:, :9].reshape(-1, 3, 3),
        org=g[:, 9:],
        hx_hy=z["hx_hy"],
        px_py=z["px_py_s"],
        rays=z["surf"],
        primary=int(ops["primary_wave"]),
    )


def field_index(ref: Ref) -> dict[int, int]:
    """基準の視野の行 → 系の視野の番号（1 始まり）。系の視野と同じ正規化座標の行だけ"""
    ops = json.loads((ref.zmx.parent / "ops.json").read_text())
    f = np.array(ops["fields"], dtype=float)
    fmax = np.max(np.hypot(f[:, 0], f[:, 1]))
    out = {}
    for i, h in enumerate(ref.hx_hy):
        for k, v in enumerate(f / fmax):
            if np.allclose(h, v, atol=1e-12):
                out[i] = k + 1
    return out


# ---------------------------------------------------------------- CLI


def run(*args: str) -> dict:
    r = subprocess.run(
        [str(BIN), *args, "--glass-dir", str(GLASS), "--json"],
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    assert r.returncode == 0, r.stderr
    return json.loads(r.stdout)


def layout(ref: Ref, tmp: Path, *outs: str, extra: tuple[str, ...] = ()) -> dict:
    a = ["layout", str(ref.zmx)]
    for o in outs:
        a += ["--out", str(tmp / o)]
    return run(*a, *extra)


def read_csv(p: Path) -> dict[str, np.ndarray]:
    with p.open(newline="", encoding="utf-8") as f:
        rows = list(csv.reader(f))
    head, body = rows[0], rows[1:]
    return {h: np.array([float(r[i]) for r in body]) for i, h in enumerate(head)}


def rays_table(ref: Ref, tmp: Path, fan: str, n: int, ext: str = "csv") -> dict[str, np.ndarray]:
    """書き出しの光線と同じ瞳の追跡表（主波長、全視野、全面）"""
    p = tmp / f"rays_{fan}{n}.{ext}"
    run("rays", str(ref.zmx), "--fan", fan, "--rays", str(n), "--save", str(p))
    return read_csv(p)


def global_points(t: dict[str, np.ndarray]) -> np.ndarray:
    ok = (t["err"] == 0) & np.isfinite(t["gx"])
    return np.stack([t["gx"][ok], t["gy"][ok], t["gz"][ok]], axis=1)


def nearest(a: np.ndarray, b: np.ndarray) -> np.ndarray:
    """a の各点から b の最も近い点までの距離"""
    out = np.empty(len(a))
    for i in range(0, len(a), 256):
        d = np.linalg.norm(a[i : i + 256, None, :] - b[None, :, :], axis=2)
        out[i : i + 256] = d.min(axis=1)
    return out


# ---------------------------------------------------------------- 立体の形


def solid_surfaces(name: str) -> list[int]:
    """立体の名前（"L1 S1-S2 N-SK16"・"M1 S8"）から面の番号"""
    m = re.search(r"\bS(\d+)(?:-S(\d+))?\b", name)
    assert m, name
    return [int(x) for x in m.groups() if x]


def check_solid_points(ref: Ref, name: str, p: np.ndarray, tol: float) -> None:
    """立体の点が系の面の上にあるか。

    レンズ: 両面の有効径の内側の点は、どちらかの面（大域座標の OpticStudio の面）の上。両面に点が十分ある。
    鏡: 鏡面の上の点が十分あり、全点が鏡面の有効径の内側（裏は厚みの分だけ離れる）。
    """
    js = solid_surfaces(name)
    rim = max(ref.surfs[k].semi for k in js)
    on, inside, within = [], [], np.zeros(len(p), dtype=bool)
    for j in js:
        s = ref.surfs[j]
        q = ref.to_local(j, p)
        r = np.hypot(q[:, 0], q[:, 1])
        on.append(np.abs(q[:, 2] - s.sag(np.minimum(r, s.semi))) < tol)
        inside.append(r < s.semi * (1 - 1e-6) - tol)
        within |= r <= rim * (1 + 1e-6) + tol
    # 縁: どの点もどれかの面の座標で縁の半径の内側（面どうしが傾いていると、片方の面の座標では外に出る）
    assert within.all(), f"{ref.name} {name}: 縁の外の点 {p[~within][:3]}"
    for j, o in zip(js, on):
        assert o.mean() > 0.2, f"{ref.name} {name}: 面 {j} の上の点 {o.mean():.0%}"
    if len(js) == 2:
        bad = inside[0] & inside[1] & ~on[0] & ~on[1]
        assert not bad.any(), f"{ref.name} {name}: 面の上にない点 {p[bad][:3]}"


def section_area_volume(ref: Ref, js: list[int]) -> float:
    """共軸のレンズの体積（面の間を回転体として数値積分）"""
    a, b = (ref.surfs[j] for j in js)
    za, zb = ref.org[js[0]][2], ref.org[js[1]][2]
    rmax = max(a.semi, b.semi)
    r = np.linspace(0.0, rmax, 200001)
    h = (zb + b.sag(np.minimum(r, b.semi))) - (za + a.sag(np.minimum(r, a.semi)))
    f = 2 * math.pi * r * h
    return float(np.sum((f[1:] + f[:-1]) * 0.5 * np.diff(r)))


def lens_volume(ref: Ref, js: list[int]) -> float:
    """レンズの体積（下の網を 2 つの細かさで求め、誤差が分割の 2 乗に比例することから外挿。相対 1e-8 ほど）"""
    v1, v2 = mesh_volume(ref, js, 1024, 256), mesh_volume(ref, js, 2048, 512)
    return v2 + (v2 - v1) / 3


def mesh_volume(ref: Ref, js: list[int], na: int, nr: int) -> float:
    """レンズの体積を .zmx と OpticStudio の面の座標から細かい網で求める（傾いた面どうしでも）。

    縁の半径は両面の大きい方、小さい方の面は外を平らな輪にする。側面は前後の縁の円を、後ろの面の軸を前の面の
    軸の側へ向けて前の面の x を後ろの面に投げた向きから同じ角度の点どうしで結ぶ線織面。
    """
    a, b = js
    rim = max(ref.surfs[a].semi, ref.surfs[b].semi)
    t = np.linspace(0.0, 2 * math.pi, na, endpoint=False)
    za, zb = ref.rot[a][:, 2], ref.rot[b][:, 2]
    zb = zb if np.dot(za, zb) >= 0 else -zb
    xa = ref.rot[a][:, 0]
    xb = xa - zb * np.dot(zb, xa)
    xb /= np.linalg.norm(xb)
    yb = np.cross(zb, xb)
    frames = {a: (xa, ref.rot[a][:, 1]), b: (xb, yb)}

    def disk(j: int) -> np.ndarray:
        """[半径, 角度, 3] の大域の点（半径 0 を含む）"""
        s = ref.surfs[j]
        r = np.linspace(0.0, s.semi, nr + 1)
        if rim > s.semi:
            r = np.append(r, rim)
        ex, ey = frames[j]
        d = np.cos(t)[None, :, None] * ex + np.sin(t)[None, :, None] * ey  # 大域の向き
        z = s.sag(np.minimum(r, s.semi))
        loc = (d @ ref.rot[j]) * r[:, None, None]  # 局所の x・y
        loc[..., 2] = z[:, None]
        return ref.to_global(j, loc.reshape(-1, 3)).reshape(len(r), na, 3)

    def tris(p: np.ndarray, q: np.ndarray, flip: bool) -> list[np.ndarray]:
        """2 つの環 p、q（角度の並び）の間の三角形"""
        p1, q1 = np.roll(p, -1, axis=0), np.roll(q, -1, axis=0)
        out = [np.stack([p, q, q1], 1), np.stack([p, q1, p1], 1)]
        return [x[:, ::-1] for x in out] if flip else out

    da, db = disk(a), disk(b)
    T = []
    for d, flip in ((da, True), (db, False)):
        for k in range(len(d) - 1):
            T += tris(d[k], d[k + 1], flip)
    T += tris(da[-1], db[-1], True)
    T = np.concatenate(T)
    v = np.einsum("ij,ij->i", T[:, 0], np.cross(T[:, 1], T[:, 2])).sum() / 6.0
    return abs(float(v))


def coaxial(ref: Ref, js: list[int]) -> bool:
    return all(
        np.allclose(ref.rot[j], np.eye(3), atol=1e-12) and np.allclose(ref.org[j][:2], 0, atol=1e-12)
        for j in js
    )


# ---------------------------------------------------------------- 追跡結果（CSV・JSON）


@pytest.mark.parametrize("name", SYSTEMS)
def test_ray_table_matches_opticstudio(name: str, tmp_path: Path) -> None:
    ref = load_ref(name)
    fi = field_index(ref)
    assert fi, "基準に系の視野がない"
    pup = ",".join(f"{float(x)!r},{float(y)!r}" for x, y in ref.px_py)
    fields = ",".join(str(k) for k in fi.values())
    tabs = {}
    for ext in ("csv", "json"):
        p = tmp_path / f"rays.{ext}"
        run(
            "rays", str(ref.zmx), "--pupil", pup, "--field", fields,
            "--wave", str(ref.primary), "--save", str(p),
        )
        tabs[ext] = p
    t = read_csv(tabs["csv"])
    js = json.loads(tabs["json"].read_text(encoding="utf-8"))
    # JSON は CSV と同じ値（CSV は往復で同じ値に戻る表記）
    assert js["rows"] == len(t["x"])
    for k, v in t.items():
        assert np.array_equal(np.asarray(js["columns"][k], dtype=float), v, equal_nan=True), k
    assert np.all(t["wave"] == ref.primary)
    nsurf = ref.rays.shape[2]
    # OpticStudio の方向余弦は鏡で向きの符号が変わる（lensbench は進む向き）。面 j の後の鏡の数で符号を戻す
    parity = np.cumprod([-1.0 if s.glass == "MIRROR" else 1.0 for s in ref.surfs[:nsurf]])
    assert set(t["surface"].astype(int)) == set(range(1, nsurf))
    n = 0
    for row, fk in fi.items():
        for i in range(len(ref.px_py)):
            for j in range(1, nsurf):
                z = ref.rays[row, i, j]
                sel = (t["field"] == fk) & (t["ray"] == i) & (t["surface"] == j)
                assert sel.sum() == 1
                k = int(np.flatnonzero(sel)[0])
                assert t["px"][k] == ref.px_py[i, 0] and t["py"][k] == ref.px_py[i, 1]
                if z[7] != 0 or not np.isfinite(z[0]):
                    continue
                assert t["err"][k] == 0
                loc = np.array([t["x"][k], t["y"][k], t["z"][k]])
                np.testing.assert_allclose(loc, z[:3], rtol=0, atol=1e-9)
                lmn = parity[j] * np.array([t["l"][k], t["m"][k], t["n"][k]])
                np.testing.assert_allclose(lmn, z[3:6], rtol=0, atol=1e-9)
                # 大域座標は OpticStudio の面の大域座標（GLRS）で局所座標を移したもの
                glo = np.array([t["gx"][k], t["gy"][k], t["gz"][k]])
                np.testing.assert_allclose(glo, ref.to_global(j, z[:3]), rtol=0, atol=1e-8)
                n += 1
    assert n > 100


# ---------------------------------------------------------------- SVG


@pytest.mark.parametrize("name", SYSTEMS)
def test_svg(name: str, tmp_path: Path) -> None:
    ref = load_ref(name)
    info = layout(ref, tmp_path, "a.svg")
    root = ET.parse(tmp_path / "a.svg").getroot()
    ns = {"s": "http://www.w3.org/2000/svg"}
    poly = root.findall(".//s:polygon", ns)
    line = root.findall(".//s:polyline", ns)
    title = lambda e: e.find("s:title", ns).text  # noqa: E731
    groups = {g.get("id"): g for g in root.findall("s:g", ns)}
    rays = groups["rays"].findall("s:polyline", ns)
    # 要素の数: 立体は多角形 1 つずつ、像面などの面と光線は折れ線
    assert [title(e) for e in poly] == info["solids"]
    assert len(line) == len(info["surfaces"]) + info["rays_2d"]
    assert len(rays) == info["rays_2d"]

    def pts(e: ET.Element) -> np.ndarray:
        v = np.array([[float(x) for x in t.split(",")] for t in e.get("points").split()])
        return np.stack([v[:, 0], -v[:, 1]], axis=1)  # (z, y)。SVG は縦を反転して書く

    # 座標の範囲: 全部の点が viewBox の中、範囲は場面の境界箱（yz）
    vb = [float(x) for x in root.get("viewBox").split()]
    allp = np.concatenate([pts(e) for e in poly + line])
    assert allp[:, 0].min() >= vb[0] - 1e-9 and allp[:, 0].max() <= vb[0] + vb[2] + 1e-9
    assert -allp[:, 1].max() >= vb[1] - 1e-9 and -allp[:, 1].min() <= vb[1] + vb[3] + 1e-9
    # 共軸の系は断面の端が立体の箱の端。傾いた立体の断面は自分の子午面なので、立体の箱（縁の円）の内側
    lo, hi = np.array(info["bounds_mm"])
    if all(coaxial(ref, solid_surfaces(title(e))) for e in poly):
        np.testing.assert_allclose(allp.min(axis=0), [lo[2], lo[1]], atol=2e-5)
        np.testing.assert_allclose(allp.max(axis=0), [hi[2], hi[1]], atol=2e-5)
    else:
        assert np.all(allp.min(axis=0) >= [lo[2] - 2e-5, lo[1] - 2e-5])
        assert np.all(allp.max(axis=0) <= [hi[2] + 2e-5, hi[1] + 2e-5])

    # 断面: 共軸の立体は x = 0 の断面なので、有効径の内側の点は面の上（SVG は 0.1 µm に丸める）。傾いた立体は
    # 自分の子午面の断面を yz に投げたもので x が分からないので、glTF の同じ立体の yz の箱の中にあることだけ
    glb = trimesh.load(layout(ref, tmp_path, "a.glb") and tmp_path / "a.glb", force="scene")
    for e in poly:
        q = pts(e)
        n = title(e)
        if coaxial(ref, solid_surfaces(n)):
            p3 = np.stack([np.zeros(len(q)), q[:, 1], q[:, 0]], axis=1)
            check_solid_points(ref, n, p3, tol=2e-5)
        g = np.asarray(glb.geometry[n].vertices)[:, [2, 1]] * 1e3
        assert np.all(q >= g.min(axis=0) - 1e-4) and np.all(q <= g.max(axis=0) + 1e-4), n

    # 光線: 追跡の大域座標（同じ y 扇）の面ごとの点が折れ線の上にある
    t = rays_table(ref, tmp_path, "y", 7)
    nf = int(t["field"].max())
    assert len(rays) == nf * 7
    for e in rays:
        f = int(re.match(r"F(\d+) W", title(e)).group(1))
        assert f in range(1, nf + 1)
    for f in range(1, nf + 1):
        sel = (t["field"] == f) & (t["err"] == 0)
        want = np.stack([t["gz"][sel], t["gy"][sel]], axis=1)
        have = np.concatenate([pts(e) for e in rays if title(e).startswith(f"F{f} ")])
        assert nearest(want, have).max() < 1e-5


# ---------------------------------------------------------------- glTF（.glb）


@pytest.mark.parametrize("name", SYSTEMS)
def test_glb(name: str, tmp_path: Path) -> None:
    ref = load_ref(name)
    info = layout(ref, tmp_path, "a.glb")
    sc = trimesh.load(tmp_path / "a.glb", force="scene")
    geo = {n: sc.graph[n] for n in sc.graph.nodes_geometry}
    for n, (m, _) in geo.items():
        assert np.allclose(m, np.eye(4)), n  # 節の変換なし（頂点が大域座標）
    names = list(sc.geometry)
    nf = len(field_index(ref))
    assert names == info["solids"] + info["surfaces"] + [f"rays F{i}" for i in range(1, nf + 1)]
    # glTF の単位は m。CLI の bounds_mm は縁の円の箱を式で出すので、周を多角形にした網の箱を含み、
    # 周を細かくした網（半径方向の輪も bounds と同じ 32）の箱とは、傾いた系でも一致する
    lo, hi = np.array(info["bounds_mm"])
    b = sc.bounds * 1e3
    assert np.all(b[0] >= lo - 2e-6) and np.all(b[1] <= hi + 2e-6)  # bounds_mm は 1e-6 に丸め
    layout(ref, tmp_path, "fine.glb", extra=("--segments", "4096", "--rings", "32"))
    fine = trimesh.load(tmp_path / "fine.glb", force="scene").bounds * 1e3
    np.testing.assert_allclose(fine, [lo, hi], atol=1e-4)

    for n in info["solids"]:
        g = sc.geometry[n]
        assert isinstance(g, trimesh.Trimesh)
        assert len(g.faces) > 0 and len(g.vertices) > 0
        v = g.vertices * 1e3
        check_solid_points(ref, n, v, tol=2e-5)
        # 位置で結ぶと閉じた立体（頂点は法線のために重なる）
        w = g.copy()
        w.merge_vertices(merge_tex=True, merge_norm=True, digits_vertex=9)
        assert w.is_watertight and w.is_winding_consistent, n
        assert w.volume > 0, n
        js = solid_surfaces(n)
        if len(js) == 2:
            want = lens_volume(ref, js)
            assert abs(w.volume * 1e9 / want - 1) < 3e-3, (n, w.volume * 1e9, want)

    # 光線: 追跡（同じ円周の瞳）の面ごとの大域座標が線の頂点にある
    t = rays_table(ref, tmp_path, "ring", 12)
    for f in range(1, nf + 1):
        g = sc.geometry[f"rays F{f}"]
        sel = (t["field"] == f) & (t["err"] == 0)
        want = np.stack([t["gx"][sel], t["gy"][sel], t["gz"][sel]], axis=1)
        have = np.asarray(g.vertices) * 1e3
        assert nearest(want, have).max() < 2e-5


# ---------------------------------------------------------------- STEP


def _cast(topods, kind: str, s):
    """TopoDS.Face(s)（版によって Face_s）"""
    f = getattr(topods, kind, None) or getattr(topods, f"{kind}_s")
    return f(s)


def read_step(p: Path):
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPControl import STEPControl_Reader

    r = STEPControl_Reader()
    assert r.ReadFile(str(p)) == IFSelect_RetDone
    assert r.TransferRoots() > 0
    return r.OneShape()


def explore(shape, kind) -> list:
    from OCP.TopExp import TopExp_Explorer

    e = TopExp_Explorer(shape, kind)
    out = []
    while e.More():
        out.append(e.Current())
        e.Next()
    return out


def mesh_points(solid) -> np.ndarray:
    """立体を細かく三角形に分けた節点（面の上の点）"""
    from OCP.BRep import BRep_Tool
    from OCP.BRepMesh import BRepMesh_IncrementalMesh
    from OCP.TopAbs import TopAbs_FACE
    from OCP.TopLoc import TopLoc_Location
    from OCP.TopoDS import TopoDS

    BRepMesh_IncrementalMesh(solid, 0.005, False, 0.1, True)
    pts = []
    for f in explore(solid, TopAbs_FACE):
        loc = TopLoc_Location()
        tri = BRep_Tool.Triangulation_s(_cast(TopoDS, 'Face', f), loc)
        assert tri is not None
        tr = loc.Transformation()
        for i in range(1, tri.NbNodes() + 1):
            q = tri.Node(i).Transformed(tr)
            pts.append((q.X(), q.Y(), q.Z()))
    return np.array(pts)


@pytest.mark.parametrize("name", SYSTEMS)
def test_step(name: str, tmp_path: Path) -> None:
    from OCP.Bnd import Bnd_Box
    from OCP.BRepBndLib import BRepBndLib
    from OCP.BRepCheck import BRepCheck_Analyzer
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps
    from OCP.TopAbs import TopAbs_SOLID
    from OCP.TopoDS import TopoDS

    ref = load_ref(name)
    # glTF は体積を比べるので細かく分ける
    info = layout(ref, tmp_path, "a.step", "a.glb", extra=("--segments", "256", "--rings", "64"))
    shape = read_step(tmp_path / "a.step")
    solids = explore(shape, TopAbs_SOLID)
    assert len(solids) == len(info["solids"])
    glb = trimesh.load(tmp_path / "a.glb", force="scene")
    for n, s in zip(info["solids"], solids):
        assert BRepCheck_Analyzer(s).IsValid(), n
        gp = GProp_GProps()
        # 精度を指定して積分する（既定の積分は傾いた線織面の立体で 0.3〜0.7% ずれる）
        BRepGProp.VolumeProperties_s(s, gp, 1e-10, False)
        vol = gp.Mass()
        assert vol > 0, n
        # 形: 面の上の点が系の面の上（STEP の面は厳密。継ぎ目の辺は読み込み器が近似で作り 5e-6 mm ずれる）
        p = mesh_points(s)
        check_solid_points(ref, n, p, tol=1e-5)
        # 境界箱は glTF の立体と同じ（別の読み込み器で読んだ同じ場面）
        b = Bnd_Box()
        BRepBndLib.AddOptimal_s(s, b, False, False)
        lo, hi = b.CornerMin(), b.CornerMax()
        g = glb.geometry[n].vertices * 1e3
        np.testing.assert_allclose([lo.X(), lo.Y(), lo.Z()], g.min(axis=0), atol=2e-2)
        np.testing.assert_allclose([hi.X(), hi.Y(), hi.Z()], g.max(axis=0), atol=2e-2)
        # 体積: レンズは .zmx と OpticStudio の面の座標から求めたものと、全部の立体を glTF の網と比べる
        js = solid_surfaces(n)
        if len(js) == 2:
            want = lens_volume(ref, js)
            if coaxial(ref, js):
                assert abs(want / section_area_volume(ref, js) - 1) < 1e-8
            assert abs(vol / want - 1) < 1e-6, (n, vol, want)
        w = glb.geometry[n].copy()
        w.merge_vertices(merge_tex=True, merge_norm=True, digits_vertex=9)
        assert abs(w.volume * 1e9 / vol - 1) < 5e-4, (n, w.volume * 1e9, vol)

    # 光線（折れ線）と面の輪郭: 立体と別の組。折れ線は 1 次の B スプラインとして読まれ、極が折れ点
    from OCP.BRepAdaptor import BRepAdaptor_Curve
    from OCP.GeomAbs import GeomAbs_BSplineCurve, GeomAbs_Circle
    from OCP.TopoDS import TopoDS_Iterator

    def kids(x) -> list:
        it, out = TopoDS_Iterator(x), []
        while it.More():
            out.append(it.Value())
            it.Next()
        return out

    wires = [k for k in kids(shape) if not explore(k, TopAbs_SOLID)]
    assert len(wires) == 1
    lines, circles = [], []
    for e in kids(wires[0]):
        c = BRepAdaptor_Curve(_cast(TopoDS, "Edge", e))
        tr = c.Trsf()
        if c.GetType() == GeomAbs_BSplineCurve:
            b = c.BSpline()
            assert b.Degree() == 1
            q = [b.Pole(i).Transformed(tr) for i in range(1, b.NbPoles() + 1)]
            lines.append(np.array([(v.X(), v.Y(), v.Z()) for v in q]))
        else:
            assert c.GetType() == GeomAbs_Circle
            circles.append(c)
    assert len(lines) == info["rays_3d"]
    # 面（像面など）の輪郭の円: 中心と向きが OpticStudio の面の大域座標、半径が .zmx の半径
    assert len(circles) == len(info["surfaces"])
    for c, n in zip(circles, info["surfaces"]):
        j = len(ref.surfs) - 1 if n == "IMA" else int(re.search(r"S(\d+)", n).group(1))
        ci = c.Circle()
        o, a = ci.Location(), ci.Axis().Direction()
        np.testing.assert_allclose([o.X(), o.Y(), o.Z()], ref.org[j], atol=1e-9)
        assert abs(abs(np.dot([a.X(), a.Y(), a.Z()], ref.rot[j][:, 2])) - 1) < 1e-12
        assert abs(ci.Radius() - ref.surfs[j].semi) < 1e-9
    # 追跡（同じ円周の瞳）の面ごとの大域座標が折れ点にある
    t = rays_table(ref, tmp_path, "ring", 12)
    assert nearest(global_points(t), np.concatenate(lines)).max() < 1e-9
