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

      <h3>Key Features</h3>
      <ul>
        <li>
          Multi-Model Support
          <ul>
            <li>Compatible with OpenAI, Gemini, Ollama, and Claude</li>
            <li>Comprehensive model parameter configuration</li>
            <li>Proxy settings support</li>
          </ul>
        </li>
        <li>
          Advanced Chat Interface
          <ul>
            <li>ChatGPT-style layout with dark/light themes</li>
            <li>Multi-language support</li>
            <li>Message reference and resend capabilities</li>
            <li>Rich text parsing (mindmap, mermaid, code blocks)</li>
          </ul>
        </li>
        <li>
          Skill Management
          <ul>
            <li>Custom skill creation</li>
            <li>Quick skill access</li>
            <li>Icon and image support</li>
          </ul>
        </li>
        <li>
          Data Management
          <ul>
            <li>Local data storage</li>
            <li>Chat history management</li>
            <li>Database backup and restore</li>
          </ul>
        </li>
      </ul>

      <h3>Open Source</h3>
      <p>
        ChatSpeed is proudly open source, built with Vue 3 and Tauri. We believe in transparency and
        community-driven development. All our code is available for review, contribution, and
        improvement at <a href="#" @click="openGithub">GitHub</a>.
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
            <li>兼容 OpenAI、Gemini、Ollama 和 Claude</li>
            <li>完整的模型参数配置</li>
            <li>代理设置支持</li>
          </ul>
        </li>
        <li>
          高级聊天界面
          <ul>
            <li>类 ChatGPT 的界面布局，支持明暗主题</li>
            <li>多语言支持</li>
            <li>消息引用和重发功能</li>
            <li>富文本解析（思维导图、流程图、代码块）</li>
          </ul>
        </li>
        <li>
          技能管理
          <ul>
            <li>自定义技能创建</li>
            <li>技能快捷调用</li>
            <li>图标和图片支持</li>
          </ul>
        </li>
        <li>
          数据管理
          <ul>
            <li>本地数据存储</li>
            <li>聊天历史管理</li>
            <li>数据库备份和还原</li>
          </ul>
        </li>
      </ul>

      <h3>开源项目</h3>
      <p>
        ChatSpeed 是一个基于 Vue 3 和 Tauri
        构建的开源项目。我们坚信透明和社区驱动的开发理念。所有代码都开放在
        <a href="#" @click="openGithub">GitHub</a>
        上供审查、贡献和改进。
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
