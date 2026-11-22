// Builds the release files for `just publish` (docs/releases.md):
//
//   node scripts/release-build.mjs <version> <folder> [--android-only | --windows-only]
//
// Each app is built after its device backend, which the bundle packages:
// Gradle takes Android's from the localstreams module's jniLibs, and dx's
// NSIS installer takes Windows's from target/device-backend (Dioxus.toml,
// `[bundle] resources`). Bundling without it still succeeds, and "On this
// device" then fails on every install, so the folder is checked at the end.
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { apkProblems, releaseProblems, zipEntries } from "./release-check.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const args = process.argv.slice(2);
const [version, folderArgument] = args.filter((arg) => !arg.startsWith("--"));
const androidOnly = args.includes("--android-only");
const windowsOnly = args.includes("--windows-only");

if (!version || !folderArgument) {
  console.error(
    "Usage: node scripts/release-build.mjs <version> <folder> [--android-only | --windows-only]",
  );
  process.exit(1);
}
const folder = resolve(folderArgument);

// The app reports the crate's version and offers updates against it, so a
// folder named for another version would publish a build that misnames itself.
const crateVersion = /^version\s*=\s*"([^"]+)"/m.exec(
  readFileSync(join(root, "Cargo.toml"), "utf8"),
)?.[1];
if (crateVersion !== version) {
  console.error(`Cargo.toml says ${crateVersion}, not ${version}. Raise it first.`);
  process.exit(1);
}

function run(command, commandArgs) {
  console.log(`\n> ${command} ${commandArgs.join(" ")}`);
  const result = spawnSync(command, commandArgs, {
    cwd: root,
    stdio: "inherit",
    // No shell: node and dx are executables, and a shell splits a path with
    // spaces in it, such as node's own C:\Program Files\nodejs\node.exe.
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

/** Every file under `dir` whose name passes `test`. */
function find(dir, test) {
  if (!existsSync(dir)) {
    return [];
  }
  return readdirSync(dir, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile() && test(entry.name))
    .map((entry) => join(entry.parentPath, entry.name));
}

function one(dir, test, what) {
  const found = find(dir, test);
  if (found.length !== 1) {
    console.error(`Expected one ${what} under ${dir}, found ${found.length}.`);
    process.exit(1);
  }
  return found[0];
}

const bundles = join(root, "target", "release-bundles");
mkdirSync(folder, { recursive: true });

if (!windowsOnly) {
  const out = join(bundles, "android");
  rmSync(out, { recursive: true, force: true });
  // Phones only: the APK carries no x86_64 app either.
  run(process.execPath, [
    "scripts/device-backend.mjs",
    "android",
    "--release",
    "--abi",
    "arm64-v8a",
  ]);
  run("dx", [
    "bundle",
    "--android",
    "--release",
    "--no-default-features",
    "--fullstack",
    "false",
    "--package-types",
    "apk",
    "--target",
    "aarch64-linux-android",
    "--out-dir",
    out,
  ]);
  const apk = join(folder, `Tawny_${version}.apk`);
  copyFileSync(
    one(out, (name) => name.endsWith(".apk"), "APK"),
    apk,
  );
  const problems = apkProblems(zipEntries(readFileSync(apk)));
  if (problems.length > 0) {
    console.error(`${apk}: ${problems.join("; ")}`);
    process.exit(1);
  }
}

if (!androidOnly) {
  const out = join(bundles, "windows");
  rmSync(out, { recursive: true, force: true });
  run(process.execPath, ["scripts/device-backend.mjs", "desktop", "--release"]);
  run("dx", [
    "bundle",
    "--desktop",
    "--release",
    "--no-default-features",
    "--fullstack",
    "false",
    "--package-types",
    "nsis",
    "--package-types",
    "updater",
    "--out-dir",
    out,
  ]);
  // The installer cannot be listed without unpacking NSIS, but dx leaves the
  // script it compiled, with a `File` line per installed file, under its own
  // bundle folder rather than --out-dir.
  const nsis = join(root, "target", "dx", "tawny", "bundle", "windows", "nsis", "Tawny.nsi");
  if (!existsSync(nsis) || !/\\tawny_backend\.dll"/.test(readFileSync(nsis, "utf8"))) {
    console.error(
      `The installer was packed without tawny_backend.dll (${nsis} does not install it). ` +
        "Check `[bundle] resources` in Dioxus.toml.",
    );
    process.exit(1);
  }
  copyFileSync(
    one(out, (name) => name.endsWith("-setup.exe"), "installer"),
    join(folder, `Tawny_${version}_x64-setup.exe`),
  );
  copyFileSync(
    one(out, (name) => name.endsWith("-setup.zip"), "updater archive"),
    join(folder, `Tawny_${version}_x64-setup.zip`),
  );
}

if (!androidOnly && !windowsOnly) {
  const problems = releaseProblems(version, folder);
  if (problems.length > 0) {
    for (const problem of problems) {
      console.error(`- ${problem}`);
    }
    process.exit(1);
  }
}
console.log(`\nTawny ${version} release files are in ${folder}.`);
