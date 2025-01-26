<template>
  <el-dialog
    v-model="visible"
    :title="t('update.newVersion') + (versionInfo?.version || '')"
    width="400px"
    :close-on-click-modal="false"
    :show-close="false">
    <div class="update-content">
      <div class="release-notes" v-if="versionInfo?.notes">
        <h4>{{ t('update.releaseNotes') }}</h4>
        <el-scrollbar height="200px">
          <div class="markdown-body" v-html="markdownToHtml(versionInfo.notes)" />
        </el-scrollbar>
      </div>
      <div class="github-release" v-if="versionInfo?.version">
        <a :href="getReleaseUrl(versionInfo.version)" target="_blank" class="release-link">
          {{ t('update.viewOnGitHub') }} <cs name="link" />
        </a>
      </div>
      <div class="skip-version">
        <el-checkbox v-model="skipVersion">{{ t('update.skipVersion') }}</el-checkbox>
      </div>
    </div>
    <template #footer>
      <span class="dialog-footer">
        <el-button @click="handleCancel">{{ t('update.cancel') }}</el-button>
        <el-button type="primary" @click="handleConfirm">
          {{ t('update.confirm') }}
        </el-button>
      </span>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, defineEmits, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { marked } from 'marked'
import DOMPurify from 'dompurify'

const props = defineProps({
  versionInfo: {
    type: Object,
    default: null,
  },
  modelValue: {
    type: Boolean,
    default: false,
  },
})

const emit = defineEmits(['update:modelValue', 'confirm', 'cancel'])
const { t } = useI18n()
const skipVersion = ref(false)

const visible = computed({
  get: () => props.modelValue,
  set: value => emit('update:modelValue', value),
})

const markdownToHtml = markdown => {
  const html = marked(markdown)
  return DOMPurify.sanitize(html)
}

const getReleaseUrl = version => {
  return `https://github.com/aidyou/chatspeed/releases/tag/v${version}`
}

const handleConfirm = () => {
  emit('confirm')
}

const handleCancel = () => {
  emit('cancel', { skip: skipVersion.value })
}
</script>

<style lang="scss" scoped>
.update-content {
  padding: var(--cs-space-xs);

  .release-notes {
    margin-bottom: var(--cs-space-md);

    h4 {
      margin: 0 0 var(--cs-space-sm);
      font-size: var(--cs-font-size);
    }

    .markdown-body {
      background: var(--el-fill-color-light);
      border-radius: 4px;
      padding: 12px;
      font-size: 14px;
      line-height: 1.6;

      :deep(h1, h2, h3) {
        font-size: 16px;
        margin: 16px 0 8px;
        &:first-child {
          margin-top: 0;
        }
      }

      :deep(ul, ol) {
        padding-left: 24px;
        margin: 8px 0;
      }

      :deep(p) {
        margin: 8px 0;
      }

      :deep(code) {
        background: var(--el-fill-color);
        padding: 2px 4px;
        border-radius: 3px;
        font-family: monospace;
      }
    }
  }

  .github-release {
    text-align: right;
    margin-top: 16px;

    .release-link {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      color: var(--el-color-primary);
      text-decoration: none;
      font-size: 14px;

      &:hover {
        text-decoration: underline;
      }

      .el-icon {
        font-size: 16px;
      }
    }
  }

  .skip-version {
    margin-top: 16px;
  }
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: 12px;
}
</style>
