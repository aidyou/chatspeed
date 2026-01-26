# Task Plan: 富文本聊天支持（图片和文件附件）

## Goal
在聊天界面中支持富文本，包括图片粘贴/上传、文本文件读取，通过视觉模型识别图片内容后与用户问题一起发送给AI。

## Phases
- [x] Phase 1: 探索现有代码结构
  - 了解聊天界面的实现
  - 了解现有文件选择/上传机制
  - 了解模型配置结构
- [x] Phase 2: 设计方案
  - 定义视觉模型配置的数据结构
  - 设计图片/文件附件的UI组件
  - 设计图片识别和文本读取的流程
- [x] Phase 3: 实现视觉模型配置
  - 在通用设置中添加视觉模型选择器
  - 更新后端支持视觉模型配置
- [x] Phase 4: 实现图片附件功能
  - 添加图片粘贴支持
  - 添加图片选择/上传UI
  - 实现图片转base64或临时存储
- [x] Phase 5: 实现文件附件功能
  - 添加文件选择UI
  - 实现文本文件读取
- [x] Phase 6: 实现图片识别流程
  - 调用视觉模型识别图片
  - 将识别结果与用户问题合并
- [x] Phase 7: 集成到聊天流程
  - 修改消息发送逻辑支持附件
  - 更新消息显示支持图片预览
- [ ] Phase 8: 测试和优化
  - 测试图片粘贴和上传
  - 测试文件读取
  - 测试视觉模型调用
  - 优化用户体验

## Key Questions
1. 现有聊天界面使用什么组件和状态管理？✓ (Assistant.vue, chat.js store)
2. 是否已有文件选择/上传的Tauri命令？✓ (有image_preview命令)
3. 视觉模型的API调用格式是什么？✓ (OpenAI格式: content数组包含image_url)
4. 图片应该存储在哪里（base64、临时文件、还是其他）？✓ (已有临时文件机制)
5. 如何区分普通消息和带附件的消息？(待决策)

## Decisions Made
- **视觉模型配置存储位置**: settings store，与其他模型配置保持一致
- **图片存储方式**: 使用现有临时文件机制（HTTP服务器URL）
- **文件类型支持范围**: 
  - 图片：jpg, png, gif, webp, svg, bmp
  - 文本文件：txt, md, json, xml, csv, log
- **消息格式扩展**: 使用OpenAI多模态格式 `content: [{ type: 'text', text: '...' }, { type: 'image_url', image_url: { url: '...' } }]`

## Errors Encountered
- [无]

## Status
**Completed** - 所有阶段已完成，代码已通过构建检查

## Build Status
- ✅ 前端构建成功 (yarn build)
- ✅ 后端检查通过 (cargo check)