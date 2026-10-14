# Local development and release dependencies

Sibling overrides live outside this repository, in the parent directory:
C:/Users/Matthew/Documents/Files/Projects/.cargo/config.toml.
Cargo discovers that file when invoked from a project beneath it. It patches
g3-auth, g3-cache, g3-native-plugins, g3-route-transitions and g3-ui using
absolute paths. CI and Docker use versioned crates.io dependencies.

Lefthook checks the staged Cargo.lock at commit time and rejects local shared
libraries unless the package belongs to this repository. It also refuses a
staged .cargo/config.toml containing shared-library overrides. Install the
hooks with lefthook install in a fresh checkout. Hooks can be bypassed with
--no-verify; they protect normal commits, not deliberate bypasses.

Local builds can change Cargo.lock. Before releasing, publish g3-auth 0.1.3,
g3-cache 0.2.1 and g3-native-plugins 0.4.4 through their gated workflows.
Temporarily rename the parent Cargo configuration, update the application
lockfile from crates.io, and run the locked release checks. Restore the parent
configuration afterward. Do not fabricate registry checksums.

Any remaining repository-specific Cargo configuration contains unrelated
settings, such as the Windows NASM workaround, rather than sibling patches.
