"""ドキュメントサイトの CLI の例を実際に走らせ、コマンド・出力・図を docs/gen/ に書く（scripts/build_docs.sh から）。

    python3 scripts/docs_examples.py target/release/lensbench

例ごとに docs/gen/<名前>.md（コマンドと出力の抜粋と図）と docs/gen/<名前>.png を書く。ページは
`{{#include gen/<名前>.md}}` で取り込む。図は matplotlib（出力の JSON・npz から）か、CLI がそのまま書いた PNG。
例の系は tests/refs と examples/ の公開データだけを使う。書いたファイルは .gitignore 済み。
"""

from __future__ import annotations

import json
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Any, Callable

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "gen"
TMP = ROOT / "target" / "docs-examples"
NOTE = "<!-- 自動生成（scripts/docs_examples.py）。手で直さない -->\n"

A0 = "tests/refs/A0/system.zmx"
GLASS = ["--glass-dir", "tests/glass"]
LENS = ["--lens-catalog", "examples/lenses/LB"]

plt.rcParams.update({"figure.dpi": 110, "font.size": 9, "axes.grid": True, "grid.alpha": 0.3})


def fig(w: float = 6.0, h: float = 3.6) -> Any:
    return plt.subplots(figsize=(w, h), constrained_layout=True)


# --- 図 ------------------------------------------------------------------------------------------


def plot_spot(_: str, png: Path) -> None:
    """rays の npz（像面の交点）から視野ごとのスポット図"""
    d = np.load(TMP / "rays.npz")
    last = d["surface"].max()
    m = (d["surface"] == last) & (d["vig"] == 0) & (d["err"] == 0)
    fields = sorted(set(d["field"][m].tolist()))
    f, axs = plt.subplots(1, len(fields), figsize=(2.2 * len(fields), 2.6), constrained_layout=True)
    colors = ["tab:blue", "tab:green", "tab:red"]
    half = 0.0
    for fi in fields:
        mf = m & (d["field"] == fi)
        half = max(half, np.abs(d["x"][mf] - d["x"][mf].mean()).max(), np.abs(d["y"][mf] - d["y"][mf].mean()).max())
    half *= 1.05e3
    for ax, fi in zip(np.atleast_1d(axs), fields):
        mf = m & (d["field"] == fi)
        cx, cy = d["x"][mf].mean(), d["y"][mf].mean()
        for w in sorted(set(d["wave"][mf].tolist())):
            k = mf & (d["wave"] == w)
            ax.plot((d["x"][k] - cx) * 1e3, (d["y"][k] - cy) * 1e3, ".", ms=2, color=colors[(w - 1) % 3],
                    label=f"{d['wavelengths_um'][w - 1]:.4f} µm")
        ax.set_aspect("equal")
        ax.set_xlim(-half, half)
        ax.set_ylim(-half, half)
        ax.set_title(f"field {fi}")
        ax.set_xlabel("x [µm]")
    np.atleast_1d(axs)[0].set_ylabel("y [µm]")
    np.atleast_1d(axs)[-1].legend(fontsize=6, loc="upper right")
    f.savefig(png)


def plot_mtf(out: str, png: Path) -> None:
    d = json.loads(out)
    f, ax = fig()
    for i, r in enumerate(d["mtf"]):
        c = f"C{i}"
        ax.plot(r["freqs"], r["tangential"], "-", color=c, label=f"field {r['field']} T")
        ax.plot(r["freqs"], r["sagittal"], "--", color=c, label=f"field {r['field']} S")
    ax.set_xlabel("spatial frequency [cycles/mm]")
    ax.set_ylabel("MTF")
    ax.set_ylim(0, 1.02)
    ax.legend(fontsize=7, ncol=2)
    f.savefig(png)


def plot_mtf_field(out: str, png: Path) -> None:
    d = json.loads(out)["field_mtf"]
    f, ax = fig()
    for i, fr in enumerate(sorted({r["freq"] for r in d})):
        rs = [r for r in d if r["freq"] == fr]
        h = [r["h"] for r in rs]
        ax.plot(h, [r["tangential"] for r in rs], "-", color=f"C{i}", label=f"{fr:g} c/mm T")
        ax.plot(h, [r["sagittal"] for r in rs], "--", color=f"C{i}", label=f"{fr:g} c/mm S")
    ax.set_xlabel("normalized field")
    ax.set_ylabel("MTF")
    ax.set_ylim(0, 1.02)
    ax.legend(fontsize=7)
    f.savefig(png)


def plot_mtf_focus(out: str, png: Path) -> None:
    d = json.loads(out)["through_focus"]
    f, ax = fig()
    for i, fr in enumerate(sorted({r["freq"] for r in d})):
        rs = [r for r in d if r["freq"] == fr]
        z = [r["dz"] for r in rs]
        ax.plot(z, [r["tangential"] for r in rs], "-", color=f"C{i}", label=f"{fr:g} c/mm T")
        ax.plot(z, [r["sagittal"] for r in rs], "--", color=f"C{i}", label=f"{fr:g} c/mm S")
    ax.set_xlabel("focus shift [mm]")
    ax.set_ylabel("MTF")
    ax.set_ylim(0, 1.02)
    ax.legend(fontsize=7)
    f.savefig(png)


def plot_rms(out: str, png: Path) -> None:
    d = json.loads(out)
    rs = d["rms"]
    key = next(k for k in ("field_value", "h", "x") if k in rs[0])
    f, ax = fig()
    ax.plot([r[key] for r in rs], [r["spot"] * 1e3 for r in rs], "o-", label="RMS spot")
    ax.set_xlabel("field [deg]")
    ax.set_ylabel("RMS spot radius [µm]")
    if "wfe" in rs[0]:
        ax2 = ax.twinx()
        ax2.plot([r[key] for r in rs], [r["wfe"] for r in rs], "s--", color="C1", label="RMS WFE")
        ax2.set_ylabel("RMS wavefront [waves]")
        ax2.grid(False)
    f.savefig(png)


def plot_psf(_: str, png: Path) -> None:
    d = np.load(TMP / "psf.npz")
    n = int(round(np.sqrt(d["i"].size)))
    x = d["x"].reshape(n, n) * 1e3
    y = d["y"].reshape(n, n) * 1e3
    i = d["i"].reshape(n, n)
    f, axs = plt.subplots(1, 2, figsize=(7, 3.2), constrained_layout=True)
    ext = [x.min() - x.mean(), x.max() - x.mean(), y.min() - y.mean(), y.max() - y.mean()]
    axs[0].imshow(i, origin="lower", extent=ext, cmap="inferno")
    axs[0].set_title("PSF (linear)")
    axs[1].imshow(np.log10(np.maximum(i / i.max(), 1e-5)), origin="lower", extent=ext, cmap="inferno")
    axs[1].set_title("PSF (log10)")
    for ax in axs:
        ax.set_xlabel("x [µm]")
        ax.grid(False)
    axs[0].set_ylabel("y [µm]")
    f.savefig(png)


def plot_zernike(out: str, png: Path) -> None:
    d = json.loads(out)["zernike"]
    f, ax = fig()
    nf = len(d)
    w = 0.8 / nf
    for k, r in enumerate(d):
        c = np.asarray(r["coef"])
        ax.bar(np.arange(1, c.size + 1) + (k - (nf - 1) / 2) * w, c, w, label=f"field {r['field']}")
    ax.set_xlabel("Fringe Zernike term")
    ax.set_ylabel("coefficient [waves]")
    ax.legend(fontsize=7)
    f.savefig(png)


def plot_grid(path: Path, title: str, cmap: str = "gray") -> Callable[[str, Path], None]:
    """fringe の json の grid（行 = +y）を画像に"""

    def p(_: str, png: Path) -> None:
        d = json.loads(path.read_text(encoding="utf-8"))
        g = np.asarray(d["grid"], dtype=float).reshape(d["ny"], d["nx"])
        f, ax = fig(3.4, 3.2)
        ax.imshow(g, origin="upper", cmap=cmap)
        ax.set_title(title)
        ax.set_xticks([])
        ax.set_yticks([])
        ax.grid(False)
        f.savefig(png)

    return p


def plot_pop(_: str, png: Path) -> None:
    d = json.loads((TMP / "pop.json").read_text(encoding="utf-8"))
    x = np.asarray(d["x"]) * 1e3
    g = np.asarray(d["grid"]["irradiance"], dtype=float).reshape(x.size, -1)
    f, axs = plt.subplots(1, 2, figsize=(7, 3.2), constrained_layout=True)
    axs[0].imshow(g, origin="lower", extent=[x.min(), x.max(), x.min(), x.max()], cmap="inferno")
    axs[0].set_title("irradiance")
    axs[0].set_xlabel("x [µm]")
    axs[0].grid(False)
    axs[1].plot(x, d["irradiance_x"])
    axs[1].set_xlabel("x [µm]")
    axs[1].set_title("cross section (y = 0)")
    f.savefig(png)


def plot_zoom(out: str, png: Path) -> None:
    d = json.loads(out)
    path = d["path"]
    pos = [p["pos"] for p in path]
    f, ax = fig()
    for k in d["vars"]:
        ax.plot(pos, [p[k] for p in path], "o-", label=k)
    ax.set_xlabel("state (0 = wide … 2 = tele)")
    ax.set_ylabel("group shift [mm]")
    ax2 = ax.twinx()
    ax2.plot(pos, [p["efl"] for p in path], "k--", label="EFL")
    ax2.set_ylabel("EFL [mm]")
    ax2.grid(False)
    ax.legend(fontsize=7, loc="upper left")
    f.savefig(png)


def plot_tolerance(out: str, png: Path) -> None:
    d = json.loads(out)
    m = np.sort(np.asarray(d["cdf"], dtype=float))
    f, ax = fig()
    ax.plot(m, np.arange(1, m.size + 1) / m.size * 100)
    ax.axvline(d["nominal"]["merit"], color="k", ls="--", label="nominal")
    ax.set_xlabel("merit (RMS wavefront [waves])")
    ax.set_ylabel("cumulative probability [%]")
    ax.legend(fontsize=7)
    f.savefig(png)


def copy_png(src: Path) -> Callable[[str, Path], None]:
    def p(_: str, png: Path) -> None:
        png.write_bytes(src.read_bytes())

    return p


# --- 例 ------------------------------------------------------------------------------------------

# (名前, 引数, 表に出す出力の行数, 図, 図だけ別の --json で取るときの引数)
Example = tuple[str, list[str], int, Callable[[str, Path], None] | None, list[str] | None]


def examples() -> list[Example]:
    t = str(TMP.relative_to(ROOT))
    return [
        ("info", ["info", A0, *GLASS], 20, None, None),
        ("spot", ["spot", A0, *GLASS], 16, plot_spot, ["rays", A0, *GLASS, "--fan", "grid", "--rays", "21", "--all-waves", "--save", f"{t}/rays.npz"]),
        ("mtf", ["mtf", A0, *GLASS, "--freqs", "0,25,50,75,100", "--field", "1"], 12, plot_mtf,
         ["mtf", A0, *GLASS, "--freqs", ",".join(str(v) for v in range(0, 161, 5)), "--json"]),
        ("mtf-field", ["mtf-field", A0, *GLASS, "--freqs", "20,40", "--points", "11", "--json"], 0, plot_mtf_field, None),
        ("mtf-focus", ["mtf-focus", A0, *GLASS, "--field", "1", "--freqs", "20,50", "--range", "0.1", "--steps", "21", "--json"], 0,
         plot_mtf_focus, None),
        ("rms", ["rms", A0, *GLASS, "--vs", "field", "--points", "11", "--json"], 0, plot_rms, None),
        ("wavefront", ["wavefront", A0, *GLASS, "--n", "64"], 14, None, None),
        ("fringe", ["fringe", A0, *GLASS, "--beam1", "1", "--beam2", "ref", "--field", "3", "--n", "160", "--out", f"{t}/fringe.json"], 12,
         plot_grid(TMP / "fringe.json", "interferogram (field 3 vs. reference sphere)"), None),
        ("zernike", ["zernike", A0, *GLASS, "--kind", "fringe", "--terms", "16", "--field", "1"], 10, plot_zernike,
         ["zernike", A0, *GLASS, "--kind", "fringe", "--terms", "16", "--json"]),
        ("psf", ["psf", A0, *GLASS, "--field", "2", "--pupil", "64", "--image", "128", "--save", f"{t}/psf.npz"], 3, plot_psf, None),
        ("layout", ["layout", A0, *GLASS, "--out", f"{t}/layout.png", "--out", f"{t}/layout.svg"], 5,
         copy_png(TMP / "layout.png"), None),
        ("ghost", ["ghost", A0, *GLASS], 22, None, None),
        ("pop", ["pop", A0, *GLASS, "--surf", "7", "--out-n", "64", "--out-half", "0.03", "--save", f"{t}/pop.json"], 12,
         plot_pop, None),
        ("optimize", ["optimize", "examples/projects/P9_catalog_three.json", *GLASS, *LENS, "--var", "2.t=1:20", "--var", "4.t=1:20",
                      "--merit", "spot", "--out", f"{t}/opt.zmx"], 18, None, None),
        ("tolerance", ["tolerance", "examples/projects/P8_catalog_two.json", *GLASS, *LENS, "--trials", "200"], 24, plot_tolerance,
         ["tolerance", "examples/projects/P8_catalog_two.json", *GLASS, *LENS, "--trials", "200", "--json"]),
        ("zoom", ["zoom", "examples/projects/P17_zoom_three_groups.json", *GLASS, "--var", "G2.shift", "--var", "G3.shift",
                  "--efl", "140,160,175", "--steps", "4"], 20, plot_zoom,
         ["zoom", "examples/projects/P17_zoom_three_groups.json", *GLASS, "--var", "G2.shift", "--var", "G3.shift", "--efl", "140,160,175",
          "--steps", "4", "--json"]),
    ]


# 例で使う系のレイアウト図（ページの最初に載せる）: (名前, 系のファイル, 追加の引数)
SYSTEMS = [
    ("sys-A0", A0, []),
    ("sys-P8", "examples/projects/P8_catalog_two.json", []),
    ("sys-P9", "examples/projects/P9_catalog_three.json", []),
    ("sys-P17", "examples/projects/P17_zoom_three_groups.json", []),
]


def system_examples() -> list[Example]:
    t = str(TMP.relative_to(ROOT))
    return [(name, ["layout", path, *GLASS, *LENS, *extra, "--width", "1200", "--out", f"{t}/{name}.png"], 0,
             copy_png(TMP / f"{name}.png"), None) for name, path, extra in SYSTEMS]


def run(cli: str, args: list[str]) -> str:
    r = subprocess.run([cli, *args], cwd=ROOT, capture_output=True, text=True, encoding="utf-8")
    if r.returncode != 0:
        raise RuntimeError(f"lensbench {shlex.join(args)}: {r.stderr.strip()}")
    return r.stdout


def main() -> int:
    cli = str(Path(sys.argv[1]).resolve())
    OUT.mkdir(parents=True, exist_ok=True)
    TMP.mkdir(parents=True, exist_ok=True)
    failed = []
    for name, args, lines, plot, plot_args in system_examples() + examples():
        try:
            out = run(cli, args)
            src = run(cli, plot_args) if plot_args else out
            md = [NOTE, "```sh\nlensbench " + shlex.join(args).replace(str(TMP.relative_to(ROOT)) + "/", "") + "\n```\n"]
            if lines:
                ls = out.rstrip().splitlines()
                more = "\n…" if len(ls) > lines else ""
                md.append("\n```text\n" + "\n".join(ls[:lines]) + more + "\n```\n")
            if plot:
                plot(src, OUT / f"{name}.png")
                plt.close("all")
                md.append(f"\n![{name} の出力](gen/{name}.png)\n")
            (OUT / f"{name}.md").write_text("".join(md), encoding="utf-8")
        except Exception as e:  # 1 つ失敗しても残りは作り、最後に失敗で終える
            failed.append(name)
            print(f"{name}: {e}", file=sys.stderr)
            (OUT / f"{name}.md").write_text(NOTE + f"（例の生成に失敗: {name}）\n", encoding="utf-8")
    if failed:
        print("失敗: " + ", ".join(failed), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
