# Agent Guidelines (revisor)

This repository is a small, system-level Rust codebase (daemon supervisor) that favors correctness, simplicity, and explicit control of OS interactions. When you contribute, match the existing style and engineering intent.

## Goals and priorities

1. **Correctness and predictable behavior** over cleverness.
2. **Small surface area**: minimal dependencies, minimal abstraction, minimal dynamic allocation when it’s easy.
3. **Clear failure modes**: errors should be handled explicitly and reported with actionable context.
4. **Unix/system programming sensibilities**: be careful around signals, nonblocking I/O, and syscalls.

## Rust edition & formatting

- This crate uses **Rust edition 2024**.
- Formatting follows `rustfmt` with `use_small_heuristics = "Max"`.
  - Expect longer single-line constructs to stay on one line when rustfmt chooses to.
- Keep formatting idiomatic and let rustfmt decide; don’t hand-format against it.

## Code organization

- Prefer **small, focused modules** (`poll`, `sys`, `control_session`, `unit_manager`, etc.).
- Public API is minimal; most items are `pub` only when needed across modules.
- Keep “platform glue” in `sys`-like modules; keep application logic elsewhere.

### Imports

- Use `use` statements grouped roughly as:
  1. `std::...`
  2. external crates (`libc`, `slab`, etc.)
  3. `crate::...`
- Avoid unnecessary glob imports. Prefer explicit imports even if slightly verbose.

## Error handling & reporting

### General approach

- Prefer `Result<_, ()>` for top-level “print-and-exit” flows where detailed error types don’t add value.
- For reusable/system-level helpers, define and use concrete error types (e.g., `sys::Error`, `unit_manager::Error`) and implement `Display`.

### Logging style

- Use `eprintln!` for errors/warnings.
- Error messages are:
  - **specific**
  - **context-rich**
  - and often include the syscall or operation name.
- Follow patterns like:
  - `"Error <doing thing>: {}"`
  - `"Failed to <action> '<name>' (pid {}): {}"`

### Don’t hide errors

- Avoid swallowing errors silently.
- Use explicit `match` arms for common recoverable cases (e.g., `WouldBlock`, `Interrupted`).
- Use `.inspect_err(...)` for side-effect logging when it improves readability.

### Panics

- Panics are acceptable only for **internal invariants** that indicate programmer error, e.g.:
  - `expect("...")` when a map must contain a key
  - `panic!("...")` when an impossible state occurs
- Do not use panics for user/environment failures (I/O, parsing, permissions).

## System calls, EINTR, and nonblocking I/O

- Use retry helpers for EINTR:
  - Prefer `sys::retry_eintr` for `io::Result` operations.
  - Prefer `sys::wrap_libc_retry_eintr` / `sys::wrap_libc` for raw `libc` calls.
- Treat `WouldBlock` as a normal event in nonblocking code; do not log it as an error.
- Ensure new file descriptors/sockets are set `O_CLOEXEC` and nonblocking where appropriate.

## State machines and event loops

- The project favors a **simple event-driven loop** with explicit state.
- When adding new pollable items:
  - Integrate with `Poller` via a `PollItem`-like enum.
  - Keep per-connection/session buffers bounded (fixed-size arrays are common here).
  - Update interest sets via `Reaction::Events(...)` to avoid busy loops.
- Maintain clear separation between:
  - “poll input”
  - “poll output”
  - and “derive desired events”

## Data structures & performance

- Prefer straightforward standard structures (`HashMap`, `BinaryHeap`, `Vec`).
- `slab::Slab` is used for stable keys and O(1) removal via swap-remove patterns.
- Keep allocations and copies modest; fixed buffers and `copy_within` patterns are acceptable and idiomatic in this codebase.

## Naming & style conventions

- Use `snake_case` for functions/vars, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Choose names that reflect intent:
  - `poll_units`, `poll_timers`, `notify_terminated`, `terminate_unit`
- Prefer small helper functions over deeply nested logic, but don’t over-abstract.

## CLI and user-facing behavior

- CLI parsing uses `getopts` and a small `util::parse_args` helper.
- Behavior should be consistent:
  - `--help` prints usage and exits successfully.
  - Argument validation prints a clear error and returns failure.
- Keep usage strings and option descriptions crisp and accurate.

## Safety and `unsafe`

- `unsafe` is allowed where necessary for syscalls, but:
  - keep the unsafe block as small as possible,
  - document invariants with comments (especially around signal safety),
  - and route through `sys` helpers where it improves consistency.

## Testing and validation (lightweight)

There may not be a full test suite; prefer these practices:

- Add small, testable helper functions when it improves confidence.
- When changing behavior in the event loop or timers, reason about:
  - starvation
  - busy looping
  - missed wakeups
  - and race-like conditions around SIGCHLD + `waitpid(WNOHANG)`.

## Dependency philosophy

- Keep dependencies minimal.
- Prefer `std` + small crates already in use (`slab`, `once_cell`, etc.).
- Introduce new crates only when they materially reduce complexity or risk.

## When you add or modify code

- Match existing patterns for:
  - error messages
  - handling `WouldBlock`/`Interrupted`
  - and internal invariants with `expect(...)`
- Keep changes tight and avoid unrelated refactors.
- If you must refactor, do it in small, reviewable steps.

## Practical examples to follow (conceptual)

- If you add a syscall wrapper, mirror `sys::wrap_libc`:
  - return a typed error with syscall name + `last_os_error`
  - treat negative returns as failure
- If you add session commands, mirror `ControlSession::process_command`:
  - parse as UTF-8
  - tokenize with simple delimiters
  - respond with bounded writes into an output buffer
  - never allow unbounded response growth

---

If you’re unsure, prefer the simplest change that maintains:
- explicit control flow,
- bounded resource usage,
- and clear diagnostics.