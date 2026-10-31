"""Regression cases for source masking and the staged readability check."""

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location("rust_style", Path(__file__).with_name("rust_style.py"))
style = importlib.util.module_from_spec(spec)
spec.loader.exec_module(style)


class RustStyleTests(unittest.TestCase):
    def test_reports_setup_and_navigation_inside_rsx(self):
        source = "fn App() {\n    let state = use_signal(|| false);\n    rsx! { Button { onclick: move |_| g3_route_transitions::animated_navigate(route) } }\n}"
        self.assertEqual([line for line, _ in style.issues(source)], [3, 3])

    def test_accepts_separation_and_markup_only_components(self):
        source = "fn App() {\n    let state = use_signal(|| false);\n\n    rsx! { Child {} }\n}\nfn Child() {\n    rsx! { div {} }\n}"
        self.assertEqual(list(style.issues(source)), [])

    def test_ignores_comments_and_every_string_form(self):
        payload = "g3_route_transitions::animated_navigate(route)"
        for literal in [f'"{payload}"', f'b"{payload}"', f'c"{payload}"', f'r"{payload}"', f'r###"{payload}"###', f'br##"{payload}"##', f'cr#"{payload}"#', f'/* {payload} /* nested */ */', f'// {payload}\n']:
            with self.subTest(literal=literal):
                self.assertEqual(list(style.issues(literal)), [])

    def test_preserves_offsets_and_line_numbers_in_multiline_literals(self):
        source = 'r##"first\nrsx! {\n    ignored\n}"##;\ng3_route_transitions::animated_navigate(route);'
        masked = style.code_mask(source)
        self.assertEqual(len(masked), len(source))
        self.assertEqual([line for line, _ in style.issues(source)], [5])

    def test_characters_escapes_and_lifetimes(self):
        source = r'''let a = '\''; let b = '\u{1f600}'; let c = b'"'; fn f<'a>(x: &'a str) {}'''
        self.assertIn("fn f<'a>(x: &'a str)", style.code_mask(source))
        self.assertNotIn("1f600", style.code_mask(source))

    def test_escaped_quotes_do_not_end_strings(self):
        source = r'''let text = "quote: \" g3_route_transitions::animated_navigate(route)";'''
        self.assertEqual(list(style.issues(source)), [])

    def test_raw_identifier_is_code(self):
        self.assertIn("r#type", style.code_mask("let r#type = 1;"))

    def test_navigation_with_spaces(self):
        self.assertEqual(len(list(style.issues("g3_route_transitions :: animated_navigate (route)"))), 1)

    def test_staged_check_reads_index_instead_of_working_copy(self):
        # Use a workspace-owned scratch directory on restricted Windows hosts.
        scratch = Path.cwd() / "target"
        scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            root = Path(directory)
            self.assertTrue(root.resolve().is_relative_to(scratch.resolve()))
            (root / "scripts").mkdir()
            checker = root / "scripts" / "rust_style.py"
            checker.write_bytes(Path(style.__file__).read_bytes())
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            source = root / "app.rs"
            source.write_text("g3_route_transitions::animated_navigate(route);", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "app.rs"], check=True)
            source.write_text("animated_navigate(route);", encoding="utf-8")
            result = subprocess.run([sys.executable, str(checker), "--staged"], capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)
            self.assertIn("import animated_navigate", result.stdout)
            subprocess.run(["git", "-C", str(root), "add", "app.rs"], check=True)
            source.write_text("g3_route_transitions::animated_navigate(route);", encoding="utf-8")
            result = subprocess.run([sys.executable, str(checker), "--staged"], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
