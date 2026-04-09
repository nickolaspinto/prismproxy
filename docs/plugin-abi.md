# WASM Plugin ABI

prismproxy loads WebAssembly plugins per route. Plugins are called on every incoming request before it is forwarded to the upstream.

## Interface

Your plugin module must export exactly one function:

```
on_request(method_ptr: i32, method_len: i32, path_ptr: i32, path_len: i32) -> i32
```

### Parameters

| Parameter | Type | Description |
|---|---|---|
| `method_ptr` | `i32` | Pointer into WASM linear memory where the HTTP method string starts (UTF-8) |
| `method_len` | `i32` | Length of the method string in bytes |
| `path_ptr` | `i32` | Pointer into WASM linear memory where the request path starts (UTF-8) |
| `path_len` | `i32` | Length of the path string in bytes |

### Return value

- `0` — allow the request (pass through to upstream)
- any non-zero — block the request (proxy returns `403 Forbidden`)

## Memory model

The proxy writes method and path into the plugin's linear memory before calling `on_request`. You do not need to allocate or export a memory — the host manages this.

## Example (WAT)

```wat
(module
  (func (export "on_request")
    (param $method_ptr i32) (param $method_len i32)
    (param $path_ptr i32)   (param $path_len i32)
    (result i32)
    ;; allow all requests
    i32.const 0
  )
)
```

## Example (Rust)

```rust
#[no_mangle]
pub extern "C" fn on_request(
    _method_ptr: i32, _method_len: i32,
    path_ptr: i32, path_len: i32,
) -> i32 {
    let path = unsafe {
        let slice = std::slice::from_raw_parts(path_ptr as *const u8, path_len as usize);
        std::str::from_utf8_unchecked(slice)
    };
    if path.starts_with("/admin") { 1 } else { 0 }
}
```

Compile with: `cargo build --target wasm32-unknown-unknown --release`

## Plugin chain

Routes can have multiple plugins. They are called in order. If any plugin returns non-zero, the request is blocked immediately — subsequent plugins in the chain are not called.

```toml
[[routes]]
path_prefix = "/api"
upstream = "127.0.0.1:3000"
plugins = ["./plugins/auth.wasm", "./plugins/rate-limit.wasm"]
```
