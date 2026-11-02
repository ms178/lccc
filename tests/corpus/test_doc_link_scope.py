"""Repository docs stay checked without accepting vendor or scratch decoys."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("doc_links", ROOT / "scripts/check_doc_links.py")
doc_links = importlib.util.module_from_spec(spec)
spec.loader.exec_module(doc_links)


class DocLinkScope(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.patch_root = mock.patch.object(doc_links, "ROOT", str(self.root))
        self.patch_segments = mock.patch.object(doc_links, "ROOT_SEGMENTS", {"docs", "src"})
        self.patch_root.start()
        self.patch_segments.start()
        self.addCleanup(self.patch_root.stop)
        self.addCleanup(self.patch_segments.stop)

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def test_only_root_scratch_is_excluded(self):
        self.write(".cache/vendor/README.md", "[lost](missing.md)\n")
        self.write("work/report.md", "[lost](missing.md)\n")
        nested = self.write("docs/work/design.md", "[lost](missing.md)\n")
        new = self.write("docs/new.md", "[lost](missing.md)\n")
        self.assertEqual(set(map(Path, doc_links.markdown_files())), {nested, new})
        self.assertEqual(doc_links.check_file(str(nested)), [(1, "missing.md")])
        self.assertEqual(doc_links.check_file(str(new)), [(1, "missing.md")])

    def test_vendor_basename_cannot_bless_a_broken_project_link(self):
        self.write(".cache/vendor/only.md", "vendor\n")
        project = self.write("docs/guide.md", "[lost](only.md)\n")
        self.assertEqual(doc_links.check_file(str(project)), [(1, "only.md")])

    def test_scratch_citations_are_not_live_markdown_links(self):
        project = self.write("docs/guide.md", "Historical output: `work/regression.json`.\n"
                             "[must exist](../work/missing.json)\n")
        self.assertEqual(doc_links.check_file(str(project)), [(2, "../work/missing.json")])

    def test_project_backtick_paths_and_valid_links_still_resolve(self):
        self.write("src/real.c", "int value;\n")
        project = self.write("docs/guide.md", "`src/real.c` and `src/missing.c`\n"
                             "[real](../src/real.c)\n")
        self.assertEqual(doc_links.check_file(str(project)), [(1, "src/missing.c")])


if __name__ == "__main__":
    unittest.main()
