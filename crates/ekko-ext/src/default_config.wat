(module
  (import "host" "ctx_set" (func $ctx_set (param i32 i32 i32 i32)))
  (import "host" "ctx_read" (func $ctx_read (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "config{}")

  (func (export "scratch") (result i32 i32)
    i32.const 256 i32.const 256)

  (func (export "mount")
    i32.const 0  i32.const 6  i32.const 6  i32.const 2  call $ctx_set
    i32.const 0  i32.const 6  call $ctx_read)

  (func (export "on_change") (param i32 i32))
)
