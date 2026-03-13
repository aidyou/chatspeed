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
        <markdown-simple :content="previewContent" />
      </div>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, watch, onMounted, computed } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { invokeWrapper } from '@/libs/tauri'
import MarkdownSimple from './MarkdownSimple.vue'
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

const previewFile = async (path) => {
  try {
    const content = await invokeWrapper('read_text_file', { filePath: path })
    previewTitle.value = getDirName(path)
    // Wrap content in code block if not already markdown
    if (!path.endsWith('.md')) {
      const ext = path.split('.').pop()
      previewContent.value = `\`\`\`${ext}\n${content}\n\`\`\``
    } else {
      previewContent.value = content
    }
    previewVisible.value = true
  } catch (e) {
    console.error('Failed to read file:', e)
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
