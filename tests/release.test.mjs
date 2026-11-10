// The release scripts: keys and signatures in minisign's format, and the
// latest.json the app updater reads.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { buildManifest } from "../scripts/release-manifest.mjs";
import {
  generateKey,
  parseSecret,
  prehash,
  publicKeyText,
  secretText,
  signFile,
  verifySignatureText,
} from "../scripts/release-signing.mjs";

test("a signed file verifies, and a changed one does not", async () => {
  const folder = mkdtempSync(join(tmpdir(), "tawny-release-"));
  try {
    const key = generateKey();
    const path = join(folder, "Tawny.apk");
    writeFileSync(path, "release bytes");
    const signature = await signFile(key, path);
    assert.match(signature, /^untrusted comment: /);
    assert.match(signature, /\ntrusted comment: timestamp:\d+\tfile:Tawny\.apk\thashed\n/);
    assert.ok(verifySignatureText(publicKeyText(key), await prehash(path), signature));
    writeFileSync(path, "release bytez");
    assert.ok(!verifySignatureText(publicKeyText(key), await prehash(path), signature));
    // Someone else's key never verifies it.
    writeFileSync(path, "release bytes");
    assert.ok(!verifySignatureText(publicKeyText(generateKey()), await prehash(path), signature));
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});

test("the public key is minisign's format and the secret round-trips", () => {
  const key = generateKey();
  const text = publicKeyText(key);
  const [comment, line] = text.trim().split("\n");
  const id = Buffer.from(key.keyId).reverse().toString("hex").toUpperCase();
  assert.equal(comment, `untrusted comment: minisign public key ${id}`);
  const bytes = Buffer.from(line, "base64");
  assert.equal(bytes.subarray(0, 2).toString(), "Ed");
  assert.equal(bytes.length, 42);
  const back = parseSecret(secretText(key));
  assert.deepEqual(Buffer.from(back.seed), key.seed);
  assert.equal(publicKeyText(back), text);
  assert.throws(() => parseSecret("RWQ..."), /not a key/);
});

test("latest.json lists each signed file under the release's download URL", () => {
  const signature = "untrusted comment: signature\nRUQ...\ntrusted comment: t\nAAA=\n";
  const manifest = buildManifest({
    version: "0.2.0",
    notes: "Faster startup.",
    pubDate: "2026-11-10T12:00:00Z",
    baseUrl: "https://github.com/Gateway3B/tawny/releases/download/v0.2.0/",
    entries: [
      { platform: "android", fileName: "Tawny.apk", signature },
      { platform: "windows-x86_64-nsis", fileName: "Tawny_0.2.0_x64-setup.zip", signature },
    ],
  });
  assert.deepEqual(manifest, {
    version: "0.2.0",
    notes: "Faster startup.",
    pub_date: "2026-11-10T12:00:00Z",
    platforms: {
      android: {
        url: "https://github.com/Gateway3B/tawny/releases/download/v0.2.0/Tawny.apk",
        signature,
      },
      "windows-x86_64-nsis": {
        url: "https://github.com/Gateway3B/tawny/releases/download/v0.2.0/Tawny_0.2.0_x64-setup.zip",
        signature,
      },
    },
  });
  const base = { version: "0.2.0", pubDate: "x", baseUrl: "https://x.test", entries: [] };
  assert.throws(() => buildManifest(base), /at least one/);
  assert.throws(() => buildManifest({ ...base, version: "next" }), /semantic/);
  const unsigned = [{ platform: "android", fileName: "a.apk", signature: "" }];
  assert.throws(() => buildManifest({ ...base, entries: unsigned }), /no minisign/);
  const unknown = [{ platform: "windows-arm64", fileName: "a.zip", signature }];
  assert.throws(() => buildManifest({ ...base, entries: unknown }), /not a platform/);
});
