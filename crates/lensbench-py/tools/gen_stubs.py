"""lensbench の型スタブ（lensbench.pyi）を作る。

実行中のモジュールから名前・引数・既定値・説明を、Rust のソース（src/*.rs）から引数と返り値の型を取り、
`crates/lensbench-py/lensbench.pyi` に書く。maturin は `<モジュール名>.pyi` を wheel に入れ、py.typed も足す。

    python crates/lensbench-py/tools/gen_stubs.py          # 書き直す
    python crates/lensbench-py/tools/gen_stubs.py --check  # 今のモジュールと食い違えば 1 で終わる（CI）

Rust の型から決められない引数・返り値は OVERRIDES で与える（なければ Any）。
"""

from __future__ import annotations

import argparse
import inspect
import re
import sys
import types
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SRC = HERE / "src"
OUT = HERE / "lensbench.pyi"

# Rust の型が Any になるもの・意味の決まっているものの上書き。キーは "関数" か "クラス.メソッド"、
# 値は {"引数名": "型", "return": "型"}
OVERRIDES: dict[str, dict[str, str]] = {
    "System.trace": {"record": "bool | Sequence[int]", "pol": "bool | int | None", "dz": "float | ArrayLike | None"},
    "System.trace_rays": {"record": "bool | Sequence[int]", "pol": "bool | int | None", "dz": "float | ArrayLike | None"},
    "trace_many": {"systems": "Sequence[System]", "record": "bool | Sequence[int]", "pol": "bool | int | None", "dz": "float | ArrayLike | None"},
    "System.set_params": {"changes": "Mapping[str, float] | Sequence[tuple[str, float]]"},
    "System.with_params": {"changes": "Mapping[str, float] | Sequence[tuple[str, float]]"},
    "System.insert_surface": {"kw": "Any"},
    "System.set_surface": {"kw": "Any"},
    "Plugin.__new__": {"config": "Any"},
    "Plugin.run": {"inputs": "Any"},
    "register_plugin": {"func": "Callable[..., Mapping[str, Any]]"},
    "run_batch": {"func": "Callable[..., Any]", "progress": "Callable[[int, int], Any] | None"},
}

F64 = "NDArray[np.float64]"
# 結果の dict の形（TypedDict、CLI-16）。名前: (説明, 必ずある鍵, 条件で付く鍵)。鍵の一致は tests/test_stubs_examples.py で確かめる
RESULTS: dict[str, tuple[str, dict[str, str], dict[str, str]]] = {
    "_TanSag": ("子午・球欠の組", {"tangential": "float", "sagittal": "float"}, {}),
    "_TraceResult": (
        "System.trace・trace_rays の結果（光線ごとの列）",
        {**{k: F64 for k in ("x", "y", "z", "l", "m", "n", "opl", "opl_os", "lens_opd")},
         "err": "NDArray[np.int32]", "vig": "NDArray[np.int32]", "ok": "NDArray[np.bool_]",
         "wavelengths": "NDArray[np.int64]", "last": "int"},
        {"hx": "float", "hy": "float", "rec_surfaces": "NDArray[np.int64]",
         **{f"rec_{k}": F64 for k in ("x", "y", "z", "l", "m", "n", "gx", "gy", "gz", "opl")},
         "jones": "NDArray[np.complex128]", "jones_z": "NDArray[np.complex128]", "jones_wavelength": "int",
         "grad": F64, "grad_params": "list[str]", "grad_land": F64,
         **{k: F64 for k in ("gx", "gy", "gz", "gl", "gm", "gn", "p0", "k0")}, "dz": "float",
         "object": "list[float]", "pol_grid": "tuple[int, int]", "pol_grid_missing": F64,
         "pol_grid_bad": "NDArray[np.bool_]"},
    ),
    "_MtfResult": (
        "回折 MTF（mtf・geometric_mtf・huygens_mtf）。tangential・sagittal は [波長][周波数]（each_wave）か [周波数]",
        {"freqs": F64, "tangential": F64, "sagittal": F64, "cutoff": "_TanSag"},
        {"waves": "list[int]", "warnings": "list[str]"},
    ),
    "_ThroughFocusMtf": ("スルーフォーカス MTF（[周波数][dz]）",
                         {"freqs": F64, "dz": F64, "tangential": F64, "sagittal": F64, "warnings": "list[str]"}, {}),
    "_FieldMtf": ("MTF vs 視野（[周波数][視野]）",
                  {"freqs": F64, "h": F64, "tangential": F64, "sagittal": F64, "warnings": "list[str]",
                   "field_value": F64}, {}),
    "_PsfResult": (
        "PSF（psf・huygens_psf）。i は [y][x]、強度は収差のない同じ瞳の中心の値を 1",
        {"x": F64, "y": F64, "i": F64, "center": "tuple[float, float]", "axes": "list[list[float]]",
         "delta": "tuple[float, float]", "peak": "float", "pupil": "int", "phase_step": "float",
         "enclosed": "float", "warnings": "list[str]"},
        {},
    ),
    "_Cardinal": ("基点（距離は mm）", {k: "float" for k in (
        "n_object", "n_image", "efl", "f_object", "f_image", "focal_object", "principal_object",
        "anti_principal_object", "nodal_object", "anti_nodal_object", "focal_image", "principal_image",
        "anti_principal_image", "nodal_image", "anti_nodal_image", "bfl", "ffl")}, {}),
    "_OptimizeResult": (
        "System.optimize の結果",
        {"system": "System", "names": "list[str]", "x0": "list[float]", "x": "list[float]", "merit0": "float",
         "merit": "float", "history": "list[float]", "iterations": "int", "evals": "int", "stop": "str",
         "note": "str | None", "inactive": "list[str]", "term_values0": "list[float]", "term_values": "list[float]"},
        {},
    ),
}
RESULT_OF = {
    "System.trace": "_TraceResult", "System.trace_rays": "_TraceResult", "System.mtf": "_MtfResult",
    "System.geometric_mtf": "_MtfResult", "System.huygens_mtf": "_MtfResult",
    "System.through_focus_mtf": "_ThroughFocusMtf", "System.field_mtf": "_FieldMtf", "System.psf": "_PsfResult",
    "System.huygens_psf": "_PsfResult", "System.cardinal": "_Cardinal", "System.optimize": "_OptimizeResult",
}
for _k, _t in RESULT_OF.items():
    OVERRIDES.setdefault(_k, {})["return"] = _t


def render_results() -> list[str]:
    out = []
    for name, (doc, req, opt) in RESULTS.items():
        base = name if not opt else f"{name}Base"
        out += ["", "", f"class {base}(TypedDict):", f'    """{doc}"""']
        out += [f"    {k}: {t}" for k, t in req.items()]
        if opt:
            out += ["", "", f"class {name}({base}, total=False):", '    """条件で付く鍵"""']
            out += [f"    {k}: {t}" for k, t in opt.items()]
    return out


def _split_top(s: str, sep: str = ",") -> list[str]:
    out, depth, cur = [], 0, ""
    for ch in s:
        if ch in "(<[":
            depth += 1
        elif ch in ")>]":
            depth -= 1
        if ch == sep and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur)
    return [x.strip() for x in out]


def _balanced(text: str, i: int) -> int:
    """text[i] が "(" のとき、対の ")" の次の位置"""
    depth = 0
    for j in range(i, len(text)):
        if text[j] == "(":
            depth += 1
        elif text[j] == ")":
            depth -= 1
            if depth == 0:
                return j + 1
    raise ValueError("括弧が閉じない")


def rust_fns() -> tuple[dict[str, dict], dict[str, str]]:
    """(キー → {args: {名前: Rust 型}, ret: Rust 型}, Rust の構造体名 → Python のクラス名)"""
    fns: dict[str, dict] = {}
    classes: dict[str, str] = {}
    for p in sorted(SRC.glob("*.rs")):
        t = p.read_text(encoding="utf-8")
        t = re.sub(r"//[^\n]*", "", t)
        for m in re.finditer(r'#\[pyclass\([^)]*name\s*=\s*"(\w+)"[^)]*\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?struct\s+(\w+)', t):
            classes[m.group(2)] = m.group(1)
        # #[pymethods] impl X { ... } の範囲
        spans = []
        for m in re.finditer(r"#\[pymethods\]\s*impl\s+(\w+)\s*\{", t):
            depth, j = 1, m.end()
            while depth:
                depth += {"{": 1, "}": -1}.get(t[j], 0)
                j += 1
            spans.append((m.start(), j, m.group(1)))
        for m in re.finditer(r"\bfn\s+(\w+)\s*(<[^>(]*>)?\s*\(", t):
            pos = m.start()
            owner = next((c for a, b, c in spans if a < pos < b), None)
            head = t[max(0, pos - 2000):pos]
            is_pyfn = re.search(r"#\[pyfunction\][^;{}]*$", head) is not None
            if owner is None and not is_pyfn:
                continue
            name = m.group(1)
            ren = re.search(r'#\[pyo3\([^)]*\bname\s*=\s*"(\w+)"[^)]*\)\][^;{}]*$', head)
            if ren:
                name = ren.group(1)
            if owner and re.search(r"#\[new\][^;{}]*$", head):
                name = "__new__"
            end = _balanced(t, m.end() - 1)
            args = {}
            for a in _split_top(t[m.end():end - 1]):
                if ":" not in a:
                    continue
                n, ty = a.split(":", 1)
                n = n.strip().removeprefix("mut ").strip()
                args[n] = ty.strip()
            rm = re.match(r"\s*->\s*([^{]*?)\s*(where\b[^{]*)?\{", t[end:])
            ret = rm.group(1).strip() if rm else "()"
            pr = re.fullmatch(r"PyResult<(.*)>", ret, re.S)
            if pr:
                ret = pr.group(1).strip()
            fns[f"{owner}.{name}" if owner else name] = {"args": args, "ret": ret}
    return fns, classes


ARGISH = {"FieldArg": "Field", "SurfArg": "Surf", "ObjArg": "ObjectPoint", "VarArg": "str | tuple[str, float, float]"}


def conv(ty: str, classes: dict[str, str], ret: bool) -> str:
    """Rust の型 → Python の型。分からなければ Any"""
    ty = re.sub(r"\s+", " ", ty).strip()
    ty = re.sub(r"&'\w+ ", "", ty)
    ty = re.sub(r"'\w+", "'_", ty).lstrip("&").strip()
    m = re.fullmatch(r"(?:PyRef|PyRefMut|Bound|Py)<'_, (\w+)>|(?:PyRef|PyRefMut|Bound|Py)<(\w+)>", ty)
    if m:
        inner = m.group(1) or m.group(2)
        if inner in classes:
            return classes[inner]
    if ty in classes:
        return classes[ty]
    if ty in ARGISH and not ret:
        return ARGISH[ty]
    m = re.fullmatch(r"Option<(.*)>", ty)
    if m:
        inner = conv(m.group(1), classes, ret)
        return "Any" if inner == "Any" else f"{inner} | None"
    m = re.fullmatch(r"Vec<(.*)>", ty)
    if m:
        inner = conv(m.group(1), classes, ret)
        return f"list[{inner}]" if ret else f"Sequence[{inner}]"
    m = re.fullmatch(r"\[(\w+); (\d+)\]", ty)
    if m:
        inner = conv(m.group(1), classes, ret)
        return f"tuple[{', '.join([inner] * int(m.group(2)))}]"
    if ty.startswith("(") and ty.endswith(")"):
        items = _split_top(ty[1:-1])
        if not items:
            return "None"
        return f"tuple[{', '.join(conv(i, classes, ret) for i in items)}]"
    for pat, py in TABLE_RET if ret else TABLE_ARG:
        if re.fullmatch(pat, ty):
            return py
    return "Any"


TABLE_ARG = [
    (r"PyArrayLike[123]<'_, \w+, AllowTypeChange>|PyArrayLike[123]<'_, \w+>|PyReadonlyArray[123]<'_, \w+>", "ArrayLike"),
    (r"f64|f32", "float"),
    (r"usize|u64|u32|i64|i32|u8", "int"),
    (r"bool", "bool"),
    (r"str|String", "str"),
    (r"PathBuf|Path", "StrPath"),
    (r"Bound<'_, PyDict>", "Mapping[str, Any]"),
]
TABLE_RET = [
    (r"f64|f32", "float"),
    (r"usize|u64|u32|i64|i32|u8", "int"),
    (r"bool", "bool"),
    (r"str|String", "str"),
    (r"PathBuf", "str"),
    (r"Bound<'_, PyDict>|Py<PyDict>", "dict[str, Any]"),
    (r"Bound<'_, PyList>", "list[Any]"),
    (r"Bound<'_, PyArray[1-9]<(f64|f32)>>", "NDArray[np.float64]"),
    (r"Bound<'_, PyArray[1-9]<bool>>", "NDArray[np.bool_]"),
    (r"Bound<'_, PyArray[1-9]<C64>>", "NDArray[np.complex128]"),
    (r"Bound<'_, PyArray[1-9]<(usize|i64|u32|i32|u64)>>", "NDArray[np.int64]"),
    (r"Self", "Self"),
]


def arg_type(ty: str, classes) -> str:
    return conv(ty, classes, False)


def ret_type(ty: str, classes) -> str:
    return conv(ty, classes, True)


def sig_params(obj) -> list[tuple[str, str, str | None]]:
    """(種類, 名前, 既定値の文字列) の並び。種類は "" "*" "**"。self・type は除く"""
    ts = getattr(obj, "__text_signature__", None)
    if ts is None:
        return [("*", "args", None), ("**", "kwargs", None)]
    out = []
    for p in _split_top(ts.strip()[1:-1]):
        if p in ("$self", "$type", "$cls", "/"):
            continue
        if p.startswith("**"):
            out.append(("**", p[2:], None))
        elif p.startswith("*"):
            out.append(("*", p[1:], None) if len(p) > 1 else ("*", "", None))
        elif "=" in p:
            n, d = p.split("=", 1)
            out.append(("", n, d))
        else:
            out.append(("", p, None))
    return out


def default_repr(d: str) -> str:
    # スタブでは既定値は ... で書く（値は説明に任せる）。短い定数はそのまま見せる
    if re.fullmatch(r"None|True|False|-?\d+(\.\d+)?(e-?\d+)?|\"[^\"]*\"", d):
        return d
    return "..."


def doc_block(doc: str | None, indent: str) -> list[str]:
    if not doc:
        return []
    doc = inspect.cleandoc(doc).replace("\\", "\\\\").replace('"""', '\\"\\"\\"')
    lines = doc.splitlines()
    if len(lines) == 1 and not lines[0].endswith('"'):
        return [f'{indent}"""{lines[0]}"""']
    return [f'{indent}"""{lines[0]}'] + [(f"{indent}{l}" if l else "") for l in lines[1:]] + [f'{indent}"""']


def render_fn(name: str, obj, key: str, fns, classes, indent: str, kind: str = "") -> list[str]:
    info = fns.get(key, {"args": {}, "ret": None})
    # 上書きのキーは Python のクラス名（"System.trace"）。key は Rust の型名のことがある（"PySystem.trace"）
    cls, _, meth = key.rpartition(".")
    ov = OVERRIDES.get(key) or OVERRIDES.get(f"{classes.get(cls, cls)}.{meth}" if cls else key, {})
    parts = []
    if kind not in ("static", "module"):
        parts.append("cls" if name == "__new__" else "self")
    for k, n, d in sig_params(obj):
        if k == "*" and not n:
            parts.append("*")
            continue
        ty = ov.get(n) or (arg_type(info["args"][n], classes) if n in info["args"] and k != "**" else "Any")
        if d is not None:
            parts.append(f"{n}: {ty} = {default_repr(d)}")
        else:
            parts.append(f"{k}{n}: {ty}")
    if name == "__new__":
        cls = key.split(".")[0]
        ret = cls
    else:
        ret = ov.get("return") or (ret_type(info["ret"], classes) if info["ret"] is not None else "Any")
        if ret == "Self":
            ret = key.split(".")[0]
    lines = []
    if kind == "static":
        lines.append(f"{indent}@staticmethod")
    lines.append(f"{indent}def {name}({', '.join(parts)}) -> {ret}:")
    body = doc_block(getattr(obj, "__doc__", None) if name != "__new__" else None, indent + "    ")
    if body:
        lines += body
    else:
        lines[-1] += " ..."
    return lines


def render(lb) -> str:
    fns, classes = rust_fns()
    py2rs = {v: k for k, v in classes.items()}
    out = [
        "# 自動生成（crates/lensbench-py/tools/gen_stubs.py）。手で直さない",
        '"""' + inspect.cleandoc(lb.__doc__ or "lensbench").splitlines()[0] + '"""',
        "",
        "import os",
        "from collections.abc import Callable, Mapping, Sequence",
        "from typing import Any, TypedDict, final",
        "",
        "import numpy as np",
        "from numpy.typing import ArrayLike, NDArray",
        "",
        "StrPath = str | os.PathLike[str]",
        "#: 視野: 番号（1 始まり）か正規化視野 (hx, hy)",
        "Field = int | tuple[float, float]",
        "#: 面: 番号（OpticStudio の面番号）か名前（コメント）",
        "Surf = int | str",
        "#: 物点: 物体面の (x, y) [mm]、無限遠なら角度 (θx, θy) [deg]",
        "ObjectPoint = tuple[float, float] | Sequence[float]",
        "",
        "__version__: str",
        "#: Python API の版（互換を壊す変更で上がる。docs/python.md）",
        "API_VERSION: tuple[int, int]",
    ]
    out += render_results()
    names = sorted(n for n in dir(lb) if not n.startswith("__") and n not in ("API_VERSION",))
    consts = [n for n in names if not callable(getattr(lb, n)) and not inspect.ismodule(getattr(lb, n))]
    for n in consts:
        v = getattr(lb, n)
        out.append(f"{n}: {type(v).__name__}" if not isinstance(v, list) else f"{n}: list[{type(v[0]).__name__}]")
    for n in names:
        o = getattr(lb, n)
        if isinstance(o, type):
            rs = py2rs.get(n, n)
            if issubclass(o, BaseException):
                # 例外: 親を書き、継承できるので @final は付けない
                base = o.__bases__[0]
                parent = base.__name__ if base.__module__ == "lensbench" else base.__qualname__
                out += ["", "", f"class {n}({parent}):"]
                out += doc_block(o.__doc__, "    ") or ["    ..."]
                continue
            out += ["", "", "@final", f"class {n}:"]
            out += doc_block(o.__doc__, "    ")
            members = []
            for m, raw in vars(o).items():
                if m.startswith("__") and m not in ("__new__", "__len__", "__getitem__", "__iter__", "__call__"):
                    continue
                members.append((m, raw))
            for m, raw in sorted(members, key=lambda x: (x[0] != "__new__", x[0])):
                key = f"{rs}.{m}"
                if type(raw).__name__ == "getset_descriptor":
                    info = fns.get(key)
                    ty = ret_type(info["ret"], classes) if info else "Any"
                    out += ["", "    @property", f"    def {m}(self) -> {ty}:"]
                    db = doc_block(raw.__doc__, "        ")
                    out += db if db else []
                    if not db:
                        out[-1] += " ..."
                    # #[setter] の fn set_<名前>（Python のメソッドには出ない）があれば書ける属性
                    if f"{rs}.set_{m}" in fns and not hasattr(o, f"set_{m}"):
                        out += ["", f"    @{m}.setter", f"    def {m}(self, value: {ty}) -> None: ..."]
                elif isinstance(raw, staticmethod) or type(raw).__name__ == "staticmethod":
                    out += [""] + render_fn(m, getattr(o, m), key, fns, classes, "    ", "static")
                elif m == "__new__":
                    if o.__text_signature__ is None:
                        continue  # 直接は作らない（staticmethod などで作る）
                    obj = types.SimpleNamespace(__text_signature__=o.__text_signature__, __doc__=None)
                    out += [""] + render_fn(m, obj, key, fns, classes, "    ")
                else:
                    out += [""] + render_fn(m, getattr(o, m), key, fns, classes, "    ")
        elif callable(o):
            out += ["", ""] + render_fn(n, o, n, fns, classes, "", "module")
    text = "\n".join(out) + "\n"
    # 型の別名は実行時のモジュールにないので _ を付ける（stubtest が名前の食い違いとして数えない）
    # （説明の文は変えない。型の書かれる行だけ: def の行と別名の行）
    def alias(line: str) -> str:
        if not re.match(r"\s*def |(StrPath|Field|Surf|ObjectPoint) = ", line):
            return line
        for a in ("StrPath", "Field", "Surf", "ObjectPoint"):
            line = re.sub(rf"(?<![\w.]){a}\b", f"_{a}", line)
        return line

    text = "\n".join(alias(line) for line in text.split("\n"))
    for rs, py in classes.items():
        text = text.replace(f"-> {rs}:", f"-> {py}:")
    # 上書きの型名を正規化
    return text.replace("OVERRIDE_KEYS_UNUSED", "")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="書かずに比べる")
    a = ap.parse_args()
    import lensbench

    text = render(lensbench)
    if a.check:
        old = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if old != text:
            print(f"{OUT} が古い。python {Path(__file__).relative_to(HERE.parent.parent)} で作り直す", file=sys.stderr)
            return 1
        return 0
    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
