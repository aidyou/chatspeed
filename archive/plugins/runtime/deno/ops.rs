use super::permissions::PluginPermissions;
use deno_core::{extension, op2, Extension, OpState};
use rust_i18n::t;
use std::fs;

#[derive(Clone)]
pub(crate) struct PluginContext {
    pub plugin_dir: String,
    pub share_dir: String,
    pub permissions: PluginPermissions,
}

pub mod ops {
    use std::future::Future;

    use super::*;
    use crate::{
        http::{client::HttpClient, types::HttpConfig},
        plugins::runtime::RuntimeError,
    };

    #[op2]
    #[string]
    pub fn read_file(
        state: &mut OpState,
        #[string] path: String,
        is_shared: bool,
    ) -> Result<String, RuntimeError> {
        let ctx = state.borrow::<PluginContext>();

        // 根据 is_shared 选择基准目录
        let base_dir = if is_shared {
            &ctx.share_dir
        } else {
            &ctx.plugin_dir
        };

        // 验证和解析路径（传入 is_shared 参数）
        let resolved_path = ctx
            .permissions
            .fs
            .resolve_path(base_dir, &path, is_shared)?;

        // Read file
        let data = fs::read(&resolved_path).map_err(|e| {
            RuntimeError::FileError(
                t!(
                    "plugin.runtime.file_read_error",
                    path = resolved_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        // Print debug information
        // log::debug!("Read {} bytes from {}", data.len(), resolved_path.display());
        // log::debug!("Raw data: {:?}", data);

        // Convert to string
        String::from_utf8(data).map_err(|e| {
            RuntimeError::FileError(
                t!(
                    "plugin.runtime.file_read_error",
                    path = resolved_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })
    }

    #[op2(fast)]
    pub fn write_file(
        state: &mut OpState,
        #[string] path: String,
        #[buffer] data: &[u8],
        is_shared: bool,
    ) -> Result<(), RuntimeError> {
        let ctx = state.borrow::<PluginContext>();
        let base_dir = if is_shared {
            &ctx.share_dir
        } else {
            &ctx.plugin_dir
        };

        // 打印调试信息
        // log::debug!("Writing {} bytes to {}", data.len(), path);
        // log::debug!("Raw data: {:?}", data);

        // 验证和解析路径
        let resolved_path = ctx
            .permissions
            .fs
            .resolve_path(base_dir, &path, is_shared)?;

        // 确保目录存在
        if let Some(dir) = resolved_path.parent() {
            fs::create_dir_all(dir).map_err(|e| {
                RuntimeError::FileError(
                    t!(
                        "plugin.runtime.failed_to_create_directory",
                        path = dir.display(),
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;
        }

        // Write file
        fs::write(&resolved_path, data).map_err(|e| {
            RuntimeError::FileError(
                t!(
                    "plugin.runtime.file_write_error",
                    path = resolved_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })
    }

    #[op2(async)]
    pub fn sleep(
        _state: &mut OpState,
        ms: i32,
    ) -> Result<impl Future<Output = Result<(), RuntimeError>>, RuntimeError> {
        Ok(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(ms as u64)).await;
            Ok(())
        })
    }

    #[op2(async)]
    #[string]
    pub fn fetch(
        state: &mut OpState,
        #[string] url: String,
        #[serde] options: serde_json::Value,
    ) -> Result<impl Future<Output = Result<String, RuntimeError>>, RuntimeError> {
        // 1. 检查权限
        {
            let ctx = state.borrow::<PluginContext>();
            if !ctx.permissions.network.check_url(&url) {
                return Err(RuntimeError::PermissionError(
                    t!("plugin.runtime.network_access_denied", url = url).to_string(),
                ));
            }
        }

        // 2. 获取必要的组件
        let client = state.borrow::<HttpClient>().clone();
        let url = url.clone();

        Ok(async move {
            // 创建请求配置
            let mut config = HttpConfig::get(&url);
            config.async_request = Some(true);

            // 添加 headers
            if let Some(headers) = options.get("headers").and_then(|v| v.as_object()) {
                for (key, value) in headers {
                    if let Some(value) = value.as_str() {
                        config = config.header(key.clone(), value.to_string());
                    }
                }
            }

            // 发送请求
            let response = client
                .send_request_async(config)
                .await
                .map_err(|e| RuntimeError::RuntimeSpecificError(e.to_string()))?;

            if response.status >= 400 {
                return Err(RuntimeError::RuntimeSpecificError(format!(
                    "HTTP request failed with status: {}",
                    response.status
                )));
            }

            Ok(response.body.unwrap_or_default())
        })
    }
}

extension! {
    ChatSpeedPlugin,
    ops = [ops::read_file, ops::write_file, ops::sleep, ops::fetch],
    state = |state: &mut OpState| {
        state.put(PluginContext {
            plugin_dir: String::new(),
            share_dir: String::new(),
            permissions: PluginPermissions::default(),
        });
    }
}

pub(crate) fn get_plugin_extension() -> Extension {
    ChatSpeedPlugin::ext()
}
