"""Regression tests for check-cargo-release-age.py."""

import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("check-cargo-release-age.py")
SPEC = importlib.util.spec_from_file_location("check_cargo_release_age", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CheckCargoReleaseAgeTests(unittest.TestCase):
    def test_pairs_a_renamed_lockfile_with_its_base_path(self):
        completed = [
            subprocess.CompletedProcess(
                [], 0, stdout="clovy-api/Cargo.lock\nsrc-tauri/Cargo.lock\n", stderr=""
            ),
            subprocess.CompletedProcess(
                [], 0, stdout="june-api/Cargo.lock\nsrc-tauri/Cargo.lock\n", stderr=""
            ),
            subprocess.CompletedProcess(
                [],
                0,
                stdout="R099\tjune-api/Cargo.lock\tclovy-api/Cargo.lock\n",
                stderr="",
            ),
        ]
        with patch.object(MODULE.subprocess, "run", side_effect=completed):
            lockfiles = MODULE.discover_lockfiles("origin/base")

        self.assertEqual(
            lockfiles,
            [
                ("clovy-api/Cargo.lock", "june-api/Cargo.lock"),
                ("src-tauri/Cargo.lock", "src-tauri/Cargo.lock"),
            ],
        )

    def test_keeps_a_true_lockfile_deletion_fail_closed(self):
        completed = [
            subprocess.CompletedProcess(
                [], 0, stdout="src-tauri/Cargo.lock\n", stderr=""
            ),
            subprocess.CompletedProcess(
                [], 0, stdout="retired/Cargo.lock\nsrc-tauri/Cargo.lock\n", stderr=""
            ),
            subprocess.CompletedProcess(
                [], 0, stdout="D\tretired/Cargo.lock\n", stderr=""
            ),
        ]
        with patch.object(MODULE.subprocess, "run", side_effect=completed):
            lockfiles = MODULE.discover_lockfiles("origin/base")

        self.assertIn(("retired/Cargo.lock", "retired/Cargo.lock"), lockfiles)

    def test_a_deleted_lockfile_is_retired_only_with_its_manifest(self):
        manifests = ["src-tauri/Cargo.toml", "kept/Cargo.toml"]
        # The whole Rust tree is gone: nothing can resolve crates from it.
        self.assertTrue(MODULE.lockfile_tree_retired("clovy-api/Cargo.lock", manifests))
        # The manifest is still tracked: losing its lock stays fail-closed.
        self.assertFalse(MODULE.lockfile_tree_retired("kept/Cargo.lock", manifests))
        self.assertTrue(MODULE.lockfile_tree_retired("Cargo.lock", manifests))
        self.assertFalse(MODULE.lockfile_tree_retired("Cargo.lock", ["Cargo.toml"]))

    def test_checks_every_manifest_with_locked_metadata(self):
        completed = subprocess.CompletedProcess([], 0, stdout="{}", stderr="")
        with patch.object(MODULE.subprocess, "run", return_value=completed) as run:
            count = MODULE.verify_manifests_locked(
                ["clovy-api/Cargo.toml", "src-tauri/Cargo.toml"]
            )
        self.assertEqual(count, 2)
        self.assertEqual(run.call_count, 2)
        for call in run.call_args_list:
            self.assertIn("--locked", call.args[0])
            self.assertIn("--manifest-path", call.args[0])

    def test_new_manifest_without_lock_fails_closed(self):
        failed = subprocess.CompletedProcess(
            [], 101, stdout="", stderr="the lock file needs to be updated"
        )
        with patch.object(MODULE.subprocess, "run", return_value=failed):
            with self.assertRaisesRegex(
                RuntimeError, "probe/Cargo.toml is not locked"
            ):
                MODULE.verify_manifests_locked(["probe/Cargo.toml"])


if __name__ == "__main__":
    unittest.main()
