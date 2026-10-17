"""Workflow Cargo selectors must identify actual workspace test targets."""
import unittest

from scripts.check_release import check_test_command


class WorkflowTargetTests(unittest.TestCase):
    def test_workspace_and_explicit_package_selection_preserve_target_ownership(self):
        targets = {"cli": {"campaign_harness"}, "engine": {"revision_receipts"}}
        for argv in (
            ["--workspace", "--test", "campaign_harness", "--test", "revision_receipts"],
            ["-p", "cli", "-p", "engine", "--test", "campaign_harness", "--test", "revision_receipts"],
            ["--package", "engine", "--test", "revision_receipts"],
        ):
            with self.subTest(argv=argv):
                check_test_command(argv, targets)
        for argv in (
            ["--workspace", "--test", "misspelled"],
            ["-p", "cli", "--test", "revision_receipts"],
            ["--workspace", "--exclude", "engine", "--test", "revision_receipts"],
        ):
            with self.subTest(argv=argv), self.assertRaises(AssertionError):
                check_test_command(argv, targets)


if __name__ == "__main__":
    unittest.main()
