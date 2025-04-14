<template>
  <div class="privacy">
    <div class="content">
      <h2>Privacy Policy</h2>
      <p>
        At ChatSpeed, we are committed to protecting your privacy and ensuring the security of your
        data. This privacy policy explains how we handle your information.
      </p>

      <h3>Data Collection and Storage</h3>
      <ul>
        <li>ChatSpeed does not collect or store any user data on our servers.</li>
        <li>
          All your data, including model configurations, skill settings, and chat history, is stored
          locally on your device.
        </li>
        <li>We have no access to your personal information or conversation content.</li>
      </ul>

      <h3>Data Transmission</h3>
      <ul>
        <li>
          When you interact with AI models, your conversations are sent directly to the respective
          AI model providers (e.g., OpenAI, Google, Anthropic).
        </li>
        <li>
          The handling of your data by these providers is subject to their respective privacy
          policies.
        </li>
        <li>We do not intercept, store, or process any of these communications.</li>
      </ul>

      <h3>Local Storage</h3>
      <ul>
        <li>
          All application data is stored locally on your device, including:
          <ul>
            <li>AI model configurations and API keys</li>
            <li>Custom skill settings and prompts</li>
            <li>Conversation history and saved chats</li>
            <li>Application preferences and settings</li>
          </ul>
        </li>
        <li>You have full control over your data and can delete it at any time.</li>
      </ul>

      <h3>Security</h3>
      <ul>
        <li>
          Your API keys and configurations are stored in the application data directory. As an
          open-source application, we maintain transparency about our data storage methods.
        </li>
        <li>
          All data remains on your local device and is subject to your device's security measures.
        </li>
      </ul>
    </div>
    <el-divider />
    <div class="content">
      <h2>隐私政策</h2>
      <p>
        在
        ChatSpeed，我们致力于保护您的隐私并确保您的数据安全。本隐私政策说明了我们如何处理您的信息。
      </p>

      <h3>数据收集与存储</h3>
      <ul>
        <li>ChatSpeed 不在我们的服务器上收集或存储任何用户数据。</li>
        <li>您的所有数据，包括模型配置、技能设置和聊天历史记录，都存储在您的本地设备上。</li>
        <li>我们无法访问您的个人信息或对话内容。</li>
      </ul>

      <h3>数据传输</h3>
      <ul>
        <li>
          当您与 AI 模型交互时，您的对话内容会直接发送给相应的 AI 模型提供商（如
          OpenAI、Google、Anthropic 等）。
        </li>
        <li>这些提供商对您数据的处理受其各自隐私政策的约束。</li>
        <li>我们不会拦截、存储或处理任何这些通信内容。</li>
      </ul>

      <h3>本地存储</h3>
      <ul>
        <li>
          所有应用程序数据都存储在您的设备上，包括：
          <ul>
            <li>AI 模型配置和 API 密钥</li>
            <li>自定义技能设置和提示词</li>
            <li>对话历史和已保存的聊天记录</li>
            <li>应用程序偏好设置</li>
          </ul>
        </li>
        <li>您对自己的数据拥有完全的控制权，可以随时删除。</li>
      </ul>

      <h3>安全性</h3>
      <ul>
        <li>
          您的 API
          密钥和配置信息存储在应用程序数据目录下。作为开源应用，我们对数据存储方式保持透明。
        </li>
        <li>所有数据都保留在您的本地设备上，受您设备的安全措施保护。</li>
      </ul>
    </div>
  </div>
</template>

<style lang="scss" scoped>
.privacy {
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
}
</style>
