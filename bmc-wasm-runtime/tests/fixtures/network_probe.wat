(module
  (import "env" "host_request_frame" (func $host_request_frame))

  (memory (export "memory") 1)

  (global $network_count (mut i32) (i32.const 0))

  (func (export "__bmc_sdk_init") (result i64)
    i64.const __SDK_VERSION__)

  (func (export "render") (param i32))

  (func (export "on_network_update")
    global.get $network_count
    i32.const 1
    i32.add
    global.set $network_count
    call $host_request_frame)

  (func (export "network_count") (result i32) global.get $network_count))
