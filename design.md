# 富文本聊天支持 - 设计方案

## 1. 数据结构设计

### 1.1 Settings Store 扩展

在 `src/stores/setting.js` 的 `defaultSettings` 中添加：

```javascript
visionModel: {
  id: '',
  model: ''
}
```

### 1.2 消息数据结构扩展

普通消息:
```javascript
{
  role: 'user',
  content: 'Hello, how are you?'
}
```

带图片的消息:
```javascript
{
  role: 'user',
  content: [
    { type: 'text', text: 'What do you see in this image?' },
    { type: 'image_url', image_url: { url: 'http://127.0.0.1:21914/tmp/image.jpg' } }
  ]
}
```

带文本文件的消息:
```javascript
{
  role: 'user',
  content: 'Please analyze this file content:\n\n' + fileContent
}
```

### 1.3 附件数据结构

```javascript
{
  id: string,           // 唯一ID
  type: 'image' | 'text', // 附件类型
  name: string,         // 文件名
  url: string,          // 图片URL（仅图片）
  content: string,      // 文本内容（仅文本文件）
  size: number          // 文件大小
}
```

## 2. UI 组件设计

### 2.1 输入框附件区域

在 `Assistant.vue` 的输入框下方添加附件区域：

```vue
<div class="attachments-area" v-if="attachments.length > 0">
  <div v-for="attachment in attachments" :key="attachment.id" class="attachment-item">
    <img v-if="attachment.type === 'image'" :src="attachment.url" />
    <cs v-else name="file" />
    <span>{{ attachment.name }}</span>
    <cs name="close" @click="removeAttachment(attachment.id)" />
  </div>
</div>
```

### 2.2 附件上传按钮

在输入框图标区域添加附件按钮：

```vue
<el-tooltip content="Add attachment" placement="top">
  <cs name="attachment" @click="onAddAttachment" />
</el-tooltip>
```

### 2.3 文件选择对话框

使用 Element Plus 的文件选择：
```vue
<el-dialog v-model="fileDialogVisible" title="Select File">
  <el-upload
    drag
    :auto-upload="false"
    :on-change="onFileSelect"
    :accept="acceptTypes"
  >
    <cs name="upload" size="48px" />
    <div>Drop file here or click to select</div>
  </el-upload>
</el-dialog>
```

## 3. 功能流程设计

### 3.1 图片粘贴流程

1. 用户在输入框中粘贴图片 (Ctrl+V / Cmd+V)
2. 捕获 `paste` 事件
3. 检查 clipboard data 中的图片
4. 将图片转换为 base64 data URI 格式
5. 添加到 attachments 数组
6. 显示在附件区域

### 3.2 图片上传流程

1. 点击附件按钮
2. 打开文件选择对话框
3. 用户选择图片文件
4. 将图片转换为 base64 data URI 格式
5. 添加到 attachments 数组
6. 显示在附件区域

### 3.3 文本文件读取流程

1. 点击附件按钮
2. 打开文件选择对话框
3. 用户选择文本文件
4. 调用 Tauri 命令读取文件内容
5. 添加到 attachments 数组
6. 显示在附件区域

### 3.4 消息发送流程

1. 用户点击发送按钮
2. 检查是否有附件
3. 如果有图片附件且配置了视觉模型:
   - 先调用视觉模型识别图片
   - 将识别结果与用户问题合并
4. 构建消息对象（支持多模态格式）
5. 发送消息到AI
6. 清空附件列表

### 3.5 图片识别流程

1. 构建视觉模型请求:
```javascript
{
  messages: [
    {
      role: 'user',
      content: [
        { type: 'text', text: 'Please describe this image in detail.' },
        { type: 'image_url', image_url: { url: imageUrl } }
      ]
    }
  ]
}
```

2. 调用视觉模型API
3. 获取识别结果
4. 将识别结果追加到用户问题前:
```javascript
const enhancedPrompt = `[Image Analysis]: ${visionResponse}\n\n[User Question]: ${userMessage}`
```

## 4. Tauri 命令设计

### 4.1 新增命令

#### save_image_to_temp
保存图片到临时目录
```rust
#[tauri::command]
pub async fn save_image_to_temp(image_data: &[u8], filename: &str) -> Result<String>
```

#### read_text_file
读取文本文件内容
```rust
#[tauri::command]
pub async fn read_text_file(file_path: &str) -> Result<String>
```

#### get_file_type
获取文件类型
```rust
#[tauri::command]
pub async fn get_file_type(file_path: &str) -> Result<String>
```

## 5. 国际化

### 5.1 新增翻译键

```json
{
  "chat": {
    "addAttachment": "Add Attachment",
    "removeAttachment": "Remove Attachment",
    "pasteImage": "Paste Image",
    "selectFile": "Select File",
    "imageAnalysis": "Image Analysis",
    "fileContent": "File Content",
    "supportedImageFormats": "Supported formats: JPG, PNG, GIF, WEBP, SVG, BMP",
    "supportedTextFormats": "Supported formats: TXT, MD, JSON, XML, CSV, LOG, PHP, GO, RS, JS, PY, TS, CSS, HTML, HTM",
    "visionModel": "Vision Model",
    "visionModelPlaceholder": "Select a vision model for image analysis"
  }
}
```

## 6. 实现优先级

### 高优先级
1. 视觉模型配置
2. 图片粘贴功能
3. 图片上传功能
4. 图片识别流程

### 中优先级
5. 文本文件读取
6. 附件UI优化
7. 消息格式扩展

### 低优先级
8. 文件预览
9. 批量文件上传
10. 附件历史记录

## 7. 技术栈

- 前端: Vue 3, Element Plus, Tauri API
- 后端: Rust, Tauri Commands
- 图片处理: image crate (已有)
- 文件读取: std::fs
- 消息格式: OpenAI Chat Completion API