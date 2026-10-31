# Rust and Dioxus readability

Use the stable settings in `rustfmt.toml`. Keep this guide and
`scripts/rust-style.mjs` aligned with the common copies in g3-stack.

## Component layout

- Put one blank line between setup code and the main `rsx!` block. A component
  containing only markup can begin directly with `rsx!`.
- Separate related sections with one blank line: state, data hooks, derived
  values, effects and callbacks, loading/error handling, and rendering. Preserve
  hook order; all hooks still run before early returns.
- Separate substantial sibling sections inside RSX. Keep small related elements
  together. Long uninterrupted sections deserve review, rather than a fixed
  blank line every few statements.
- Expand markup with several properties or children, and handlers containing
  multiple statements, branches or asynchronous operations. One operation per
  line makes validation, state changes and error handling easier to follow.
- Extract components along state ownership and subscriptions. Check hook
  lifetimes, callback captures and hydration whenever moving code.

## Imports and Rust code

- Import frequently used functions and types, including `animated_navigate`.
  Keep qualification when it clarifies an ambiguous name, is required by a
  macro, or belongs to generated code. Preserve useful framework preludes.
- Group imports as standard library, external crates, then local modules.
  Rustfmt sorts within groups; the grouping itself is reviewed on stable Rust.
- Separate declarations and implementations. Use descriptive names across long
  scopes and comments explaining decisions, constraints or failure modes.
- Keep literal contents, embedded scripts, public APIs and behavior unchanged
  in formatting commits. Put component extraction in a separate change.
- Give lint overrides a `reason`. Review new opt-in lints on the pinned
  toolchain before making broader lint groups mandatory.

## Checks

`node scripts/rust-style.mjs --staged` checks the actual staged Rust contents,
including code inside RSX, without reading unrelated working-tree changes.
CI checks all tracked Rust files. `node scripts/rust-style.mjs --base HEAD^`
can inspect only the latest committed change.
`node scripts/rust-style.mjs --report` inventories existing readability debt.
The checker masks comments and Rust string/character literals, including raw
strings, so embedded JavaScript, SQL and documentation are ignored.

Run the repository's format, feature-specific check, strict lint and test
commands as well. Independent testbeds, playgrounds and macro crates need their
own coverage. Workspace members opt in to `[workspace.lints]` with
`[lints] workspace = true`; independent packages declare their own policy.

Keep RSX formatting manual while the app's Dioxus formatter restriction applies.
Before adopting an RSX formatter, reproduce the historical closure failures on
disposable copies and verify preserved syntax, literal contents and comments,
plus identical output on a second run.

References: [Rust Style Guide](https://doc.rust-lang.org/style-guide/),
[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html),
[rustfmt settings](https://github.com/rust-lang/rustfmt/blob/main/Configurations.md).
