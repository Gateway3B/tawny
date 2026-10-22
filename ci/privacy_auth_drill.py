"""Exercise the real WebSocket transaction protocol in an owned memory database.

Run from outside any sibling Cargo patch scope to verify the portable lockfile.
No application configuration, host database volume or production endpoint is used.
"""
from pathlib import Path
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid

IMAGE = "surrealdb/surrealdb:v3.2.4"
ROOT = Path(__file__).resolve().parents[1]
TEST = "db::access::tests::database_editor_can_erase_without_access_to_other_databases_over_websocket"


def docker(*args):
    result = subprocess.run(
        ["docker", *args], capture_output=True, text=True, timeout=90, check=False
    )
    if result.returncode:
        raise RuntimeError("Synthetic database fixture operation failed")
    return result.stdout.strip()


def cleanup(container_id, network_id, runner=docker):
    failures = []
    for kind, resource in [("container", container_id), ("network", network_id)]:
        if resource is None:
            continue
        if not re.fullmatch(r"[0-9a-f]{64}", resource):
            failures.append(kind)
            continue
        try:
            if kind == "container":
                runner("rm", "--force", "--volumes", resource)
            else:
                runner("network", "rm", resource)
        except (RuntimeError, subprocess.SubprocessError):
            failures.append(kind)
    if failures:
        raise RuntimeError("Synthetic database fixture cleanup needs attention")


def main():
    container_id = network_id = None
    try:
        try:
            image_id = docker("image", "inspect", IMAGE, "--format", "{{.Id}}")
        except RuntimeError:
            docker("pull", IMAGE)
            image_id = docker("image", "inspect", IMAGE, "--format", "{{.Id}}")
        if not re.fullmatch(r"sha256:[0-9a-f]{64}", image_id):
            raise RuntimeError("Synthetic database image could not be verified")
        suffix = uuid.uuid4().hex
        # Docker Desktop does not publish ports on internal networks. Bind only
        # to host loopback and deny outbound connections in SurrealDB itself.
        network_id = docker("network", "create", "--opt", "com.docker.network.bridge.host_binding_ipv4=127.0.0.1", "--label",
                            "net.g3tech.synthetic.privacy-auth=true", f"tawny-privacy-auth-{suffix}")
        container_id = docker(
            "create", "--name", f"tawny-privacy-auth-{suffix}",
            "--label", "net.g3tech.synthetic.privacy-auth=true",
            "--network", network_id, "--publish", "127.0.0.1::8000",
            "--read-only", "--tmpfs", "/tmp:rw,noexec,nosuid,size=64m",
            "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--pids-limit", "128", "--memory", "512m", "--cpus", "1",
            "--user", "65534:65534", "--log-driver", "none", image_id,
            "start", "--bind", "0.0.0.0:8000", "--user", "fixture-root",
            "--pass", "synthetic-privacy-fixture-password", "memory",
            "--deny-net", "--deny-scripting",
        )
        docker("start", container_id)
        published = docker("port", container_id, "8000/tcp")
        match = re.fullmatch(r"127\.0\.0\.1:([0-9]+)", published)
        if match is None or not 1 <= int(match[1]) <= 65535:
            raise RuntimeError("Synthetic database listener could not be verified")
        endpoint = "127.0.0.1:" + match[1]
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        for _ in range(60):
            try:
                with opener.open("http://" + endpoint + "/health", timeout=1) as response:
                    if response.status == 200:
                        break
            except (urllib.error.URLError, TimeoutError, ConnectionError):
                time.sleep(0.25)
        else:
            raise RuntimeError("Synthetic database fixture did not become ready")
        environment = {key: value for key, value in os.environ.items()
                       if not key.startswith("SURREALDB_")}
        environment["TAWNY_PRIVACY_TEST_WS_URL"] = "ws://" + endpoint
        environment.setdefault("AWS_LC_SYS_PREBUILT_NASM", "1")
        environment.setdefault("CARGO_BUILD_JOBS", "2")
        environment["CARGO_TARGET_DIR"] = str(ROOT / "target")
        # Run outside sibling Cargo patch scopes; never rewrite the lockfile.
        subprocess.run(
            ["cargo", "+1.95.0", "test", "--offline", "--locked", "--manifest-path", str(ROOT / "Cargo.toml"),
             "--no-default-features", "--features", "server", TEST, "--", "--ignored", "--exact"],
            env=environment, cwd=Path.home(), check=True, timeout=1800,
        )
        print("Synthetic WebSocket/database-editor transaction drill passed.")
    finally:
        cleanup(container_id, network_id)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.SubprocessError, OSError):
        print("Synthetic database authentication drill failed; inspect its checks and cleanup.", file=sys.stderr)
        sys.exit(1)
