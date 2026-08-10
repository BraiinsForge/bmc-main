# Frontend

## gRPC Schema Compatibility

The frontend and backend are built and released together from the same `.proto` definitions. A release does not support
running its frontend against a backend from another release, so the web gRPC API does not need to remain compatible with
older frontend builds.

Treat a schema change as one atomic product change: update the `.proto` definitions and backend implementation,
regenerate the frontend bindings, and update the frontend callers together. Do not retain obsolete RPCs or fields, add
compatibility shims, or reserve removed protobuf field names and numbers solely for older frontend builds.

Revisit this policy if frontend and backend versions become independently deployable, external clients become a
supported API consumer, or protobuf messages become persisted data.
