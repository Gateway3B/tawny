import subprocess
import unittest
from unittest.mock import Mock

from privacy_linux_drill import cleanup, identity


class LinuxFixtureTests(unittest.TestCase):
    def test_cleanup_uses_only_owned_ids_and_removes_the_network_last(self):
        invoke = Mock(return_value=subprocess.CompletedProcess([], 0))
        cleanup(["a" * 64, "b" * 64], "c" * 64, invoke)
        self.assertEqual([call.args[0] for call in invoke.call_args_list], [
            ["docker", "rm", "--force", "--volumes", "b" * 64],
            ["docker", "rm", "--force", "--volumes", "a" * 64],
            ["docker", "network", "rm", "c" * 64],
        ])

    def test_failed_cleanup_still_attempts_the_other_owned_resources(self):
        for failure in (subprocess.TimeoutExpired("docker", 30),
                        subprocess.CompletedProcess([], 1), OSError("unavailable")):
            invoke = Mock(side_effect=[failure, subprocess.CompletedProcess([], 0), subprocess.CompletedProcess([], 0)])
            with self.assertRaisesRegex(RuntimeError, "^Owned fixture cleanup failed$"):
                cleanup(["a" * 64, "b" * 64], "c" * 64, invoke)
            self.assertEqual(invoke.call_count, 3)

    def test_invalid_ids_are_never_used_for_cleanup(self):
        invoke = Mock(return_value=subprocess.CompletedProcess([], 0))
        with self.assertRaisesRegex(RuntimeError, "^Owned fixture cleanup failed$"):
            cleanup(["user-container-name", "a" * 64], "../network", invoke)
        self.assertEqual([call.args[0] for call in invoke.call_args_list], [
            ["docker", "rm", "--force", "--volumes", "a" * 64],
        ])
        cleanup([], None, invoke)
        self.assertEqual(invoke.call_count, 1)

    def test_only_exact_create_ids_are_accepted_as_owned(self):
        for value in (b"other-container", b"", b"0" * 65, b"../container"):
            with self.assertRaisesRegex(RuntimeError, "^Unknown owned Docker resource$"):
                identity(value)
        self.assertEqual(identity(b"a" * 64 + b"\n"), "a" * 64)


if __name__ == "__main__":
    unittest.main()
