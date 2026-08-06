(module
  (import "env" "host_request_frame" (func $host_request_frame))

  (memory (export "memory") 1)

  (global $update_count (mut i32) (i32.const 0))

  (func (export "__bmc_sdk_init") (result i64)
    i64.const __SDK_VERSION__)

  (func (export "render") (param i32))

  (func (export "__UPDATE_HOOK__")
    global.get $update_count
    i32.const 1
    i32.add
    global.set $update_count
    __REQUEST_FRAME__)

  (func (export "update_count") (result i32) global.get $update_count))
