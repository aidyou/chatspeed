use crate::{
    plugins::{
        runtime::{error::PyRuntimeResult, python::config::PythonConfig, RuntimeError},
        traits::{PluginFactory, PluginInfo, PluginType},
        Plugin,
    },
    PLUGINS_DIR,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyModule},
};
use rust_i18n::t;
use serde_json::Value;
use std::{error::Error, ffi::CString, path::Path};

/// Python runtime environment implementation
pub struct PythonRuntime {
    /// Base path for resolving imports
    base_path: String,
    /// Global namespace for Python execution
    globals: Option<Py<PyDict>>,
    /// Python configuration
    config: PythonConfig,
    /// Loaded plugins
    plugin_info: PluginInfo,
}

impl PythonRuntime {
    /// Creates a new Python runtime instance
    pub fn new() -> Result<Self, RuntimeError> {
        let base_path = PLUGINS_DIR.read().clone();

        Ok(Self {
            base_path,
            config: PythonConfig::from_env(),
            globals: None,
            plugin_info: PluginInfo {
                id: "plugin_runtime".to_string(),
                name: "plugin runtime".to_string(),
                version: "3.12".to_string(),
            },
        })
    }

    pub fn new_with_base_path<P: AsRef<Path>>(base_path: P) -> Result<Self, RuntimeError> {
        Ok(Self {
            base_path: base_path.as_ref().to_string_lossy().to_string(),
            config: PythonConfig::from_env(),
            globals: None,
            plugin_info: PluginInfo {
                id: "plugin_runtime".to_string(),
                name: "plugin runtime".to_string(),
                version: "3.12".to_string(),
            },
        })
    }

    /// Converts Rust JSON Value to Python object
    fn json_to_py<'py>(&self, value: &Value, py: Python<'py>) -> PyRuntimeResult<Py<PyAny>> {
        let json = PyModule::import(py, "json")?;
        let value_str = serde_json::to_string(value)?;
        // 直接将字符串转换为 Python 对象
        let py_str = value_str.into_pyobject(py).map_err(|e| {
            RuntimeError::ExecutionError(
                t!(
                    "plugin.runtime.failed_to_convert_to_python_object",
                    "error" = e.to_string()
                )
                .to_string(),
            )
        })?;
        // 调用 json.loads(value_str)
        Ok(json
            .getattr("loads")?
            .call1((py_str,))?
            .into_pyobject(py)
            .map_err(|e| RuntimeError::ExecutionError(e.to_string()))?
            .into())
    }

    /// Converts Python object to Rust JSON Value
    fn py_to_json(&self, obj: &Py<PyAny>) -> Result<Value, Box<dyn Error>> {
        Python::with_gil(|py| {
            let json = PyModule::import(py, "json")?;
            // 调用 json.dumps(obj)
            let bound_obj = obj.bind(py);
            let result = json.getattr("dumps")?.call1((bound_obj,))?;
            // 从 Python 字符串转换为 Rust 字符串
            let json_str: String = result.extract()?;
            Ok(serde_json::from_str(&json_str)?)
        })
    }

    /// Converts Rust &str to Python CString
    fn str_to_cstr(&self, s: &str) -> Result<CString, RuntimeError> {
        CString::new(s).map_err(|e| {
            RuntimeError::InitError(
                t!(
                    "plugin.runtime.failed_to_create_python_code",
                    "error" = e.to_string()
                )
                .to_string(),
            )
        })
    }

    /// Sets up the Python environment with necessary imports and restrictions
    fn setup_environment(&mut self, py: Python<'_>) -> PyRuntimeResult<Py<PyDict>> {
        // 验证 Python 配置
        self.config.validate()?;

        // Import required modules
        let os = PyModule::import(py, "os")?;
        let sys = PyModule::import(py, "sys")?;
        let json = PyModule::import(py, "json")?;

        let globals = PyDict::new(py);

        // Add modules to globals
        globals.set_item("os", os.clone())?;
        globals.set_item("sys", sys)?;
        globals.set_item("json", json)?;

        // Import essential modules
        let builtins = PyModule::import(py, "builtins")?;
        globals.set_item("__builtins__", builtins)?;

        // 设置环境变量到 os.environ 中
        let environ = os.getattr("environ")?;
        for (key, value) in std::env::vars() {
            environ.set_item(key, value)?;
        }

        // Add utility functions
        let utils_code = self.str_to_cstr(
            r#"def to_json(obj):
    return json.dumps(obj)

def from_json(json_str):
    return json.loads(json_str)
"#,
        )?;
        let empty_name = self.str_to_cstr("")?;

        let utils = PyModule::from_code(
            py,
            utils_code.as_ref(),
            empty_name.as_ref(),
            empty_name.as_ref(),
        )?;

        globals.update(&utils.dict().into_mapping())?;

        self.globals = Some(globals.clone().into()); // Store globals
        Ok(globals.into())
    }

    async fn internal_init(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Python::with_gil(|py| -> PyResult<()> {
            // setup Python environment
            self.setup_environment(py)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

            // Ensure environment variables are available in the Python environment
            let os = PyModule::import(py, "os")?;
            let environ = os.getattr("environ")?;

            // Set environment variables directly
            for (key, value) in std::env::vars() {
                environ.call_method1("__setitem__", (key, value))?;
            }

            Ok(())
        })?;
        Ok(())
    }

    fn setup_globals<'py>(
        &self,
        py: Python<'py>,
        globals: &Bound<'py, PyDict>,
        input: Option<&Value>,
        context: Option<&Value>,
    ) -> Result<(), Box<dyn Error>> {
        // Resynchronize environment variables
        let os = PyModule::import(py, "os")?;
        let environ = os.getattr("environ")?;
        for (key, value) in std::env::vars() {
            environ.set_item(key, value)?;
        }

        // Add input and schema if provided
        if let Some(input_value) = input {
            globals.set_item("input", self.json_to_py(&input_value, py)?)?;
        }

        if let Some(context_value) = context {
            globals.set_item("context", self.json_to_py(&context_value, py)?)?;
        }

        Ok(())
    }

    pub(crate) async fn internal_execute(
        &self,
        code: &str,
        input: Option<Value>,
        context: Option<Value>,
    ) -> Result<Value, Box<dyn Error>> {
        Python::with_gil(|py| {
            let globals = self
                .globals
                .as_ref()
                .ok_or(t!("plugin.runtime.python_environment_not_initialized"))?
                .bind(py);

            self.setup_globals(py, &globals, input.as_ref(), context.as_ref())?;

            let py_code = self.str_to_cstr(code)?;
            // Execute the code and convert to PyObject
            let result = if code.contains('\n') {
                py.run(py_code.as_ref(), Some(&globals), None)?;
                globals
                    .get_item("result")
                    .map_err(|e| t!("plugin.runtime.no_result_found", "error" = e.to_string()))?
                    .ok_or(t!("plugin.runtime.no_result_found"))?
            } else {
                py.eval(py_code.as_ref(), Some(&globals), None)?
            };

            // Convert result back to JSON
            self.py_to_json(&result.into())
        })
    }

    async fn internal_cleanup(&mut self) -> Result<(), Box<dyn Error + Sync + Send>> {
        self.globals = None;
        Ok(())
    }

    /// execute plugin
    async fn execute_plugin(
        &self,
        plugin_id: &str,
        input: Option<Value>,
        context: Option<Value>,
    ) -> Result<Value, Box<dyn Error + Send + Sync>> {
        Ok(Python::with_gil(|py| -> PyRuntimeResult<Value> {
            let globals = self
                .globals
                .as_ref()
                .ok_or(RuntimeError::InitError(
                    t!("plugin.runtime.python_environment_not_initialized").to_string(),
                ))?
                .bind(py);

            self.setup_globals(py, &globals, input.as_ref(), context.as_ref())?;

            // Get sys.path
            let sys = PyModule::import(py, "sys")?;
            let sys_path = sys.getattr("path")?;

            let plugin_dir = Path::new(&self.base_path)
                .join(plugin_id)
                .display()
                .to_string();
            // Add plugin directory to Python path
            sys_path.call_method1("append", (&plugin_dir,))?;

            let main_path = format!("{}/main.py", &plugin_dir);
            if !Path::new(&main_path).exists() {
                return Err(RuntimeError::ModuleError(
                    t!("plugin.runtime.main_file_not_found", "path" = main_path).to_string(),
                ));
            }

            // Use runpy to execute the file
            let runpy = PyModule::import(py, "runpy")?;
            let globals = runpy.call_method1("run_path", (main_path,))?;

            // Get the main function
            let main_fn = globals.get_item("main").map_err(|_| {
                RuntimeError::SyntaxError(
                    t!("plugin.runtime.plugin_must_have_main_function").to_string(),
                )
            })?;

            // Call the main function
            let result = main_fn.call0()?;

            // Convert the result to JSON
            self.py_to_json(&result.into())
                .map_err(|e| RuntimeError::JsonError(e.to_string()))
        })?)
    }

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

#[async_trait::async_trait]
impl Plugin for PythonRuntime {
    fn plugin_info(&self) -> &PluginInfo {
        &self.plugin_info
    }

    fn plugin_type(&self) -> &PluginType {
        &PluginType::Python
    }

    async fn init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.internal_init().await
    }

    async fn execute(
        &mut self,
        input: Option<Value>,
        plugin_info: Option<PluginInfo>,
    ) -> Result<Value, Box<dyn Error + Send + Sync>> {
        self.init_plugin_info(plugin_info)?;

        return self
            .execute_plugin(self.plugin_info.id.as_str(), input, None)
            .await;
    }

    async fn destroy(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.internal_cleanup().await
    }
}

pub struct PythonRuntimeFactory;

impl PythonRuntimeFactory {
    pub fn new() -> Self {
        Self {}
    }
}

impl PluginFactory for PythonRuntimeFactory {
    fn create_instance(
        &self,
        _init_options: Option<&Value>,
    ) -> Result<Box<dyn Plugin>, Box<dyn Error + Send + Sync>> {
        Ok(Box::new(PythonRuntime::new()?))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::runtime::Runtime as TokioRuntime;

    use super::*;

    // Helper function to run async tests
    fn run_async<F: std::future::Future>(future: F) -> F::Output {
        let rt = TokioRuntime::new().unwrap();
        rt.block_on(future)
    }

    #[test]
    fn test_python_runtime() {
        run_async(async {
            let mut runtime = PythonRuntime::new().unwrap();
            runtime.init().await.unwrap();

            // Test simple expression
            let result = runtime
                .internal_execute("2 + 2", None, None)
                .await
                .unwrap()
                .as_i64()
                .unwrap();
            assert_eq!(result, 4);

            // Test with input parameters
            let input = serde_json::json!({
                "x": 10,
                "y": 20
            });
            let code = "input['x'] + input['y']";
            let result = runtime
                .internal_execute(code, Some(input), None)
                .await
                .unwrap()
                .as_i64()
                .unwrap();
            assert_eq!(result, 30);

            // Test utility functions
            let code = r#"
data = {"name": "test", "value": 42}
result = data
"#;
            let result = runtime.internal_execute(code, None, None).await.unwrap();
            assert_eq!(result, serde_json::json!({"name": "test", "value": 42}));

            runtime.destroy().await.unwrap();
        });
    }

    #[test]
    fn test_complex_python_code() {
        run_async(async {
            let mut runtime = PythonRuntime::new().unwrap();
            runtime.init().await.unwrap();

            // Test complex data processing with function definition and list comprehension
            let input = serde_json::json!({
                "items": [
                    {"id": 1, "name": "item1", "tags": ["a", "b"]},
                    {"id": 2, "name": "item2", "tags": ["b", "c"]},
                    {"id": 3, "name": "item3", "tags": ["a", "c"]}
                ],
                "filter_tag": "a"
            });

            let code = r#"
def process_items(items, filter_tag):
    try:
        # Filter items containing the specified tag using list comprehension
        filtered = [item for item in items if filter_tag in item['tags']]

        # Transform the filtered items into a new structure
        result = {
            'matching_items': filtered,
            'count': len(filtered),
            'ids': [item['id'] for item in filtered],
            'summary': {
                'names': [item['name'] for item in filtered],
                'all_tags': list(set(tag for item in filtered for tag in item['tags']))
            }
        }
        return result
    except Exception as e:
        return {'error': str(e)}

# Process the input data
result = process_items(input['items'], input['filter_tag'])
"#;

            let result = runtime
                .internal_execute(code, Some(input), None)
                .await
                .unwrap();

            // Verify the complex result structure
            let result_obj = result.as_object().unwrap();
            assert_eq!(result_obj["count"].as_i64().unwrap(), 2);

            let ids = result_obj["ids"].as_array().unwrap();
            assert_eq!(ids, &[json!(1), json!(3)]);

            let summary = result_obj["summary"].as_object().unwrap();
            let names = summary["names"].as_array().unwrap();
            assert_eq!(names, &[json!("item1"), json!("item3")]);

            let all_tags = summary["all_tags"].as_array().unwrap();
            assert!(all_tags.contains(&json!("a")));
            assert!(all_tags.contains(&json!("b")));
            assert!(all_tags.contains(&json!("c")));

            // Test error handling
            let invalid_input = serde_json::json!({
                "items": "not_a_list",
                "filter_tag": "a"
            });

            let error_result = runtime
                .internal_execute(code, Some(invalid_input), None)
                .await
                .unwrap();
            assert!(error_result.as_object().unwrap().contains_key("error"));

            runtime.destroy().await.unwrap();
        });
    }

    #[test]
    fn test_file_operations() {
        use std::fs;

        run_async(async {
            // 使用项目的临时目录
            let test_dir = ".test".to_string();
            fs::create_dir_all(&test_dir)
                .map_err(|e| {
                    format!(
                        "failed to create directory: {}, error: {}",
                        &test_dir,
                        e.to_string()
                    )
                })
                .unwrap();

            let mut runtime = PythonRuntime::new_with_base_path(&test_dir).unwrap();
            runtime.init().await.unwrap();

            // 测试 JSON 文件读写
            let input = serde_json::json!({
                "file_path": format!("{}/data.json", test_dir),
                "data": {
                    "records": [
                        {"id": 1, "name": "item1", "tags": ["a", "b"]},
                        {"id": 2, "name": "item2", "tags": ["b", "c"]},
                        {"id": 3, "name": "item3", "tags": ["a", "c"]}
                    ],
                    "metadata": {
                        "total": 2,
                        "version": "1.0"
                    }
                }
            });

            let code = r#"
import json
import os

def write_and_read_json(file_path, data):
    try:
        # 写入 JSON 文件
        with open(file_path, 'w', encoding='utf-8') as f:
            json.dump(data, f, indent=2)

        # 读取并验证 JSON 文件
        with open(file_path, 'r', encoding='utf-8') as f:
            loaded_data = json.load(f)

        # 获取文件信息
        file_stats = os.stat(file_path)

        result = {
            'loaded_data': loaded_data,
            'file_size': file_stats.st_size,
            'success': loaded_data == data
        }
        return result
    except Exception as e:
        return {'error': str(e)}

# 执行文件操作
result = write_and_read_json(input['file_path'], input['data'])
"#;

            let result = runtime
                .internal_execute(code, Some(input), None)
                .await
                .unwrap();

            // 验证结果
            let result_obj = result.as_object().unwrap();
            assert!(result_obj["success"].as_bool().unwrap());
            assert!(result_obj["file_size"].as_i64().unwrap() > 0);

            // 验证加载的数据与原始数据匹配
            let loaded_data = &result_obj["loaded_data"];
            assert_eq!(loaded_data["records"].as_array().unwrap().len(), 3);
            assert_eq!(loaded_data["metadata"]["total"].as_i64().unwrap(), 2);

            // 测试文本文件处理
            let input = serde_json::json!({
                "base_path": test_dir,
                "files": {
                    "input.txt": "Line 1\nLine 2\nTest line\nLine 4\nFinal line",
                    "patterns.txt": "Test\nLine 2\nFinal"
                }
            });

            let code = r#"
def process_text_files(base_path, files):
    try:
        # 写入输入文件
        for filename, content in files.items():
            file_path = f"{base_path}/{filename}"
            with open(file_path, 'w', encoding='utf-8') as f:
                f.write(content)

        # 读取和处理文件
        input_path = f"{base_path}/input.txt"
        patterns_path = f"{base_path}/patterns.txt"

        # 读取模式
        with open(patterns_path, 'r', encoding='utf-8') as f:
            patterns = [line.strip() for line in f.readlines()]

        # 读取并匹配行
        matching_lines = []
        with open(input_path, 'r', encoding='utf-8') as f:
            for line in f:
                line = line.strip()
                if any(pattern in line for pattern in patterns):
                    matching_lines.append(line)

        # 写入结果到输出文件
        output_path = f"{base_path}/output.txt"
        with open(output_path, 'w', encoding='utf-8') as f:
            f.write('\n'.join(matching_lines))

        # 返回处理结果
        result = {
            'matching_lines': matching_lines,
            'pattern_count': len(patterns),
            'match_count': len(matching_lines),
            'files_processed': list(files.keys())
        }
        return result
    except Exception as e:
        return {'error': str(e)}

# 处理文本文件
result = process_text_files(input['base_path'], input['files'])
"#;

            let result = runtime
                .internal_execute(code, Some(input), None)
                .await
                .unwrap();

            // 验证文本处理结果
            let result_obj = result.as_object().unwrap();
            let matching_lines = result_obj["matching_lines"].as_array().unwrap();
            assert_eq!(matching_lines.len(), 3); // 应该匹配 "Line 2", "Test line", "Final line"
            assert_eq!(result_obj["pattern_count"].as_i64().unwrap(), 3);
            assert_eq!(result_obj["match_count"].as_i64().unwrap(), 3);

            // 验证输出文件
            let output_path = format!("{}/output.txt", test_dir);
            assert!(fs::metadata(&output_path).is_ok());
            let output_content = fs::read_to_string(&output_path).unwrap();
            assert!(output_content.contains("Line 2"));
            assert!(output_content.contains("Test line"));
            assert!(output_content.contains("Final line"));

            runtime.destroy().await.unwrap();

            // 清理测试目录
            fs::remove_dir_all(test_dir).unwrap();
        });
    }

    #[test]
    fn test_http_requests() {
        run_async(async {
            let mut runtime = PythonRuntime::new().unwrap();
            runtime.init().await.unwrap();

            // 测试 HTTP GET 和 POST 请求
            let code = r#"
import urllib.request
import urllib.error
import json
import ssl

def make_http_requests():
    try:
        results = {}

        # 创建一个不验证证书的 SSL 上下文（仅用于测试）
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE

        # GET 请求测试
        get_url = "https://httpbin.org/get?param1=test&param2=123"
        get_req = urllib.request.Request(
            get_url,
            headers={'User-Agent': 'Python Test Client'}
        )

        with urllib.request.urlopen(get_req, context=ctx) as response:
            get_data = json.loads(response.read().decode('utf-8'))
            results['get'] = {
                'status': response.status,
                'params': get_data.get('args', {}),
                'headers': get_data.get('headers', {})
            }

        # POST 请求测试
        post_url = "https://httpbin.org/post"
        post_data = json.dumps({
            'name': 'test_user',
            'data': [1, 2, 3],
            'config': {'active': True}
        }).encode('utf-8')

        post_req = urllib.request.Request(
            post_url,
            data=post_data,
            headers={
                'Content-Type': 'application/json',
                'User-Agent': 'Python Test Client'
            },
            method='POST'
        )

        with urllib.request.urlopen(post_req, context=ctx) as response:
            post_data = json.loads(response.read().decode('utf-8'))
            results['post'] = {
                'status': response.status,
                'json_data': post_data.get('json', {}),
                'headers': post_data.get('headers', {})
            }

        # 错误请求测试
        try:
            error_url = "https://httpbin.org/status/404"
            urllib.request.urlopen(error_url, context=ctx)
            results['error_test'] = {'success': False, 'message': '应该抛出 404 错误'}
        except urllib.error.HTTPError as e:
            results['error_test'] = {
                'success': True,
                'status': e.code,
                'message': str(e)
            }

        # 超时测试
        try:
            timeout_url = "https://httpbin.org/delay/5"
            urllib.request.urlopen(timeout_url, timeout=1, context=ctx)
            results['timeout_test'] = {'success': False, 'message': '应该发生超时'}
        except (urllib.error.URLError, TimeoutError) as e:
            results['timeout_test'] = {
                'success': True,
                'message': str(e)
            }

        return results
    except Exception as e:
        return {'error': str(e)}

# 执行 HTTP 请求测试
result = make_http_requests()
"#;

            let result = runtime.internal_execute(code, None, None).await.unwrap();

            // 验证结果
            let result_obj = result.as_object().unwrap();

            // 验证 GET 请求
            let get_result = result_obj["get"].as_object().unwrap();
            assert_eq!(get_result["status"].as_i64().unwrap(), 200);
            let get_params = get_result["params"].as_object().unwrap();
            assert_eq!(get_params["param1"].as_str().unwrap(), "test");
            assert_eq!(get_params["param2"].as_str().unwrap(), "123");

            // 验证 POST 请求
            let post_result = result_obj["post"].as_object().unwrap();
            assert_eq!(post_result["status"].as_i64().unwrap(), 200);
            let post_data = post_result["json_data"].as_object().unwrap();
            assert_eq!(post_data["name"].as_str().unwrap(), "test_user");
            assert!(post_data["config"].as_object().unwrap()["active"]
                .as_bool()
                .unwrap());

            // 验证错误处理
            let error_result = result_obj["error_test"].as_object().unwrap();
            assert!(error_result["success"].as_bool().unwrap());
            assert_eq!(error_result["status"].as_i64().unwrap(), 404);

            // 验证超时处理
            let timeout_result = result_obj["timeout_test"].as_object().unwrap();
            assert!(timeout_result["success"].as_bool().unwrap());
            assert!(timeout_result["message"]
                .as_str()
                .unwrap()
                .contains("timed out"));

            runtime.destroy().await.unwrap();
        });
    }

    #[test]
    fn test_async_operations() {
        run_async(async {
            let mut runtime = PythonRuntime::new().unwrap();
            runtime.init().await.unwrap();

            // 测试异步操作
            let code = r#"
import asyncio
import json
import time
from concurrent.futures import ThreadPoolExecutor

async def async_task(task_id, delay):
    await asyncio.sleep(delay)
    return {
        'task_id': task_id,
        'delay': delay,
        'timestamp': time.time()
    }

async def fetch_data(urls):
    tasks = []
    for i, url in enumerate(urls):
        # 模拟不同的处理时间
        delay = (i + 1) * 0.1
        tasks.append(async_task(i, delay))

    # 并发执行所有任务
    results = await asyncio.gather(*tasks)
    return results

def cpu_intensive_task(n):
    # 模拟CPU密集型任务
    result = 0
    for i in range(n):
        result += i * i
    return result

async def main():
    # 准备测试数据
    urls = [f"https://api.example.com/data/{i}" for i in range(5)]

    # 创建线程池执行器
    executor = ThreadPoolExecutor(max_workers=3)
    loop = asyncio.get_event_loop()

    try:
        # 1. 测试异步任务
        start_time = time.time()
        results = await fetch_data(urls)
        async_duration = time.time() - start_time

        # 2. 测试同步执行CPU密集型任务
        start_time = time.time()
        cpu_result = await loop.run_in_executor(executor, cpu_intensive_task, 1000000)
        cpu_duration = time.time() - start_time

        # 3. 测试超时处理
        try:
            async with asyncio.timeout(0.1):
                await asyncio.sleep(0.2)
            timeout_result = {'success': False, 'message': '应该发生超时'}
        except asyncio.TimeoutError:
            timeout_result = {'success': True, 'message': '成功捕获超时'}

        # 4. 测试取消操作
        long_task = asyncio.create_task(asyncio.sleep(10))
        await asyncio.sleep(0.1)
        long_task.cancel()

        try:
            await long_task
            cancel_result = {'success': False, 'message': '任务应该被取消'}
        except asyncio.CancelledError:
            cancel_result = {'success': True, 'message': '成功取消任务'}

        return {
            'async_results': results,
            'async_duration': async_duration,
            'cpu_result': cpu_result,
            'cpu_duration': cpu_duration,
            'timeout_test': timeout_result,
            'cancel_test': cancel_result
        }
    finally:
        executor.shutdown(wait=False)

# 执行异步测试
result = asyncio.run(main())
"#;

            let result = runtime.internal_execute(code, None, None).await.unwrap();

            // 验证结果
            let result_obj = result.as_object().unwrap();

            // 验证异步任务结果
            let async_results = result_obj["async_results"].as_array().unwrap();
            assert_eq!(async_results.len(), 5);

            // 验证执行时间（应该小于串行执行的总时间）
            let async_duration = result_obj["async_duration"].as_f64().unwrap();
            assert!(async_duration < 0.5 + 0.4 + 0.3 + 0.2 + 0.1); // 总延迟时间

            // 验证 CPU 密集型任务
            assert!(result_obj["cpu_result"].as_i64().is_some());
            assert!(result_obj["cpu_duration"].as_f64().unwrap() > 0.0);

            // 验证超时测试
            let timeout_test = result_obj["timeout_test"].as_object().unwrap();
            assert!(timeout_test["success"].as_bool().unwrap());

            // 验证取消操作
            let cancel_test = result_obj["cancel_test"].as_object().unwrap();
            assert!(cancel_test["success"].as_bool().unwrap());

            runtime.destroy().await.unwrap();
        });
    }

    #[test]
    fn test_environment_variables() {
        use std::env;

        run_async(async {
            let mut runtime = PythonRuntime::new().unwrap();
            runtime
                .init()
                .await
                .map_err(|e| eprintln!("初始化 Python 运行时失败: {}", e))
                .unwrap();

            // 设置一些测试环境变量
            env::set_var("PYTHON_TEST_VAR1", "test_value1");
            env::set_var("PYTHON_TEST_VAR2", "test_value2");
            env::set_var("PYTHON_TEST_PATH", "/usr/local/test:/usr/test");

            // 先验证 Rust 端环境变量设置成功
            assert_eq!(env::var("PYTHON_TEST_VAR1").unwrap(), "test_value1");

            let code = r#"
import os
import json
import sys

def test_environment():
    try:
        results = {}

        # 获取所有环境变量用于调试
        all_env = dict(os.environ)
        results['debug_all_env'] = {k: v for k, v in all_env.items() if k.startswith('PYTHON_TEST_')}

        # 1. 测试获取环境变量
        var1 = os.environ.get('PYTHON_TEST_VAR1', 'not_found')
        var2 = os.environ.get('PYTHON_TEST_VAR2', 'not_found')
        path = os.environ.get('PYTHON_TEST_PATH', 'not_found')

        results['env_vars'] = {
            'var1': var1,
            'var2': var2,
            'path': path,
            'not_exist': os.environ.get('PYTHON_TEST_NOT_EXIST', 'default_value')
        }

        # 添加调试信息
        results['debug_info'] = {
            'var1_direct': os.environ['PYTHON_TEST_VAR1'] if 'PYTHON_TEST_VAR1' in os.environ else 'not_found',
            'var1_getenv': os.getenv('PYTHON_TEST_VAR1', 'not_found'),
            'env_keys': list(os.environ.keys())
        }

        # 2. 测试设置新环境变量
        os.environ['PYTHON_TEST_NEW_VAR'] = 'new_value'
        results['new_var'] = os.getenv('PYTHON_TEST_NEW_VAR')

        # 3. 测试环境变量的修改
        if path:
            os.environ['PYTHON_TEST_PATH'] = path + ':/new/path'
            results['modified_path'] = os.getenv('PYTHON_TEST_PATH')
        else:
            results['modified_path'] = '/new/path'
            os.environ['PYTHON_TEST_PATH'] = '/new/path'

        # 4. 测试系统信息
        results['system_info'] = {
            'platform': sys.platform,
            'python_version': sys.version.split()[0],
            'cwd': os.getcwd(),
            'path_separator': os.pathsep
        }
        results['http_proxy'] = os.getenv('http_proxy')

        # 5. 测试路径操作
        current_path = os.getenv('PYTHON_TEST_PATH', '')
        results['path_parts'] = current_path.split(os.pathsep) if current_path else []

        return results
    except Exception as e:
        return {'error': str(e), 'error_type': str(type(e))}

# 执行环境变量测试并返回结果
result = test_environment()
print("Debug: Result =", result)  # 添加调试输出
result  # 确保返回结果
"#;

            let result = runtime.internal_execute(code, None, None).await.unwrap();
            let result_obj = result.as_object().unwrap();

            // 检查是否有错误发生
            if result_obj.contains_key("error") {
                panic!(
                    "Python execution error: {} ({})",
                    result_obj["error"].as_str().unwrap(),
                    result_obj["error_type"]
                        .as_str()
                        .unwrap_or("unknown error type")
                );
            }

            // 打印调试信息
            if let Some(debug_info) = result_obj.get("debug_info").and_then(|v| v.as_object()) {
                println!("Debug Info:");
                println!(
                    "  var1_direct: {:?}",
                    debug_info.get("var1_direct").and_then(|v| v.as_str())
                );
                println!(
                    "  var1_getenv: {:?}",
                    debug_info.get("var1_getenv").and_then(|v| v.as_str())
                );
                println!(
                    "  env_keys: {:?}",
                    debug_info.get("env_keys").and_then(|v| v.as_array())
                );
            }

            if let Some(all_env) = result_obj.get("debug_all_env").and_then(|v| v.as_object()) {
                println!("\nAll PYTHON_TEST_ environment variables:");
                for (k, v) in all_env.iter() {
                    println!("  {}: {:?}", k, v.as_str());
                }
            }

            // 验证环境变量获取
            let env_vars = result_obj
                .get("env_vars")
                .and_then(|v| v.as_object())
                .expect("env_vars should be an object");

            // 使用更安全的方式验证值
            let var1_value = env_vars.get("var1").and_then(|v| v.as_str()).unwrap_or("");

            if var1_value != "test_value1" {
                println!("\nEnvironment variable verification failed:");
                println!("  Expected: test_value1");
                println!("  Actual: {}", var1_value);
                println!("  Current process env: {:?}", env::var("PYTHON_TEST_VAR1"));
            }

            assert_eq!(var1_value, "test_value1", "PYTHON_TEST_VAR1 value mismatch");
            assert_eq!(
                env_vars.get("var2").and_then(|v| v.as_str()).unwrap_or(""),
                "test_value2",
                "PYTHON_TEST_VAR2 value mismatch"
            );
            assert_eq!(
                env_vars
                    .get("http_proxy")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                env::var("http_proxy").unwrap_or("".to_string()),
            );

            // 其他验证...

            // 清理环境变量
            env::remove_var("PYTHON_TEST_VAR1");
            env::remove_var("PYTHON_TEST_VAR2");
            env::remove_var("PYTHON_TEST_PATH");
            env::remove_var("PYTHON_TEST_NEW_VAR");

            runtime.destroy().await.unwrap();
        });
    }

    #[test]
    fn test_plugin_loading() {
        use std::fs;
        use tempfile::TempDir;

        run_async(async {
            // 创建临时目录作为插件目录
            let plugin_dir = TempDir::new().unwrap();
            let plugin_path = plugin_dir.path();

            // 创建插件文件
            let main_content = r#"
import helper

def main():
    return helper.create_greeting()
"#;

            let helper_content = r#"
def create_greeting():
    return {"message": "Hello, World!"}
"#;

            // 写入插件文件
            fs::write(plugin_path.join("main.py"), main_content).unwrap();
            fs::write(plugin_path.join("helper.py"), helper_content).unwrap();

            // 初始化运行时并加载插件
            let mut runtime = PythonRuntime::new().unwrap();
            runtime.init().await.unwrap();

            // 测试方式1：直接读取文件内容执行
            let result = runtime
                .execute_plugin(plugin_path.to_str().unwrap(), None, None)
                .await
                .unwrap();

            let result_obj = result.as_object().unwrap();
            assert_eq!(result_obj["message"].as_str().unwrap(), "Hello, World!");

            runtime.destroy().await.unwrap();
        });
    }
}
