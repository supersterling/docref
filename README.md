# docref

Semantic code references for markdown — detect when referenced code changes.

## The problem

Documentation lies. Not on purpose — it starts accurate. But code evolves: functions get renamed, signatures change, entire modules get refactored. The markdown that references them still compiles, still renders, still looks fine. It just quietly stops being true.

Grep won't save you. A function renamed from `validate` to `check_input` leaves zero trace in the docs that still say "see the `validate` function." Traditional link checkers only verify that files exist, not that the code inside them matches what you documented.

## How docref solves it

docref uses [tree-sitter](https://tree-sitter.github.io/tree-sitter/) to parse source files and extract the exact body of each referenced symbol — function, type, constant, method. It then produces a **semantic hash** (SHA-256) of the normalized token stream, stripping whitespace and comments. This hash goes into a lockfile (`.docref.lock`).

When you run `docref check`, it re-parses, re-hashes, and compares. If a symbol's body changed, the reference is **stale**. If the symbol or file is gone entirely, the reference is **broken**.

The semantic hashing is the key insight: reformatting your code, adding comments, or changing indentation won't trigger false positives. Only actual changes to the code's tokens — the stuff that matters — produce a different hash.

## Quick start

Say you have a Rust file and a markdown doc that references it:

```rust
// src/lib.rs
const BASE_OFFSET: i32 = 10;

fn add(x: i32) -> i32 {
    x + BASE_OFFSET
}
```

```markdown
<!-- docs/guide.md -->
# Guide

The constant [`BASE_OFFSET`](../src/lib.rs#BASE_OFFSET) sets the base value.

The [`add`](../src/lib.rs#add) function applies the offset.
```

Now set up tracking:

```bash
# Install
cargo install docref

# Create a config so docref knows where to look
cat > .docref.toml << 'EOF'
include = ["docs/"]
EOF

# Scan markdown, resolve symbols, hash them, write the lockfile
docref init

# Verify everything is fresh
docref check  # exit 0 — all good
```

Later, someone changes the function:

```rust
fn add(x: i32, y: i32) -> i32 {
    x + y + BASE_OFFSET
}
```

```bash
docref check  # exit 1 — stale reference detected
```

The docs still say `add` "applies the offset," but the function now takes two arguments. Time to update the docs, then tell docref you've caught up:

```bash
docref update docs/guide.md#add
```

## Reference syntax

docref recognizes four forms of markdown links as trackable references:

```
[text](path/to/file.rs#symbol)          symbol reference
[text](path/to/file.rs#Type.method)     dot-scoped reference
[text](ns:path/to/file.rs#symbol)       namespaced reference
[text](path/to/file.rs)                 whole-file reference
```

**Symbol references** (`#symbol`) track a specific function, type, constant, or variable. Use `docref resolve <file>` to see what symbols are addressable in a given file.

**Dot-scoped references** (`#Type.method`) target a symbol nested inside a parent — like a method on a struct or a function inside a class. The parent and child are separated by a dot.

**Namespaced references** (`ns:path`) use a short alias instead of a relative path. Useful in monorepos where docs and source live far apart in the directory tree.

**Whole-file references** (no `#`) track the entire file's content. Use these for config files, scripts, templates, or anything where no specific symbol applies.

## Supported languages

| Extension       | Language   |
|-----------------|------------|
| `.rs`           | Rust       |
| `.ts` `.tsx`    | TypeScript |
| `.js` `.jsx`    | JavaScript |
| `.py`           | Python     |
| `.go`           | Go         |
| `.bash` `.sh`   | Bash       |

All languages support bare symbol and whole-file references. Dot-scoped references work wherever the language has nested declarations (methods on types, functions inside classes, etc.).

## Configuration

docref uses `.docref.toml` in your project root:

```toml
include = ["docs/", "src/"]         # only scan these paths for markdown
exclude = ["docs/archive/"]          # skip these within included paths
extends = "../.docref.toml"          # inherit from a parent config

[namespaces]
auth = "services/auth"               # auth:src/lib.rs → services/auth/src/lib.rs
```

**Include/exclude patterns are path prefixes, not globs.**

**Important:** Without a `.docref.toml`, docref scans *all* markdown under the project root — including `node_modules/`, `vendor/`, `.next/`, and every other directory. Always create a config with `include` patterns before running `docref init`.

## Commands

```
docref init                          Scan markdown, hash symbols, write .docref.lock
docref check                         Verify all references (exit 0/1/2)
docref status                        Show freshness of all tracked references
docref update <file#symbol>          Re-hash after intentional code changes
docref update --from <file.md>       Re-hash all refs from a markdown file
docref update --all                  Re-hash everything
docref fix                           Auto-fix all broken refs (closest match)
docref fix <file#sym> <newsym>       Fix a specific broken reference
docref resolve <file>                List addressable symbols in a source file
docref refs <file#symbol>            Show which markdown files reference a target
docref namespace add <name> <path>   Map a short name to a directory
docref namespace list                Show all namespace mappings
docref namespace remove <name>       Remove a namespace mapping
docref namespace rename <old> <new>  Rename (rewrites config + lockfile + markdown)
docref info                          Show comprehensive reference document
docref info --json                   Machine-readable output
docref watch                         Watch source files, re-check on changes
```

## Path resolution

**Paths are relative to the markdown file, not the project root.**

```markdown
<!-- In docs/guide.md, referencing src/config.rs: -->
[load](../src/config.rs#load)        <!-- CORRECT (relative from docs/) -->
[load](src/config.rs#load)           <!-- WRONG (resolves to docs/src/config.rs) -->
```

For cross-directory references, prefer namespaces over deep relative paths:

```markdown
<!-- Fragile — breaks if the markdown file moves: -->
[load](../../services/auth/src/config.rs#load)

<!-- Robust — works from any markdown file: -->
[load](auth:src/config.rs#load)
```

Set up namespaces with `docref namespace add <name> <path>`.

## Exit codes

| Code | Meaning                        |
|------|--------------------------------|
| 0    | All references fresh           |
| 1    | Stale references found         |
| 2    | Broken references found        |
| 3    | Runtime error                  |

Use exit code 1 or 2 as a CI gate to block merges when documentation drifts from code.

## License

MIT
