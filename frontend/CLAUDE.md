For building, linting, testing we use justfile.

### Frontend Development

The frontend uses pnpm 12.x and requires Node.js 24. Both majors are declared in `package.json` `devEngines`,
which mise reads through `frontend/mise.toml`; the default `nix develop` shell provides mise.
pnpm 11+ refuses another major (`onFail: "error"`); the `just` recipes check it for any pnpm.
mise refuses to run under an untrusted project config, hence the one-time `mise trust` per checkout:

```bash
cd frontend/

# Once per checkout: let mise read the version pins
mise trust

# Install dependencies
pnpm install

# Development server (check package.json for available scripts)
just dev

# Build for production
just build

# Run tests
just test

# Lint/format (uses Biome)
just lint
just format
```



## Frontend Structure (TypeScript/React)

Located in `frontend/`:

- React 19 with React Router for navigation
- gRPC-Web via ConnectRPC for backend communication
- Carbon Design System for UI components
- TypeScript with strict typing
- pnpm 12.x for package management; install scripts are blocked unless reviewed in `pnpm-workspace.yaml`

## Frontend Tests

The runner is `@rstest/core` (Vitest-compatible API), **not Vitest**. Run with `just fe::test` (type-check +
tests) or `just fe::validate` (format + lint + test). Config lives in `rstest.config.ts` and `rstest.setup.ts`.

- Specs are `*.spec.tsx` and use `@testing-library/react/pure` (manual `cleanup` in `afterEach`). The `rstest`
  namespace provides `useFakeTimers` / `advanceTimersByTimeAsync`.
- Mock gRPC by registering on the shared interceptor `mocks` from `@/proto/transport`:
  `mocks.service(pb.services.SomeService, { rpcName: ({ req }) => ({ ...response }) })`. It is active when
  `NODE_ENV !== 'development'` (true under rstest) and returns the mocker's plain object as the message.
  `ServiceMocks<S>` from `@/lib/proto` types a partial mock set.
- To render a page-level component, wrap it in `<HelmetProvider><IntlProvider locale="en"><MemoryRouter>`.
- jsdom gotchas, handled in the config/setup: jsdom has no `ResizeObserver` (needed by `useSize`) — a no-op stub
  is registered via `setupFiles`; and `@swc/plugin-formatjs` runs **without** `ast: true` so
  `formatMessage({ defaultMessage })` resolves to a string under a message-less `IntlProvider` (ast:true emits ICU
  AST objects that break rendering a raw `defaultMessage`).

## Protocol Buffer Workflow

Proto files are in `bmc-grpc/proto/web/`. `just gen` at the repository root regenerates `src/proto/gen/`
together with the backend's Rust from one `buf.gen.yaml`; commit the generated files with the proto change.
