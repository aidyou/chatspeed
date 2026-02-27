# ChatSpeed WebAssembly 沙箱隔离原型计划

## 概述

本计划详细描述了 ChatSpeed Agent Runtime 中 **第三方技能（Skills）安全沙箱隔离** 的实现方案。通过创建受控的、资源受限的 WebAssembly (Wasm) 执行环境，将不可信的技能代码与主系统隔离开，确保系统安全性和稳定性。

**核心目标**：建立用户对第三方技能的**可信执行环境**，为 ChatSpeed 的技能生态扩展奠定安全基础。

## 1. 为什么需要沙箱隔离？

| 风险 | 后果 | 沙箱的防护作用 |
|------|------|----------------|
| **恶意代码** | 删除文件、窃取密钥、执行勒索命令 | 限制文件系统访问，禁止危险系统调用 |
| **资源滥用** | 内存泄漏、CPU 占用 100%、磁盘写满 | 设置内存上限、CPU 时间配额、磁盘配额 |
| **不稳定代码** | 崩溃、死锁、无限循环 | 进程级隔离，崩溃不影响主执行器 |
| **信息泄露** | 读取敏感环境变量、扫描网络 | 过滤环境变量，限制网络连接 |
| **依赖冲突** | 技能依赖的库版本与宿主冲突 | 独立的依赖环境，避免版本污染 |

**核心原则**：**任何第三方技能默认不可信，必须在最小权限的沙箱中运行。**

## 2. 技术方案对比

| 方案 | 安全性 | 性能开销 | 实现复杂度 | 适合场景 |
|------|--------|----------|------------|----------|
| **进程隔离** | ⭐⭐⭐⭐⭐（最高） | 高（进程创建/销毁） | 中 | 需要最强隔离，技能运行时间长 |
| **WebAssembly (Wasm)** | ⭐⭐⭐⭐（很高） | 低（近似原生） | 中高 | 轻量级技能，需要快速启动/销毁 |
| **语言运行时隔离**（如 Rust `unsafe` 边界） | ⭐⭐（有限） | 极低 | 低 | 信任的技能作者，内部技能 |
| **容器化**（如 Docker） | ⭐⭐⭐⭐⭐（最高） | 高（镜像拉取、启动） | 高 | 需要完整操作系统环境的重型技能 |
| **gVisor / Firecracker** | ⭐⭐⭐⭐⭐（最高） | 中 | 很高 | 生产级多租户，云原生环境 |

**推荐选择：WebAssembly (Wasm)**，原因如下：
- **安全沙箱**：Wasm 指令集经过设计，内存隔离，无法直接调用系统调用。
- **轻量快速**：启动时间 <1ms，内存开销小，适合频繁调用的技能。
- **跨平台**：字节码格式统一，Windows/macOS/Linux 无需适配。
- **Rust 生态友好**：`wasmtime`、`wasmer` 等运行时成熟，Rust 可轻松编译为 Wasm。
- **资源控制精细**：可限制内存、CPU 指令数、执行时间。

## 3. WebAssembly 沙箱架构设计

### 3.1 整体架构
```
┌─────────────────────────────────────────────────┐
│               ChatSpeed 主执行器                 │
│  ┌─────────────┐  ┌─────────────┐               │
│  │ ToolManager │  │ SkillManager│               │
│  └──────┬──────┘  └──────┬──────┘               │
│         │                │                      │
│         ▼                ▼                      │
│  ┌─────────────────────────────────────┐        │
│  │         Wasm 运行时环境 (wasmtime)   │        │
│  │  ┌─────────────┐  ┌─────────────┐  │        │
│  │  │   Skill A    │  │   Skill B    │  │        │
│  │  │  (Wasm模块)  │  │  (Wasm模块)  │  │        │
│  │  └─────────────┘  └─────────────┘  │        │
│  └─────────────────────────────────────┘        │
└─────────────────────────────────────────────────┘
```

### 3.2 技能 Wasm 模块接口定义 (Skill ABI)

技能必须导出固定的函数供宿主调用：

```rust
// 技能清单 skill.json
{
  "name": "github-analyzer",
  "version": "1.0.0",
  "entry_point": "run",  // Wasm 导出函数名
  "permissions": [
    "net:https://api.github.com",
    "fs:read:/tmp",
    "env:GITHUB_TOKEN"
  ],
  "resources": {
    "max_memory_mb": 50,
    "max_execution_ms": 5000,
    "max_cpu_instructions": 100_000_000
  }
}

// Wasm 模块必须导出的函数签名（通过 wit-bindgen 定义）
// wit/skill.wit
interface skill {
  // 初始化技能，返回技能提供的工具列表
  init: function() -> list<tool-descriptor>;
  
  // 执行工具调用
  run: function(
    tool-name: string,
    params: list<param>
  ) -> result<output, error>;
  
  // 清理资源
  shutdown: function() -> unit;
}

record tool-descriptor {
  name: string,
  description: string,
  parameters: list<param-schema>,
}

record param-schema {
  name: string,
  type: string, // "string", "number", "boolean", "array", "object"
  required: bool,
}
```

### 3.3 主机 (Host) 环境实现

```rust
// src-tauri/src/skills/wasm_sandbox.rs
pub struct WasmSandbox {
    engine: wasmtime::Engine,
    store: wasmtime::Store<SandboxState>,
    module: wasmtime::Module,
    instance: wasmtime::Instance,
}

pub struct SandboxState {
    // 资源使用追踪
    resources: ResourceTracker,
    // 权限检查器
    permissions: PermissionChecker,
    // 技能配置
    config: SkillConfig,
    // 通信通道（用于技能调用宿主工具）
    host_sender: mpsc::Sender<HostCall>,
}

impl WasmSandbox {
    pub fn new(skill_path: &Path, config: SkillConfig) -> Result<Self> {
        // 1. 创建配置了限制的引擎
        let mut config = wasmtime::Config::new();
        config.wasm_multi_memory(true);
        config.consume_fuel(true); // 启用燃料计量（CPU限制）
        config.max_wasm_stack(1024 * 1024); // 1MB 栈限制
        
        let engine = wasmtime::Engine::new(&config)?;
        
        // 2. 编译 Wasm 模块（可缓存）
        let module = wasmtime::Module::from_file(&engine, skill_path)?;
        
        // 3. 创建存储，初始化状态
        let mut store = wasmtime::Store::new(&engine, SandboxState::new(config));
        store.add_fuel(1_000_000)?; // 分配初始燃料
        
        // 4. 定义主机函数（技能可调用）
        let imports = Self::create_host_imports(&mut store);
        
        // 5. 实例化模块
        let instance = wasmtime::Instance::new(&mut store, &module, &imports)?;
        
        Ok(Self { engine, store, module, instance })
    }
    
    fn create_host_imports(store: &mut Store<SandboxState>) -> Vec<wasmtime::Extern> {
        // 定义技能可调用的宿主函数
        // 例如：日志记录、受限的文件访问、网络请求（需权限）
        vec![
            // 技能调用宿主日志
            wasmtime::Func::wrap(
                store,
                |caller: wasmtime::Caller<'_, SandboxState>,
                 level: i32, message_ptr: i32, message_len: i32| {
                    let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
                    let message = read_string_from_memory(&memory, message_ptr, message_len);
                    
                    // 检查权限：技能是否有权日志？
                    caller.data().permissions.check("log")?;
                    
                    log::log!(level.into(), "[Skill] {}", message);
                    Ok(())
                },
            ),
            // 更多宿主函数...
        ]
    }
    
    pub fn call_tool(&mut self, tool_name: &str, params: &[Value]) -> Result<Value> {
        // 1. 检查资源配额
        self.store.data_mut().resources.check_quota()?;
        
        // 2. 获取 Wasm 导出函数
        let run_func = self.instance.get_func(&mut self.store, "run")
            .ok_or_else(|| Error::SkillMissingExport("run".into()))?;
        
        // 3. 准备参数
        let mut wasm_params = vec![];
        // ... 将 Rust 值转换为 Wasm 值
        
        // 4. 调用并计量燃料（CPU时间）
        let start_fuel = self.store.get_fuel()?;
        let results = run_func.call(&mut self.store, &wasm_params)?;
        let used_fuel = start_fuel - self.store.get_fuel()?;
        
        // 5. 更新资源使用
        self.store.data_mut().resources.record_cpu_usage(used_fuel);
        
        // 6. 转换并返回结果
        Ok(convert_wasm_to_rust(&results[0]))
    }
}
```

### 3.4 资源追踪与限制器

```rust
pub struct ResourceTracker {
    // CPU 限制（通过燃料计量）
    max_fuel: u64,
    used_fuel: AtomicU64,
    
    // 内存限制
    max_memory_bytes: u64,
    current_memory: AtomicU64,
    
    // 执行时间限制
    max_duration: Duration,
    start_time: Instant,
    
    // 系统调用计数
    syscall_counts: HashMap<String, AtomicU32>,
    syscall_limits: HashMap<String, u32>,
}

impl ResourceTracker {
    pub fn check_quota(&self) -> Result<()> {
        // 检查 CPU
        if self.used_fuel.load(Ordering::Relaxed) >= self.max_fuel {
            return Err(Error::ResourceLimitExceeded("CPU燃料".into()));
        }
        
        // 检查内存
        if self.current_memory.load(Ordering::Relaxed) >= self.max_memory_bytes {
            return Err(Error::ResourceLimitExceeded("内存".into()));
        }
        
        // 检查执行时间
        if self.start_time.elapsed() > self.max_duration {
            return Err(Error::ResourceLimitExceeded("执行时间".into()));
        }
        
        Ok(())
    }
    
    pub fn record_syscall(&self, name: &str) -> Result<()> {
        if let Some(limit) = self.syscall_limits.get(name) {
            let count = self.syscall_counts
                .get(name)
                .unwrap()
                .fetch_add(1, Ordering::Relaxed);
            
            if count >= *limit {
                return Err(Error::ResourceLimitExceeded(
                    format!("系统调用 {} 次数", name)
                ));
            }
        }
        Ok(())
    }
}
```

### 3.5 权限模型

```rust
pub struct PermissionChecker {
    // 技能声明的权限列表
    declared_permissions: HashSet<String>,
    // 运行时授予的权限（用户可能只批准部分）
    granted_permissions: HashSet<String>,
    // 权限到资源的映射
    permission_mapping: HashMap<String, Vec<ResourcePattern>>,
}

impl PermissionChecker {
    pub fn check(&self, permission: &str) -> Result<()> {
        // 1. 检查是否声明了该权限
        if !self.declared_permissions.contains(permission) {
            return Err(Error::PermissionNotDeclared(permission.into()));
        }
        
        // 2. 检查是否被授予
        if !self.granted_permissions.contains(permission) {
            return Err(Error::PermissionDenied(permission.into()));
        }
        
        // 3. 检查具体资源访问（如文件路径匹配）
        if let Some(patterns) = self.permission_mapping.get(permission) {
            // 根据当前操作检查资源是否匹配模式
            // 例如：permission "fs:read:/tmp/*" 匹配路径 "/tmp/foo.txt"
        }
        
        Ok(())
    }
}

// 权限示例：
// - "fs:read:/home/user/project/*"  // 读取特定目录
// - "fs:write:/tmp"                 // 写入临时目录
// - "net:https://api.github.com/*"  // 访问 GitHub API
// - "net:*"                         // 访问任何网络（危险）
// - "env:GITHUB_TOKEN"              // 读取特定环境变量
// - "shell:execute:git"             // 执行 git 命令
// - "tool:call:web_fetch"           // 调用宿主 web_fetch 工具
```

## 4. 实施路线图

### Phase 4.4: Wasm 沙箱原型（作为 Phase 4 的补充任务）

#### 任务 4.4.1: 技术选型与搭建 (1-2天)
- 选择 Wasm 运行时：`wasmtime`（推荐，CNCF 项目，活跃维护）
- 定义技能 ABI：使用 `wit-bindgen` 定义接口
- 创建基础项目结构：`src-tauri/src/skills/`

#### 任务 4.4.2: 基础沙箱实现 (3-5天)
- 实现 `WasmSandbox` 结构体，支持加载/运行 Wasm 模块
- 实现燃料计量（CPU限制）和内存限制
- 实现基础宿主函数：日志、时间、随机数

#### 任务 4.4.3: 权限系统实现 (2-3天)
- 实现 `PermissionChecker`，支持声明式权限
- 实现权限到资源的映射检查
- 添加用户审批流程（通过网关事件）

#### 任务 4.4.4: 资源限制器实现 (2-3天)
- 实现 `ResourceTracker`，追踪 CPU/内存/时间
- 添加系统调用拦截和计数
- 实现超时和资源耗尽处理

#### 任务 4.4.5: 通信机制实现 (2-3天)
- 实现技能调用宿主工具的通道
- 实现序列化（`serde` + `bincode`）
- 添加异步支持（`tokio`）

#### 任务 4.4.6: 示例技能开发 (1-2天)
- 创建示例技能：`example-skill/`
- Rust 技能：演示文件处理和网络请求
- 其他语言技能（如 Go 编译到 Wasm）：演示跨语言兼容性

#### 任务 4.4.7: 集成测试 (2-3天)
- 恶意技能测试：尝试越权访问
- 资源耗尽测试：内存泄漏、无限循环
- 性能测试：启动时间、吞吐量
- 恢复测试：沙箱崩溃不影响宿主

## 5. 安全增强建议

### 5.1 深度防御
- **代码审查**：重要技能需人工审核源码
- **签名验证**：技能模块需数字签名（如 minisign）
- **来源验证**：只允许从可信仓库（如 GitHub 特定组织）加载

### 5.2 运行时保护
- **系统调用过滤**：基于 seccomp 进一步限制（Linux）
- **内存加密**：敏感数据在 Wasm 内存中加密
- **控制流完整性**：Wasm 模块的 CFI 验证

### 5.3 监控与审计
- **行为分析**：记录技能的所有操作，检测异常模式
- **性能基线**：建立正常性能基线，检测资源滥用
- **审计日志**：所有权限检查、资源使用都记录结构化日志

### 5.4 更新与撤销
- **热更新**：技能可在线更新，无需重启主程序
- **紧急终止**：管理员可立即终止任何技能
- **黑名单**：已知恶意技能的哈希黑名单

## 6. 与其他 Phase 的协同

- **Phase 2 (网关协议)**：沙箱的权限审批需要网关的 `ConfirmationRequired` 事件
- **Phase 3 (执行器)**：`ToolManager` 需要从沙箱中注册技能提供的工具
- **Phase 4 (安全工具集)**：沙箱作为技能加载的安全基础
- **Phase 5 (子代理)**：每个子代理可运行在独立沙箱中，实现多层隔离
- **Phase 1.7 (监控)**：沙箱资源使用数据需要导出到监控系统

## 7. 原型验证指标

原型完成后，应验证以下指标：

| 指标 | 目标值 | 测试方法 |
|------|--------|----------|
| 技能启动时间 | < 10ms | 测量 1000 次冷启动平均时间 |
| 内存开销（空技能）| < 5MB | 测量 Wasm 运行时内存占用 |
| 性能损失（vs 原生）| < 15% | 运行相同计算的性能对比 |
| 隔离有效性 | 100% 阻止越权访问 | 恶意技能测试套件 |
| 资源限制准确度 | 误差 < 5% | 对比请求资源与实际使用 |
| 崩溃隔离 | 技能崩溃不影响宿主 | 强制段错误、panic 测试 |
| 权限检查开销 | < 100μs | 测量权限检查的延迟 |
| 并发支持 | 支持 50+ 并发技能 | 压力测试并发加载和执行 |

## 8. 依赖与约束

### 8.1 技术依赖
- **wasmtime** >= 0.40.0：Wasm 运行时
- **wit-bindgen**：Wasm 接口类型定义
- **serde** + **bincode**：跨边界数据序列化
- **tokio**：异步运行时支持
- **ring** / **rustls**：数字签名验证

### 8.2 平台约束
- **Linux**：支持 seccomp 增强隔离
- **macOS**：支持基本的 Wasm 沙箱
- **Windows**：支持基本的 Wasm 沙箱（可能需要额外配置）

### 8.3 性能约束
- 单技能内存占用不超过配置的 150%
- 技能启动延迟不超过 50ms（冷启动）
- 权限检查延迟不超过 500μs

## 9. 风险与缓解措施

| 风险 | 可能性 | 影响 | 缓解措施 |
|------|--------|------|----------|
| Wasm 运行时漏洞 | 低 | 高 | 使用最新稳定版 wasmtime；启用所有安全特性 |
| 权限模型绕过 | 中 | 高 | 严格的权限检查；最小权限原则；定期安全审计 |
| 资源耗尽攻击 | 中 | 中 | 严格的资源限制；监控和告警；自动终止超限技能 |
| 技能依赖冲突 | 高 | 低 | 独立依赖环境；版本隔离；依赖锁定 |
| 性能下降 | 中 | 中 | 性能基准测试；优化序列化；缓存编译结果 |
| 跨平台兼容性问题 | 中 | 中 | 全面的跨平台测试；条件编译；平台特定优化 |

## 10. 后续演进方向

### 10.1 短期（6个月内）
- 支持更多语言编译到 Wasm（Python、JavaScript、Go）
- 增加技能市场和管理界面
- 实现技能热重载和版本管理

### 10.2 中期（1年内）
- 引入更细粒度的权限模型（基于能力的安全性）
- 支持技能间安全通信
- 实现分布式技能执行（边缘计算）

### 10.3 长期（1年以上）
- 基于硬件隔离的增强安全（Intel SGX、AMD SEV）
- AI 驱动的技能行为分析和异常检测
- 去中心化技能注册和验证

## 总结

Wasm 沙箱隔离原型是 ChatSpeed 技能系统的**安全基石**，为第三方技能的**可信执行**提供了技术保障。采用 WebAssembly 方案能在安全、性能和实现复杂度间取得良好平衡。

实施时建议采用 **渐进式策略**：先实现基础隔离，再逐步增强安全特性。通过精细的权限控制、严格的资源限制和完整的隔离边界，建立用户对第三方技能的信任，为 ChatSpeed 的生态扩展奠定安全基础。

**关键成功因素**：
1. 完善的权限模型和最小权限原则
2. 精确的资源控制和监控
3. 全面的安全测试和审计
4. 良好的开发者体验和工具链支持
5. 与现有系统的平滑集成