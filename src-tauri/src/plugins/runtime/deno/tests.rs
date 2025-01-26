#[cfg(test)]
mod tests {
    use crate::plugins::runtime::deno::DenoRuntime;
    use crate::plugins::Plugin;

    use log::LevelFilter;
    use simplelog::{ColorChoice, CombinedLogger, ConfigBuilder, TermLogger, TerminalMode};
    use std::fs;
    use std::sync::Once;
    use tempfile::TempDir;

    static INIT: Once = Once::new();

    /// 初始化测试环境
    fn init_test_env() {
        INIT.call_once(|| {
            let config = ConfigBuilder::new()
                .set_target_level(LevelFilter::Debug)
                .set_location_level(LevelFilter::Debug) // 启用位置信息
                .set_time_level(LevelFilter::Debug) // 启用时间戳
                .build();

            CombinedLogger::init(vec![TermLogger::new(
                LevelFilter::Debug,
                config,
                TerminalMode::Mixed,
                ColorChoice::Auto,
            )])
            .unwrap();
        });
    }

    fn setup_test_plugin(
        dir: &TempDir,
        plugin_id: &str,
        manifest: Option<&str>,
        main_js: &str,
    ) -> std::path::PathBuf {
        init_test_env(); // 调用初始化函数

        // 创建插件目录结构
        let base_dir = dir.path().join("plugins");
        fs::create_dir_all(&base_dir).unwrap();

        // 设置全局插件目录
        // {
        //     let mut plugins_dir_path = crate::PLUGINS_DIR.write().unwrap();
        //     *plugins_dir_path = plugins_dir.to_string_lossy().to_string();
        // }

        // println!("plugins base dir: {}", crate::PLUGINS_DIR.read().unwrap());

        // 创建具体的插件目录
        let plugin_dir = base_dir.join(plugin_id);
        fs::create_dir_all(&plugin_dir).unwrap();

        // println!("plugin dir: {}", plugin_dir.display());

        // 创建 data 目录（用于文件系统权限测试）
        fs::create_dir_all(plugin_dir.join("data")).unwrap();

        // 创建 shared 目录（用于文件系统权限测试）
        {
            let mut shared_dir_path = crate::SHARED_DATA_DIR.write();
            *shared_dir_path = dir.path().join("shared").to_string_lossy().to_string();
        }
        fs::create_dir_all(dir.path().join("shared")).unwrap();

        // 写入 manifest.json
        let manifest_content = manifest.unwrap_or(
            r#"{
            "permissions": {
                "fs": {
                    "allow_shared": false
                },
                "network": {
                    "enabled": true,
                    "allowed_domains": ["httpbin.org"]
                }
            }
        }"#,
        );

        fs::write(plugin_dir.join("manifest.json"), manifest_content).unwrap();

        // 写入 main.js
        fs::write(plugin_dir.join("main.js"), main_js).unwrap();

        dbg!(&plugin_dir.join("main.js"));

        base_dir
    }

    /// 清理全局状态
    fn cleanup_test_state() {
        // 重置插件目录
        let mut dir = crate::PLUGINS_DIR.write();
        *dir = String::new();
        // 重置共享目录
        let mut dir = crate::SHARED_DATA_DIR.write();
        *dir = String::new();
    }

    #[tokio::test]
    async fn test_file_system_permissions_comprehensive() {
        let temp_dir = TempDir::new().unwrap();
        let shared_dir = temp_dir.path().join("shared");

        // Set up shared directory
        // {
        //     let mut shared_dir_path = crate::SHARED_DATA_DIR.write().unwrap();
        //     *shared_dir_path = shared_dir.to_string_lossy().to_string();
        // }

        let test_cases = vec![
            (
                "deny_shared",
                r#"{"permissions": {"fs": {"allow_shared": false}}}"#,
                false,
            ),
            (
                "allow_shared",
                r#"{"permissions": {"fs": {"allow_shared": true}}}"#,
                true,
            ),
        ];

        for (test_id, permissions, can_access_shared) in test_cases {
            log::debug!("Testing plugin: {}", test_id);

            // 清理之前的测试文件
            let shared_file = shared_dir.join("test.txt");
            if shared_file.exists() {
                fs::remove_file(&shared_file).unwrap();
            }

            let plugin_dir = setup_test_plugin(
                &temp_dir,
                test_id,
                Some(permissions),
                r#"
                export async function main() {
                    const results = {
                        plugin_dir: { success: false, error: null, content: null },
                        shared_dir: { success: false, error: null, content: null },
                    };
                    console.log("init result", results);

                    // Test plugin directory access
                    try {
                        await writeFile("data/test.txt", "hello", false);
                        results.plugin_dir.content = await readFile("data/test.txt", false);
                        results.plugin_dir.success = results.plugin_dir.content === "hello";
                    } catch (e) {
                        results.plugin_dir.error = e.toString();
                        console.error('Plugin dir error:', e.toString());
                    }

                    // Test shared directory access
                    try {
                        await writeFile("../shared/test.txt", "world", true);
                        results.shared_dir.content = await readFile("../shared/test.txt", true);
                        results.shared_dir.success = results.shared_dir.content === "world";
                    } catch (e) {
                        results.shared_dir.error = e.toString();
                        console.error('Shared dir error:', e.toString());
                    }
                    console.log(results);

                    return results;
                }
                "#,
            );

            let mut runtime =
                DenoRuntime::new_with_base_path(plugin_dir, shared_dir.clone().into()).unwrap();

            let result = runtime.execute_module(test_id, None).await.unwrap();
            let obj = result.as_object().unwrap();

            // 验证插件目录访问（应该总是成功）
            let plugin_dir_result = obj.get("plugin_dir").unwrap().as_object().unwrap();
            assert!(
                plugin_dir_result.get("success").unwrap().as_bool().unwrap(),
                "Plugin directory access should always be allowed. Error: {:?}",
                plugin_dir_result.get("error")
            );
            assert_eq!(
                plugin_dir_result.get("content").unwrap().as_str().unwrap(),
                "hello",
                "Plugin directory content mismatch"
            );

            // 验证共享目录访问（基于权限）
            let shared_dir_result = obj.get("shared_dir").unwrap().as_object().unwrap();
            log::debug!(
                "Checking shared directory access: can_access_shared={}, result={:?}",
                can_access_shared,
                shared_dir_result
            );

            if can_access_shared {
                assert!(
                    shared_dir_result.get("success").unwrap().as_bool().unwrap(),
                    "Shared directory access should be allowed"
                );
                assert_eq!(
                    shared_dir_result.get("content").unwrap().as_str().unwrap(),
                    "world",
                    "Shared directory content mismatch"
                );
            } else {
                let success = shared_dir_result.get("success").unwrap().as_bool().unwrap();
                let error = shared_dir_result.get("error").unwrap().as_str().unwrap();

                assert!(
                    !success,
                    "Shared directory access should be denied (success={})",
                    success
                );
                assert!(
                    error.contains("Access denied"),
                    "Should get 'access denied' error, got: {}",
                    error
                );
            }

            runtime.destroy().await.unwrap();
        }

        cleanup_test_state();
    }

    #[tokio::test]
    async fn test_network_permissions() {
        let temp_dir = TempDir::new().unwrap();

        // 创建测试插件
        let manifest = r#"{
            "permissions": {
                "network": {
                    "enabled": true,
                    "allowed_domains": ["httpbin.org"]
                }
            }
        }"#;

        let base_dir = setup_test_plugin(
            &temp_dir,
            "test_plugin",
            Some(manifest),
            r#"export async function main() {
                try {
                    // 测试允许的域名
                    console.log("Starting HTTP request...");
                    console.log("Attempting to fetch from httpbin.org...");

                    // 使用自定义的sleep函数实现超时
                    const timeout = async () => {
                        await sleep(5000);  // 5秒超时
                        throw new Error('Request timeout after 5 seconds');
                    };

                    const fetchPromise = fetch("https://httpbin.org/get");
                    const allowed = await Promise.race([fetchPromise, timeout()]);
                    console.log("HTTP request completed successfully:", allowed);

                    // 测试禁止的域名（应该失败）
                    try {
                        console.log("Testing blocked domain...");
                        await fetch("https://example.com");
                        return { success: false, error: "Should not be able to access example.com", allowed: JSON.parse(allowed) };
                    } catch (e) {
                        console.log("Expected error for blocked domain:", e);
                        return { success: true, error: e.toString(), allowed: JSON.parse(allowed) };
                    }

                } catch (e) {
                    console.error("Test failed:", e);
                    return { success: false, error: e.toString(), allowed: JSON.parse(allowed)};
                }
            }"#,
        );

        // 直接在当前 tokio runtime 中执行测试
        let runtime = DenoRuntime::new_with_base_path(base_dir, temp_dir.path().into()).unwrap();

        let result = runtime.execute_module("test_plugin", None).await.unwrap();
        let obj = result.as_object().unwrap();
        dbg!(&obj);
        assert!(obj.get("success").unwrap().as_bool().unwrap());
        assert!(obj.get("allowed").unwrap().is_object());
    }

    #[tokio::test]
    async fn test_sleep_operation() {
        let temp_dir = TempDir::new().unwrap();

        // 创建测试插件
        let manifest = r#"{
            "permissions": {
                "fs": {
                    "allow_shared": false
                },
                "network": {
                    "enabled": false
                }
            }
        }"#;

        let plugin_base_dir = setup_test_plugin(
            &temp_dir,
            "test_sleep",
            Some(manifest),
            r#"
            export async function main() {
                try {
                    console.log("Starting sleep test...");
                    const start = Date.now();
                    
                    // Sleep for 100ms
                    await sleep(100);
                    
                    const elapsed = Date.now() - start;
                    console.log(`Sleep completed. Elapsed time: ${elapsed}ms`);
                    
                    // Verify that at least 100ms have passed
                    return {
                        success: true,
                        elapsed,
                        slept_enough: elapsed >= 100
                    };
                } catch (e) {
                    console.error("Sleep test failed:", e);
                    return {
                        success: false,
                        error: e.toString()
                    };
                }
            }
            "#,
        );

        let mut runtime =
            DenoRuntime::new_with_base_path(plugin_base_dir, temp_dir.path().into()).unwrap();
        let result = runtime.execute_module("test_sleep", None).await.unwrap();

        let obj = result.as_object().unwrap();
        assert!(obj.get("success").unwrap().as_bool().unwrap());
        assert!(obj.get("slept_enough").unwrap().as_bool().unwrap());

        let elapsed = obj.get("elapsed").unwrap().as_f64().unwrap();
        assert!(
            elapsed >= 100.0,
            "Sleep duration was too short: {}ms",
            elapsed
        );

        runtime.destroy().await.unwrap();
        cleanup_test_state();
    }

    #[tokio::test]
    async fn test_module_loading() {
        init_test_env();
        let temp_dir = TempDir::new().unwrap();

        // 创建测试插件
        let plugin_base_dir = setup_test_plugin(
            &temp_dir,
            "test-module-loading",
            None,
            r#"
            export function testModule() {
                return "Module loaded successfully";
            }

            export function testAssert() {
                return "Assert test passed";
            }

            export async function main() {
                try {
                    const mod_result = testModule();
                    const assert_result = testAssert();
                    
                    return {
                        success: true,
                        results: {
                            testModule: mod_result,
                            testAssert: assert_result
                        }
                    };
                } catch (e) {
                    console.error("Module test failed:", e);
                    return {
                        success: false,
                        error: e.toString()
                    };
                }
            }
            "#,
        );

        let mut runtime =
            DenoRuntime::new_with_base_path(plugin_base_dir, temp_dir.path().into()).unwrap();
        let result = runtime
            .execute_module("test-module-loading", None)
            .await
            .unwrap();

        let obj = result.as_object().unwrap();
        assert!(obj.get("success").unwrap().as_bool().unwrap());

        let results = obj.get("results").unwrap().as_object().unwrap();
        assert_eq!(
            results.get("testModule").unwrap().as_str().unwrap(),
            "Module loaded successfully"
        );
        assert_eq!(
            results.get("testAssert").unwrap().as_str().unwrap(),
            "Assert test passed"
        );

        runtime.destroy().await.unwrap();
        cleanup_test_state();
    }

    #[tokio::test]
    async fn test_file_operations() {
        init_test_env();
        let temp_dir = TempDir::new().unwrap();

        // 创建测试插件
        let plugin_base_dir = setup_test_plugin(
            &temp_dir,
            "test_files",
            None,
            r#"
            export async function testWriteFile() {
                await writeFile("./data/test.txt", "Hello", false);
                return "File written successfully";
            }

            export async function testReadFile() {
                const content = await readFile("./data/test.txt", false);
                return content;
            }

            export async function testAppendFile() {
                const content = await testReadFile();
                await writeFile("./data/test.txt", content + " World", false);
                return "Content appended successfully";
            }

            export async function main() {
                try {
                    const write_result = await testWriteFile();
                    const read_result = await testReadFile();
                    const append_result = await testAppendFile();
                    const final_content = await testReadFile();

                    return {
                        success: true,
                        results: {
                            writeFile: write_result,
                            readFile: read_result,
                            appendFile: append_result,
                            finalContent: final_content
                        }
                    };
                } catch (e) {
                    console.error("File operations failed:", e);
                    return {
                        success: false,
                        error: e.toString()
                    };
                }
            }
            "#,
        );

        let mut runtime =
            DenoRuntime::new_with_base_path(plugin_base_dir, temp_dir.path().into()).unwrap();
        let result = runtime.execute_module("test_files", None).await.unwrap();

        let obj = result.as_object().unwrap();
        assert!(obj.get("success").unwrap().as_bool().unwrap());

        let results = obj.get("results").unwrap().as_object().unwrap();
        assert_eq!(
            results.get("writeFile").unwrap().as_str().unwrap(),
            "File written successfully"
        );
        assert_eq!(results.get("readFile").unwrap().as_str().unwrap(), "Hello");
        assert_eq!(
            results.get("appendFile").unwrap().as_str().unwrap(),
            "Content appended successfully"
        );
        assert_eq!(
            results.get("finalContent").unwrap().as_str().unwrap(),
            "Hello World"
        );

        runtime.destroy().await.unwrap();
        cleanup_test_state();
    }

    #[tokio::test]
    async fn test_complex_network_operations() {
        init_test_env();
        let temp_dir = TempDir::new().unwrap();

        // 创建测试插件
        let plugin_base_dir = setup_test_plugin(
            &temp_dir,
            "test-complex-network",
            None,
            r#"
            export async function testAllowedDomain() {
                try {
                    await fetch("https://httpbin.org/get");
                    return "Allowed domain fetch successful";
                } catch (error) {
                    return `Error: ${error.message}`;
                }
            }

            export async function testBlockedDomain() {
                try {
                    await fetch("http://blocked.test.local/data");
                    return "Unexpected success";
                } catch (error) {
                    if (error.message.includes("Permission error")) {
                        return "Permission error";
                    }
                    return `Unexpected error: ${error.message}`;
                }
            }

            export async function testTimeout() {
                try {
                    // 创建两个 Promise：一个是请求，一个是超时
                    const fetchPromise = fetch("https://httpbin.org/delay/5");
                    const timeoutPromise = new Promise((_, reject) => {
                        sleep(100);
                        reject(new Error("Request timed out"));
                    });
                    
                    // 使用 Promise.race 来实现超时
                    await Promise.race([fetchPromise, timeoutPromise]);
                    return "Unexpected: request should have timed out";
                } catch (error) {
                    if (error.message === "Request timed out") {
                        return "Request aborted as expected";
                    }
                    return `Unexpected error: ${error.message}`;
                }
            }

            export async function main() {
                try {
                    const allowed_result = await testAllowedDomain();
                    const blocked_result = await testBlockedDomain();
                    const timeout_result = await testTimeout();

                    return {
                        success: true,
                        results: {
                            testAllowedDomain: allowed_result,
                            testBlockedDomain: blocked_result,
                            testTimeout: timeout_result
                        }
                    };
                } catch (e) {
                    console.error("Network tests failed:", e);
                    return {
                        success: false,
                        error: e.toString()
                    };
                }
            }
            "#,
        );

        let mut runtime =
            DenoRuntime::new_with_base_path(plugin_base_dir, temp_dir.path().into()).unwrap();
        let result = runtime
            .execute_module("test-complex-network", None)
            .await
            .unwrap();

        let obj = result.as_object().unwrap();
        assert!(obj.get("success").unwrap().as_bool().unwrap());

        let results = obj.get("results").unwrap().as_object().unwrap();
        assert_eq!(
            results.get("testAllowedDomain").unwrap().as_str().unwrap(),
            "Allowed domain fetch successful"
        );
        assert!(results
            .get("testBlockedDomain")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("Permission error"));
        assert_eq!(
            results.get("testTimeout").unwrap().as_str().unwrap(),
            "Request aborted as expected"
        );

        runtime.destroy().await.unwrap();
        cleanup_test_state();
    }

    #[tokio::test]
    async fn test_error_handling() {
        init_test_env();
        let temp_dir = TempDir::new().unwrap();

        // 创建测试插件
        let plugin_base_dir = setup_test_plugin(
            &temp_dir,
            "test-error-handling",
            None,
            r#"
            export function testSyntaxError() {
                try {
                    eval("this is not valid javascript");
                    return "Unexpected success";
                } catch (e) {
                    return e.toString();
                }
            }

            export function testRuntimeError() {
                try {
                    const obj = null;
                    obj.nonexistent;
                    return "Unexpected success";
                } catch (e) {
                    return e.toString();
                }
            }

            export async function testAsyncError() {
                try {
                    await Promise.reject(new Error("Async operation failed"));
                    return "Unexpected success";
                } catch (e) {
                    return e.toString();
                }
            }

            export async function main() {
                try {
                    const syntax_result = testSyntaxError();
                    const runtime_result = testRuntimeError();
                    const async_result = await testAsyncError();

                    return {
                        success: true,
                        results: {
                            testSyntaxError: syntax_result,
                            testRuntimeError: runtime_result,
                            testAsyncError: async_result
                        }
                    };
                } catch (e) {
                    console.error("Error handling tests failed:", e);
                    return {
                        success: false,
                        error: e.toString()
                    };
                }
            }
            "#,
        );

        let mut runtime =
            DenoRuntime::new_with_base_path(plugin_base_dir, temp_dir.path().into()).unwrap();
        let result = runtime
            .execute_module("test-error-handling", None)
            .await
            .unwrap();

        let obj = result.as_object().unwrap();
        assert!(obj.get("success").unwrap().as_bool().unwrap());

        let results = obj.get("results").unwrap().as_object().unwrap();
        assert!(results
            .get("testSyntaxError")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("SyntaxError"));
        assert!(results
            .get("testRuntimeError")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("TypeError"));
        assert!(results
            .get("testAsyncError")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("Async operation failed"));

        runtime.destroy().await.unwrap();
        cleanup_test_state();
    }
}
