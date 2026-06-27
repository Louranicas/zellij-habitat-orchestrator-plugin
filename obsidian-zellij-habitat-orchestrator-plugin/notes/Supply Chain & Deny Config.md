# Supply Chain & Deny Config

> Back to: [[MOC]] · [[Security & Admission Boundary]] · [[Release & Provenance]]
> Source: `deny.toml` · `Cargo.lock`

`cargo deny check` + `cargo audit` together form the **supply-chain gate**.
Neither blocks the build: both must exit 0 before a release ships.

---

## `deny.toml` — full config

### Target graph

```toml
[graph]
targets = [
    { triple = "x86_64-unknown-linux-gnu" },
    { triple = "wasm32-wasip1" },
]
```

Analyses both host (test + sidecar) and WASM (plugin) trees.

### Advisories — 3 suppressed, documented

```toml
[advisories]
ignore = [
    { id = "RUSTSEC-2025-0052",
      reason = "Transitive through zellij-tile 0.43.1/zellij-utils; no direct safe upgrade in this plugin workspace yet." },
    { id = "RUSTSEC-2024-0375",
      reason = "Transitive through zellij-tile 0.43.1 clap 3.x stack; tracked as upstream Zellij dependency risk." },
    { id = "RUSTSEC-2024-0370",
      reason = "Transitive through zellij-tile 0.43.1 clap derive stack; tracked as upstream Zellij dependency risk." },
]
```

All 3 advisories are **transitive-only through `zellij-tile`** — no direct
dependency of this workspace is affected. They are not fixable without
Zellij upstream updating their clap dependency or this workspace dropping
`zellij-tile` entirely (which would remove the WASM plugin capability).

**Risk posture:** acknowledged, tracked, not mitigable at this layer. The
advisories are `clap 3.x` and `zellij-utils` concerns — not in any code path
that this plugin or sidecar exercises directly.

### Licenses allowed

```toml
[licenses]
allow = [
    "0BSD", "Apache-2.0", "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause", "BSD-3-Clause", "CC0-1.0", "ISC", "MIT",
    "MPL-2.0", "OpenSSL", "Unicode-3.0", "Unicode-DFS-2016",
    "Unlicense", "WTFPL", "Zlib",
]
confidence-threshold = 0.8
```

Broad allowlist covering all standard OSS licenses. Private workspace crates
(`[licenses.private] ignore = true`) are excluded from license scanning.

### Bans

```toml
[bans]
multiple-versions = "warn"    # warn, don't fail, on duplicate semver versions
wildcards = "deny"            # disallow wildcard (*) version constraints
highlight = "all"
```

### Sources

```toml
[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
allow-git = []
```

All crates must come from `crates.io`. No git dependencies allowed. This
prevents supply-chain substitution via custom registries or git refs.

---

## `cargo audit` — RustSec advisory database

Run as part of the release checklist:
```bash
cargo audit
```

At v0.1.2 seal: exits 0 after the 3 documented advisories are suppressed in
`deny.toml`. `cargo audit` and `cargo deny check` are complementary — `audit`
gives a detailed report, `deny` provides config-file-based policy.

---

## Why `zellij-tile 0.43.1` is the constraint

The patched binary `a4c68619` is Zellij 0.44.3 but the `zellij-tile` crate
pinned in `Cargo.lock` is `0.43.1`. This version lag introduces the inherited
clap 3.x advisories. The constraint is:

- Zellij 0.44.3 binary (hand-patched PTY double-panic fix)
- `zellij-tile = "0.43.1"` (the API version compatible with 0.44.3)
- `clap 3.x` (zellij-tile's own dependency — clap 4.x broke the API)

Until Zellij upstream bumps to clap 4.x, the advisories remain in this
inherited chain. **Never run `cargo install zellij` (D7)** — doing so would
overwrite the hand-patched binary.

---

## See also

- [[Security & Admission Boundary]] — production guardrails and fail-closed rules
- [[Release & Provenance]] — v0.1.2 publish checklist including cargo audit
- [[notes/Durable Lessons & Design Decisions]] — D7 (never reinstall zellij)
