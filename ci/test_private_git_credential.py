"""Exercise the real helper protocol using disposable, synthetic access only."""
from pathlib import Path
import os
import shutil
import subprocess
import unittest


def shell():
    found = shutil.which("sh")
    if found:
        return found
    git = shutil.which("git")
    candidate = Path(git).resolve().parents[1] / "bin" / "sh.exe" if git else None
    if candidate and candidate.is_file():
        return str(candidate)
    raise RuntimeError("A POSIX shell is required for the helper checks")


class CredentialScope(unittest.TestCase):
    def run_helper(self, path="g3techhq/g3-brand", protocol="https", host="github.com", action="get", credential=True):
        environment = os.environ.copy()
        environment.pop("G3_BRAND_READ_TOKEN", None)
        if credential:
            environment["G3_BRAND_READ_TOKEN"] = "synthetic-scoped-credential"
        result = subprocess.run(
            [shell(), str(Path(__file__).with_name("private-git-credential.sh")), action],
            input=f"protocol={protocol}\nhost={host}\npath={path}\n\n".encode(),
            capture_output=True, env=environment, timeout=5, check=True,
        )
        self.assertEqual(result.stderr, b"")
        return result.stdout.decode()

    def test_only_the_exact_private_repository_receives_access(self):
        for path in ["g3techhq/g3-brand", "g3techhq/g3-brand.git"]:
            self.assertEqual(self.run_helper(path=path), "username=x-access-token\npassword=synthetic-scoped-credential\n\n")

    def test_other_repositories_and_similar_paths_receive_nothing(self):
        for path in ["gateway3b/tawny", "g3techhq/g3-brand-evil", "g3techhq/g3-brand/submodule", "other/g3-brand", "", "g3techhq/g3-brand.git/extra"]:
            self.assertEqual(self.run_helper(path=path), "")

    def test_other_protocols_and_hosts_receive_nothing(self):
        for protocol, host in [("http", "github.com"), ("https", "github.com.evil.invalid"), ("https", "other.invalid"), ("ssh", "github.com")]:
            self.assertEqual(self.run_helper(protocol=protocol, host=host), "")

    def test_store_and_erase_return_nothing(self):
        for action in ["store", "erase", ""]:
            self.assertEqual(self.run_helper(action=action), "")

    def test_missing_access_returns_nothing(self):
        self.assertEqual(self.run_helper(credential=False), "")


if __name__ == "__main__":
    unittest.main()
