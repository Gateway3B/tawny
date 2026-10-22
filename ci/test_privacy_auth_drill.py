import unittest

import privacy_auth_drill as drill


class CleanupTests(unittest.TestCase):
    def test_cleanup_uses_only_exact_created_resource_ids(self):
        calls = []
        container, network = "a" * 64, "b" * 64
        drill.cleanup(container, network, runner=lambda *args: calls.append(args))
        self.assertEqual(calls, [
            ("rm", "--force", "--volumes", container),
            ("network", "rm", network),
        ])

    def test_cleanup_refuses_names_and_still_removes_the_owned_network(self):
        calls = []
        with self.assertRaisesRegex(RuntimeError, "cleanup needs attention"):
            drill.cleanup("tawny-surrealdb-1", "b" * 64, runner=lambda *args: calls.append(args))
        self.assertEqual(calls, [("network", "rm", "b" * 64)])

    def test_cleanup_attempts_both_resources_even_when_one_fails(self):
        calls = []
        def failing(*args):
            calls.append(args)
            raise RuntimeError("private-fixture-canary")
        with self.assertRaisesRegex(RuntimeError, "^Synthetic database fixture cleanup needs attention$"):
            drill.cleanup("a" * 64, "b" * 64, runner=failing)
        self.assertEqual(len(calls), 2)
        drill.cleanup(None, None, runner=failing)
        self.assertEqual(len(calls), 2)
