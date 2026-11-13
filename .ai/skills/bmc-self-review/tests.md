# Tests checklist

A test earns its place by failing when the behaviour it is named after breaks.

- **Fails when the behaviour breaks.** For each new or changed test, name the one-line change to production code that
  should make it fail, and check by reading that its assertion depends on that change. A test whose assertion can't fail
  is Major. Common causes:
  - it compares a value with itself (`assert_eq!(tracker.on_failure(), tracker.delay)`);
  - it starts from the state it claims to reach (it checks that a reset zeroes a counter that was never raised);
  - it only checks that the call didn't panic;
  - it restates what the code renders, such as the same copy in the component and the spec, which freezes the wording in
    two places and passes whatever the behaviour does.
- **Every acceptance criterion and every claim has a test.** Each acceptance criterion in the ticket, and each behaviour
  that a doc comment, the commit body or the MR description promises, is pinned by a test or listed as verified by hand:
  in the MR description once there is one, and in the report before.
- **The production path.** The test drives the code that ships, not a copy, a test-only reimplementation or a mock of
  the unit under test. Mock only what sits outside it.
- **Expected values come from outside the code under test.** Use literals, the spec, or a hand-worked example. A value
  computed by the same function or constant being tested proves nothing. Spec constants and repeated literals are fine.
- **Exact assertions.** Prefer `assert_eq!` on the whole value to `assert!(x > 0)`. Add a message when the values alone
  don't say why they matter (see the `comment-discipline` skill).
- **Deterministic.** No fixed sleep standing in for a condition (poll the condition under a timeout), no service the
  test doesn't own (a loopback listener it starts is fine), and no reliance on test order. Nor does it expect the system
  to hold still where the test doesn't control it: a port the test freed can be bound by anything on the machine, so it
  can't count on that port staying closed. A capture that reads one shared buffer from tests on several threads is flaky
  under the parallel runner: count per thread, as `bmc_render::test_tracing::counting_warns` does, or assert on returned
  state.
- **Use existing helpers.** Search the crate's `test_support.rs`, `tests/` and dev-dependencies before writing a new
  fixture builder. A second copy of a helper is the same finding as a second copy of production logic.
- **Where the tests go.** Tests go in the commit that adds the code, or in their own commits directly after it when that
  makes the change easier to review. Each commit still builds and passes on its own.
- **Rendering changes** in `bmc-render`, a widget or an overlay are checked in the gallery (`just gallery::capture` for
  a headless shot). A wasm widget is checked against its baselines with `just wasm::verify <widget>`, and an intended
  change re-captures them with `just wasm::update-baselines <widget>`. A rebase onto a master that moved the renderer
  needs a fresh re-capture too.
