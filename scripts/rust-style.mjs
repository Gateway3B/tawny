#!/usr/bin/env node
// Check Rust readability rules that rustfmt cannot enforce inside RSX.
//
// Shared with the g3 repositories; maintain the common copy in g3-stack.
// Uses only Node's standard library. This checker never rewrites source.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const RAW = /(?:br|cr|r)(#*)"/y;
const QUOTED = /(?:b|c)?"/y;
const CHARACTER = /b?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\r\n])'/uy;

function matchAt(pattern, source, offset) {
  pattern.lastIndex = offset;
  return pattern.exec(source);
}

/**
 * Find comments and literals without interpreting embedded code.
 *
 * Rust permits nested block comments, raw strings with arbitrary hash counts,
 * byte/C strings, character literals and lifetimes. In particular, embedded
 * JavaScript, SQL and documentation must never be interpreted as Rust code.
 */
export function* nonCodeRanges(source) {
  let offset = 0;
  while (offset < source.length) {
    let end = offset;
    if (source.startsWith("//", offset)) {
      end = source.indexOf("\n", offset);
      if (end < 0) end = source.length;
    } else if (source.startsWith("/*", offset)) {
      end = offset + 2;
      let depth = 1;
      while (end < source.length && depth) {
        if (source.startsWith("/*", end)) {
          depth += 1;
          end += 2;
        } else if (source.startsWith("*/", end)) {
          depth -= 1;
          end += 2;
        } else {
          end += 1;
        }
      }
    } else {
      const raw = matchAt(RAW, source, offset);
      const quoted = !raw && matchAt(QUOTED, source, offset);
      const character = !raw && !quoted && matchAt(CHARACTER, source, offset);
      if (raw) {
        const closing = `"${raw[1]}`;
        const last = source.indexOf(closing, offset + raw[0].length);
        end = last < 0 ? source.length : last + closing.length;
      } else if (quoted) {
        end = offset + quoted[0].length;
        while (end < source.length) {
          if (source[end] === "\\") {
            end += 2;
          } else if (source[end] === '"') {
            end += 1;
            break;
          } else {
            end += 1;
          }
        }
      } else if (character) {
        end = offset + character[0].length;
      }
    }
    if (end > offset) {
      yield [offset, Math.min(end, source.length)];
      offset = end;
    } else {
      offset += 1;
    }
  }
}

/** Hide comments and literals, retaining offsets and newlines. */
export function codeMask(source) {
  const masked = source.split("");
  for (const [start, end] of nonCodeRanges(source)) {
    for (let index = start; index < end; index += 1) {
      if (source[index] !== "\r" && source[index] !== "\n") masked[index] = " ";
    }
  }
  return masked.join("");
}

// Every Unicode line boundary, so line numbers match what editors show. Built
// from a string because Biome rejects control characters in a regex literal.
// biome-ignore lint/complexity/useRegexLiterals: a literal trips noControlCharactersInRegex
const LINE_BREAK = new RegExp("\r\n|[\n\r\v\f\x1c\x1d\x1e\x85\u2028\u2029]");

/** [line, message] pairs for concrete, locally actionable issues. */
export function issues(source) {
  const found = [];
  const masked = codeMask(source);
  const lines = masked.split(LINE_BREAK);
  if (lines.length > 1 && lines.at(-1) === "") lines.pop();
  lines.forEach((line, index) => {
    if (index && /^ {4}rsx!\s*\{/.test(line)) {
      const previous = lines[index - 1].trim();
      if (previous && !previous.endsWith("{")) {
        found.push([index + 1, "add a blank line between setup and the main rsx! block"]);
      }
    }
  });
  for (const match of masked.matchAll(/\bg3_route_transitions\s*::\s*animated_navigate\s*\(/g)) {
    const line = masked.slice(0, match.index).split("\n").length;
    found.push([line, "import animated_navigate with a use statement"]);
  }
  return found;
}

function git(root, ...args) {
  return execFileSync("git", ["-C", root, ...args], { maxBuffer: 1 << 30 });
}

const withoutBom = (text) => (text.startsWith("﻿") ? text.slice(1) : text);

function main() {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      staged: { type: "boolean" },
      base: { type: "string" },
      report: { type: "boolean" },
    },
  });
  if (values.staged && values.base) {
    console.error("--staged and --base are mutually exclusive");
    return 2;
  }
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const list = (...args) =>
    git(root, ...args)
      .toString()
      .split("\0");
  let files;
  if (positionals.length) {
    files = positionals;
  } else if (values.staged) {
    files = list("diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z", "--", "*.rs");
  } else if (values.base) {
    files = list(
      "diff",
      values.base,
      "HEAD",
      "--name-only",
      "--diff-filter=ACMR",
      "-z",
      "--",
      "*.rs",
    );
  } else {
    files = list("ls-files", "-z", "--", "*.rs");
  }
  let count = 0;
  for (const name of files.filter(Boolean)) {
    const path = resolve(root, name);
    let source;
    if (values.staged) {
      source = withoutBom(git(root, "show", `:${name}`).toString("utf8"));
    } else if (values.base) {
      source = withoutBom(git(root, "show", `HEAD:${name}`).toString("utf8"));
    } else if (existsSync(path) && statSync(path).isFile()) {
      source = withoutBom(readFileSync(path, "utf8"));
    } else {
      continue;
    }
    for (const [line, message] of issues(source)) {
      console.log(`${name}:${line}: ${message}`);
      count += 1;
    }
  }
  if (count) console.log(`${count} Rust readability issue(s). See docs/code-style.md.`);
  return count && !values.report ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main();
}
