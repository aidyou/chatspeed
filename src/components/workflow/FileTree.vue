<template>
  <div class="file-tree">
    <div class="tree-header">
      <span class="title">{{ $t('settings.agent.authorizedPaths') }}</span>
      <el-tooltip :content="$t('common.refresh')" placement="top">
        <cs name="refresh" class="refresh-icon" @click="refreshAll" :class="{ rotating: loading }" />
      </el-tooltip>
    </div>

    <div v-if="roots.length === 0" class="empty-tree">
      {{ $t('settings.agent.authorizedPathsTip') }}
    </div>

    <div v-else class="tree-content">
      <div v-for="root in roots" :key="root" class="root-container">
        <div class="root-item" @click="toggleExpand(root)">
          <cs :name="isExpanded(root) ? 'folder-open' : 'folder'" size="14px" />
          <span class="root-name" :title="root">{{ getDirName(root) }}</span>
        </div>
        <div v-if="isExpanded(root)" class="children">
          <tree-node v-for="child in getChildren(root)" :key="child.path" :node="child" :expanded-map="expandedNodes"
            :git-status="gitStatusMap" @toggle="toggleExpand" @preview="previewFile" />
        </div>
      </div>
    </div>

    <!-- File Preview Dialog -->
    <el-dialog v-model="previewVisible" :title="previewTitle" width="80%" top="5vh" class="file-preview-dialog"
      append-to-body destroy-on-close>
      <div class="preview-content">
        <markdown :content="previewContent" />
      </div>
    </el-dialog>
  </div>
</template>

<script setup>
import { ref, watch, onMounted, computed } from 'vue'
import { invokeWrapper } from '@/libs/tauri'
import Markdown from '@/components/chat/Markdown.vue'
import TreeNode from './TreeNode.vue'

const props = defineProps({
  paths: {
    type: Array,
    default: () => []
  }
})

const roots = computed(() => props.paths)
const expandedNodes = ref(new Map())
const childrenMap = ref(new Map())
const gitStatusMap = ref(new Map())
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
    
    // Also fetch git status for the root if it's a new expansion at top level
    if (roots.value.includes(path)) {
      await fetchGitStatus(path)
    }
  } catch (e) {
    console.error('Failed to load directory:', path, e)
  }
}

const fetchGitStatus = async (rootPath) => {
  try {
    const status = await invokeWrapper('get_git_status', { path: rootPath })
    // Backend now returns absolute paths as keys
    Object.entries(status).forEach(([absPath, code]) => {
      // Standardize path separator for consistent matching
      const normalizedPath = absPath.replace(/\\/g, '/')
      gitStatusMap.value.set(normalizedPath, code)
    })
  } catch (e) {
    // Ignore errors (e.g. not a git repo)
  }
}

const getChildren = (path) => childrenMap.value.get(path) || []

const refreshAll = async () => {
  loading.value = true
  const expanded = Array.from(expandedNodes.value.keys())
  gitStatusMap.value.clear()
  
  for (const path of expanded) {
    await loadDir(path)
  }
  
  // Refresh roots that might not be in expandedNodes but are visible
  for (const root of roots.value) {
    if (!expandedNodes.value.has(root)) {
       // Just refresh git status for roots
       await fetchGitStatus(root)
    }
  }
  loading.value = false
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
  gitStatusMap.value.clear()
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

    .refresh-icon {
      cursor: pointer;
      color: var(--cs-text-color-secondary);
      &:hover { color: var(--el-color-primary); }
      &.rotating { animation: cs-rotate 1s linear infinite; }
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
      gap: 8px;
      padding: 6px 15px;
      cursor: pointer;
      transition: background 0.2s;

      &:hover { background: var(--cs-hover-bg-color); }

      .root-name {
        font-weight: 600;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
      }
    }

    .children {
      padding-left: 15px;
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
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}
</style>
