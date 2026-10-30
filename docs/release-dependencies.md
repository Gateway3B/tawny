# Local development and release dependencies

Sibling overrides live outside this repository, in the parent directory:
C:/Users/Matthew/Documents/Files/Projects/.cargo/config.toml.
Cargo discovers that file when invoked from a project beneath it. It patches
the g3 crates, the Dioxus crates and the ecosystem forks using absolute paths.

`Cargo.toml` names each shared crate by its crates.io version and puts its
remote under `[patch.crates-io]`: the Dioxus crates, dioxus-icons,
dioxus-cookie and dioxus-sdk-time from their Gateway3B forks, and the g3 crates from
`g3techhq/<crate>` `main`, since nothing is published until Dioxus 0.8.0. A
config patch overrides a manifest patch for the same crate, so local builds use
the sibling checkouts and CI and Docker use the remotes.

Lefthook checks the staged Cargo.lock at commit time and rejects local shared
libraries unless the package belongs to this repository. It also refuses a
staged .cargo/config.toml containing shared-library overrides. Install the
hooks with lefthook install in a fresh checkout. Hooks can be bypassed with
--no-verify; they protect normal commits, not deliberate bypasses.

Local builds change Cargo.lock to point at the sibling paths, so leave it out
of commits while the overrides are active. Before pushing, push the fork and
g3 crate commits the app needs, temporarily rename the parent Cargo
configuration, re-lock against the remotes, commit the lockfile and run the
locked release checks. Restore the parent configuration afterward. Do not
fabricate registry checksums.

Any remaining repository-specific Cargo configuration contains unrelated
settings, such as the Windows NASM workaround, rather than sibling patches.
