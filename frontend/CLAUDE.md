For building, linting, testing we use justfile.

### Frontend Development

The frontend uses Yarn 4.x and requires Node.js 24.6.0 (managed by Volta):

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

## Protocol Buffer Workflow

Proto files are in `bmc-grpc/proto/web/`. Changes to `.proto` files require:

**Frontend**: Regenerate TypeScript code using `@bufbuild/buf` and `@bufbuild/protoc-gen-es`

The frontend has build tooling configured for protobuf generation (check `frontend/justfile`) .
