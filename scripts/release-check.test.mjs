import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { apkProblems, releaseProblems, zipEntries } from "./release-check.mjs";

/** A zip of empty, stored files: enough for the central directory. */
function zip(names, comment = "") {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const name of names) {
    const nameBytes = Buffer.from(name, "utf8");
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(nameBytes.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, nameBytes);
    centrals.push(central, nameBytes);
    offset += local.length + nameBytes.length;
  }
  const directory = Buffer.concat(centrals);
  const commentBytes = Buffer.from(comment, "utf8");
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(names.length, 8);
  end.writeUInt16LE(names.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(commentBytes.length, 20);
  return Buffer.concat([...locals, directory, end, commentBytes]);
}

test("reads entry names, past a trailing comment", () => {
  const names = ["AndroidManifest.xml", "lib/arm64-v8a/libmain.so"];
  assert.deepEqual(zipEntries(zip(names, "signed")), names);
});

test("refuses what is not a zip", () => {
  assert.throws(() => zipEntries(Buffer.alloc(100)), /not a zip/);
});

test("an APK needs the backend beside the app for each ABI", () => {
  assert.deepEqual(
    apkProblems(["lib/arm64-v8a/libmain.so", "lib/arm64-v8a/libtawny_backend.so"]),
    [],
  );
  const problems = apkProblems([
    "lib/arm64-v8a/libmain.so",
    "lib/arm64-v8a/libtawny_backend.so",
    "lib/x86_64/libmain.so",
  ]);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /lib\/x86_64\/libtawny_backend\.so/);
  // 0.1.7's APK: Chaquopy's libraries and the app, no backend.
  assert.equal(
    apkProblems(["lib/arm64-v8a/libmain.so", "lib/arm64-v8a/libpython3.13.so"]).length,
    1,
  );
});

test("a release folder needs all three files and a complete APK", () => {
  const folder = mkdtempSync(join(tmpdir(), "tawny-release-"));
  try {
    assert.equal(releaseProblems("1.2.3", folder).length, 3);
    writeFileSync(join(folder, "Tawny_1.2.3_x64-setup.exe"), "");
    writeFileSync(join(folder, "Tawny_1.2.3_x64-setup.zip"), "");
    writeFileSync(join(folder, "Tawny_1.2.3.apk"), zip(["lib/arm64-v8a/libmain.so"]));
    const problems = releaseProblems("1.2.3", folder);
    assert.equal(problems.length, 1);
    assert.match(problems[0], /libtawny_backend\.so/);
    writeFileSync(
      join(folder, "Tawny_1.2.3.apk"),
      zip(["lib/arm64-v8a/libmain.so", "lib/arm64-v8a/libtawny_backend.so"]),
    );
    assert.deepEqual(releaseProblems("1.2.3", folder), []);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});
