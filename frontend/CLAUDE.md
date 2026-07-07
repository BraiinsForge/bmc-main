For building, linting, testing we use justfile.

### Frontend Development

The frontend uses Yarn 4.x and requires Node.js 24 (managed by Volta; the exact version is pinned in `package.json`):

```bash
cd frontend/

# Install dependencies
yarn install

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
- Yarn 4.x for package management

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

Proto files are in `bmc-grpc/proto/web/`. Changes to `.proto` files require:

**Frontend**: Regenerate TypeScript code using `@bufbuild/buf` and `@bufbuild/protoc-gen-es`

The frontend has build tooling configured for protobuf generation (check `frontend/justfile`) .
