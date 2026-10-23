#!/usr/bin/env python3
"""Exercise a local app image with an encrypted synthetic pre-request backup.

No images are published, host data mounted, ports exposed or live credentials
forwarded. This local crypto exercise does not establish B2 durability/expiry.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
LABEL = "net.g3tech.synthetic.privacy-linux=tawny"
HTTP = """
import base64, os, sys, urllib.request
path = sys.argv[1]
body = sys.stdin.buffer.read()
headers = {'Authorization':'Basic ' + base64.b64encode(b'root:synthetic-linux-drill').decode(),
    'surreal-ns':'restore', 'surreal-db':'tawny', 'Accept':'application/json',
    'Content-Type':'text/plain'}
request = urllib.request.Request('http://' + os.environ['DRILL_DB'] + ':8000/' + path,
    data=body if path != 'export' else None, headers=headers)
with urllib.request.urlopen(request, timeout=15) as response:
    sys.stdout.buffer.write(response.read())
"""

# Exercise the packaged Dioxus routes, not substitute handlers. Cookies are
# forwarded explicitly because the release cookie is Secure and this isolated
# synthetic network uses HTTP instead of the production HTTPS reverse proxy.
LEGAL_HTTP = """
import http.cookies, json, sys, urllib.error, urllib.request
origin = sys.argv[1]
cookies = {}
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
def request(path, body=None, expected=200):
    headers = {'Accept':'application/json', 'Content-Type':'application/json',
        'Cookie':'; '.join(key + '=' + value for key, value in cookies.items())}
    data = json.dumps(body).encode() if body is not None else None
    query = urllib.request.Request(origin + path, data=data, headers=headers)
    try:
        response = opener.open(query, timeout=20)
    except urllib.error.HTTPError as problem:
        response = problem
    with response:
        if response.status != expected:
            raise RuntimeError('Unexpected packaged legal fixture status')
        if path.startswith('/api/v1/auth/') or expected != 200:
            if response.headers.get('Cache-Control') != 'private, no-store':
                raise RuntimeError('Packaged private response could be cached')
        for field in response.headers.get_all('Set-Cookie', []):
            parsed = http.cookies.SimpleCookie()
            parsed.load(field)
            cookies.update({key: value.value for key, value in parsed.items()})
        payload = response.read()
        return json.loads(payload) if expected == 200 else None
account = request('/api/v1/auth/guest', {})
owner = account['id']
if not owner.startswith('app_user:') or not account['is_guest']:
    raise RuntimeError('Synthetic guest was not created')
if request('/api/v1/auth/account')['id'] != owner:
    raise RuntimeError('Synthetic session did not bind its account')
request('/api/v1/viewer', expected=503)
export = json.loads(request('/api/v1/auth/export', {'expected_owner':owner}))
if not isinstance(export, dict) or owner not in json.dumps(export):
    raise RuntimeError('Essential account download was unavailable')
request('/api/v1/auth/delete', {'expected_owner':owner})
request('/api/v1/auth/account', expected=401)
print('event=privacy_linux_drill_packaged_terms_and_essential_access_verified')
"""


def run(command, **options):
    return subprocess.run(command, check=True, capture_output=True, timeout=60, **options).stdout


def identity(output):
    value = output.decode().strip()
    if not re.fullmatch(r"[0-9a-f]{64}", value):
        raise RuntimeError("Unknown owned Docker resource")
    return value


def local_image(reference):
    value = run(["docker", "image", "inspect", reference, "--format", "{{.Id}}"]).decode().strip()
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", value):
        raise RuntimeError("A locally available immutable image is required")
    return value


def cleanup(containers, network, invoke=subprocess.run):
    failed = False
    for kind, resource in [*(("container", item) for item in reversed(containers)), ("network", network)]:
        if resource is None:
            continue
        if not re.fullmatch(r"[0-9a-f]{64}", resource):
            failed = True
            continue
        command = ["docker", "rm", "--force", "--volumes", resource] if kind == "container" else ["docker", "network", "rm", resource]
        try:
            result = invoke(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30, check=False)
            failed = failed or result.returncode != 0
        except (OSError, subprocess.TimeoutExpired):
            failed = True
    if failed:
        raise RuntimeError("Owned fixture cleanup failed")


def drill(app_image, crypto_image):
    app_image, crypto_image = local_image(app_image), local_image(crypto_image)
    database_image = local_image("surrealdb/surrealdb:v3.2.4")
    name = "tawny-privacy-linux-" + uuid.uuid4().hex[:12]
    network, containers = None, []
    environment = dict(os.environ)
    forwarded = {
        "RESTORE_CONTAINER": name + "-db", "G3_PRIVACY_RESTORE": "quarantined-v1",
        "SURREALDB_AUTH_LEVEL": "root", "SURREALDB_HOST": "ws://" + name + "-db:8000",
        "SURREALDB_NAMESPACE": "restore", "SURREALDB_NAME": "tawny",
        "SURREALDB_USER": "root", "SURREALDB_PASSWORD": "synthetic-linux-drill",
    }
    environment.update(forwarded)

    def create(arguments, **options):
        container = identity(run(["docker", "create", "--label", LABEL, *arguments], **options))
        containers.append(container)
        return container

    def remove(container):
        run(["docker", "rm", "--force", "--volumes", container])
        containers.remove(container)

    try:
        network = identity(run(["docker", "network", "create", "--internal", "--label", LABEL, name]))
        database = create(["--name", name + "-db", "--network", network, "--log-driver=none",
                           "--label", "net.g3tech.restore-scratch=true", "--read-only",
                           "--tmpfs", "/tmp:rw,noexec,nosuid,size=64m", "--cap-drop=ALL",
                           "--security-opt=no-new-privileges:true", "--user=65534:65534",
                           "--pids-limit=128", "--memory=768m", "--cpus=1", database_image,
                           "start", "--user", "root", "--pass", "synthetic-linux-drill", "memory",
                           "--deny-net", "--deny-scripting"])
        run(["docker", "start", database])
        utility = create(["--network", network, "--log-driver=none", "--read-only",
                          "--tmpfs", "/tmp:rw,noexec,nosuid,size=32m", "--cap-drop=ALL",
                          "--security-opt=no-new-privileges:true", "--user=10001:10001",
                          "--env", "DRILL_DB=" + name + "-db", "--entrypoint=python3",
                          crypto_image, "-c", "import time; time.sleep(3600)"])
        run(["docker", "start", utility])

        def http(path, body=b""):
            return run(["docker", "exec", "-i", utility, "python3", "-c", HTTP, path], input=body)

        def sql(query):
            results = json.loads(http("sql", query.encode()))
            if not isinstance(results, list) or not results or any(row["status"] != "OK" for row in results):
                raise RuntimeError("Synthetic query failed")
            return results[-1]["result"]

        for attempt in range(30):
            try:
                sql("DEFINE NAMESPACE IF NOT EXISTS restore; DEFINE DATABASE IF NOT EXISTS tawny; RETURN true;")
                break
            except (subprocess.CalledProcessError, OSError):
                if attempt == 29:
                    raise RuntimeError("Synthetic database did not become ready") from None
                time.sleep(0.2)
        for schema in sorted((ROOT / "database" / "schema").glob("*.surql")):
            sql(schema.read_text(encoding="utf-8"))
        sql("""
CREATE app_user:linux_owner SET display_name = 'Original Linux Owner', is_guest = true, created_at = time::now(), updated_at = time::now();
CREATE app_user:linux_other SET display_name = 'Preserved Linux Owner', is_guest = true, created_at = time::now(), updated_at = time::now();
CREATE playlist:linux_owner SET owner = app_user:linux_owner, playlist_id = 'saved', name = 'Old personal playlist';
CREATE playlist:linux_other SET owner = app_user:linux_other, playlist_id = 'saved', name = 'Preserved playlist';
CREATE playlist:demo SET playlist_id = 'demo', name = 'Unowned demo';
CREATE library_state:linux_owner SET owner = app_user:linux_owner, queue_json = '["private-video-canary"]';
CREATE library_state:linux_other SET owner = app_user:linux_other, queue_json = '["preserved-video"]';
CREATE video:shared SET video_id = 'synthetic-shared-video', title = 'Shared fact', channel_id = 'synthetic-channel',
    channel_name = 'Shared channel', thumbnail_url = '', published_at = '', published_sort = '';
RELATE playlist:linux_owner->playlist_item->video:shared SET position = 0;
RELATE playlist:linux_other->playlist_item->video:shared SET position = 0;
""")
        for key, account in [("owned", "linux_owner"), ("other", "linux_other"), ("anonymous", None)]:
            data = {"user_auth_session_id": json.dumps(account)} if account else {}
            payload = json.dumps({"data": data, "private_fixture": "session-secret-canary"})
            owner = "app_user:" + account if account else "NONE"
            # Only fixed fixture names/payloads, never user or deployment values.
            sql(f"CREATE sessions:{key} SET sessionid = '{key}', sessionexpires = '4102444800', sessionstore = {json.dumps(payload)}, session_account = {owner}, account_indexed = true;")
        subject = sql("RETURN crypto::sha256(<string>app_user:linux_owner);")
        event = {"schema_version": 1, "app": "tawny", "request_id": str(uuid.uuid4()),
                 "subject": subject, "subject_version": 1, "action": "delete"}
        print("event=privacy_linux_drill_fixture_ready", flush=True)

        def replay(value, mode, success=True):
            arguments = ["--network", network, "--read-only", "--tmpfs", "/tmp:rw,noexec,nosuid,size=16m",
                         "--cap-drop=ALL", "--security-opt=no-new-privileges:true", "--pids-limit=128",
                         "--memory=1g", "--cpus=2", "--log-driver=none", "--no-healthcheck", "--user=10001:10001", "-i"]
            for key in forwarded:
                arguments.extend(["--env", key])
            arguments.extend(["--entrypoint=/app/server", app_image, "privacy-replay", mode])
            container = create(arguments, env=environment)
            result = subprocess.run(["docker", "start", "--attach", "--interactive", container],
                                    input=json.dumps(value).encode(), capture_output=True, timeout=270)
            if (result.returncode == 0) != success:
                raise RuntimeError("Linux replay returned an unexpected result")
            remove(container)

        backup = http("export")
        if not backup or len(backup) > 8 * 1024 * 1024:
            raise RuntimeError("Unexpected synthetic backup size")
        run(["docker", "exec", utility, "age-keygen", "-o", "/tmp/identity"])
        recipient = run(["docker", "exec", utility, "age-keygen", "-y", "/tmp/identity"]).decode().strip()
        run(["docker", "exec", "-i", utility, "age", "-r", recipient, "-o", "/tmp/old-backup.age"], input=gzip.compress(backup))
        print("event=privacy_linux_drill_backup_encrypted", flush=True)
        replay(event, "--verify", success=False)
        replay(event, "--apply")
        replay(event, "--verify")
        sql("REMOVE DATABASE tawny; DEFINE DATABASE tawny;")
        decrypted = gzip.decompress(run(["docker", "exec", utility, "age", "--decrypt", "--identity", "/tmp/identity", "/tmp/old-backup.age"]))
        if hashlib.sha256(decrypted).digest() != hashlib.sha256(backup).digest():
            raise RuntimeError("Encrypted backup did not round-trip exactly")
        imported = json.loads(http("import", decrypted))
        if not isinstance(imported, list) or any(row["status"] != "OK" for row in imported):
            raise RuntimeError("Synthetic backup import failed")
        if sql("RETURN (SELECT VALUE display_name FROM ONLY app_user:linux_owner);") != "Original Linux Owner":
            raise RuntimeError("The restored backup did not contain the old value")
        print("event=privacy_linux_drill_old_backup_imported", flush=True)
        replay(event, "--verify", success=False)
        replay(event, "--apply")
        replay(event, "--verify")
        replay(event, "--apply")
        replay(event, "--verify")
        conflict = event | {"subject_version": 2}
        replay(conflict, "--apply", success=False)
        if sql("""RETURN count(SELECT id FROM app_user) = 1 AND app_user:linux_other.display_name = 'Preserved Linux Owner'
            AND count(SELECT id FROM playlist) = 2 AND playlist:linux_other.owner = app_user:linux_other
            AND playlist:demo.owner = NONE AND count(SELECT id FROM playlist_item) = 1
            AND count(SELECT id FROM library_state) = 1 AND count(SELECT id FROM sessions) = 2
            AND count(SELECT id FROM video) = 1;""") is not True:
            raise RuntimeError("Restored ownership inventory is incorrect")
        results = json.loads(http("sql", b"CREATE library_state SET owner = app_user:linux_owner;"))
        if not any(row["status"] == "ERR" for row in results):
            raise RuntimeError("A late private write was not rejected")
        print("event=privacy_linux_drill_primary_replay_verified", flush=True)

        # Verify the packaged server also starts with scoped credentials under
        # the shared overlay's non-root/read-only constraints. No user traffic.
        sql("DEFINE USER fixture_runtime ON DATABASE PASSWORD 'synthetic-linux-runtime' ROLES EDITOR;")
        runtime = {
            "SURREALDB_AUTH_LEVEL": "database", "SURREALDB_HOST": forwarded["SURREALDB_HOST"],
            "SURREALDB_NAMESPACE": "restore", "SURREALDB_NAME": "tawny",
            "SURREALDB_USER": "fixture_runtime", "SURREALDB_PASSWORD": "synthetic-linux-runtime",
            "G3_PRIVACY_RECOVERY": "disabled", "TAWNY_DATA_DIR": "/data",
            "G3_LEGAL_ENFORCEMENT": "required",
            "G3_CATALOG_ORIGIN": "http://127.0.0.1:9",
            "TAWNY_PUBLIC_URL": "http://" + name + "-app:8080",
        }
        arguments = ["--name", name + "-app", "--network", network, "--read-only",
                     "--tmpfs", "/tmp:rw,noexec,nosuid,size=32m",
                     "--tmpfs", "/data:rw,noexec,nosuid,size=64m,uid=10001,gid=10001,mode=0700",
                     "--user=10001:10001", "--cap-drop=ALL", "--security-opt=no-new-privileges:true",
                     "--pids-limit=256", "--memory=1g", "--cpus=2", "--log-driver=none", "--no-healthcheck"]
        for key, value in runtime.items():
            arguments.extend(["--env", key + "=" + value])
        arguments.append(app_image)
        app = create(arguments)
        run(["docker", "start", app])
        probe = "import urllib.request; urllib.request.urlopen('http://" + name + "-app:8080/api/v1/health', timeout=2)"
        for attempt in range(30):
            try:
                run(["docker", "exec", utility, "python3", "-c", probe])
                break
            except subprocess.CalledProcessError:
                if attempt == 29:
                    raise RuntimeError("Packaged scoped server did not become ready") from None
                time.sleep(0.2)
        print("event=privacy_linux_drill_scoped_server_ready", flush=True)
        result = run(["docker", "exec", utility, "python3", "-c", LEGAL_HTTP,
                      "http://" + name + "-app:8080"])
        print(result.decode().strip(), flush=True)
    finally:
        cleanup(containers, network)
    print("event=privacy_linux_encrypted_backup_drill_passed image=" + app_image)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True, help="Already-built local app image")
    parser.add_argument("--crypto-image", required=True, help="Already-built local Python/age image")
    options = parser.parse_args()
    try:
        drill(options.image, options.crypto_image)
    except Exception:
        raise SystemExit("event=privacy_linux_drill_failed_owned_fixture_cleanup_attempted") from None
