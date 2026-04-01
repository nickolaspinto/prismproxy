use std::path::Path;
use tracing::info;
use wasmtime::{Engine, Linker, Module, Store};

use crate::error::ProxyError;

/// A compiled WASM plugin ready to be instantiated per-request.
pub struct Plugin {
    name: String,
    module: Module,
}

impl Plugin {
    /// Compile a WASM plugin from a `.wasm` or `.wat` file on disk.
    pub fn from_file(engine: &Engine, path: impl AsRef<Path>) -> Result<Self, ProxyError> {
        let path = path.as_ref();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let module = Module::from_file(engine, path)
            .map_err(|e| ProxyError::Plugin(format!("compile '{}': {e}", path.display())))?;

        info!(plugin = %name, "compiled plugin");
        Ok(Self { name, module })
    }

    pub fn from_module(name: impl Into<String>, module: Module) -> Self {
        Self { name: name.into(), module }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn module(&self) -> &Module {
        &self.module
    }
}

/// The plugin runtime engine — shared across all plugins and requests.
pub struct PluginRuntime {
    engine: Engine,
    linker: Linker<()>,
    plugins: Vec<Plugin>,
}

impl PluginRuntime {
    pub fn new() -> Result<Self, ProxyError> {
        let engine = Engine::default();
        let linker = Linker::new(&engine);
        info!("WASM plugin runtime initialized");
        Ok(Self { engine, linker, plugins: Vec::new() })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Load a plugin from a file path (supports .wasm and .wat).
    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<(), ProxyError> {
        let plugin = Plugin::from_file(&self.engine, path)?;
        info!(plugin = %plugin.name(), "loaded plugin");
        self.plugins.push(plugin);
        Ok(())
    }

    /// Inject a pre-compiled module (used in tests).
    pub fn push_module(&mut self, name: impl Into<String>, module: Module) {
        self.plugins.push(Plugin::from_module(name, module));
    }

    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
    }

    /// Run all plugins against an incoming request.
    ///
    /// Returns `true` if any plugin blocked the request, `false` if all passed.
    /// Plugins are run in order; the first block wins.
    pub fn run_on_request(&self, method: &str, path: &str) -> Result<bool, ProxyError> {
        let method_bytes = method.as_bytes();
        let path_bytes = path.as_bytes();

        for plugin in &self.plugins {
            let mut store = Store::new(&self.engine, ());
            let instance = self
                .linker
                .instantiate(&mut store, plugin.module())
                .map_err(|e| {
                    ProxyError::Plugin(format!("instantiate '{}': {e}", plugin.name()))
                })?;

            let memory = instance
                .get_memory(&mut store, "memory")
                .ok_or_else(|| {
                    ProxyError::Plugin(format!("'{}' missing 'memory' export", plugin.name()))
                })?;

            // Host layout: method at offset 0, path at offset 256.
            memory.write(&mut store, 0, method_bytes).map_err(|e| {
                ProxyError::Plugin(format!("write method for '{}': {e}", plugin.name()))
            })?;
            memory.write(&mut store, 256, path_bytes).map_err(|e| {
                ProxyError::Plugin(format!("write path for '{}': {e}", plugin.name()))
            })?;

            let on_request = instance
                .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "on_request")
                .map_err(|e| {
                    ProxyError::Plugin(format!(
                        "'{}' missing 'on_request' export: {e}",
                        plugin.name()
                    ))
                })?;

            let result = on_request
                .call(
                    &mut store,
                    (
                        0,
                        method_bytes.len() as i32,
                        256,
                        path_bytes.len() as i32,
                    ),
                )
                .map_err(|e| {
                    ProxyError::Plugin(format!("'{}' on_request trap: {e}", plugin.name()))
                })?;

            if result != 0 {
                info!(plugin = %plugin.name(), %method, %path, "plugin blocked request");
                return Ok(true);
            }
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_initializes() {
        let runtime = PluginRuntime::new().unwrap();
        assert!(runtime.plugins().is_empty());
    }

    #[test]
    fn load_nonexistent_file_fails() {
        let mut runtime = PluginRuntime::new().unwrap();
        let result = runtime.load("nonexistent.wasm");
        assert!(result.is_err());
    }

    #[test]
    fn empty_runtime_always_passes() {
        let runtime = PluginRuntime::new().unwrap();
        let blocked = runtime.run_on_request("GET", "/anything").unwrap();
        assert!(!blocked);
    }

    #[test]
    fn block_all_plugin_blocks_request() {
        let block_all_wat = r#"
(module
  (memory (export "memory") 1)
  (func (export "on_request") (param i32 i32 i32 i32) (result i32)
    i32.const 1))
"#;
        let mut runtime = PluginRuntime::new().unwrap();
        let module = Module::new(runtime.engine(), block_all_wat.as_bytes()).unwrap();
        runtime.push_module("block-all", module);

        let blocked = runtime.run_on_request("GET", "/anything").unwrap();
        assert!(blocked);
    }

    #[test]
    fn pass_all_plugin_passes_request() {
        let pass_all_wat = r#"
(module
  (memory (export "memory") 1)
  (func (export "on_request") (param i32 i32 i32 i32) (result i32)
    i32.const 0))
"#;
        let mut runtime = PluginRuntime::new().unwrap();
        let module = Module::new(runtime.engine(), pass_all_wat.as_bytes()).unwrap();
        runtime.push_module("pass-all", module);

        let blocked = runtime.run_on_request("POST", "/data").unwrap();
        assert!(!blocked);
    }

    #[test]
    fn path_checking_plugin_blocks_correct_path() {
        let block_path_wat = r#"
(module
  (memory (export "memory") 1)
  (func (export "on_request")
    (param $mp i32) (param $ml i32) (param $pp i32) (param $pl i32)
    (result i32)
    (if (i32.lt_u (local.get $pl) (i32.const 6)) (then (return (i32.const 0))))
    (if (i32.ne (i32.load8_u (local.get $pp))                          (i32.const 47))  (then (return (i32.const 0))))
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 1))) (i32.const 98))  (then (return (i32.const 0))))
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 2))) (i32.const 108)) (then (return (i32.const 0))))
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 3))) (i32.const 111)) (then (return (i32.const 0))))
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 4))) (i32.const 99))  (then (return (i32.const 0))))
    (if (i32.ne (i32.load8_u (i32.add (local.get $pp) (i32.const 5))) (i32.const 107)) (then (return (i32.const 0))))
    i32.const 1)
)
"#;
        let mut runtime = PluginRuntime::new().unwrap();
        let module = Module::new(runtime.engine(), block_path_wat.as_bytes()).unwrap();
        runtime.push_module("block-path", module);

        assert!(runtime.run_on_request("GET", "/blocked").unwrap());
        assert!(runtime.run_on_request("GET", "/blockme").unwrap());
        assert!(!runtime.run_on_request("GET", "/other").unwrap());
        assert!(!runtime.run_on_request("GET", "/").unwrap());
    }
}
