"""Check Rust readability rules that rustfmt cannot enforce inside RSX.

Shared with the g3 repositories; maintain the common copy in g3-stack.
Uses only Python's standard library. This checker never rewrites source.
"""

import argparse
import re
import subprocess
from pathlib import Path

RAW = re.compile(r'(?:br|cr|r)(#*)"')
QUOTED = re.compile(r'(?:b|c)?"')
CHARACTER = re.compile(r"(?:b)?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\r\n])'")


def non_code_ranges(source):
    """Find comments and literals without interpreting embedded code.

    Rust permits nested block comments, raw strings with arbitrary hash counts,
    byte/C strings, character literals and lifetimes. In particular, embedded
    JavaScript, SQL and documentation must never be interpreted as Rust code.
    """
    offset = 0
    while offset < len(source):
        end = offset
        if source.startswith("//", offset):
            end = source.find("\n", offset)
            if end < 0:
                end = len(source)
        elif source.startswith("/*", offset):
            end = offset + 2
            depth = 1
            while end < len(source) and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
        else:
            raw = RAW.match(source, offset)
            quoted = QUOTED.match(source, offset)
            character = CHARACTER.match(source, offset)
            if raw:
                closing = '"' + raw[1]
                last = source.find(closing, raw.end())
                end = len(source) if last < 0 else last + len(closing)
            elif quoted:
                end = quoted.end()
                while end < len(source):
                    if source[end] == "\\":
                        end += 2
                    elif source[end] == '"':
                        end += 1
                        break
                    else:
                        end += 1
            elif character:
                end = character.end()
        if end > offset:
            yield offset, min(end, len(source))
            offset = end
        else:
            offset += 1


def code_mask(source):
    """Hide comments and literals, retaining offsets and newlines."""
    masked = list(source)
    for start, end in non_code_ranges(source):
        for index in range(start, end):
            if source[index] not in "\r\n":
                masked[index] = " "
    return "".join(masked)


def issues(source):
    """Yield (line, message) pairs for concrete, locally actionable issues."""
    masked = code_mask(source)
    lines = masked.splitlines()
    for index, line in enumerate(lines):
        if re.match(r"    rsx!\s*\{", line) and index:
            previous = lines[index - 1].strip()
            if previous and not previous.endswith("{"):
                yield index + 1, "add a blank line between setup and the main rsx! block"
    for match in re.finditer(r"\bg3_route_transitions\s*::\s*animated_navigate\s*\(", masked):
        yield masked.count("\n", 0, match.start()) + 1, "import animated_navigate with a use statement"


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--staged", action="store_true", help="inspect staged Rust blobs, including changes inside RSX")
    parser.add_argument("--base", help="inspect files changed since this Git revision")
    parser.add_argument("--report", action="store_true", help="report existing debt without failing")
    parser.add_argument("files", nargs="*", help="optional paths relative to the repository root")
    args = parser.parse_args()
    if args.staged and args.base:
        parser.error("--staged and --base are mutually exclusive")
    root = Path(__file__).resolve().parent.parent
    if args.files:
        files = args.files
    elif args.staged:
        files = git(root, "diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z", "--", "*.rs").decode().split("\0")
    elif args.base:
        files = git(root, "diff", args.base, "HEAD", "--name-only", "--diff-filter=ACMR", "-z", "--", "*.rs").decode().split("\0")
    else:
        files = git(root, "ls-files", "-z", "--", "*.rs").decode().split("\0")
    count = 0
    for name in filter(None, files):
        path = root / name
        if args.staged:
            source = git(root, "show", ":" + name).decode("utf-8-sig")
        elif args.base:
            source = git(root, "show", "HEAD:" + name).decode("utf-8-sig")
        elif path.is_file():
            source = path.read_text(encoding="utf-8-sig")
        else:
            continue
        for line, message in issues(source):
            print(f"{name}:{line}: {message}")
            count += 1
    if count:
        print(f"{count} Rust readability issue(s). See docs/code-style.md.")
    return int(bool(count) and not args.report)


if __name__ == "__main__":
    raise SystemExit(main())
