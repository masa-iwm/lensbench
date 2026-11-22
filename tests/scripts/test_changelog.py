"""scripts/changelog.py の試験（python3 -m unittest discover -s tests/scripts）"""
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def _load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


cl = _load("changelog")
rn = _load("release_notes")

LOG = """# 変更の記録

前書き

## 0.9.0（開発中）

0.8.0 からの変更。

### 結果が変わる所

- **前からある**: 説明
  続き

### 直した所

- **前の直し**: 説明

## 0.8.0（2026-11-22）

### 足したもの

- 古い
"""


class Fragments(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name) / "changelog.d"
        self.dir.mkdir()
        (self.dir / "README.md").write_text("# 説明\n", encoding="utf-8")

    def tearDown(self):
        self.tmp.cleanup()

    def put(self, name, text):
        (self.dir / name).write_text(text, encoding="utf-8")

    def test_check_names_and_body(self):
        self.put("12-a.fixed.md", "- **a**: 直した\n")
        self.put("bad.md", "- x\n")
        self.put("13-b.unknown.md", "- x\n")
        self.put("14-c.added.md", "説明だけ\n")
        self.put("15-d.added.md", "- x\n### 見出し\n")
        frags, errors = cl.fragments(self.dir)
        self.assertEqual([p.name for p, _ in frags["fixed"]], ["12-a.fixed.md"])
        self.assertEqual(len(errors), 4)

    def test_order_numeric(self):
        self.put("100-z.added.md", "- c\n")
        self.put("9-y.added.md", "- b\n")
        self.put("1000-x.added.md", "- d\n")
        self.put("note.added.md", "- e\n")
        frags, _ = cl.fragments(self.dir)
        self.assertEqual([t for _, t in frags["added"]], ["- b", "- c", "- d", "- e"])

    def test_render(self):
        self.put("2-a.fixed.md", "- 直し\n")
        self.put("1-b.breaking.md", "- 壊す\n  続き\n")
        frags, _ = cl.fragments(self.dir)
        self.assertEqual(cl.render(frags), "### 互換が切れる所\n\n- 壊す\n  続き\n\n### 直した所\n\n- 直し")

    def test_collect(self):
        log = Path(self.tmp.name) / "CHANGELOG.md"
        log.write_text(LOG, encoding="utf-8")
        self.put("5-new.fixed.md", "- **新しい直し**: 説明\n")
        self.put("6-add.added.md", "- **足した**: 説明\n")
        self.put("7-br.breaking.md", "- **壊す**: 説明\n")
        self.assertEqual(cl.collect(log, self.dir, "0.9.0", "2026-12-01"), 0)
        got = log.read_text(encoding="utf-8")
        want = """# 変更の記録

前書き

## 0.9.0（2026-12-01）

0.8.0 からの変更。

### 互換が切れる所

- **壊す**: 説明

### 結果が変わる所

- **前からある**: 説明
  続き

### 足したもの

- **足した**: 説明

### 直した所

- **前の直し**: 説明
- **新しい直し**: 説明

## 0.8.0（2026-11-22）

### 足したもの

- 古い
"""
        self.assertEqual(got, want)
        self.assertEqual(sorted(p.name for p in self.dir.iterdir()), ["README.md"])

    def test_collect_refuses_bad_fragment(self):
        log = Path(self.tmp.name) / "CHANGELOG.md"
        log.write_text(LOG, encoding="utf-8")
        self.put("5-new.fixed.md", "直した\n")
        self.assertEqual(cl.collect(log, self.dir, "0.9.0", None), 1)
        self.assertEqual(log.read_text(encoding="utf-8"), LOG)
        self.assertTrue((self.dir / "5-new.fixed.md").exists())

    def test_release_notes_include_fragments(self):
        self.put("5-new.fixed.md", "- **新しい直し**: 説明\n")
        rc = rn.notes("v0.9.0-rc.1", LOG, self.dir)
        self.assertIn("- **前の直し**: 説明\n- **新しい直し**: 説明", rc)
        # いちばん上でない版の節には足さない
        self.assertEqual(rn.notes("v0.8.0", LOG, self.dir), "### 足したもの\n\n- 古い\n")
        # 断片の無いときは前と同じ
        empty = Path(self.tmp.name) / "none"
        self.assertEqual(rn.notes("v0.9.0", LOG, empty), rn.notes("v0.9.0", LOG))

    def test_real_fragments(self):
        _frags, errors = cl.fragments(ROOT / "changelog.d")
        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
