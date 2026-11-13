# Frontend checklist

Biome and `just fe::validate` catch formatting and lint. The `frontend-style` and `bmc-ui-copy` skills set the code and
copy conventions. This list covers what reviews keep finding on top of them.

- **Every user-facing string goes through `react-intl`.** That includes errors, placeholders, `aria-label`s and toasts,
  with a `description` where the `bmc-ui-copy` skill asks for one. A hard-coded English string is a finding.
- **Errors reach the user.** A failed request that ends an operation shows the user something they can act on. A
  rejection that only goes to `console.error`, or a `catch` that falls back silently, leaves the UI lying. A probe that
  retries until a deadline ends the operation only when it gives up. The message names what happened: a failed change, a
  change that worked but couldn't be reloaded, and an unknown result are three different messages.
- **Auth failures go to login.** An auth failure on any request (`Unauthenticated` or `PermissionDenied` over gRPC, a
  401 over plain fetch) takes the user to that platform's login page. Route it through `parseError`, which already does.
- **Requests end.** A write that others queue behind, or that blocks a whole page while it's pending, has a deadline
  (`timeoutMs`), so a server that stops answering can't leave it stuck for good. A stream that stays open while a view
  is shown has none, but ends with the view. A page aborts its requests and streams when the user leaves it, so a late
  reply can't toast on the next page.
- **Loading and failed are not Off.** A setting or capability that hasn't loaded renders as unknown, never as a default
  that looks real. A capability gate fails closed.
- **Validate at the form.** Input is checked and converted to its typed value before the request is built. Don't rely on
  the control's type or on the backend to reject it.
- **No stale resubmits.** After a save, a form reloads the saved state from the server, from the response when it
  carries that state and with a fetch when it doesn't, rather than keeping what it loaded. Otherwise the next submit
  resends values the server has already replaced. While a request is pending, every control that changes the same
  resource is disabled, not only the submit.
- **Derive state, don't latch it.** A value computed from props or the store is computed during render. A `useState` +
  `useEffect` copy goes stale the moment its source changes. An expensive structure built from loaded data, such as a
  lookup `Map`, is built once on load instead.
- **Stories.** A visual state a reviewer needs to see, in a new component or one a change adds to an existing component,
  is shown in a story, the component's own or a parent's. A refactor carries the old component's stories over.
- **Generated types.** Request and response shapes are imported from the generated protobuf types under
  `frontend/src/proto`, or derived from them (`Pick`, `Omit`, a mapped type). A hand-written interface that mirrors one
  is a finding. A `switch` over a generated enum is exhaustive, through `assertUnreachable`.
- **Capabilities, not products.** Behaviour that depends on the device reads the capability surface (`/system.js`,
  `frontend/src/lib/capabilities.ts`), not a product-name comparison.
- **Specs** follow [tests.md](tests.md): a spec that can't fail when the component breaks is not a test.
