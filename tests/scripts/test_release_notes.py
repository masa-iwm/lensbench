"""scripts/release_notes.py の試験（python3 -m unittest discover -s tests/scripts）"""
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("release_notes", ROOT / "scripts" / "release_notes.py")
rn = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rn)

LOG = """# 変更の記録

## 0.5.10

十番

## 0.5.1

直し

## 0.5.0（開発中）

### 足したもの

- いろいろ

## 0.4.0 以前

前
"""


class ReleaseNotes(unittest.TestCase):
    def test_section(self):
        self.assertEqual(rn.section(LOG, "0.5.0"), "### 足したもの\n\n- いろいろ")
        self.assertEqual(rn.section(LOG, "0.5.1"), "直し")
        self.assertIsNone(rn.section(LOG, "0.6.0"))

    def test_notes(self):
        self.assertEqual(rn.notes("v0.5.1", LOG), "直し\n")
        rc = rn.notes("v0.5.0-rc.2", LOG)
        self.assertTrue(rc.startswith("0.5.0 のリリース候補"))
        self.assertIn("- いろいろ", rc)
        self.assertIn(rn.CHANGELOG_URL, rn.notes("v0.6.0", LOG))
        self.assertIn(rn.CHANGELOG_URL, rn.notes("vfoo", LOG))

    def test_real_changelog(self):
        text = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
        self.assertIn("### 互換が切れる所", rn.notes("v0.5.0-rc.3", text))


if __name__ == "__main__":
    unittest.main()
