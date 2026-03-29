use std::path::Path;
use tracing::info;
use wasmtime::{Engine, Module};

use crate::error::ProxyError;

/// A compiled WASM plugin ready to be instantiated per-request.
pub struct Plugin {
    name: String,
    module: Module,
}

impl Plugin {
    /// Compile a WASM plugin from a `.wasm` file on disk.
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

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn module(&self) -> &Module {
        &self.module
    }
}

/// The plugin runtime engine — shared across all plugins.
pub struct PluginRuntime {
    engine: Engine,
    plugins: Vec<Plugin>,
}

impl PluginRuntime {
    pub fn new() -> Result<Self, ProxyError> {
        let engine = Engine::default();
        info!("WASM plugin runtime initialized");
        Ok(Self {
            engine,
            plugins: Vec::new(),
        })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<(), ProxyError> {
        let plugin = Plugin::from_file(&self.engine, path)?;
        info!(plugin = %plugin.name(), "loaded plugin");
        self.plugins.push(plugin);
        Ok(())
    }

    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
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
}
