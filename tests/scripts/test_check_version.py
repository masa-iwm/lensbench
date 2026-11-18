"""scripts/check_version.py の試験（python3 -m unittest discover -s tests/scripts）"""
import importlib.util
import os
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("check_version", ROOT / "scripts" / "check_version.py")
cv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cv)


class CheckVersion(unittest.TestCase):
    def test_order(self):
        vs = ["0.4.0", "0.5.0-dev", "0.5.0-rc.1", "0.5.0-rc.2", "0.5.0-rc.10", "0.5.0", "0.5.1", "0.10.0", "1.0.0-dev"]
        self.assertEqual(sorted(reversed(vs), key=cv.key), vs)

    def test_check(self):
        tags = ["v0.3.0", "v0.4.0", "v0.4.1", "other"]
        self.assertEqual(cv.check("0.5.0-dev", tags, []), [])
        self.assertEqual(cv.check("0.4.2", tags, []), [])
        self.assertTrue(cv.check("0.4.1", tags, []))  # タグの後に上げていない
        self.assertTrue(cv.check("0.4.1-dev", tags, []))
        self.assertTrue(cv.check("0.5", tags, []))  # 形が違う
        self.assertEqual(cv.check("0.4.1", tags, ["v0.4.1"]), [])  # タグのコミット
        self.assertTrue(cv.check("0.5.0-dev", tags, ["v0.4.1"]))
        self.assertEqual(cv.check("0.5.0-dev", [], []), [])  # タグが無い
        self.assertEqual(cv.check("0.5.0", tags + ["v0.5.0-rc.1"], []), [])
        # リリース候補のタグの後に main を開発中の名前へ戻してよい（リリース候補は比べる相手にしない）
        self.assertEqual(cv.check("0.5.0-dev", tags + ["v0.5.0-rc.1", "v0.5.0-rc.2"], []), [])
        self.assertEqual(cv.check("0.5.0-rc.1", tags + ["v0.5.0-rc.1"], ["v0.5.0-rc.1"]), [])  # rc のタグのコミット
        # 開発中の版のままリリース候補のタグを打てる
        self.assertEqual(cv.check("0.5.0-dev", tags + ["v0.5.0-rc.1"], ["v0.5.0-rc.1"]), [])
        self.assertTrue(cv.check("0.5.0-dev", tags + ["v0.6.0-rc.1"], ["v0.6.0-rc.1"]))
        self.assertTrue(cv.check("0.5.0-dev", tags + ["v0.5.0"], ["v0.5.0"]))

    def test_tag_matches(self):
        self.assertTrue(cv.tag_matches("0.5.0", "0.5.0"))
        self.assertTrue(cv.tag_matches("0.5.0-rc.1", "0.5.0-dev"))
        self.assertTrue(cv.tag_matches("0.5.0-rc.12", "0.5.0-dev"))
        self.assertTrue(cv.tag_matches("0.5.0-rc.1", "0.5.0-rc.1"))
        self.assertFalse(cv.tag_matches("0.5.0", "0.5.0-dev"))  # リリースは版を上げてから
        self.assertFalse(cv.tag_matches("0.5.1-rc.1", "0.5.0-dev"))
        self.assertFalse(cv.tag_matches("0.5.0-rc", "0.5.0-dev"))
        self.assertFalse(cv.tag_matches("0x5x0-rc.1", "0.5.0-dev"))
        self.assertFalse(cv.tag_matches("0.5.0-rc.1", "0.5.0"))

    def test_tag_mode_cp1252_stdout(self):
        # Windows の Python の標準出力（cp1252）でも日本語の出力で落ちず、終了コードが判定どおり
        version = cv.tomllib.load(open(cv.ROOT / "Cargo.toml", "rb"))["workspace"]["package"]["version"]
        env = dict(os.environ, PYTHONIOENCODING="cp1252")
        script = str(ROOT / "scripts" / "check_version.py")
        ok = subprocess.run([sys.executable, script, "--tag", f"v{version}"], env=env, capture_output=True)
        self.assertEqual(ok.returncode, 0, ok.stderr)
        bad = subprocess.run([sys.executable, script, "--tag", "v0.0.0-x"], env=env, capture_output=True)
        self.assertEqual(bad.returncode, 1, bad.stderr)


if __name__ == "__main__":
    unittest.main()
