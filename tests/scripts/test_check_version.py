"""scripts/check_version.py の試験（python3 -m unittest discover -s tests/scripts）"""
import importlib.util
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


if __name__ == "__main__":
    unittest.main()
