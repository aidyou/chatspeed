<template>
  <div class="tree-node">
    <div class="node-item" :class="{ 'is-dir': node.is_dir }" @click="handleClick">
      <cs :name="node.is_dir ? (isExpanded ? 'folder-open' : 'folder') : getFileIcon(node.name)" size="14px"
        :class="node.is_dir ? 'dir-icon' : 'file-icon'" />
      <span class="node-name">{{ node.name }}</span>
      <div v-if="gitCode" class="git-status" :class="gitStatusClass" :title="gitCode"></div>
    </div>

    <div v-if="node.is_dir && isExpanded" class="node-children">
      <tree-node v-for="child in children" :key="child.path" :node="child" :expanded-map="expandedMap"
        :git-status="gitStatus" @toggle="$emit('toggle', $event)" @preview="$emit('preview', $event)" />
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { invokeWrapper } from '@/libs/tauri'

const props = defineProps({
  node: Object,
  expandedMap: Object,
  gitStatus: Object // Map
})

const emit = defineEmits(['toggle', 'preview'])

const isExpanded = computed(() => props.expandedMap.has(props.node.path))
const children = ref([])

const gitCode = computed(() => {
  // Try to match the full path in gitStatus Map
  // Note: git status returns paths relative to repo root
  return props.gitStatus.get(props.node.path.replace(/\\/g, '/'))
})

const gitStatusClass = computed(() => {
  if (!gitCode.value) return ''
  const code = gitCode.value.trim()
  if (code === 'M') return 'modified'
  if (code === 'A' || code === '??') return 'added'
  if (code === 'D') return 'deleted'
  return 'other'
})

const loadChildren = async () => {
  if (props.node.is_dir && isExpanded.value) {
    try {
      children.value = await invokeWrapper('list_dir', { path: props.node.path })
    } catch (e) {
      console.error('Failed to load children:', props.node.path, e)
    }
  }
}

const handleClick = () => {
  if (props.node.is_dir) {
    emit('toggle', props.node.path)
  } else {
    emit('preview', props.node.path)
  }
}

const getFileIcon = (name) => {
  const ext = name.split('.').pop().toLowerCase()
  if (['js', 'ts', 'jsx', 'tsx', 'vue'].includes(ext)) return 'file-code'
  if (['md', 'txt', 'log'].includes(ext)) return 'file-text'
  if (['json', 'yaml', 'yml', 'xml'].includes(ext)) return 'file-json'
  if (['py', 'rs', 'go', 'c', 'cpp', 'java'].includes(ext)) return 'file-code'
  if (['png', 'jpg', 'jpeg', 'gif', 'svg'].includes(ext)) return 'file-image'
  return 'file'
}

watch(isExpanded, (newVal) => {
  if (newVal) loadChildren()
}, { immediate: true })
</script>

<style lang="scss" scoped>
.tree-node {
  .node-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 15px;
    cursor: pointer;
    transition: background 0.2s;
    position: relative;

    &:hover {
      background: var(--cs-hover-bg-color);
    }

    .node-name {
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      flex: 1;
    }

    .dir-icon {
      color: var(--el-color-primary);
    }

    .file-icon {
      color: var(--cs-text-color-secondary);
    }

    .git-status {
      width: 6px;
      height: 6px;
      border-radius: 50%;
      flex-shrink: 0;

      &.modified {
        background-color: #e6a23c; // Warning/Yellow
      }

      &.added {
        background-color: #67c23a; // Success/Green
      }

      &.deleted {
        background-color: #f56c6c; // Danger/Red
      }
    }
  }

  .node-children {
    padding-left: 12px;
    border-left: 1px solid var(--cs-border-color-light);
    margin-left: 21px;
  }
}
</style>
