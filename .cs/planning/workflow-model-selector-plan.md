# Workflow 模型切换功能实施计划

## 需求概述
在 Workflow.vue 聊天框底部添加模型切换功能，支持：
- **持久化配置**：保存到 workflow 表，session 级别覆盖 agent 设置
- **完整模型角色**：支持所有 6 个模型角色（plan, act, vision, coding, copywriting, browsing）
- **独立配置面板**：Tab 切换 + 竖排二级联动
- **智能切换**：根据 planningMode 自动切换当前显示的 Tab（plan 模式显示 plan tab，否则显示 act tab）
- **斜杠命令支持**：为将来的 `/models` 命令预留接口

## 核心概念：当前活动模型

### 定义
**当前活动模型** = 根据 `planningMode` 状态动态决定的模型角色
- `planningMode = true` → 当前活动模型 = **plan 模型**
- `planningMode = false` → 当前活动模型 = **act 模型**

### 行为逻辑
1. **触发按钮显示**：始终显示当前活动模型的名称和图标
2. **配置面板默认 Tab**：打开面板时，自动选中当前活动模型对应的 Tab
3. **用户可手动切换**：用户可以在面板中手动切换到其他角色进行配置

## 技术方案

### 1. 数据结构设计

#### 1.1 Workflow 表扩展
在 workflow 表中添加 `models` 字段（TEXT/JSON），存储格式：
```json
{
  "plan": { 
    "id": 1,           // provider ID (0 表示 proxy 模式)
    "model": "gpt-4",  // model ID 或 "proxy_group@alias"
    "temperature": 0.7,
    "contextSize": 128000,
    "maxTokens": 4096
  },
  "act": { ... },
  "vision": { ... },
  "coding": { ... },
  "copywriting": { ... },
  "browsing": { ... }
}
```

#### 1.2 前端数据流
```
Agent 默认配置 → Workflow 覆盖配置 → 运行时使用
```

### 2. 组件设计

#### 2.1 新建组件：`ModelConfigPanel.vue`

**功能**：
- 紧凑的触发按钮（显示当前活动模型名称和图标）
- 点击后弹出独立配置面板
- Tab 切换 6 个模型角色
- 竖排二级联动选择器（上方 provider 列表，下方 model 列表）
- 支持 provider 模式和 proxy 模式
- 参数调整（temperature, contextSize, maxTokens）

**Props**：
```typescript
{
  modelValue: Object,  // 当前 workflow 的 models 配置
  planningMode: Boolean, // 是否处于 plan 模式
  disabled: Boolean    // 是否禁用
}
```

**Emits**：
```typescript
{
  'update:modelValue': Object, // 更新 models 配置
  'open': void,                // 面板打开事件（为斜杠命令预留）
  'close': void                // 面板关闭事件
}
```

**UI 布局**：
```
┌─────────────────────────────────────────┐
│  [触发按钮] GPT-4 (act)  ▼              │
└─────────────────────────────────────────┘
         ↓ 点击后弹出
┌────────────────────────────────────────────────────────┐
│  模型配置                                  [关闭]     │
├────────────────────────────────────────────────────────┤
│  [plan] [act] [vision] [coding] [copy] [browsing]     │ ← Tab 切换
├────────────────────────────────────────────────────────┤
│  ┌──────────────────────────────────────────────────┐ │
│  │ Provider                                         │ │
│  │ ┌────────────────────────────────────────────┐   │ │
│  │ │ ○ OpenAI                                   │   │ │
│  │ │ ● Anthropic  ← 选中                        │   │ │
│  │ │ ○ Google                                   │   │ │
│  │ │ ○ Azure                                    │   │ │
│  │ └────────────────────────────────────────────┘   │ │
│  │                                                  │ │
│  │ Model                                            │ │
│  │ ┌────────────────────────────────────────────┐   │ │
│  │ │ ○ claude-3-opus                            │   │ │
│  │ │ ● claude-3-sonnet  ← 选中                  │   │ │
│  │ │ ○ claude-3-haiku                           │   │ │
│  │ └────────────────────────────────────────────┘   │ │
│  └──────────────────────────────────────────────────┘ │
│                                                        │
│  模式切换                                              │
│  ┌──────────────────────────────────────────────────┐ │
│  │ ◉ Provider    ○ Proxy                           │ │
│  └──────────────────────────────────────────────────┘ │
│                                                        │
│  (如果是 Proxy 模式，显示代理配置)                    │
│  ┌──────────────────────────────────────────────────┐ │
│  │ 代理组: [默认组 ▼]  别名: [gpt-4 ▼]             │ │
│  └──────────────────────────────────────────────────┘ │
│                                                        │
│  参数配置                                              │
│  ┌──────────────────────────────────────────────────┐ │
│  │ Temperature: [========●=====] 0.7               │ │
│  │ Context: [128000]    Max Tokens: [4096]        │ │
│  └──────────────────────────────────────────────────┘ │
│                                                        │
│                              [取消] [保存]             │
└────────────────────────────────────────────────────────┘
```

**布局特点**：
1. **Tab 导航**：顶部 Tab 切换 6 个模型角色
2. **竖排二级联动**：
   - Provider 列表和 Model 列表垂直排列
   - 每个 list 都有清晰的标题和边框
   - 选中的项目有明显的视觉反馈
3. **模式切换**：
   - Provider/Proxy 模式使用 Radio Button 水平排列
   - Proxy 配置区域条件显示
4. **参数区域**：紧凑排列，节省空间

#### 2.2 修改组件：`Workflow.vue`
**集成位置**：
在 `input-footer` 的 `footer-left` 区域，AgentSelector 旁边添加：

```vue
<div class="footer-left">
  <AgentSelector ... />
  
  <!-- 新增：模型配置按钮 -->
  <ModelConfigPanel
    ref="modelConfigPanelRef"
    v-model="workflowModels"
    :planning-mode="planningMode"
    :disabled="!currentWorkflowId"
    @open="onModelConfigOpen"
    @close="onModelConfigClose"
  />
  
  <!-- 其他按钮 ... -->
</div>
```

### 3. Store 扩展

#### 3.1 Workflow Store 扩展
添加以下方法：

```javascript
// 加载 workflow 时解析 models 字段
const loadWorkflowModels = (workflow) => {
  if (workflow.models && typeof workflow.models === 'string') {
    try {
      return JSON.parse(workflow.models)
    } catch (e) {
      console.error('Failed to parse models:', e)
      return null
    }
  }
  return workflow.models || null
}

// 更新 workflow 的 models 配置
const updateWorkflowModels = async (workflowId, models) => {
  try {
    await invokeWrapper('update_workflow_models', {
      sessionId: workflowId,
      models: JSON.stringify(models)
    })
    // 更新本地状态
    const workflowIndex = workflows.value.findIndex(w => w.id === workflowId)
    if (workflowIndex !== -1) {
      workflows.value[workflowIndex].models = models
    }
  } catch (err) {
    await _handleError(err)
  }
}
```

#### 3.2 Agent Store 扩展
添加获取默认模型配置的方法：

```javascript
// 获取 agent 的默认模型配置
const getAgentModels = (agentId) => {
  const agent = agents.value.find(a => a.id === agentId)
  if (!agent) return null
  
  if (agent.models && typeof agent.models === 'string') {
    try {
      return JSON.parse(agent.models)
    } catch (e) {
      console.error('Failed to parse agent models:', e)
      return null
    }
  }
  return agent.models || null
}
```

### 4. 后端扩展

#### 4.1 数据库迁移
在 workflow 表添加 `models` 字段：

```sql
ALTER TABLE workflows ADD COLUMN models TEXT;
```

#### 4.2 Rust 命令扩展
添加更新 models 的命令：

```rust
#[tauri::command]
pub async fn update_workflow_models(
    sessionId: String,
    models: String,
    // ... 其他参数
) -> Result<(), String> {
    // 更新数据库
    // 通知运行时引擎更新模型配置
}
```

### 5. 智能切换逻辑

#### 5.1 当前活动模型计算
```javascript
// 当前活动角色
const activeModelRole = computed(() => {
  return planningMode.value ? 'plan' : 'act'
})

// 当前活动模型的配置
const activeModelConfig = computed(() => {
  // 1. 优先使用 workflow 级别的配置
  if (workflowModels.value && workflowModels.value[activeModelRole.value]) {
    return workflowModels.value[activeModelRole.value]
  }
  
  // 2. 回退到 agent 的配置
  if (selectedAgent.value && selectedAgent.value.models) {
    const agentModels = typeof selectedAgent.value.models === 'string' 
      ? JSON.parse(selectedAgent.value.models) 
      : selectedAgent.value.models
    return agentModels[activeModelRole.value]
  }
  
  // 3. 回退到全局默认
  return modelStore.defaultModelProvider
})
```

#### 5.2 触发按钮显示
```vue
<template>
  <div class="model-config-trigger" @click="togglePanel">
    <img :src="activeProviderLogo" class="provider-logo" />
    <span class="model-name">{{ activeModelName }}</span>
    <span class="role-badge">{{ activeModelRole }}</span>
    <cs name="caret-down" />
  </div>
</template>
```

#### 5.3 Tab 默认选中
```javascript
// 监听 planningMode 变化，自动切换 Tab
watch(
  () => props.planningMode,
  (newVal) => {
    activeTab.value = newVal ? 'plan' : 'act'
  },
  { immediate: true }
)

// 打开面板时，选中当前活动角色
const openPanel = () => {
  activeTab.value = props.planningMode ? 'plan' : 'act'
  panelVisible.value = true
}
```

### 6. 斜杠命令支持

#### 6.1 预留接口
```javascript
// 在 Workflow.vue 中添加
const modelConfigPanelRef = ref(null)

const openModelConfigPanel = () => {
  modelConfigPanelRef.value?.openPanel()
}

// 监听斜杠命令
watch(inputMessage, (newVal) => {
  if (newVal === '/models') {
    openModelConfigPanel()
    // 清空输入或保留命令供后续处理
  }
})
```

#### 6.2 全局事件
```javascript
// 通过 Tauri 事件系统支持跨窗口调用
listen('cs://open-model-config', () => {
  openModelConfigPanel()
})
```

## 实施步骤

### Phase 1: 基础架构（优先级：高）
1. ✅ 研究现有代码结构
2. ✅ 设计 UI 布局方案
3. 创建 `ModelConfigPanel.vue` 组件框架
4. 实现 workflow store 的 models 相关方法
5. 实现后端数据库迁移和 API

### Phase 2: UI 实现（优先级：高）
1. 实现 Tab 切换 UI
2. 实现竖排二级联动选择器 UI
3. 实现 provider/proxy 模式切换
4. 实现参数调整界面
5. 集成到 Workflow.vue

### Phase 3: 逻辑实现（优先级：高）
1. 实现智能切换逻辑（根据 planningMode 自动切换 Tab）
2. 实现配置加载和保存
3. 实现配置覆盖逻辑（workflow > agent > global）
4. 测试配置持久化

### Phase 4: 优化和扩展（优先级：中）
1. 优化 UI/UX（动画、提示、错误处理）
2. 添加斜杠命令支持
3. 添加全局事件支持
4. 编写单元测试

## 文件清单

### 新建文件
- `src/components/workflow/ModelConfigPanel.vue` - 模型配置面板组件

### 修改文件
- `src/views/Workflow.vue` - 集成模型配置面板
- `src/stores/workflow.js` - 添加 models 管理方法
- `src/stores/agent.js` - 添加获取 agent models 的辅助方法
- `src-tauri/src/workflow/mod.rs` - 添加 update_workflow_models 命令
- `src-tauri/src/database/schema.rs` - 添加 models 字段迁移

### 国际化文件
- `src/i18n/locales/zh-CN.json` - 添加中文翻译
- `src/i18n/locales/en-US.json` - 添加英文翻译

## 验收标准

1. ✅ 可以在 workflow 界面打开模型配置面板
2. ✅ Tab 切换 6 个模型角色，默认根据 planningMode 自动选中
3. ✅ 竖排二级联动选择 provider 和 model
4. ✅ 配置保存到 workflow 表，刷新页面后仍然存在
5. ✅ workflow 配置优先级高于 agent 配置
6. ✅ 触发按钮显示当前活动模型（根据 planningMode 动态变化）
7. ✅ 支持 provider 和 proxy 两种模式
8. ✅ 可以调整 temperature、contextSize、maxTokens 参数
9. ✅ UI 美观，交互流畅，布局合理
10. ✅ 为斜杠命令预留了接口

## 风险和注意事项

1. **数据迁移**：需要确保现有 workflow 数据兼容性
2. **性能**：模型列表可能很长，需要虚拟滚动优化
3. **错误处理**：需要处理 provider 不存在、model 不存在等异常情况
4. **国际化**：所有新增文本需要支持多语言
5. **测试**：需要测试各种边界情况（空配置、配置损坏等）
6. **布局优化**：竖排二级联动需要精心设计，确保在小屏幕上也能良好显示

## 时间估算

- Phase 1: 2-3 小时
- Phase 2: 4-5 小时
- Phase 3: 2-3 小时
- Phase 4: 2-3 小时

**总计**: 10-14 小时
