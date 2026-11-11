// Builds the device backend: this crate's server, as a library the app loads
// to keep a library on the device without a server (src/device_backend).
//
//   node scripts/device-backend.mjs desktop [--release]
//   node scripts/device-backend.mjs android [--release] [--abi arm64-v8a|x86_64]
//
// The crate is a binary, and rustc will not emit a cdylib from one ("cannot
// mix bin crate type with others"). So, as dx does for the Android app itself,
// the binary is linked as a shared library: `-shared` with a version script
// on Linux and Android, `/DLL` with the two exports named on Windows. Its
// `main` is never called.
//
// Desktop output lands in target/device-backend/, which `just dev-desktop`
// points the app at; Android output lands in the localstreams module's
// jniLibs, which Gradle packages into the APK beside the app's own library.
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const args = process.argv.slice(2);
const platform = args[0];
const release = args.includes("--release");
const abiArgument = args.includes("--abi") ? args[args.indexOf("--abi") + 1] : null;

// The server's own profiles, so a development build shares compiled
// dependencies with the server dx builds.
const profile = release ? "server-release" : "server-dev";

const ANDROID_ABIS = {
  "arm64-v8a": "aarch64-linux-android",
  x86_64: "x86_64-linux-android",
};

const EXPORTS = ["tawny_backend_abi", "tawny_backend_start"];
// Only the two entry points are exported. `--export-dynamic`, which dx uses
// for the app, exported every symbol: 70 MB of symbol table in a phone build.
const versionScript = join(root, "target", "device-backend", "exports.map");
mkdirSync(join(root, "target", "device-backend"), { recursive: true });
writeFileSync(
  versionScript,
  `{ global: ${EXPORTS.join("; ")}; local: *; };
`,
);
const ELF_SHARED = ["-shared", `-Wl,--version-script=${versionScript.replaceAll("\\", "/")}`];

/** Build the backend and return the directory its output lands in. */
function build(target, linkArgs, env = {}) {
  const cargoArgs = [
    "rustc",
    "--bin",
    "tawny",
    "--no-default-features",
    "--features",
    "server",
    "--profile",
    profile,
    ...(target ? ["--target", target] : []),
    "--",
    ...linkArgs.flatMap((arg) => ["-C", `link-arg=${arg}`]),
  ];
  console.log(`cargo ${cargoArgs.join(" ")}`);
  const result = spawnSync("cargo", cargoArgs, {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, ...env },
    shell: process.platform === "win32",
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
  return join(root, "target", ...(target ? [target] : []), profile);
}

function place(built, dir, name) {
  if (!existsSync(built)) {
    console.error(`No backend library at ${built}`);
    process.exit(1);
  }
  mkdirSync(dir, { recursive: true });
  copyFileSync(built, join(dir, name));
  return join(dir, name);
}

function desktop() {
  const [linkArgs, file, name] =
    process.platform === "win32"
      ? [
          ["/DLL", ...EXPORTS.map((symbol) => `/EXPORT:${symbol}`)],
          "tawny.exe",
          "tawny_backend.dll",
        ]
      : process.platform === "darwin"
        ? [["-dynamiclib"], "tawny", "libtawny_backend.dylib"]
        : [ELF_SHARED, "tawny", "libtawny_backend.so"];
  const built = join(build(null, linkArgs), file);
  const placed = place(built, join(root, "target", "device-backend"), name);
  console.log(`\n${placed}\nRun the desktop app with TAWNY_BACKEND_LIBRARY set to it.`);
}

function android() {
  const ndk = process.env.ANDROID_NDK_HOME || process.env.NDK_HOME;
  if (!ndk || !existsSync(ndk)) {
    console.error("Set ANDROID_NDK_HOME (or NDK_HOME) to an Android NDK.");
    process.exit(1);
  }
  const host = { win32: "windows-x86_64", darwin: "darwin-x86_64", linux: "linux-x86_64" }[
    process.platform
  ];
  const bin = join(ndk, "toolchains", "llvm", "prebuilt", host, "bin");
  const script = process.platform === "win32" ? ".cmd" : "";
  const abis = abiArgument ? [abiArgument] : Object.keys(ANDROID_ABIS);
  for (const abi of abis) {
    const target = ANDROID_ABIS[abi];
    if (!target) {
      console.error(`Unknown ABI ${abi}; use ${Object.keys(ANDROID_ABIS).join(" or ")}.`);
      process.exit(1);
    }
    // 24: the app's minimum SDK (android/localstreams/build.gradle.kts).
    const clang = join(bin, `${target}24-clang${script}`);
    const variable = target.toUpperCase().replaceAll("-", "_");
    const underscored = target.replaceAll("-", "_");
    const sysroot = `--sysroot=${join(bin, "..", "sysroot").replaceAll("\\", "/")}`;
    const out = build(target, ELF_SHARED, {
      [`CARGO_TARGET_${variable}_LINKER`]: clang,
      [`CC_${underscored}`]: clang,
      [`CXX_${underscored}`]: join(bin, `${target}24-clang++${script}`),
      [`AR_${underscored}`]: join(bin, `llvm-ar${process.platform === "win32" ? ".exe" : ""}`),
      // rquickjs-sys generates its bindings for Android with bindgen, which
      // needs the NDK's headers.
      // bindgen reads the target's name with hyphens or, in newer versions,
      // with underscores, and splits the value as a shell would, so a
      // Windows path goes with forward slashes.
      [`BINDGEN_EXTRA_CLANG_ARGS_${target}`]: sysroot,
      [`BINDGEN_EXTRA_CLANG_ARGS_${underscored}`]: sysroot,
      ANDROID_NDK_HOME: ndk,
    });
    const placed = place(
      join(out, "tawny"),
      join(root, "android", "localstreams", "src", "main", "jniLibs", abi),
      "libtawny_backend.so",
    );
    // Debug info is most of a development build (over 400 MB) and is no use
    // inside an APK. The exported symbols stay.
    const strip = spawnSync(
      join(bin, `llvm-strip${process.platform === "win32" ? ".exe" : ""}`),
      ["--strip-debug", placed],
      { stdio: "inherit" },
    );
    if (strip.status !== 0) {
      process.exit(strip.status ?? 1);
    }
    console.log(`${abi}: ${placed}`);
  }
}

if (platform === "desktop") {
  desktop();
} else if (platform === "android") {
  android();
} else {
  console.error("Usage: node scripts/device-backend.mjs desktop|android [--release] [--abi ABI]");
  process.exit(1);
}
