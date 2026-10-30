# Dioxus

The app is written against **Dioxus 0.8.0-alpha.3** from the
[Gateway3B/dioxus](https://github.com/Gateway3B/dioxus) fork: upstream `main`
plus our `g3:` commits (listed in the fork's `G3-PATCHES.md`). `dioxus`,
`dioxus-fullstack` and `manganis` are pinned with `=`, taken from the fork
through `[patch.crates-io]`, and must match a `dx` built from the same fork.

| File | What it is |
| --- | --- |
| [patterns.md](patterns.md) | **Start here.** How the g3 apps use Dioxus: props, hooks, stores, cached data, hydration, and the tooling pitfalls we have hit. Ours; kept current. |
| [framework-agents.md](framework-agents.md) | Dioxus's own agent guide, from the framework repository. |
| [architecture/](architecture/00-OVERVIEW.md) | The framework's internal architecture notes: core, CLI, RSX, signals and stores, fullstack, renderers, hot reload, assets, router, WASM splitting, native plugins, manifests. |

`framework-agents.md` and `architecture/` are copied unchanged from the fork's
`main` (`AGENTS.md` and `notes/architecture/`); the commit is in each file's
first line. They describe the framework, not this app. Refresh them when the
fork is rebased; do not edit them here.

Where they and `patterns.md` disagree about how to write code *in this app*,
`patterns.md` wins.
