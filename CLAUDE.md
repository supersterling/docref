# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

docref is a Rust CLI tool that creates semantic code references in markdown documentation. You write `[text](file#symbol)` or `[text](file)` links in markdown, and docref tracks whether the referenced code has changed via SHA-256 hashes stored in a lockfile (`.docref.lock`). It uses tree-sitter for language-aware symbol resolution and semantic hashing that ignores whitespace/comments.

## Build & Test Commands

```bash
cargo build                    # Build debug
cargo build --release          # Build release
cargo test                     # All tests (unit + integration)
cargo test --lib               # Unit tests only
cargo test --test integration  # Integration tests only
cargo test <test_name>         # Single test by name
cargo clippy                   # Lint (extremely strict — see below)
cargo fmt                      # Format (nightly required, unstable features enabled)
cargo fmt -- --check           # Check formatting without modifying
```

## Lint & Format Severity

This project has **~450 clippy lints set to deny** in `Cargo.toml` under `[lints.clippy]`. Key strictness settings from `clippy.toml`:

- `cognitive-complexity-threshold = 10`
- `excessive-nesting-threshold = 3` (forces function extraction at 3 levels)
- `too-many-lines-threshold = 40` (max function body length)
- `check-private-items = true` (doc comments required on private items)
- `module-items-ordered-within-groupings = "all"` (alphabetical ordering enforced)

Notable allowed/warn overrides:
- `needless_return = "allow"` — explicit returns are the project style
- `expect_used = "deny"` and `indexing_slicing = "deny"` — no panicking code; use `get()` + error handling

**rustfmt** uses nightly with `unstable_features = true`. Key non-default settings: `max_width = 120`, `group_imports = "StdExternalCrate"`, `imports_granularity = "Module"`, `imports_layout = "Vertical"`, `match_arm_leading_pipes = "Always"`, `struct_lit_single_line = false`, `single_line_if_else_max_width = 0`, `struct_field_align_threshold = 60`.

## Architecture

The data flow is: **markdown → scanner → resolver → hasher → lockfile**.

### Pipeline Stages

1. **Scanner** (`scanner.rs`) — Walks markdown files, regex-extracts `[text](path#symbol)` links, groups `Reference` structs by target file path. Handles relative path normalization and namespace-prefixed targets (`auth:src/lib.rs`).

2. **Grammar** (`grammar.rs`) — Maps file extensions to tree-sitter `Language` objects. Supported: `.rs`, `.ts`, `.tsx`, `.js`, `.jsx`, `.py`, `.go`, `.sh`/`.bash`.

3. **Resolver** (`resolver.rs`) — Parses source files with tree-sitter, walks the CST to find named declarations. Supports bare symbols (`add`), dot-scoped symbols (`Config.validate`), and whole-file references. Each language has its own declaration-walking logic. Returns `ResolvedSymbol` with byte ranges.

4. **Hasher** (`hasher.rs`) — Re-parses the resolved byte range, walks leaf tokens stripping comments/whitespace, joins with spaces, SHA-256 hashes the normalized form. This makes hashes resilient to formatting changes.

5. **Lockfile** (`lockfile.rs`) — TOML-serialized sorted `Vec<LockEntry>`. Entries ordered by `(source, target, symbol)`. Enforces sort invariant on read.

6. **Freshness** (`freshness.rs`) — Compares lockfile entries against current source. Returns `Fresh`/`Stale`/`Broken` per entry. Used by `check`, `status`, and `watch` commands.

### Supporting Modules

- **Config** (`config.rs`) — Loads `.docref.toml` with include/exclude path-prefix filters and namespace mappings. Supports `extends` chains with cycle detection.
- **Namespace** (`namespace.rs`) — CRUD operations on namespace mappings using `toml_edit` for format-preserving config edits. Rename cascades across config, lockfile, and markdown files.
- **Commands** (`commands.rs`) — Orchestrates all CLI subcommands: `init`, `check`, `status`, `update`, `fix`, `resolve`, `refs`, `info`.
- **Diagnostics** (`diagnostics.rs`) — Renders structured markdown error messages with fix suggestions. Includes fuzzy matching that strips generics for `fix` suggestions.
- **Watch** (`watch.rs`) — Uses `notify` crate for filesystem watching with debounce, re-runs `check` on changes.

### Key Types (`types.rs`)

- `SymbolQuery` — `Bare("add")`, `Scoped { parent: "Config", child: "validate" }`, or `WholeFile`
- `SemanticHash` — Newtype over hex-encoded SHA-256 string
- `Reference` — Parsed markdown link with source location, target path, and symbol query

### Exit Codes (check command)

- `0` — all fresh
- `1` — stale references (code changed since docs written)
- `2` — broken references (symbol/file missing)
- `3` — internal error

## Integration Tests

Tests live in `tests/integration.rs` using fixture directories under `tests/fixtures/`. Each test copies a fixture into a `tempfile::TempDir` and runs the compiled binary via `std::process::Command`. Fixtures cover: basic Rust/TS, configured include/exclude, namespaced monorepos, scoped symbols, whole-file refs, Go, Python, JS, Bash.

## Code Style

- **Explicit returns everywhere** — `needless_return` is allowed; every function uses `return`
- **Every item has a doc comment** — including private functions and struct fields
- **Alphabetical ordering** — modules, functions within impl blocks, struct fields
- **Result types over panics** — no `unwrap()` or `expect()` in non-test code; use `get()` with proper error variants
- **Guard clauses with early returns** — keep nesting under 3 levels
- **Functions under 40 lines** — extract helpers aggressively
