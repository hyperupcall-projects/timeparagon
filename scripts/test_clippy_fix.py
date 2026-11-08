"""Safety checks for the file-scoped Clippy fixer (no Cargo build required)."""

import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "clippy_fix", Path(__file__).with_name("clippy-fix.py"))
fixer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixer)


def span(path, start, end, replacement, applicability="MachineApplicable"):
    return dict(file_name=str(path), byte_start=start, byte_end=end,
                suggested_replacement=replacement,
                suggestion_applicability=applicability)


class ClippyFixTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.selected = Path(self.temp.name) / "changed.rs"
        self.other = Path(self.temp.name) / "unchanged.rs"
        self.selected.write_bytes("// café\nlet x = true;\n".encode())
        self.other.write_bytes(b"let y = true;\n")
        self.original = self.selected.read_bytes()

    def apply(self, *groups):
        return fixer.apply_fixes(
            [{"children": [{"spans": group} for group in groups]}],
            {self.selected})

    def test_utf8_offsets_and_duplicate_targets(self):
        start = self.original.index(b"true")
        edit = span(self.selected, start, start + 4, "false")
        self.assertTrue(self.apply([edit], [edit]))
        self.assertEqual(self.selected.read_bytes(),
                         self.original.replace(b"true", b"false"))

    def test_does_not_edit_unselected_file(self):
        self.assertFalse(self.apply([span(self.other, 0, 3, "const")]))
        self.assertEqual(self.other.read_bytes(), b"let y = true;\n")

    def test_adjacent_edits_in_one_suggestion(self):
        start = self.original.index(b"true")
        self.assertTrue(self.apply([span(self.selected, start, start + 4, "false"),
                                   span(self.selected, start + 4, start + 5, "")]))
        self.assertEqual(self.selected.read_bytes(),
                         self.original.replace(b"true;", b"false"))

    def test_rejects_entire_cross_file_suggestion(self):
        self.assertFalse(self.apply([span(self.selected, 0, 2, "/*"),
                                     span(self.other, 0, 3, "const")]))
        self.assertEqual(self.selected.read_bytes(), self.original)

    def test_rejects_uncertain_multipart_suggestion(self):
        self.assertFalse(self.apply([span(self.selected, 0, 2, "/*"),
                                     span(self.selected, 3, 4, "x", "MaybeIncorrect")]))
        self.assertEqual(self.selected.read_bytes(), self.original)

    def test_overlapping_group_is_deferred_atomically(self):
        self.assertTrue(self.apply([span(self.selected, 0, 2, "/*")],
                                   [span(self.selected, 3, 4, "x"),
                                    span(self.selected, 0, 3, "// ")]))
        self.assertEqual(self.selected.read_bytes(), b"/*" + self.original[2:])

    def test_applies_multiple_edits_in_reverse_byte_order(self):
        self.assertTrue(self.apply([span(self.selected, 0, 2, "///"),
                                   span(self.selected, len(self.original),
                                        len(self.original), "// end\n")]))
        self.assertEqual(self.selected.read_bytes(),
                         b"///" + self.original[2:] + b"// end\n")


if __name__ == "__main__":
    unittest.main()
