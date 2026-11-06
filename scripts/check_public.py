#!/usr/bin/env python3
"""公開リポジトリに置いてはいけないファイルが追跡されていないかを調べる。

- 拡張子: ZMF・ZEC・ZDA・ZAR（Zemax のカタログ、暗号化膜、アーカイブ）
- AGF: tests/glass/ 以下の、出典を書いたテスト用カタログだけを許す
- COATING.DAT などメーカー・Zemax 由来の名前
使い方: python3 scripts/check_public.py [--history]
  --history を付けると、全コミットの全ファイル名を調べる（公開前の検査）。
"""
import subprocess, sys, re

DENY_EXT = re.compile(r"\.(zmf|zec|zda|zar)$", re.I)
AGF = re.compile(r"\.agf$", re.I)
AGF_OK = re.compile(r"^tests/glass/[^/]+\.agf$", re.I)
DENY_NAME = re.compile(r"(^|/)(coating\.dat)$", re.I)

def files(history):
    if history:
        out = subprocess.run(["git", "log", "--all", "--pretty=format:", "--name-only"],
                             capture_output=True, text=True, check=True).stdout
    else:
        out = subprocess.run(["git", "ls-files"], capture_output=True, text=True, check=True).stdout
    return sorted({l.strip() for l in out.splitlines() if l.strip()})

def main():
    bad = []
    for f in files("--history" in sys.argv):
        if DENY_EXT.search(f) or DENY_NAME.search(f) or (AGF.search(f) and not AGF_OK.match(f)):
            bad.append(f)
    if bad:
        print("公開できないファイルがあります:")
        for f in bad:
            print("  " + f)
        return 1
    print("OK: 公開できないファイルは見つかりませんでした")
    return 0

if __name__ == "__main__":
    sys.exit(main())
