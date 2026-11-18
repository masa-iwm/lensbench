"""scripts/docs_versions.py の試験（python3 -m unittest discover -s tests/scripts）"""
import importlib.util
import json
import tarfile
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("docs_versions", ROOT / "scripts" / "docs_versions.py")
dv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dv)


def site(d: Path, mark: str) -> Path:
    (d / "schema").mkdir(parents=True)
    (d / "index.html").write_text(mark)
    (d / "schema" / "project.schema.json").write_text(json.dumps({"mark": mark}))
    (d / "api" / "rust").mkdir(parents=True)
    (d / "api" / "rust" / "index.html").write_text("api")
    (d / "app").mkdir()
    (d / "app" / "index.html").write_text("app")
    return d


class DocsVersions(unittest.TestCase):
    def test_snapshot_assemble(self):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            snaps = t / "snaps"
            for tag in ["v0.5.0", "v0.5.1", "v0.6.0"]:
                dv.snapshot(site(t / tag, tag), snaps / f"lensbench-docs-{tag}.tar.gz")
            (snaps / "other.zip").write_text("")
            main = site(t / "main", "main")
            idx = dv.assemble(main, snaps, "0.7.0-dev")
            self.assertEqual(idx["dev"], "0.7.0-dev")
            self.assertFalse(idx["dev_released"])
            self.assertEqual([(x["version"], x["tag"], x["path"]) for x in idx["versions"]],
                             [("0.6", "v0.6.0", "v0.6/"), ("0.5", "v0.5.1", "v0.5/")])
            self.assertEqual(json.loads((main / "versions.json").read_text()), idx)
            # 直しの番号の大きい方、/api/・/app/ は入れない
            self.assertEqual((main / "v0.5" / "index.html").read_text(), "v0.5.1")
            self.assertFalse((main / "v0.5" / "api").exists())
            self.assertFalse((main / "v0.5" / "app").exists())
            self.assertEqual((main / "index.html").read_text(), "main")
            # スキーマ: リリースの版はその添付から、リリース前の main の版は main から
            sch = lambda v: json.loads((main / "schema" / v / "project.schema.json").read_text())["mark"]
            self.assertEqual(sch("v0.5"), "v0.5.1")
            self.assertEqual(sch("v0.6"), "v0.6.0")
            self.assertEqual(sch("v0.7"), "main")
            # main の版がリリース済みの版と同じなら、リリースのスキーマを使う（main で上書きしない）
            main2 = site(t / "main2", "main2")
            self.assertTrue(dv.assemble(main2, snaps, "0.6.0")["dev_released"])
            self.assertEqual(json.loads((main2 / "schema" / "v0.6" / "project.schema.json").read_text())["mark"], "v0.6.0")

    def test_schema_from_tag_tree(self):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            # GitHub の tarball の形（<リポジトリ>-<コミット>/ の下）
            top = t / "repo" / "o-lensbench-abc"
            (top / "docs" / "schema").mkdir(parents=True)
            (top / "docs" / "schema" / "project.schema.json").write_text(json.dumps({"mark": "tree"}))
            (top / "docs" / "schema" / "README.md").write_text("x")
            (top / "src").mkdir()
            (top / "src" / "a.rs").write_text("x")
            with tarfile.open(t / "repo.tgz", "w:gz") as tf:
                tf.add(top, arcname=top.name)
            snaps = t / "snaps"
            dv.pack_schema(t / "repo.tgz", snaps / "lensbench-schema-v0.5.1.tar.gz")
            with tarfile.open(snaps / "lensbench-schema-v0.5.1.tar.gz") as tf:
                self.assertEqual(tf.getnames(), ["project.schema.json"])
            # v0.5.0 はドキュメントもあり、v0.5.1 はスキーマだけ（CI の成果物が取れなかった）
            dv.snapshot(site(t / "v0.5.0", "v0.5.0"), snaps / "lensbench-docs-v0.5.0.tar.gz")
            # v0.4.0 はスキーマだけ
            dv.pack_schema(t / "repo.tgz", snaps / "lensbench-schema-v0.4.0.tar.gz")
            main = site(t / "main", "main")
            idx = dv.assemble(main, snaps, "0.6.0-dev")
            self.assertEqual([x["tag"] for x in idx["versions"]], ["v0.5.0"])
            self.assertEqual((main / "v0.5" / "index.html").read_text(), "v0.5.0")
            sch = lambda v: json.loads((main / "schema" / v / "project.schema.json").read_text())["mark"]
            self.assertEqual(sch("v0.5"), "tree")
            self.assertEqual(sch("v0.4"), "tree")
            self.assertEqual(sch("v0.6"), "main")
            self.assertFalse((main / "v0.4").exists())

    def test_no_snapshots(self):
        with tempfile.TemporaryDirectory() as t:
            main = site(Path(t) / "main", "main")
            idx = dv.assemble(main, Path(t) / "none", "0.5.0-dev")
            self.assertEqual(idx, {"dev": "0.5.0-dev", "dev_released": False, "versions": []})
            self.assertTrue((main / "schema" / "v0.5" / "project.schema.json").is_file())


if __name__ == "__main__":
    unittest.main()
