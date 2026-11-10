// Writes the release file the app updater reads (`latest.json`), in the
// Tauri updater's format.
//
//   node scripts/release-manifest.mjs --version 0.2.0 \
//     --base-url https://github.com/Gateway3B/tawny/releases/download/v0.2.0 \
//     [--notes-file notes.md] --out latest.json \
//     android=artifacts/Tawny.apk windows-x86_64-nsis=artifacts/Tawny_0.2.0_x64-setup.zip
//
// Each `platform=file` names a file already signed by release-signing.mjs;
// its `.minisig` becomes the entry's signature, and its URL is the file name
// under --base-url, where the release workflow uploads it. The app finds its
// entry as `{target}-{arch}-{installer}`, then `{target}-{arch}`, then
// `{target}` (see g3-native-plugins' README, "App updates").
import { readFileSync, writeFileSync } from "node:fs";
import { basename } from "node:path";
import { fileURLToPath } from "node:url";

const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const PLATFORM =
  /^(android|ios|windows|darwin|linux)(-(aarch64|x86_64|armv7|i686)(-(apk|ipa|nsis|msi|app|appimage|deb))?)?$/;

/** The release JSON for already-signed files. */
export function buildManifest({ version, notes, pubDate, baseUrl, entries }) {
  if (!SEMVER.test(version)) throw new Error(`${version} is not a semantic version`);
  if (!baseUrl.startsWith("https://")) throw new Error("The base URL must be HTTPS");
  if (entries.length === 0) throw new Error("A release needs at least one platform");
  const platforms = {};
  for (const { platform, fileName, signature } of entries) {
    if (!PLATFORM.test(platform))
      throw new Error(`${platform} is not a platform the updater looks up`);
    if (platforms[platform]) throw new Error(`${platform} is listed twice`);
    if (!signature.startsWith("untrusted comment:"))
      throw new Error(`${fileName} has no minisign signature`);
    platforms[platform] = {
      url: `${baseUrl.replace(/\/+$/, "")}/${encodeURIComponent(fileName)}`,
      signature,
    };
  }
  return {
    version,
    ...(notes ? { notes } : {}),
    pub_date: pubDate,
    platforms,
  };
}

function parse(argv) {
  const options = { entries: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument.startsWith("--")) {
      options[argument.slice(2)] = argv[index + 1];
      index += 1;
    } else {
      const split = argument.indexOf("=");
      if (split < 1) throw new Error(`Expected platform=file, got ${argument}`);
      options.entries.push({ platform: argument.slice(0, split), path: argument.slice(split + 1) });
    }
  }
  return options;
}

function main(argv) {
  const options = parse(argv);
  for (const required of ["version", "base-url", "out"]) {
    if (!options[required]) throw new Error(`--${required} is required`);
  }
  const manifest = buildManifest({
    version: options.version.replace(/^v/, ""),
    notes: options["notes-file"] ? readFileSync(options["notes-file"], "utf8").trim() : undefined,
    pubDate: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
    baseUrl: options["base-url"],
    entries: options.entries.map(({ platform, path }) => ({
      platform,
      fileName: basename(path),
      signature: readFileSync(`${path}.minisig`, "utf8"),
    })),
  });
  writeFileSync(options.out, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`Wrote ${options.out}: ${Object.keys(manifest.platforms).join(", ")}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
