"""Web 版の日本語フォントを作る: Noto Sans JP（SIL OFL 1.1）を太さ 400 に固定し、
ASCII・Latin-1・ギリシャ文字・記号・JIS X 0208（第 1・第 2 水準）と画面の言語表の字に絞る。

    python scripts/web_font.py OUT.ttf

要るもの: fonttools（pip install fonttools）。元のフォントは google/fonts から取る（取ったものは
target/web-font に残して使い回す）。
"""
import pathlib
import sys
import urllib.request

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

URL = "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf"
LICENSE = "https://raw.githubusercontent.com/google/fonts/main/ofl/notosansjp/OFL.txt"
ROOT = pathlib.Path(__file__).resolve().parent.parent


def fetch(url: str, path: pathlib.Path) -> pathlib.Path:
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(url) as r:
            path.write_bytes(r.read())
    return path


def chars() -> set[int]:
    cs = set(range(0x20, 0x7F)) | set(range(0xA0, 0x100))
    cs |= set(range(0x370, 0x400))  # ギリシャ文字（λ・μ・θ など）
    cs |= set(range(0x2000, 0x2BFF))  # 句読点・上付き・矢印・数学記号・罫線・図形
    cs |= set(range(0x3000, 0x3100))  # 和文の記号・ひらがな・カタカナ
    cs |= set(range(0xFF00, 0xFFF0))  # 全角
    # JIS X 0208（EUC-JP の 2 バイトの範囲）
    for hi in range(0xA1, 0xFF):
        for lo in range(0xA1, 0xFF):
            try:
                cs.add(ord(bytes([hi, lo]).decode("euc_jp")))
            except UnicodeDecodeError:
                pass
    # 画面の言語表・ソースの字（第 2 水準の外の字も落とさない）
    for p in list((ROOT / "crates/lensbench-gui").rglob("*.rs")) + list(
        (ROOT / "crates/lensbench-gui/lang").glob("*")
    ):
        if p.is_file():
            cs |= {ord(c) for c in p.read_text(encoding="utf-8", errors="ignore")}
    return cs


def main() -> None:
    out = pathlib.Path(sys.argv[1])
    cache = ROOT / "target/web-font"
    src = fetch(URL, cache / "NotoSansJP-wght.ttf")
    fetch(LICENSE, cache / "OFL.txt")
    f = TTFont(src)
    f = instancer.instantiateVariableFont(f, {"wght": 400})
    opt = subset.Options()
    opt.layout_features = ["*"]
    opt.name_IDs = ["*"]
    opt.notdef_outline = True
    s = subset.Subsetter(opt)
    s.populate(unicodes=chars())
    s.subset(f)
    out.parent.mkdir(parents=True, exist_ok=True)
    f.save(out)
    (out.parent / "OFL.txt").write_bytes((cache / "OFL.txt").read_bytes())
    print(f"{out}: {out.stat().st_size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
