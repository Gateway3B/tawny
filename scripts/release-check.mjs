// Checks a release folder before it is published (docs/releases.md):
//
//   node scripts/release-check.mjs <version> <folder>
//
// The files must be there under their release names, and the APK must carry
// the device backend for every ABI it carries the app for. Without it, "Use
// Tawny on this device" answers that this build has none, which is how 0.1.7
// shipped: the release steps built the APK without the library, and nothing
// failed. The Windows installer is NSIS-compressed and cannot be listed here;
// `scripts/release-build.mjs` checks the NSIS script dx compiled it from.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

/** The names of the entries in a zip file (an APK is one). */
export function zipEntries(buffer) {
  // The end-of-central-directory record is in the last 22 bytes plus a
  // comment of at most 64 KiB; search backwards for its signature.
  const floor = Math.max(0, buffer.length - 22 - 0xffff);
  let end = -1;
  for (let at = buffer.length - 22; at >= floor; at--) {
    if (buffer.readUInt32LE(at) === 0x06054b50) {
      end = at;
      break;
    }
  }
  if (end < 0) {
    throw new Error("not a zip file");
  }
  const count = buffer.readUInt16LE(end + 10);
  let at = buffer.readUInt32LE(end + 16);
  const names = [];
  for (let index = 0; index < count; index++) {
    if (buffer.readUInt32LE(at) !== 0x02014b50) {
      throw new Error("damaged zip central directory");
    }
    const nameLength = buffer.readUInt16LE(at + 28);
    const extraLength = buffer.readUInt16LE(at + 30);
    const commentLength = buffer.readUInt16LE(at + 32);
    names.push(buffer.toString("utf8", at + 46, at + 46 + nameLength));
    at += 46 + nameLength + extraLength + commentLength;
  }
  return names;
}

/** What is wrong with an APK's native libraries, if anything. */
export function apkProblems(entries) {
  const abis = entries.map((name) => /^lib\/([^/]+)\/libmain\.so$/.exec(name)?.[1]).filter(Boolean);
  if (abis.length === 0) {
    return ["it has no lib/<abi>/libmain.so: not a Tawny APK"];
  }
  return abis
    .filter((abi) => !entries.includes(`lib/${abi}/libtawny_backend.so`))
    .map(
      (abi) =>
        `it has no lib/${abi}/libtawny_backend.so: "On this device" would not work. ` +
        `Build it with \`just backend-android --release --abi ${abi}\` before \`dx bundle\`.`,
    );
}

/** Every problem with the release folder; empty when it can be published. */
export function releaseProblems(version, folder) {
  const problems = [];
  const files = {
    apk: join(folder, `Tawny_${version}.apk`),
    installer: join(folder, `Tawny_${version}_x64-setup.exe`),
    updater: join(folder, `Tawny_${version}_x64-setup.zip`),
  };
  for (const path of Object.values(files)) {
    if (!existsSync(path)) {
      problems.push(`${path} is missing`);
    }
  }
  if (existsSync(files.apk)) {
    let entries;
    try {
      entries = zipEntries(readFileSync(files.apk));
    } catch (error) {
      problems.push(`${files.apk}: ${error.message}`);
    }
    for (const problem of entries ? apkProblems(entries) : []) {
      problems.push(`${files.apk}: ${problem}`);
    }
  }
  return problems;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [version, folder] = process.argv.slice(2);
  if (!version || !folder) {
    console.error("Usage: node scripts/release-check.mjs <version> <folder>");
    process.exit(1);
  }
  const problems = releaseProblems(version, folder);
  if (problems.length > 0) {
    console.error(`Not publishing Tawny ${version}:`);
    for (const problem of problems) {
      console.error(`- ${problem}`);
    }
    process.exit(1);
  }
  console.log(`Tawny ${version}: the release files are there and the APK has the device backend.`);
}
