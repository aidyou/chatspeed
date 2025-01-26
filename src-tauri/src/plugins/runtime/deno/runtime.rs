/*!
Runtime implementation for Deno-based plugins.

This module implements a runtime environment for executing Deno/JavaScript plugins in a way that
safely manages Deno's single-threaded execution model across different lifetimes.

# Design Overview

## Challenge: Single-threaded Runtime
Deno's JavaScript runtime (`JsRuntime`) is inherently single-threaded and must be operated
from the same thread where it was created. This presents challenges when integrating with
Rust's async ecosystem and when the runtime needs to outlive the scope where it was created.

## Solution: Channel-based Architecture
To address these limitations, this implementation uses a channel-based approach:

1. Runtime Isolation:
   - The Deno runtime runs in a dedicated thread created via `std::thread::spawn`
   - This ensures the runtime remains active and stable regardless of the parent thread's state

2. Communication Pattern:
   - Uses `std::sync::mpsc` channels to establish bi-directional communication between:
     * The main application thread (sender)
     * The Deno runtime thread (receiver)
   - Commands and responses are passed through these channels, allowing safe cross-thread interaction

3. Lifetime Management:
   - The channel architecture decouples the runtime's lifetime from the calling context
   - The runtime thread continues executing as long as the channel remains open
   - Clean shutdown is handled through the Drop trait, ensuring proper resource cleanup

This design enables safe concurrent access to the Deno runtime while maintaining proper
lifetime management and thread safety.
*/

use super::ops::{self};
use super::{PluginContext, PluginPermissions};
use crate::constants::PLUGINS_DIR;
use crate::plugins::runtime::RuntimeError;
use crate::plugins::traits::{PluginFactory, PluginInfo, PluginType};
use crate::plugins::Plugin;
use crate::SHARED_DATA_DIR;
use crate::http::client::HttpClient;

use async_trait::async_trait;
use deno_core::{FsModuleLoader, JsRuntime, ModuleId, ModuleSpecifier, RuntimeOptions};
use rust_i18n::t;
use serde_json::Value;
use url::Url;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::{error::Error, path::Path};
use tokio::task::LocalSet;

const INIT_RUNTIME_JS: &str = r#"
// String conversion functions
function strToUint8Array(str) {
    const bytes = new Uint8Array(str.length);
    for (let i = 0; i < str.length; i++) {
        bytes[i] = str.charCodeAt(i);
    }
    return bytes;
}

function uint8ArrayToStr(arr) {
    // 打印调试信息
    console.debug('Converting array:', Array.from(arr));

    // 确保是 Uint8Array
    const uint8Arr = new Uint8Array(arr);

    // 使用 reduce 构建字符串
    const str = Array.from(uint8Arr).reduce((acc, byte) => acc + String.fromCharCode(byte), '');

    console.debug('Converted string:', str);
    return str;
}

Object.defineProperties(globalThis, {
    'writeFile': {
        value: async (path, data, isShared = false) => {
            try {
                console.debug('Writing data:', data);
                const bytes = strToUint8Array(data);
                console.debug('Converted to bytes:', Array.from(bytes));
                await Deno.core.ops.write_file(path, bytes, isShared);
            } catch (e) {
                throw new Error(`Failed to write file: ${e}`);
            }
        },
        writable: false,
        enumerable: true,
        configurable: false
    },
    'readFile': {
        value: async (path, isShared = false) => {
            try {
                const content = await Deno.core.ops.read_file(path, isShared);
                console.debug('Read content:', content);
                return content;
            } catch (e) {
                throw new Error(`Failed to read file: ${e}`);
            }
        },
        writable: false,
        enumerable: true,
        configurable: false
    },
    'fetch': {
        value: async (url, options = {}) => {
            try {
                const response = await Deno.core.ops.fetch(url, options);
                return response;
            } catch (e) {
                throw new Error(`Failed to fetch: ${e}`);
            }
        },
        writable: false,
        enumerable: true,
        configurable: false
    },
    'console': {
        value: Object.defineProperties({}, {
            'log': {
                value: (...args) => { Deno.core.print('[JS][LOG] ' + args.map(String).join(' ') + '\n'); },
                writable: false,
                enumerable: true,
                configurable: false
            },
            'debug': {
                value: (...args) => { Deno.core.print('[JS][DEBUG] ' + args.map(String).join(' ') + '\n'); },
                writable: false,
                enumerable: true,
                configurable: false
            },
            'info': {
                value: (...args) => { Deno.core.print('[JS][INFO] ' + args.map(String).join(' ') + '\n'); },
                writable: false,
                enumerable: true,
                configurable: false
            },
            'warn': {
                value: (...args) => { Deno.core.print('[JS][WARN] ' + args.map(String).join(' ') + '\n'); },
                writable: false,
                enumerable: true,
                configurable: false
            },
            'error': {
                value: (...args) => { Deno.core.print('[JS][ERROR] ' + args.map(String).join(' ') + '\n'); },
                writable: false,
                enumerable: true,
                configurable: false
            }
        }),
        writable: false,
        enumerable: true,
        configurable: false
    },
    'logger': {
        value: globalThis.console,
        writable: false,
        enumerable: true,
        configurable: false
    },
    'sleep': {
        value: async (ms) => {
            try {
                await Deno.core.ops.sleep(ms);
            } catch (e) {
                throw new Error(`Failed to sleep: ${e}`);
            }
        },
        writable: false,
        enumerable: true,
        configurable: false
    }
});
"#;

#[derive(Debug)]
enum RuntimeCommand {
    GetModuleId {
        spec: Url,
        response: Sender<Result<ModuleId, RuntimeError>>,
    },
    ExecutePlugin {
        plugin_id: String,
        mod_id: ModuleId,
        input: Option<Value>,
        response: Sender<Result<Value, RuntimeError>>,
    },
    Shutdown,
    ShutdownAck,
}

pub struct DenoRuntime {
    base_path: PathBuf,
    share_dir: PathBuf,
    runtime_tx: Sender<RuntimeCommand>,
    ack_rx: Receiver<RuntimeCommand>,
    initialized: Arc<std::sync::atomic::AtomicBool>,
    plugin_info: PluginInfo,
}

impl DenoRuntime {
    /// Initialize the runtime thread and return the sender channel
    fn init_runtime_thread(plugin_dir: String, share_dir: String) -> Result<(Sender<RuntimeCommand>, Receiver<RuntimeCommand>), RuntimeError> {
        let (tx, rx) = channel::<RuntimeCommand>();
        let (ack_tx, ack_rx) = channel::<RuntimeCommand>();

        thread::spawn(move || {
            let local = LocalSet::new();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            let mut js_runtime = JsRuntime::new(RuntimeOptions {
                module_loader: Some(Rc::new(FsModuleLoader)),
                extensions: vec![ops::get_plugin_extension()],
                ..Default::default()
            });

            // inject tokio runtime handle
            js_runtime.op_state().borrow_mut().put(runtime.handle().clone());
            
            // inject HTTP client
            if let Ok(http_client) = HttpClient::new() {
                log::debug!("HTTP client initialized");
                js_runtime.op_state().borrow_mut().put(http_client);
            }
                
            js_runtime.op_state().borrow_mut().put(PluginContext {
                plugin_dir:plugin_dir.clone(),
                share_dir:share_dir.clone(),
                permissions: PluginPermissions::default(),
            });

            // Inject custom JavaScript code
            let init_result: Result<_, RuntimeError> = (|| {
                let mut scope = js_runtime.handle_scope();
                let code = v8::String::new(&mut scope, INIT_RUNTIME_JS)
                    .ok_or_else(|| RuntimeError::InitError(t!("plugin.runtime.failed_to_create_js_code").to_string()))?;
                
                let script = v8::Script::compile(&mut scope, code, None)
                    .ok_or_else(|| RuntimeError::InitError(t!("plugin.runtime.failed_to_compile_js_code").to_string()))?;
                
                script.run(&mut scope)
                    .ok_or_else(|| RuntimeError::InitError(t!("plugin.runtime.failed_to_run_js_code").to_string()))?;
                log::debug!("Runtime initialized: customer js injected");
                Ok(())
            })();

            // If initialization fails, log the error and continue
            if let Err(e) = init_result {
                log::error!("Failed to initialize runtime: {}", e);
            }

            local.block_on(&runtime, async {
                let result: Result<(), RuntimeError> = async {
                    let mut shutdown_requested = false;
                    while !shutdown_requested {
                        match rx.recv() {
                            Ok(cmd) => {
                                match cmd {
                                    RuntimeCommand::Shutdown => {
                                        let _ = ack_tx.send(RuntimeCommand::ShutdownAck);
                                        shutdown_requested = true;
                                    }
                                    RuntimeCommand::ShutdownAck => {
                                        // Ignore the confirmation message, it's only used for cleanup
                                    }
                                    RuntimeCommand::GetModuleId { spec, response } => {
                                        let result = js_runtime
                                            .load_main_es_module(&spec)
                                            .await
                                            .map_err(|e| RuntimeError::ExecutionError(e.to_string()));

                                        // evaluate module
                                        match result {
                                            Ok(mod_id) => {
                                                match js_runtime.mod_evaluate(mod_id).await {
                                                    Ok(_) => {
                                                        let _ = response.send(Ok(mod_id));
                                                    },
                                                    Err(e) => {
                                                        let _ = response.send(Err(RuntimeError::ExecutionError(e.to_string())));
                                                    }
                                                }
                                            },
                                            Err(e) => {
                                                let _ = response.send(Err(e));
                                            }
                                        }
                                    }
                                    RuntimeCommand::ExecutePlugin { plugin_id, mod_id,input, response } => {
                                        let permission = Self::load_permissions(plugin_dir.clone(), &plugin_id)?;
                                        // 1. Setup the plugin context
                                        js_runtime.op_state().borrow_mut().put(PluginContext {
                                            plugin_dir: plugin_dir.clone(),
                                            share_dir: share_dir.clone(),
                                            permissions: permission,
                                        });

                                        Self::inject_input(&mut js_runtime, input.clone())?;

                                        // 2. Get the module namespace
                                        let namespace = js_runtime
                                            .get_module_namespace(mod_id)
                                            .map_err(|e| RuntimeError::ModuleError(t!("plugin.runtime.get_module_namespace_error", error = e.to_string()).to_string()))?;

                                        log::debug!("Found module namespace: {:?}", &namespace);
                                        let mut scope = js_runtime.handle_scope();
                                        let local = v8::Local::new(&mut scope, namespace);
                                        
                                        // Convert local to object
                                        let obj = local.to_object(&mut scope)
                                            .ok_or_else(|| RuntimeError::ModuleError(t!("plugin.runtime.failed_to_export_to_object").to_string()))?;

                                        // 3. Get main function
                                        let main_key = v8::String::new(&mut scope, "main")
                                            .ok_or_else(|| RuntimeError::ModuleError(t!("plugin.runtime.failed_to_get_main_key").to_string()))?;

                                        let main = obj.get(&mut scope, main_key.into())
                                            .ok_or_else(|| RuntimeError::ModuleError(t!("plugin.runtime.main_not_found").to_string()))?;

                                        if !main.is_function() {
                                            return Err(RuntimeError::ModuleError(t!("plugin.runtime.main_not_function").to_string()));
                                        }

                                        // 4. Convert to function type and get source code
                                        let main_fn = v8::Local::<v8::Function>::try_from(main)
                                            .map_err(|e| RuntimeError::ModuleError(t!("plugin.runtime.failed_to_convert_to_function", error = e.to_string()).to_string()))?;
                                        
                                        let undefined = v8::undefined(&mut scope);
                                        let result = main_fn.call(&mut scope, undefined.into(), &[])
                                            .ok_or_else(|| RuntimeError::ExecutionError(t!("plugin.runtime.function_call_failed").to_string()))?;

                                        log::debug!("Function result: {:?}", result);
                                        log::debug!("Is promise: {}", result.is_promise());

                                        let result = {
                                            // Check if the result is a Promise
                                            if let Ok(promise) = v8::Local::<v8::Promise>::try_from(result) {
                                                // Create a global reference to keep the promise alive
                                                let promise = v8::Global::new(&mut scope, promise);
                                                
                                                // Release the scope
                                                drop(scope);
                                                
                                                log::debug!("Waiting for Promise to complete...");
                                                
                                                // Run the event loop until the Promise is resolved
                                                loop {
                                                    js_runtime.run_event_loop(Default::default()).await
                                                        .map_err(|e| {
                                                            log::error!("Event loop error: {}", e);
                                                            RuntimeError::ExecutionError(
                                                                t!("plugin.runtime.event_loop_error", error = e.to_string()).to_string(),
                                                            )
                                                        })?;
                                                    
                                                    // Create a new scope to check the result
                                                    let mut scope = js_runtime.handle_scope();
                                                    let local_promise = v8::Local::new(&mut scope, promise.clone());
                                                    
                                                    log::debug!("Promise state: {:?}", local_promise.state());
                                                    match local_promise.state() {
                                                        v8::PromiseState::Fulfilled => {
                                                            let result = local_promise.result(&mut scope);
                                                            let json_str = v8::json::stringify(&mut scope, result)
                                                                .ok_or_else(|| {
                                                                    log::error!("Failed to stringify result, json_str: {:?}", result);
                                                                    RuntimeError::JsonError( t!("plugin.runtime.failed_to_stringify_result").to_string())
                                                            })?;
                                                            let json = json_str.to_rust_string_lossy(&mut scope);
                                                            log::debug!("Promise result: {}", json);
                                                            break serde_json::from_str(&json)
                                                                .map_err(|e| RuntimeError::JsonError(
                                                                    t!("plugin.runtime.invalid_json", error = e.to_string(), json=json.clone()).to_string(),
                                                                ));
                                                        }
                                                        v8::PromiseState::Rejected => {
                                                            let error = local_promise.result(&mut scope);
                                                            let error_str = v8::json::stringify(&mut scope, error)
                                                                .map(|s| s.to_rust_string_lossy(&mut scope))
                                                                .unwrap_or_else(|| t!("plugin.runtime.unknown_error").to_string());
                                                            break Err(RuntimeError::ExecutionError(format!(
                                                                "{}: {}",
                                                                t!("plugin.runtime.promise_rejected"),
                                                                error_str
                                                            )));
                                                        }
                                                        v8::PromiseState::Pending => {
                                                            // Promise is still pending, continue running the event loop
                                                            continue;
                                                        }
                                                    }
                                                }
                                            } else {
                                                let json = v8::json::stringify(&mut scope, result)
                                                    .ok_or_else(|| RuntimeError::ExecutionError(
                                                        t!("plugin.runtime.failed_to_stringify_result").to_string(),
                                                    ))?;
                                                
                                                let result_str = json.to_rust_string_lossy(&mut scope);
                                                log::debug!("Function result: {}", result_str);
                                                
                                                serde_json::from_str(&result_str)
                                                    .map_err(|e| RuntimeError::JsonError(
                                                        t!("plugin.runtime.invalid_json", error = e.to_string(), json=result_str.clone()).to_string(),
                                                    ))?
                                            }
                                        };
                                        let _ = response.send(result);
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Runtime channel error: {}", e);
                                let _ = ack_tx.send(RuntimeCommand::ShutdownAck);
                                break;
                            }
                        }
                    }
                    Ok(())
                }.await;

                if let Err(e) = result {
                    log::error!("Runtime error: {}", e);
                }
            });
        });

        Ok((tx, ack_rx))
    }


    fn inject_input(js_runtime: &mut JsRuntime, input: Option<Value>) -> Result<(), RuntimeError> {
        if let Some(input) = input {
            let input_str = serde_json::to_string(&input).map_err(|e| RuntimeError::JsonError(e.to_string()))?;
            let mut scope = js_runtime.handle_scope();
            let code = format!("globalThis.input = Object.freeze({});", input_str);
            let code = v8::String::new(&mut scope, &code)
                .ok_or_else(|| RuntimeError::InitError(t!("plugin.runtime.failed_to_create_js_code").to_string()))?;
            
            let script = v8::Script::compile(&mut scope, code, None)
                .ok_or_else(|| RuntimeError::InitError(t!("plugin.runtime.failed_to_compile_js_code").to_string()))?;
            
            script.run(&mut scope)
                .ok_or_else(|| RuntimeError::InitError(t!("plugin.runtime.failed_to_run_js_code").to_string()))?;
            log::debug!("Input injected to globalThis.input: {}", input_str);
        }
        Ok(())
    }

    pub fn new() -> Result<Self, RuntimeError> {
        let base_path = PLUGINS_DIR.read().clone();
        let share_dir = SHARED_DATA_DIR.read().clone();

        let (runtime_tx, ack_rx) = Self::init_runtime_thread(base_path.clone(), share_dir.clone())?;

        Ok(Self {
            base_path: PathBuf::from(base_path),
            share_dir: PathBuf::from(share_dir),
            runtime_tx,
            ack_rx,
            initialized: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            plugin_info: PluginInfo {
                id: "deno_runtime".to_string(),
                name: "deno runtime".to_string(),
                version: "0.327.0".to_string(),
            },
        })
    }

    pub fn new_with_base_path<P: AsRef<Path>>(base_path: P, share_dir: P) -> Result<Self, RuntimeError> {
        let base_path = base_path.as_ref().to_path_buf();
        let share_dir = share_dir.as_ref().to_path_buf();
        let (runtime_tx, ack_rx) = Self::init_runtime_thread(base_path.to_string_lossy().to_string(), share_dir.to_string_lossy().to_string())?;

        Ok(Self {
            base_path,
            share_dir,
            runtime_tx,
            ack_rx,
            initialized: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            plugin_info: PluginInfo {
                id: "deno_runtime".to_string(),
                name: "deno runtime".to_string(),
                version: "0.327.0".to_string(),
            },
        })
    }

    async fn get_module_id(&self, file_path: &str) -> Result<ModuleId, RuntimeError> {
        let file_path = file_path.to_string();
        let module_spec = ModuleSpecifier::from_file_path(&file_path).map_err(|_| {
            RuntimeError::ModuleError(t!("plugin.runtime.invalid_base_path").to_string())
        })?;

        let (tx, rx) = channel::<Result<ModuleId, RuntimeError>>();
        self.runtime_tx
            .send(RuntimeCommand::GetModuleId {
                spec: module_spec,
                response: tx,
            })
            .map_err(|e| {
                RuntimeError::ChannelSendError(
                    t!("plugin.runtime.channel_send_error", error = e.to_string()).to_string(),
                )
            })?;

        rx.recv().map_err(|e| {
            RuntimeError::ChannelReceiveError(
                t!(
                    "plugin.runtime.channel_receive_error",
                    error = e.to_string()
                )
                .to_string(),
            )
        })?
    }

    pub async fn execute_module(&self, plugin_id: &str, input: Option<Value>) -> Result<Value, RuntimeError> {
        // 1. Find entry file
        let entry_path = self.find_plugin_entry(plugin_id)?;
        log::debug!("Found plugin entry: {}", entry_path.display());

        // 2. Get module ID
        let mod_id = self.get_module_id(&entry_path.to_string_lossy()).await?;
        log::debug!("Module loaded with ID: {}", mod_id);

        // 4. Execute main function
        let (tx, rx) = channel::<Result<Value, RuntimeError>>();
        self.runtime_tx
            .send(RuntimeCommand::ExecutePlugin {
                plugin_id: plugin_id.to_string(),
                mod_id,
                input,
                response: tx,
            })
            .map_err(|e| {
                RuntimeError::ChannelSendError(
                    t!("plugin.runtime.channel_send_error", error = e.to_string()).to_string(),
                )
            })?;

        rx.recv().map_err(|e| {
            RuntimeError::ChannelReceiveError(
                t!(
                    "plugin.runtime.channel_receive_error",
                    error = e.to_string()
                )
                .to_string(),
            )
        })?
    }

    /// Find the entry file (main.js or main.ts) in the plugin directory
    fn find_plugin_entry(&self, plugin_id: &str) -> Result<PathBuf, RuntimeError> {
        let plugin_dir = Path::new(&self.base_path).join(plugin_id);
        let js_path = plugin_dir.join("main.js");
        let ts_path = plugin_dir.join("main.ts");

        if js_path.exists() {
            Ok(js_path)
        } else if ts_path.exists() {
            Ok(ts_path)
        } else {
            Err(RuntimeError::ModuleError(
                t!(
                    "plugin.runtime.main_file_not_found",
                    "path" = format!(
                        "{}/main.js, {}/main.ts",
                        plugin_dir.display(),
                        plugin_dir.display()
                    )
                )
                .to_string(),
            ))
        }
    }

    /// Load plugin permissions from plugin manifest
    ///
    /// # Arguments
    /// * `plugin_id` - The ID of the plugin to load permissions for, it's an dirname in plugin base path
    fn load_permissions(base_path: String, plugin_id: &str) -> Result<PluginPermissions, RuntimeError> {
        let plugin_dir = Path::new(&base_path).join(plugin_id);
        log::debug!("Loading permissions from: {}", plugin_dir.display());
        let permissions = PluginPermissions::from_manifest(&plugin_dir.to_string_lossy())
            .map_err(|e| RuntimeError::PermissionError(e.to_string()))?;
        log::debug!("Loaded permissions: {:?}", permissions);
        Ok(permissions)
    }

    /// Initialize plugin info
    /// 
    /// # Arguments
    /// * `plugin_info` - The plugin info
    ///     * `id` - The ID of the plugin
    ///     * `name` - The name of the plugin
    ///     * `version` - The version of the plugin
    /// 
    /// # Returns
    /// * `Result<(), RuntimeError>`
    fn init_plugin_info(&mut self, plugin_info: Option<PluginInfo>) -> Result<(), RuntimeError> {
        if let Some(plugin) = plugin_info {
            if plugin.id.is_empty() {
                return Err(RuntimeError::ExecutionError(
                    t!("plugin.runtime.plugin_id_not_specified").to_string(),
                ));
            }
            if plugin.name.is_empty() {
                return Err(RuntimeError::ExecutionError(
                    t!("plugin.runtime.plugin_name_is_null").to_string(),
                ));
            }
            if plugin.version.is_empty() {
                return Err(RuntimeError::ExecutionError(
                    t!("plugin.runtime.plugin_version_is_null").to_string(),
                ));
            }
            self.plugin_info = plugin;
        } else {
            return Err(RuntimeError::ExecutionError(
                t!("plugin.runtime.plugin_context_is_null").to_string(),
            ));
        }
        Ok(())
    }
}

impl Drop for DenoRuntime {
    fn drop(&mut self) {
        if self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = self.runtime_tx.send(RuntimeCommand::Shutdown);
            let _ = self
                .ack_rx
                .recv_timeout(std::time::Duration::from_millis(100));
        }
    }
}

#[async_trait]
impl Plugin for DenoRuntime {
    async fn init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    fn plugin_info(&self) -> &PluginInfo {
        &self.plugin_info
    }

    fn plugin_type(&self) -> &PluginType {
        &PluginType::JavaScript
    }

    async fn execute(
        &mut self,
        input: Option<Value>,
        plugin_info: Option<PluginInfo>,
    ) -> Result<Value, Box<dyn Error + Send + Sync>> {
        self.init_plugin_info(plugin_info)?;

        Ok(self.execute_module(self.plugin_info.id.as_str(), input).await?)
    }

    async fn destroy(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        if !self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }

        self.initialized
            .store(false, std::sync::atomic::Ordering::SeqCst);

        match self.runtime_tx.send(RuntimeCommand::Shutdown) {
            Ok(_) => match self.ack_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(RuntimeCommand::ShutdownAck) => Ok(()),
                Ok(_) => Err(Box::new(RuntimeError::ExecutionError(
                    t!("plugin.runtime.unknown_error").to_string(),
                ))),
                Err(e) => Err(Box::new(RuntimeError::ExecutionError(
                    t!("plugin.runtime.execution_error", error = e.to_string()).to_string(),
                ))),
            },
            Err(e) => Err(Box::new(RuntimeError::ExecutionError(
                t!("plugin.runtime.execution_error", error = e.to_string()).to_string(),
            ))),
        }
    }
}


pub struct DenoRuntimeFactory;

impl DenoRuntimeFactory {
    pub fn new() -> Self {
        Self
    }
}

impl PluginFactory for DenoRuntimeFactory {
    fn create_instance(
        &self,
        _init_options: Option<&Value>,
    ) -> Result<Box<dyn Plugin>, Box<dyn Error + Send + Sync>> {
        Ok(Box::new(DenoRuntime::new()?))
    }
}

unsafe impl Send for DenoRuntime {}
unsafe impl Sync for DenoRuntime {}