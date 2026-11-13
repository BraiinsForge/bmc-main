# Rust checklist

Clippy and rustfmt already gate everything they can catch. This list covers the judgement calls they can't.

## Types

- **One state, one enum.** Fields that must agree with each other (`is_connected`, `is_connecting`, `gave_up`) are one
  enum with a variant per state, and each variant carries the data that exists only in that state. Ask what a
  combination of the fields that should never happen would mean. If the answer is "a bug", the type allows it.
- **A mode is an enum, not a `bool`.** A `bool` parameter, field or return value that picks a behaviour, rather than
  answering a yes/no question, becomes an enum whose variants name the choices. `render(true)` tells a reader nothing.
- **`Option` for absence.** `0`, `-1`, `u64::MAX`, `""` or `Duration::ZERO` never mean "none". A doc comment that says
  "0 when never" is the tell. The one exception is the required string param defaulting to `""` that "Defaults and
  Validators" in `.ai/instructions.md` allows.
- **Unknown is its own state.** Absent, failed to read and malformed stay distinct where the consumer acts differently
  on them. A failed read never turns into a value that looks real (`None` as "not connected", `unwrap_or_default()` as
  `NONE`).
- **Integer widths:**
  - counts, lengths, indices and sizes are `usize`;
  - signed differences are `isize`;
  - any other fixed width is a finding only for a problem it causes: casts back and forth, a value that can outgrow it,
    or a mismatch with what it is compared to. Where a wire format, a protocol, FFI, a hardware register or an external
    API sets the width, keep it at that edge and convert once.
- **Domain types over raw numbers.** Use `Duration` and `SystemTime`/`Instant` instead of `u64` seconds or milliseconds.
  Where a wire or persisted format carries raw seconds or milliseconds, keep the number at that edge and convert to
  `Duration` once. Use a newtype for an id that could be confused with another id or with a count.
- **No pointless casts.** A cast that only exists to move between two widths you chose yourself means one of them is
  wrong.

## Panics

- Every `expect`, `unwrap` or `unreachable!` in production code needs a justification: why the invariant behind it can't
  be enforced at compile time. If you can't give one, restructure so the type carries the invariant. The
  `expect("BUG: …")` message states the justification.

  ```rust
  // don't: check, then unwrap the same Option again
  if self.session.is_some() {
      self.session.as_ref().expect("BUG: checked above").send(frame);
  }
  // do: keep the value the check produced
  if let Some(session) = &self.session {
      session.send(frame);
  }
  ```

- An index needs the same only when it comes from outside input, or was checked against a different collection than the
  one it indexes. An index bounded by the same slice's `len()`, or produced by iterating it, needs nothing.

- Input from outside the process can be anything: a network reply, a file, a header, a config value, a widget, an IPC
  peer. It goes through a fallible parse that returns an error. "The server always sends an integer" doesn't hold.

- A function that panics unless its caller does something first is Major. Make the type enforce the order, or return an
  error.

- `debug_assert!` is never the only guard on input that can plausibly occur, because release builds compile it out. Use
  it only for invariants this module establishes itself.

## Reuse and ownership

- **Reimplementation.** Before keeping a new helper, backoff, parser, clamp, formatter or state machine, search the
  workspace for the operation itself, not just its name: `rg 'saturating_mul|checked_pow'`, `rg 'MAX_.*DELAY'`, or the
  literal constants. A second copy of logic that can drift (a state machine, a protocol rule, a parser) is Major; a few
  lines of glue are a Nit. Call the existing code, or move it somewhere both callers can reach.
- **Standard parsers.** URLs, IP addresses, times, versions and MIME types are standardized formats. A hand parser for
  one is a finding unless the developer chose it, as `.ai/instructions.md` asks, and the commit body records that. The
  two cases it exempts, a grammar the repo defines and `*.example.com` host matching, are not findings.
- **Libraries over ad hoc code.** See the `code-style` skill.

## Behaviour

- **Docs match the code.** Every doc comment claim ("gives up after N attempts", "never blocks") holds in the code. A
  claim that's half done is Major.
- **No dead or speculative API.** Every new `pub` item has a caller in the branch, or serves a documented external
  consumer: widget authors through `bmc-wasm-runtime/sdk`, or `bos-main` through the shared crates. A new module with
  neither is a finding, unless the ticket scopes it as a library for a later ticket. Remove fields that nothing reads.
- **No secrets in logs or errors.** A session id, password, token or key never reaches a log line or an error message,
  directly or through a `Debug` derive that gets logged. Log that it is present, not its value.
- **Errors are never swallowed.** `let _ =`, `.ok()`, or `if let Ok` with no `else` on a fallible call needs a reason:
  the error is logged with context, returned, or provably irrelevant. "Ignore it and carry on" is a decision the code
  states.
- **Bounded waits.** A wait that can stall an operation or hold a lock or gate has a timeout, and each call fits the
  overall deadline it serves. A transfer that is still moving data may run without a total cap behind an idle timeout;
  say which one it has.
- **Arithmetic on outside input or an unbounded counter** says what happens at the limit, with `checked_`, `saturating_`
  or `wrapping_`. Release builds don't check overflow, so it wraps silently into a wrong value.
- **Classify failures.** Retry only a failure that can clear on its own. A device fault (disk full, permissions) is not
  reported as the client's bad input.
- **Cleanup on every exit path.** Resources, locks, temp files and spawned tasks are released on early return and error
  paths too. That usually means a `Drop` guard rather than cleanup code at the end.
- **Bounded growth.** A collection that grows with outside input (requests, widgets, log lines, retries) has a cap or an
  eviction rule.
- **Named constants.** Magic numbers get a `const` with a unit in the name or type. A value derived from another value
  is computed from it, not copied.
- **Test-only accessors** sit behind `#[cfg(test)]` or a test feature, not in the public API.

## Readability

These hold for tests as much as for production code.

- **Multi-line text is written as it reads.** A script, a manifest or any other multi-line literal uses `indoc!` or
  `formatdoc!`, not a string of `\n` joins or `concat!` pieces.
- **Structured data is built typed.** JSON, TOML or a manifest is built from the crate's own type, or at least with
  `serde_json::json!`. A `format!` with escaped quotes, or a template patched with `.replace`, is a finding.
- **One language per literal.** JSON escaped inside shell inside a Rust string is hard to read and easy to break. Build
  each layer on its own and join them in a helper. Data reaches a script through a quoted heredoc (`cat <<'EOF'`), not
  `echo '…'`, which dash runs with backslash escapes and an apostrophe breaks.
- **A fixture repeated across tests** becomes one `const` or builder in the crate's test support.

## Files and modules

- 1000 lines is the point to start asking whether a file holds more than one concern, not a limit. When the branch
  creates a file past it, or grows one well past it, look for the seams the code already has: separate concerns, a type
  with its own `impl` blocks, inline tests. Split along those. A file with no such seam stays long, and the line count
  alone is never a reason to cut it.
- When a file is already past it before the branch touches it, the same question applies, in a commit of its own. Weigh
  it against other work-in-progress branches that touch the file: a move-only split conflicts with all of them.
- Inline tests count toward the length. When they are what makes a file long, moving them to `name/tests.rs` is the
  first step. Moving only the tests out doesn't make the module newly split: an older `name.rs` keeps its layout with
  `name/tests.rs` beside it, and converting it to `name/mod.rs` is a move-only commit of its own.
- Split as the module system section of `.ai/instructions.md` describes: a thin `mod.rs` that declares the submodules
  and holds the public surface, and one file per concern.
