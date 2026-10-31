// Regression cases for source masking and the staged readability check.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { codeMask, issues } from "./rust-style.mjs";

const lines = (source) => issues(source).map(([line]) => line);

test("reports setup and navigation inside rsx", () => {
  const source =
    "fn App() {\n    let state = use_signal(|| false);\n    rsx! { Button { onclick: move |_| g3_route_transitions::animated_navigate(route) } }\n}";
  assert.deepEqual(lines(source), [3, 3]);
});

test("accepts separation and markup-only components", () => {
  const source =
    "fn App() {\n    let state = use_signal(|| false);\n\n    rsx! { Child {} }\n}\nfn Child() {\n    rsx! { div {} }\n}";
  assert.deepEqual(issues(source), []);
});

test("ignores comments and every string form", () => {
  const payload = "g3_route_transitions::animated_navigate(route)";
  for (const literal of [
    `"${payload}"`,
    `b"${payload}"`,
    `c"${payload}"`,
    `r"${payload}"`,
    `r###"${payload}"###`,
    `br##"${payload}"##`,
    `cr#"${payload}"#`,
    `/* ${payload} /* nested */ */`,
    `// ${payload}\n`,
  ]) {
    assert.deepEqual(issues(literal), [], literal);
  }
});

test("preserves offsets and line numbers in multiline literals", () => {
  const source =
    'r##"first\nrsx! {\n    ignored\n}"##;\ng3_route_transitions::animated_navigate(route);';
  assert.equal(codeMask(source).length, source.length);
  assert.deepEqual(lines(source), [5]);
});

test("characters, escapes and lifetimes", () => {
  const source = String.raw`let a = '\''; let b = '\u{1f600}'; let c = b'"'; fn f<'a>(x: &'a str) {}`;
  assert.ok(codeMask(source).includes("fn f<'a>(x: &'a str)"));
  assert.ok(!codeMask(source).includes("1f600"));
});

test("escaped quotes do not end strings", () => {
  const source = String.raw`let text = "quote: \" g3_route_transitions::animated_navigate(route)";`;
  assert.deepEqual(issues(source), []);
});

test("a raw identifier is code", () => {
  assert.ok(codeMask("let r#type = 1;").includes("r#type"));
});

test("navigation with spaces", () => {
  assert.equal(issues("g3_route_transitions :: animated_navigate (route)").length, 1);
});

test("the staged check reads the index instead of the working copy", () => {
  // Use a workspace-owned scratch directory on restricted Windows hosts.
  const scratch = resolve("target");
  mkdirSync(scratch, { recursive: true });
  const root = mkdtempSync(join(scratch, "rust-style-"));
  try {
    const inside = relative(scratch, root);
    assert.ok(inside && !inside.startsWith("..") && !isAbsolute(inside));
    mkdirSync(join(root, "scripts"));
    const checker = join(root, "scripts", "rust-style.mjs");
    copyFileSync(fileURLToPath(new URL("./rust-style.mjs", import.meta.url)), checker);
    execFileSync("git", ["init", "-q", root]);
    const source = join(root, "app.rs");
    const run = () => spawnSync(process.execPath, [checker, "--staged"], { encoding: "utf8" });
    writeFileSync(source, "g3_route_transitions::animated_navigate(route);");
    execFileSync("git", ["-C", root, "add", "app.rs"]);
    writeFileSync(source, "animated_navigate(route);");
    let result = run();
    assert.equal(result.status, 1);
    assert.match(result.stdout, /import animated_navigate/);
    execFileSync("git", ["-C", root, "add", "app.rs"]);
    writeFileSync(source, "g3_route_transitions::animated_navigate(route);");
    result = run();
    assert.equal(result.status, 0);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
