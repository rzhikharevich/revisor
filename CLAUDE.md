# CLAUDE.md

## Project Overview

Revisor is a runit-inspired daemon supervisor written in Rust (2024 edition). It consists of two binaries: `revisor` (the main daemon) and `rvctl` (CLI control utility).

## Build

```sh
cargo build
```

Size-optimized build (requires nightly):

```sh
cargo +nightly build -Z build-std=std,panic_abort -Z build-std-features=optimize_for_size --profile=small
```

There is no test suite.

## Code Style

### Formatting

Run `rustfmt` before committing. The project uses `use_small_heuristics = "Max"` in `rustfmt.toml`, which allows rustfmt to use longer lines and inline more aggressively.

### Imports

Imports are ordered in four groups, separated by blank lines:

1. `extern crate` declarations
2. `mod` declarations
3. `use` from `std` / `crate`
4. `use` from external crates and local modules

```rust
extern crate libc;

mod poll;
mod sys;

use std::io;
use std::os::fd::AsRawFd;

use libc::pid_t;

use crate::sys;
```

### Error Handling

- Use `Result<T, E>` with custom error types (`sys::Error`, `unit_manager::Error`).
- Propagate errors with `?` and `.map_err()`.
- Log errors to stderr with `eprintln!()`, including context: `eprintln!("Error doing X: {}", err)`.
- Do not use `anyhow`, `thiserror`, or similar crates.

### Unsafe Code

- Wrap libc calls in `sys::wrap_libc()` rather than calling them directly.
- Add a `// SAFETY:` comment explaining the safety invariants whenever `unsafe` is used.

### General Conventions

- Minimal dependencies — avoid adding crates when the standard library or a small wrapper suffices.
- No `#[macro_use]`; use explicit paths or `use` imports instead.
- Use `#[allow(dead_code)]` on items that are part of the public API but not yet used internally.
