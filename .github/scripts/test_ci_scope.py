import unittest
from unittest.mock import mock_open, patch

from ci_scope import main, requires_rust


class ScopeTests(unittest.TestCase):
    def test_frontend_changes_with_release_notes(self):
        self.assertFalse(requires_rust([
            "app/src/components/report/Canvas.svelte",
            "app/src/lib/client.ts",
            "app/src/app.css",
            "app/static/icon.svg",
            "app/tests/markdown.test.ts",
            "app/package-lock.json",
            "app/vite.config.ts",
            "CHANGELOG.md",
            "docs/DEVLOG.md",
        ]))

    def test_generated_contracts_keep_rust_validation(self):
        for path in ("app/src/lib/ops.gen.ts", "app/src/lib/bindings.ts"):
            with self.subTest(path=path):
                self.assertTrue(requires_rust(["app/src/App.svelte", path]))

    def test_core_dependencies_fixtures_and_unknown_paths_keep_rust(self):
        for path in (
            "Cargo.lock",
            "Cargo.toml",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "crates/pbi-ai/prompts/explain.md",
            "reference/examples/acme-outfitters/expected.json",
            "app/src-tauri/src/lib.rs",
            ".github/workflows/ci.yml",
            ".github/scripts/ci_scope.py",
            "scripts/license-inventory.py",
            "docs/COMMANDS.md",
            "docs/AI.md",
            "THIRD_PARTY_NOTICES.md",
            "app/new-build-config.json",
        ):
            with self.subTest(path=path):
                self.assertTrue(requires_rust(["app/src/App.svelte", path]))

    def test_empty_or_docs_only_ranges_default_to_full_validation(self):
        self.assertTrue(requires_rust([]))
        self.assertTrue(requires_rust(["docs/DEVLOG.md"]))

    def test_renamed_core_file_is_not_hidden_by_frontend_destination(self):
        self.assertTrue(requires_rust([
            "crates/pbi-model/src/example.rs",
            "app/src/example.ts",
        ]))

    def test_missing_app_cannot_skip_both_validation_jobs(self):
        output = mock_open()
        with (
            patch.dict("os.environ", {"BASE_SHA": "a" * 40, "GITHUB_OUTPUT": "unused"}, clear=True),
            patch("ci_scope.changed_paths", return_value=["app/package.json"]),
            patch("ci_scope.Path.is_file", return_value=False),
            patch("builtins.open", output),
            patch("builtins.print"),
        ):
            main()
        output().write.assert_called_once_with("rust=true\napp=false\n")


if __name__ == "__main__":
    unittest.main()
