<template>
  <div class="file-tree">
    <div class="tree-header">
      <span class="title">{{ $t('settings.agent.authorizedPaths') }}</span>
      <div class="header-actions">
        <el-tooltip :content="$t('settings.agent.addPath')" placement="top">
          <cs name="add" class="action-icon" @click="onAddPath" />
        </el-tooltip>
        <el-tooltip :content="$t('common.refresh')" placement="top">
          <cs name="refresh" class="action-icon refresh-icon" @click="refreshAll" :class="{ rotating: loading }" />
        </el-tooltip>
      </div>
    </div>

    <div v-if="roots.length === 0" class="empty-tree">
      {{ $t('settings.agent.authorizedPathsTip') }}
    </div>

    <div v-else class="tree-content">
      <div v-for="root in roots" :key="root" class="root-container">
        <div class="root-item">
          <div class="root-info" @click="toggleExpand(root)">
            <cs :name="isExpanded(root) ? 'ext-folder-open' : 'ext-folder'" size="14px" />
            <span class="root-name" :title="root">{{ getDirName(root) }}</span>
          </div>
          <div class="root-actions">
            <cs name="copy" size="12px" class="action-btn copy-btn" @click.stop="copyRootPath(root)" />
            <cs name="ext-folder-open" size="12px" class="action-btn open-btn" @click.stop="openAuthorizedFolder(root)" />
            <cs name="refresh" size="12px" class="action-btn refresh-btn" @click.stop="refreshRoot(root)" />
            <cs name="trash" size="12px" class="action-btn remove-btn" @click.stop="onRemovePath(root)" />
          </div>
        </div>
        <div v-if="isExpanded(root)" class="children">
          <tree-node v-for="child in getChildren(root)" :key="child.path" :node="child" :expanded-map="expandedNodes"
            @toggle="toggleExpand" @preview="previewFile" />
        </div>
      </div>
    </div>

    <!-- File Preview Dialog -->
    <el-dialog v-model="previewVisible" :title="previewTitle" width="80%" top="5vh" class="file-preview-dialog"
      append-to-body destroy-on-close>
      <div class="preview-content">
        <div v-if="previewMode === 'unsupported'" class="unsupported-preview">
          <div class="unsupported-preview__icon">
            <cs name="warning" size="28px" />
          </div>
          <div class="unsupported-preview__title">This file type is not supported for inline preview.</div>
          <div class="unsupported-preview__desc">
            You can open it with your system default application.
          </div>
          <div class="unsupported-preview__path">{{ previewFilePath }}</div>
          <el-button type="primary" @click="openPreviewFileWithDefaultApp">Open with Default App</el-button>
        </div>
        <div v-else-if="previewMode === 'image'" class="media-preview image-preview">
          <div class="media-preview__actions">
            <el-button @click="openPreviewFileWithDefaultApp">Open with Default App</el-button>
          </div>
          <img :src="previewAssetUrl" :alt="previewTitle" class="image-preview__img" />
        </div>
        <div v-else-if="previewMode === 'audio'" class="media-preview audio-preview">
          <div class="media-preview__actions">
            <el-button @click="openPreviewFileWithDefaultApp">Open with Default App</el-button>
          </div>
          <audio :src="previewAssetUrl" controls preload="metadata" class="audio-preview__player" />
        </div>
        <div v-else-if="previewMode === 'video'" class="media-preview video-preview">
          <div class="media-preview__actions">
            <el-button @click="openPreviewFileWithDefaultApp">Open with Default App</el-button>
          </div>
          <video :src="previewAssetUrl" controls preload="metadata" class="video-preview__player" />
        </div>
        <file-preview-diff
          v-else-if="previewMode === 'diff'"
          :file-path="previewFilePath"
          :old-content="previewBaseContent"
          :new-content="previewRawContent" />
        <markdown-simple v-else :content="previewContent" :disable-interaction="true" />
      </div>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, watch, onMounted, computed } from 'vue'
import { convertFileSrc } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { invokeWrapper } from '@/libs/tauri'
import { writeClipboard } from '@/libs/clipboard'
import { imagePreview } from '@/libs/fs'
import { showMessage } from '@/libs/util'
import MarkdownSimple from './MarkdownSimple.vue'
import FilePreviewDiff from './FilePreviewDiff.vue'
import TreeNode from './TreeNode.vue'

const props = defineProps({
  paths: {
    type: Array,
    default: () => []
  }
})

const emit = defineEmits(['addPath', 'removePath'])

const roots = computed(() => props.paths)
const expandedNodes = ref(new Map())
const childrenMap = ref(new Map())
const loading = ref(false)

// Preview state
const previewVisible = ref(false)
const previewTitle = ref('')
const previewContent = ref('')
const previewRawContent = ref('')
const previewBaseContent = ref('')
const previewMode = ref('markdown')
const previewFilePath = ref('')
const previewAssetUrl = ref('')

const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'gif', 'svg', 'webp', 'bmp', 'ico', 'tiff', 'tif', 'avif'])
const AUDIO_EXTENSIONS = new Set(['mp3', 'wav', 'ogg', 'm4a', 'aac', 'flac', 'opus'])
const VIDEO_EXTENSIONS = new Set(['mp4', 'webm', 'mov', 'm4v', 'ogv', 'mkv'])
const OFFICE_EXTENSIONS = new Set([
  'doc', 'docx', 'ppt', 'pptx', 'xls', 'xlsx', 'pdf', 'odt', 'ods', 'odp', 'rtf', 'pages', 'numbers', 'key'
])

const getFileExtension = (path = '') => {
  const fileName = path.split(/[/\\]/).pop() || ''
  const lastDotIndex = fileName.lastIndexOf('.')
  return lastDotIndex > -1 ? fileName.slice(lastDotIndex + 1).toLowerCase() : ''
}

const getPreviewType = (path) => {
  const ext = getFileExtension(path)
  if (IMAGE_EXTENSIONS.has(ext)) return 'image'
  if (AUDIO_EXTENSIONS.has(ext)) return 'audio'
  if (VIDEO_EXTENSIONS.has(ext)) return 'video'
  if (OFFICE_EXTENSIONS.has(ext)) return 'unsupported'
  return 'text'
}

const isExpanded = (path) => expandedNodes.value.has(path)

const getDirName = (path) => {
  const parts = path.split(/[/\\]/).filter(p => p !== '')
  return parts[parts.length - 1] || path
}

const onAddPath = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: 'Select Directory'
    })
    if (selected && !roots.value.includes(selected)) {
      emit('addPath', selected)
    }
  } catch (error) {
    console.error('Failed to add path:', error)
  }
}

const onRemovePath = (path) => {
  emit('removePath', path)
}

const openAuthorizedFolder = async (path) => {
  if (!path) return
  try {
    await invokeWrapper('open_path_in_file_manager', { path })
  } catch (error) {
    console.error('Failed to open authorized folder:', error)
  }
}

const copyRootPath = async (path) => {
  if (!path) return
  try {
    await writeClipboard(path)
    showMessage('Path copied', 'success')
  } catch (error) {
    console.error('Failed to copy root path:', error)
    showMessage('Failed to copy path', 'error')
  }
}

const toggleExpand = async (path) => {
  if (expandedNodes.value.has(path)) {
    expandedNodes.value.delete(path)
  } else {
    expandedNodes.value.set(path, true)
    await loadDir(path)
  }
}

const loadDir = async (path) => {
  try {
    const list = await invokeWrapper('list_dir', { path })
    childrenMap.value.set(path, list)
  } catch (e) {
    console.error('Failed to load directory:', path, e)
  }
}

const getChildren = (path) => childrenMap.value.get(path) || []

const refreshAll = async () => {
  loading.value = true
  const expanded = Array.from(expandedNodes.value.keys())

  for (const path of expanded) {
    await loadDir(path)
  }

  loading.value = false
}

const refreshRoot = async (path) => {
  if (expandedNodes.value.has(path)) {
    await loadDir(path)
  }
}

const openPreviewFileWithDefaultApp = async () => {
  if (!previewFilePath.value) return

  try {
    await invokeWrapper('open_path_in_file_manager', { path: previewFilePath.value })
  } catch (error) {
    console.error('Failed to open file with default app:', error)
    showMessage('Failed to open file', 'error')
  }
}

const previewFile = async (path) => {
  try {
    previewTitle.value = getDirName(path)
    previewFilePath.value = path
    previewAssetUrl.value = ''
    previewContent.value = ''
    previewRawContent.value = ''
    previewBaseContent.value = ''

    const previewType = getPreviewType(path)

    if (previewType === 'unsupported') {
      previewMode.value = 'unsupported'
      previewVisible.value = true
      return
    }

    if (previewType === 'image' || previewType === 'audio' || previewType === 'video') {
      previewMode.value = previewType
      previewAssetUrl.value = previewType === 'image'
        ? await imagePreview(path)
        : convertFileSrc(path)

      if (!previewAssetUrl.value) {
        throw new Error(`Failed to resolve preview URL for ${path}`)
      }

      previewVisible.value = true
      return
    }

    const [content, baseContent] = await Promise.all([
      invokeWrapper('read_text_file', { filePath: path }),
      invokeWrapper('read_git_base_text_file', { filePath: path }).catch(() => null)
    ])

    previewRawContent.value = content
    previewBaseContent.value = typeof baseContent === 'string' ? baseContent : ''

    if (typeof baseContent === 'string' && baseContent !== content) {
      previewMode.value = 'diff'
      previewContent.value = ''
    } else if (getFileExtension(path) !== 'md') {
      previewMode.value = 'markdown'
      const ext = getFileExtension(path) || 'text'
      previewContent.value = `\`\`\`${ext}\n${content}\n\`\`\``
    } else {
      previewMode.value = 'markdown'
      previewContent.value = content
    }
    previewVisible.value = true
  } catch (e) {
    console.error('Failed to preview file:', e)
    previewMode.value = 'unsupported'
    previewVisible.value = true
  }
}

watch(() => props.paths, (newPaths) => {
  // Clear state when roots change
  expandedNodes.value.clear()
  childrenMap.value.clear()
}, { deep: true })

onMounted(() => {
  // Optionally auto-expand the first root
  if (roots.value.length > 0) {
    toggleExpand(roots.value[0])
  }
})
</script>

<style lang="scss" scoped>
.file-tree {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--cs-bg-color);
  color: var(--cs-text-color-primary);
  font-size: 13px;

  .tree-header {
    padding: 10px 15px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid var(--cs-border-color);
    flex-shrink: 0;

    .title {
      font-weight: bold;
      font-size: 12px;
      color: var(--cs-text-color-secondary);
      text-transform: uppercase;
    }

    .header-actions {
      display: flex;
      align-items: center;
      gap: 8px;

      .action-icon {
        cursor: pointer;
        color: var(--cs-text-color-secondary);

        &:hover {
          color: var(--el-color-primary);
        }

        &.rotating {
          animation: cs-rotate 1s linear infinite;
        }
      }
    }
  }

  .empty-tree {
    padding: 40px 20px;
    text-align: center;
    color: var(--cs-text-color-placeholder);
    font-style: italic;
  }

  .tree-content {
    flex: 1;
    overflow-y: auto;
    padding: 10px 0;

    .root-container {
      margin-bottom: 4px;
    }

    .root-item {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 6px 15px;
      cursor: pointer;
      transition: background 0.2s;

      &:hover {
        background: var(--cs-hover-bg-color);

        .root-actions {
          opacity: 1;
        }
      }

      .root-info {
        display: flex;
        align-items: center;
        gap: 8px;
        flex: 1;
        min-width: 0;
      }

      .root-name {
        font-weight: 600;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
      }

      .root-actions {
        display: flex;
        align-items: center;
        gap: 4px;
        opacity: 0;
        transition: opacity 0.2s;

        .action-btn {
          cursor: pointer;
          color: var(--cs-text-color-secondary);
          padding: 4px;
          border-radius: 4px;

          &:hover {
            background: var(--cs-bg-color-light);
          }
        }

        .refresh-btn:hover {
          color: var(--el-color-primary);
        }

        .remove-btn:hover {
          color: var(--el-color-danger);
        }
      }
    }

    .children {
      position: relative;
      padding-left: 0;
      margin-left: 15px;

      // Tree guide line (dashed)
      &::before {
        content: '';
        position: absolute;
        left: 0;
        top: 0;
        bottom: 0;
        width: 1px;
        border-left: 1px dashed var(--cs-border-color);
      }
    }
  }
}

.file-preview-dialog {
  :deep(.el-dialog__body) {
    padding: 0;
    background: var(--cs-bg-color);
  }

  .preview-content {
    max-height: 75vh;
    overflow-y: auto;
    padding: 20px;
  }

  .media-preview {
    display: flex;
    flex-direction: column;
    gap: 16px;
    align-items: center;
  }

  .media-preview__actions {
    width: 100%;
    display: flex;
    justify-content: flex-end;
  }

  .image-preview__img {
    display: block;
    max-width: 100%;
    max-height: calc(75vh - 80px);
    object-fit: contain;
    border-radius: 8px;
    background: var(--cs-bg-color-light);
  }

  .audio-preview {
    min-height: 240px;
    justify-content: center;
  }

  .audio-preview__player {
    width: min(100%, 720px);
  }

  .video-preview__player {
    width: 100%;
    max-height: calc(75vh - 80px);
    border-radius: 8px;
    background: #000;
  }

  .unsupported-preview {
    min-height: 260px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    text-align: center;
  }

  .unsupported-preview__icon {
    color: var(--el-color-warning);
  }

  .unsupported-preview__title {
    font-size: 16px;
    font-weight: 600;
    color: var(--cs-text-color-primary);
  }

  .unsupported-preview__desc {
    color: var(--cs-text-color-secondary);
  }

  .unsupported-preview__path {
    max-width: 100%;
    padding: 10px 14px;
    word-break: break-all;
    border-radius: 8px;
    color: var(--cs-text-color-secondary);
    background: var(--cs-bg-color-light);
  }
}

@keyframes cs-rotate {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}
</style>
