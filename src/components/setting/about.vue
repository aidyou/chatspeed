<template>
  <div class="about">
    <div class="content">
      <h2>About ChatSpeed</h2>
      <div class="version">Version: {{ version }}</div>
      <p>
        ChatSpeed is an innovative open-source AI chat application designed to enhance your
        interaction with various AI models. Our mission is to provide a fast, efficient, and
        user-friendly interface for AI conversations while ensuring complete data privacy and local
        storage.
      </p>

      <h3>Core Features</h3>
      <ul>
        <li>
          Multi-Model Support
          <ul>
            <li>
              Compatible with OpenAI, Gemini, Ollama and Claude via OpenAI-compatible protocols
            </li>
            <li>Multiple API keys per model with automatic rotation (A/B switching)</li>
            <li>Streaming responses with real-time character output</li>
            <li>Chain-of-thought visualization (Deepseek) and reasoning output (OpenAI)</li>
            <li>Complete parameter configuration (temperature, top_p, top_k, max_tokens, etc.)</li>
            <li>Customizable HTTP proxy support</li>
          </ul>
        </li>
        <li>
          Web Search
          <ul>
            <li>Integrated search engines: Google, Bing, Baidu</li>
            <li>Automatic webpage context parsing via Chatspeed crawler</li>
            <li>Real-time web retrieval expanding AI knowledge</li>
            <li>Deep search with multi-query task decomposition</li>
          </ul>
        </li>
        <li>
          Chat Interface
          <ul>
            <li>Clean UI with light/dark themes</li>
            <li>Multi-language interface support</li>
            <li>Message referencing and resend capabilities</li>
            <li>Rich content parsing: code blocks, mind maps, flowcharts, tables, formulas</li>
          </ul>
        </li>
        <li>
          AI Assistant
          <ul>
            <li>Instant Q&A responses</li>
            <li>Auto-detect input language for translation</li>
            <li>Enables Deepseek R1-like reasoning through prompt-enhanced general models</li>
            <li>AI-generated mind maps and flowcharts</li>
            <li>Custom skill creation beyond pre-built options</li>
          </ul>
        </li>
        <li>
          Skill Management
          <ul>
            <li>Visual skill builder</li>
            <li>Quick-access skills with hotkeys</li>
            <li>Icon library with custom uploads</li>
          </ul>
        </li>
        <li>
          Smart Notes
          <ul>
            <li>Save notable AI responses to knowledge base</li>
            <li>Tag-based organization and search</li>
            <li>Multi-format display: reasoning steps, code, diagrams, tables</li>
          </ul>
        </li>
        <li>
          Data Security
          <ul>
            <li>Local encrypted storage</li>
            <li>Chat history archiving</li>
            <li>Database backup/restore</li>
          </ul>
        </li>
      </ul>

      <h3>Open Source</h3>
      <p>
        ChatSpeed is an open-source project built with Vue 3 and Tauri under MIT license. All code
        is available on <a href="#" @click="openGithub">GitHub</a>, welcoming developer
        contributions.
      </p>
    </div>
    <el-divider />
    <div class="content">
      <h2>关于 ChatSpeed</h2>
      <div class="version">版本：{{ version }}</div>
      <p>
        ChatSpeed 是一款创新的开源 AI 聊天应用，旨在提升您与各类 AI
        模型的交互体验。我们的使命是在确保完全的数据隐私和本地存储的同时，提供一个快速、高效且用户友好的
        AI 对话界面。
      </p>

      <h3>核心功能</h3>
      <ul>
        <li>
          多模型支持
          <ul>
            <li>
              兼容 OpenAI、Gemini、Ollama 和 Claude，通过 OpenAI 兼容协议可接入绝大多数主流大模型
            </li>
            <li>支持单个模型配置多个 API 密钥，对话时自动轮换使用（AB 模式交替调用）</li>
            <li>流式输出模式（逐字输出体验）</li>
            <li>支持 Deepseek 的思考链输出展示与 OpenAI 的推理输出</li>
            <li>完整的模型参数配置体系（温度、top_p、max_tokens 等）</li>
            <li>HTTP网络代理设置支持</li>
          </ul>
        </li>
        <li>
          联网搜索
          <ul>
            <li>支持搜索引擎：Google、Bing、百度</li>
            <li>
              通过部署 Chatspeed 爬虫组件，可自动解析对话中的网页链接内容作为上下文，增强 AI
              处理能力
            </li>
            <li>实时检索网络信息，拓展 AI 知识边界</li>
            <li>深度搜索功能：根据问题自动拆解任务并执行多轮搜索，最终汇总分析结果</li>
          </ul>
        </li>
        <li>
          高级聊天界面
          <ul>
            <li>简洁的界面布局，支持明/暗主题模式</li>
            <li>多语言界面支持</li>
            <li>消息引用与重新发送功能</li>
            <li>支持富文本解析，包括：代码块、思维导图、流程图、表格、公式等</li>
          </ul>
        </li>
        <li>
          智能助手
          <ul>
            <li>即时问答响应</li>
            <li>智能翻译：自动识别输入语言并转换为第一/第二语言</li>
            <li>通过技能通用模型能力，实现类 Deepseek R1 的推理能力</li>
            <li>AI 辅助生成思维导图与流程图</li>
            <li>除内置技能外，支持创建任意自定义技能</li>
          </ul>
        </li>
        <li>
          技能管理
          <ul>
            <li>可视化技能创建</li>
            <li>技能快捷唤出（支持快捷键绑定）</li>
            <li>图标素材支持（内置库+自定义上传）</li>
          </ul>
        </li>
        <li>
          智记功能
          <ul>
            <li>在 AI 对话过程中，可随时将精彩回答保存至智记知识库</li>
            <li>支持按标签分类归档，提供高效的知识管理能力</li>
            <li>富媒体展示：支持推理过程、代码块、思维导图、流程图、表格、公式等专业格式解析</li>
          </ul>
        </li>
        <li>
          数据管理
          <ul>
            <li>本地加密存储</li>
            <li>对话历史归档管理</li>
            <li>数据库备份与恢复</li>
          </ul>
        </li>
      </ul>

      <h3>开源项目</h3>
      <p>
        ChatSpeed 是基于 Vue 3 和 Tauri 构建的开源项目，遵循 MIT 协议。所有代码均已开源至
        <a href="#" @click="openGithub">GitHub 代码平台</a>
        ，欢迎开发者参与贡献。
      </p>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { openUrl as invokeOpenUrl } from '@tauri-apps/plugin-opener'

const version = ref('')

onMounted(() => {
  getVersion().then(v => {
    version.value = v
  })
})

const openGithub = () => {
  invokeOpenUrl('https://github.com/aidyou/chatspeed')
}
</script>

<style lang="scss" scoped>
.about {
  padding: var(--cs-space-lg);

  .content {
    max-width: 800px;
    margin: 0 auto;

    h2 {
      font-size: var(--cs-font-size-xxl);
      margin-bottom: var(--cs-space-lg);
      color: var(--el-text-color-primary);
    }

    h3 {
      font-size: var(--cs-font-size-xl);
      margin: var(--cs-space-lg) 0 var(--cs-space-md);
      color: var(--el-text-color-primary);
    }

    p {
      font-size: var(--cs-font-size);
      line-height: 1.6;
      margin-bottom: var(--cs-space);
      color: var(--el-text-color-regular);
    }

    .version {
      font-size: var(--cs-font-size);
      color: var(--el-text-color-secondary);
      margin: var(--cs-space-sm) 0 var(--cs-space-lg);
    }

    ul {
      padding-left: var(--cs-space-lg);
      margin-bottom: var(--cs-space);

      li {
        font-size: var(--cs-font-size);
        line-height: 1.6;
        margin-bottom: var(--cs-space-sm);
        color: var(--el-text-color-regular);

        ul {
          margin: var(--cs-space-sm) 0 var(--cs-space-sm) var(--cs-space);
        }
      }
    }
  }

  a {
    color: var(--el-color-primary);
    text-decoration: none;

    &:hover {
      text-decoration: underline;
    }
  }
}
</style>
