# wasmi 0.45 → 1.0 Migration

## Motivation

wasmi 1.0 drops the `downcast-rs` dependency (wasmi_core used 2.x while wayland-backend pulls 1.x, causing a duplicate
crate ban in `cargo deny`). Also: MSRV bump to 1.86, `Store<T>` no longer requires `T: 'static`, fuel-resumable
execution, and the API is now stable.

## Breaking API changes (0.45 → 1.0)

These were the only breaking changes that affected our codebase.

### 1. `TrapCode` moved from `wasmi::core` to crate root

```
// Before
e.as_trap_code() == Some(wasmi::core::TrapCode::OutOfFuel)

// After
e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel)
```

### 2. `Linker::instantiate` + `start` merged into `instantiate_and_start`

```
// Before
let instance = linker.instantiate(&mut store, &module)?.start(&mut store)?;

// After
let instance = linker.instantiate_and_start(&mut store, &module)?;
```

### 3. `Instance::get_typed_func` now takes `&mut Store` instead of `&Store`

```
// Before
instance.get_typed_func::<(u32, u32), ()>(&store, "init")

// After
instance.get_typed_func::<(u32, u32), ()>(&mut store, "init")
```

In practice this did **not** cause any compilation errors in our codebase — all call sites already had a mutable store
binding.

### 4. Everything else is unchanged

These APIs are identical in 1.0:

- `Config::default()`, `config.consume_fuel(true)`
- `Engine::new(&config)`
- `Module::new(&engine, wasm_bytes)`
- `Store::new(&engine, host_state)`, `store.set_fuel()`, `store.data()`, `store.data_mut()`, `store.into_data()`
- `Linker::new(&engine)`, `linker.func_wrap()`
- `Caller`, `Caller::data()`, `Caller::data_mut()`, `Caller::get_export()`
- `Extern::into_memory()`, `memory.data()`, `memory.data_mut()`
- `TypedFunc`, `.call()`
- `Error`, `.as_trap_code()`

## Changes made

1. Bumped `wasmi = "0.45"` → `"1.0"` in workspace `Cargo.toml`
2. Fixed `TrapCode` path in `runtime.rs`
3. Fixed `instantiate` + `start` → `instantiate_and_start` in `runtime.rs`
4. Removed `downcast-rs@1` skip entry from `deny.toml`
5. Verified: `cargo check`, `cargo clippy`, `cargo test`, `cargo deny check bans` — all pass
