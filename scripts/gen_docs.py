"""ドキュメントサイト（mdBook）の自動生成ページを作る。

    python3 scripts/gen_docs.py --cli target/release/lensbench   # docs/reference/cli.md・python-api.md を書く

- CLI: `lensbench help --json`（clap の定義の JSON）から、サブコマンドの一覧・使い方・引数の表・終了コードを作る
- Python: 型スタブ `crates/lensbench-py/lensbench.pyi` の名前・引数・説明から作る（wheel のビルドは要らない）

書いたファイルは .gitignore 済み。手で直さず、元（clap の説明・Rust の doc コメント）を直す。
"""

from __future__ import annotations

import argparse
import ast
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "reference"
PYI = ROOT / "crates" / "lensbench-py" / "lensbench.pyi"

NOTE = "<!-- 自動生成（scripts/gen_docs.py）。手で直さない -->\n\n"


def cli_json(cli: str) -> dict:
    """`lensbench help --json`（clap の定義の JSON。登録簿の解析・終了コードの表も）"""
    out = subprocess.run([cli, "help", "--json"], capture_output=True, text=True, check=True, encoding="utf-8")
    return json.loads(out.stdout)


def cell(s: str | None) -> str:
    """表の欄に入れる文字（| と改行を除く）"""
    return (s or "").replace("|", "&#124;").replace("\n", " ").strip()


def arg_name(a: dict) -> str:
    if a["positional"]:
        n = f"<{a['value_name']}>"
        return f"`{n}...`" if a["multiple"] else f"`{n}`"
    n = f"--{a['long']}" if a["long"] else f"-{a['short']}"
    if a["short"] and a["long"]:
        n = f"-{a['short']}, {n}"
    if not a["flag"]:
        n += f" <{a['value_name']}>"
    return f"`{n}`"


def arg_note(a: dict) -> str:
    notes = []
    h = a["long_help"] or a["help"] or ""
    if a["required"] and not a["positional"] and "必須" not in h:
        notes.append("必須")
    if a["multiple"] and not a["positional"] and "複数可" not in h:
        notes.append("複数可")
    if a["choices"]:
        notes.append("値: " + "・".join(f"`{c}`" for c in a["choices"]))
    return "（" + "、".join(notes) + "）" if notes else ""


def args_table(args: list[dict]) -> str:
    if not args:
        return "引数なし。\n"
    rows = ["| 引数 | 既定 | 説明 |\n|---|---|---|\n"]
    for a in args:
        d = "" if a["flag"] else ",".join(a["default"])
        rows.append(f"| {arg_name(a)} | {('`' + cell(d) + '`') if d else ''} | {cell(a['long_help'] or a['help'])}{arg_note(a)} |\n")
    return "".join(rows)


def command_md(c: dict, level: int, prefix: str) -> list[str]:
    name = f"{prefix} {c['name']}".strip()
    md = [f"\n{'#' * level} {name}\n\n"]
    about = c["long_about"] or c["about"] or ""
    if about:
        md.append(about.strip() + "\n\n")
    md.append(f"```text\n{c['usage']}\n```\n\n")
    # 全体の引数（--threads）は「共通」に 1 度だけ
    md.append(args_table([a for a in c["args"] if not a["global"]]))
    for s in c["subcommands"]:
        md.extend(command_md(s, level + 1, name))
    return md


def gen_cli(cli: str) -> str:
    v = cli_json(cli)
    md = [NOTE, "# CLI リファレンス\n\n",
          f"lensbench {v['version']} の `lensbench help --json` から作る（`lensbench <コマンド> --help` と同じ中身）。"
          "使い方の例は [使い方（CLI）](../cli.md)。\n\n",
          "| コマンド | 説明 |\n|---|---|\n"]
    for c in v["subcommands"]:
        md.append(f"| [`{c['name']}`](#{c['name']}) | {cell(c['about'])} |\n")
    md.append("\n## 共通\n\n")
    md.append(f"```text\n{v['usage']}\n```\n\n")
    md.append(args_table(v["args"]))
    md.append("\n終了コード:\n\n| コード | 意味 |\n|---|---|\n")
    for e in v["exit_codes"]:
        md.append(f"| {e['code']} | {cell(e['meaning'])} |\n")
    for c in v["subcommands"]:
        md.extend(command_md(c, 2, ""))
    return "".join(md)


# --- Python ---------------------------------------------------------------------------------------


def sig(fn: ast.FunctionDef, drop_self: bool) -> str:
    a = fn.args
    parts: list[str] = []
    pos = a.posonlyargs + a.args
    defaults = [None] * (len(pos) - len(a.defaults)) + list(a.defaults)
    for i, (arg, d) in enumerate(zip(pos, defaults)):
        if drop_self and i == 0 and arg.arg in ("self", "cls"):
            continue
        s = arg.arg + (f": {ast.unparse(arg.annotation)}" if arg.annotation else "")
        if d is not None:
            s += f" = {ast.unparse(d)}"
        parts.append(s)
    if a.vararg:
        parts.append("*" + a.vararg.arg + (f": {ast.unparse(a.vararg.annotation)}" if a.vararg.annotation else ""))
    elif a.kwonlyargs:
        parts.append("*")
    for arg, d in zip(a.kwonlyargs, a.kw_defaults):
        s = arg.arg + (f": {ast.unparse(arg.annotation)}" if arg.annotation else "")
        if d is not None:
            s += f" = {ast.unparse(d)}"
        parts.append(s)
    if a.kwarg:
        parts.append("**" + a.kwarg.arg + (f": {ast.unparse(a.kwarg.annotation)}" if a.kwarg.annotation else ""))
    ret = f" -> {ast.unparse(fn.returns)}" if fn.returns else ""
    return f"({', '.join(parts)}){ret}"


def doc(node: ast.AST) -> str:
    d = ast.get_docstring(node, clean=True)  # type: ignore[arg-type]
    return (d.strip() + "\n") if d else ""


def is_property(fn: ast.FunctionDef) -> bool:
    return any(isinstance(d, ast.Name) and d.id == "property" for d in fn.decorator_list)


def gen_python() -> str:
    src = PYI.read_text(encoding="utf-8")
    tree = ast.parse(src)
    lines = src.splitlines()
    md = [NOTE, "# Python API リファレンス\n\n",
          ("`import lensbench as lb` の名前の一覧です（型スタブ `lensbench.pyi` から作る）。"
           "版・単位・座標の約束は [Python API の約束](../python.md)、使い方は [Python から使う](../python-guide.md)。\n\n")]
    consts, funcs, classes = [], [], []
    for node in tree.body:
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name) and not node.target.id.startswith("_"):
            prev = lines[node.lineno - 2].strip() if node.lineno >= 2 else ""
            note = prev[2:].strip() if prev.startswith("#:") else ""
            consts.append((node.target.id, ast.unparse(node.annotation), note))
        elif isinstance(node, ast.FunctionDef) and not node.name.startswith("_"):
            funcs.append(node)
        elif isinstance(node, ast.ClassDef) and not node.name.startswith("_"):
            classes.append(node)

    def first(node: ast.AST) -> str:
        """説明の 1 文目（表に入れるので | と改行を除く）"""
        d = ast.get_docstring(node, clean=True) or ""  # type: ignore[arg-type]
        s = d.strip().split("\n\n")[0].replace("\n", " ")
        depth = 0
        for i, ch in enumerate(s):  # 括弧の外の最初の「。」か「. 」まで（説明は英語が元）
            depth += ch in "（("
            depth -= ch in "）)"
            if depth <= 0 and (ch == "。" or (ch == "." and s[i + 1 : i + 2] == " ")):
                s = s[: i + 1]
                break
        if len(s) > 90:
            s = s[:90] + "…"
        return s.replace("|", "&#124;")

    def members(cls: ast.ClassDef) -> list[ast.FunctionDef]:
        return [f for f in cls.body if isinstance(f, ast.FunctionDef)
                and (not f.name.startswith("_") or f.name in ("__new__", "__init__"))]

    def mid(cls: ast.ClassDef, fn: ast.FunctionDef) -> str:
        return f"{cls.name}-{'new' if fn.name in ('__new__', '__init__') else fn.name}".lower()

    # 目次: 関数とクラス（クラスはメソッド・属性へのリンクも）
    md.append("## 目次\n\n### 関数\n\n| 関数 | 説明 |\n|---|---|\n")
    for fn in funcs:
        md.append(f"| [`{fn.name}`](#fn-{fn.name.lower()}) | {first(fn)} |\n")
    md.append("\n### クラス\n\n| クラス | 説明 |\n|---|---|\n")
    for cls in classes:
        md.append(f"| [`{cls.name}`](#cls-{cls.name.lower()}) | {first(cls)} |\n")
    md.append("\n[定数](#定数)\n\n")

    md.append("## 定数\n\n| 名前 | 型 | 説明 |\n|---|---|---|\n")
    for name, ty, note in consts:
        md.append(f"| `{name}` | `{ty}` | {note} |\n")

    md.append("\n## 関数\n\n")
    for fn in funcs:
        md.append(f'<a id="fn-{fn.name.lower()}"></a>\n\n### {fn.name}\n\n```python\n{fn.name}{sig(fn, False)}\n```\n\n{doc(fn)}\n')

    md.append("## クラス\n")
    for cls in classes:
        ms = members(cls)
        md.append(f'\n<a id="cls-{cls.name.lower()}"></a>\n\n### {cls.name}\n\n{doc(cls)}\n')
        if ms:
            links = []
            for fn in ms:
                label = cls.name + "(…)" if fn.name in ("__new__", "__init__") else fn.name
                links.append(f"[`{label}`](#{mid(cls, fn)})")
            md.append("メソッド・属性: " + "・".join(links) + "\n\n")
        for fn in ms:
            anchor = f'<a id="{mid(cls, fn)}"></a>\n\n'
            if is_property(fn):
                ret = f": {ast.unparse(fn.returns)}" if fn.returns else ""
                md.append(f"{anchor}#### {cls.name}.{fn.name}\n\n```python\n{fn.name}{ret}  # 属性\n```\n\n{doc(fn)}\n")
            else:
                shown = cls.name if fn.name in ("__new__", "__init__") else f"{cls.name}.{fn.name}"
                md.append(f"{anchor}#### {shown}\n\n```python\n{shown}{sig(fn, True)}\n```\n\n{doc(fn)}\n")
    return "".join(md)


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--cli", help="lensbench の実行ファイル（省くと CLI のページは作らない）")
    a = p.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    if a.cli:
        (OUT / "cli.md").write_text(gen_cli(a.cli), encoding="utf-8")
    else:
        (OUT / "cli.md").write_text(NOTE + "# CLI リファレンス\n\n`scripts/gen_docs.py --cli <lensbench>` で作る。\n",
                                    encoding="utf-8")
    (OUT / "python-api.md").write_text(gen_python(), encoding="utf-8")


if __name__ == "__main__":
    main()
