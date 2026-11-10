// Signs release files for the app updater (g3-native-plugins' `updater`).
//
//   node scripts/release-signing.mjs keygen <folder>
//   TAWNY_UPDATER_KEY=... node scripts/release-signing.mjs sign <file>...
//
// The app checks every update it downloads against a minisign public key
// compiled into it (UPDATER_PUBLIC_KEY in src/updater.rs). This writes keys
// and signatures in minisign's own format, with Node's Ed25519 and BLAKE2b,
// so the release workflow needs no minisign binary on its three runners.
// `minisign -V` verifies what it signs.
//
// `keygen` writes `tawny-updater.pub` (the public key, for the app) and
// `tawny-updater.key` (the secret, for the TAWNY_UPDATER_KEY repository
// secret) into a folder outside the repository. The secret is unencrypted:
// keep it in the secret store, nowhere else.
//
// `sign` writes `<file>.minisig` beside each file: a prehashed ("ED")
// signature with a trusted comment naming the file, which is what
// `minisign -S` writes.
import {
  createHash,
  createPrivateKey,
  createPublicKey,
  generateKeyPairSync,
  randomBytes,
  sign,
  verify,
} from "node:crypto";
import { createReadStream, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

const SECRET_PREFIX = "tawny-updater-key:v1:";

const base64url = (bytes) => Buffer.from(bytes).toString("base64url");

/** A new key pair: the key id, the Ed25519 seed and the public key. */
export function generateKey() {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const jwk = privateKey.export({ format: "jwk" });
  return {
    keyId: randomBytes(8),
    seed: Buffer.from(jwk.d, "base64url"),
    publicKey: Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url"),
  };
}

/** minisign prints a key id as the 8 bytes read as a little-endian number. */
const keyIdHex = (keyId) => Buffer.from(keyId).reverse().toString("hex").toUpperCase();

/** The `minisign.pub` file for a key. */
export function publicKeyText(key) {
  const encoded = Buffer.concat([Buffer.from("Ed"), key.keyId, key.publicKey]).toString("base64");
  return `untrusted comment: minisign public key ${keyIdHex(key.keyId)}\n${encoded}\n`;
}

/** The secret as one line, for a repository secret. */
export function secretText(key) {
  return SECRET_PREFIX + Buffer.concat([key.keyId, key.seed, key.publicKey]).toString("base64");
}

export function parseSecret(text) {
  const trimmed = text.trim();
  if (!trimmed.startsWith(SECRET_PREFIX)) {
    throw new Error("TAWNY_UPDATER_KEY is not a key from `release-signing.mjs keygen`");
  }
  const bytes = Buffer.from(trimmed.slice(SECRET_PREFIX.length), "base64");
  if (bytes.length !== 72) throw new Error("TAWNY_UPDATER_KEY has the wrong length");
  return {
    keyId: bytes.subarray(0, 8),
    seed: bytes.subarray(8, 40),
    publicKey: bytes.subarray(40, 72),
  };
}

function keyObjects(key) {
  const x = base64url(key.publicKey);
  return {
    privateKey: createPrivateKey({
      key: { kty: "OKP", crv: "Ed25519", d: base64url(key.seed), x },
      format: "jwk",
    }),
    publicKey: createPublicKey({ key: { kty: "OKP", crv: "Ed25519", x }, format: "jwk" }),
  };
}

/** BLAKE2b-512 of a file, read in pieces: an installer can be large. */
export async function prehash(path) {
  const hash = createHash("blake2b512");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest();
}

/** The `.minisig` text for a file's prehash. */
export function signatureText(key, digest, fileName, timestamp = Math.floor(Date.now() / 1000)) {
  const { privateKey } = keyObjects(key);
  const signature = sign(null, digest, privateKey);
  const trusted = `timestamp:${timestamp}\tfile:${fileName}\thashed`;
  // The global signature covers the signature and the trusted comment, so
  // the comment cannot be swapped.
  const global = sign(null, Buffer.concat([signature, Buffer.from(trusted)]), privateKey);
  const line = Buffer.concat([Buffer.from("ED"), key.keyId, signature]).toString("base64");
  return [
    "untrusted comment: signature from the Tawny release key",
    line,
    `trusted comment: ${trusted}`,
    global.toString("base64"),
    "",
  ].join("\n");
}

/** Check a `.minisig` text against a public key: the tests' half. */
export function verifySignatureText(publicKeyFile, digest, text) {
  const keyBytes = Buffer.from(publicKeyFile.trim().split("\n")[1], "base64");
  const lines = text.trim().split("\n");
  const signatureBytes = Buffer.from(lines[1], "base64");
  if (signatureBytes.subarray(0, 2).toString() !== "ED") return false;
  if (!signatureBytes.subarray(2, 10).equals(keyBytes.subarray(2, 10))) return false;
  const publicKey = createPublicKey({
    key: { kty: "OKP", crv: "Ed25519", x: base64url(keyBytes.subarray(10)) },
    format: "jwk",
  });
  const signature = signatureBytes.subarray(10);
  const trusted = lines[2].replace(/^trusted comment: /, "");
  return (
    verify(null, digest, publicKey, signature) &&
    verify(
      null,
      Buffer.concat([signature, Buffer.from(trusted)]),
      publicKey,
      Buffer.from(lines[3], "base64"),
    )
  );
}

export async function signFile(key, path) {
  const text = signatureText(key, await prehash(path), basename(path));
  writeFileSync(`${path}.minisig`, text);
  return text;
}

async function main(argv) {
  const [command, ...rest] = argv;
  if (command === "keygen") {
    const folder = rest[0];
    if (!folder)
      throw new Error("Usage: release-signing.mjs keygen <folder outside the repository>");
    mkdirSync(folder, { recursive: true });
    const publicPath = join(folder, "tawny-updater.pub");
    const secretPath = join(folder, "tawny-updater.key");
    if (existsSync(secretPath)) throw new Error(`${secretPath} exists; refusing to replace a key`);
    const key = generateKey();
    writeFileSync(publicPath, publicKeyText(key));
    writeFileSync(secretPath, `${secretText(key)}\n`, { mode: 0o600 });
    console.log(`Public key: ${publicPath} (goes in the app)`);
    console.log(`Secret key: ${secretPath} (goes in the TAWNY_UPDATER_KEY repository secret)`);
    return;
  }
  if (command === "sign") {
    const secret = process.env.TAWNY_UPDATER_KEY;
    if (!secret) throw new Error("Set TAWNY_UPDATER_KEY to the secret from `keygen`");
    if (rest.length === 0) throw new Error("Usage: release-signing.mjs sign <file>...");
    const key = parseSecret(secret);
    for (const path of rest) {
      await signFile(key, path);
      console.log(`Signed ${path}`);
    }
    return;
  }
  if (command === "public-key") {
    // Print the public key that belongs to TAWNY_UPDATER_KEY, to check it
    // matches the one in the app.
    process.stdout.write(
      publicKeyText(parseSecret(process.env.TAWNY_UPDATER_KEY ?? readFileSync(0, "utf8"))),
    );
    return;
  }
  throw new Error("Usage: release-signing.mjs keygen <folder> | sign <file>... | public-key");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
