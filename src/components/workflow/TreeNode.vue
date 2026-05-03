<template>
  <div class="tree-node">
    <div
      class="node-item"
      :class="{ 'is-dir': node.is_dir }"
      :draggable="true"
      @click="handleClick"
      @dragstart="handleDragStart">
      <span class="node-icon">
        <cs :name="node.is_dir ? (isExpanded ? 'ext-folder-open' : 'ext-folder') : getFileIcon(node.name)"
          size="14px" />
      </span>
      <span class="node-name" :class="gitStatusClass">{{ node.name }}</span>
      <div v-if="node.git_status" class="git-status" :class="gitStatusClass" :title="node.git_status"></div>
      <div v-if="node.is_dir" class="node-actions">
        <cs name="copy" size="12px" class="action-btn copy-btn" @click.stop="copyNodePath" />
        <cs name="ext-folder-open" size="12px" class="action-btn open-btn" @click.stop="openNodePath" />
      </div>
    </div>

    <div v-if="node.is_dir && isExpanded" class="node-children">
      <tree-node v-for="child in children" :key="child.path" :node="child" :expanded-map="expandedMap"
        @toggle="$emit('toggle', $event)" @preview="$emit('preview', $event)" />
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { invokeWrapper } from '@/libs/tauri'
import { writeClipboard } from '@/libs/clipboard'
import { showMessage } from '@/libs/util'

const props = defineProps({
  node: Object,
  expandedMap: Object
})

const emit = defineEmits(['toggle', 'preview'])

const isExpanded = computed(() => props.expandedMap.has(props.node.path))
const children = ref([])

const gitStatusClass = computed(() => {
  if (!props.node.git_status) return ''
  const code = props.node.git_status.trim()
  if (code.includes('D')) return 'deleted'
  if (code === '??' || code.includes('A')) return 'added'
  if (code.includes('M') || code.includes('R') || code.includes('C') || code.includes('U')) return 'modified'
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

const copyNodePath = async () => {
  try {
    await writeClipboard(props.node.path)
    showMessage('Path copied', 'success')
  } catch (error) {
    console.error('Failed to copy path:', error)
    showMessage('Failed to copy path', 'error')
  }
}

const openNodePath = async () => {
  try {
    await invokeWrapper('open_path_in_file_manager', { path: props.node.path })
  } catch (error) {
    console.error('Failed to open directory:', error)
  }
}

const handleDragStart = event => {
  const path = String(props.node?.path || '').trim()
  if (!path || !event.dataTransfer) return

  event.dataTransfer.effectAllowed = 'copy'
  event.dataTransfer.setData('application/x-chatspeed-workflow-path', path)
  event.dataTransfer.setData('text/plain', path)
}

const getFileIcon = (name) => {
  const ext = name.split('.').pop().toLowerCase()
  // 压缩包
  if (['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'tgz'].includes(ext)) return 'ext-zip'
  // 图片
  if (['png', 'jpg', 'jpeg', 'gif', 'svg', 'webp', 'bmp', 'ico', 'tiff', 'raw'].includes(ext)) return 'ext-pic'
  // 音视频
  if (['mp3', 'wav', 'ogg', 'm4a', 'aac', 'flac', 'opus', 'mp4', 'webm', 'mov', 'm4v', 'ogv', 'mkv'].includes(ext)) return 'ext-file'
  // 文档
  if (['doc', 'docx'].includes(ext)) return 'ext-docx'
  if (['ppt', 'pptx'].includes(ext)) return 'ext-pptx'
  if (['xls', 'xlsx'].includes(ext)) return 'ext-xlsx'
  if (['pdf'].includes(ext)) return 'ext-pdf'
  if (['csv'].includes(ext)) return 'ext-csv'
  // 代码 - Web
  if (['html', 'htm', 'xhtml'].includes(ext)) return 'ext-html'
  if (['css', 'scss', 'sass', 'less', 'styl'].includes(ext)) return 'ext-css'
  if (['js', 'mjs', 'cjs'].includes(ext)) return 'ext-js'
  if (['ts', 'mts', 'cts'].includes(ext)) return 'ext-ts'
  if (['jsx'].includes(ext)) return 'ext-jsx'
  if (['tsx'].includes(ext)) return 'ext-tsx'
  if (['vue'].includes(ext)) return 'ext-vue'
  // 代码 - 后端/系统
  if (['c', 'h', 'hpp'].includes(ext)) return 'ext-c'
  if (['cpp', 'cc', 'cxx', 'hxx'].includes(ext)) return 'ext-cpp'
  if (['java'].includes(ext)) return 'ext-java'
  if (['rs'].includes(ext)) return 'ext-rs'
  if (['go'].includes(ext)) return 'ext-go'
  if (['swift'].includes(ext)) return 'ext-swift'
  if (['kt', 'kts', 'jsp', 'scala'].includes(ext)) return 'ext-java'
  if (['rb'].includes(ext)) return 'ext-rb'
  if (['php', 'php4', 'php5'].includes(ext)) return 'ext-php'
  // Python
  if (['py', 'pyw', 'pyc', 'pyd', 'pyi'].includes(ext)) return 'ext-py'
  // Shell/脚本
  if (['sh', 'bash', 'zsh', 'fish', 'ps1', 'psm1', 'bat', 'cmd', 'fish'].includes(ext)) return 'ext-shell'
  // 可执行文件
  if (['exe', 'msi', 'dmg', 'app', 'bin', 'pkg', 'deb', 'rpm', 'apk'].includes(ext)) return 'ext-exe'
  // 配置文件/数据
  if (['yaml', 'yml'].includes(ext)) return 'ext-yaml'
  if (['json'].includes(ext)) return 'ext-json'
  if (['xml'].includes(ext)) return 'ext-xml'
  if (['toml'].includes(ext)) return 'ext-toml'
  if (['ini', 'conf', 'cfg', 'properties', 'env'].includes(ext)) return 'ext-setting'
  // Git 相关
  if (name === '.git' || name === '.gitignore' || name === '.gitattributes' || name === '.gitmodules') return 'ext-git'
  // 文本/文档
  if (['md', 'markdown'].includes(ext)) return 'ext-md'
  if (['txt'].includes(ext)) return 'ext-txt'
  if (['log'].includes(ext)) return 'ext-log'
  // 默认
  return 'ext-file'
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

      &.modified {
        color: #e6a23c;
      }

      &.added {
        color: #67c23a;
      }

      &.deleted {
        color: #f56c6c;
        text-decoration: line-through;
      }
    }

    .node-icon {
      flex-shrink: 0;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 16px;

      .cs {
        color: var(--cs-text-color-secondary);
      }
    }

    &.is-dir .node-icon .cs {
      color: var(--el-color-primary);
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

      &.other {
        background-color: var(--cs-text-color-secondary);
      }
    }

    .node-actions {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      flex-shrink: 0;
      opacity: 0;
      transition: opacity 0.2s ease;

      .action-btn {
        color: var(--cs-text-color-secondary);

        &:hover {
          color: var(--el-color-primary);
        }
      }
    }

    &:hover .node-actions {
      opacity: 1;
    }
  }

  .node-children {
    position: relative;
    padding-left: 0;
    margin-left: 21px;

    // Tree guide line (dashed) - vertical line
    &::before {
      content: '';
      position: absolute;
      left: 0;
      top: 0;
      bottom: 0;
      width: 1px;
      border-left: 1px dashed var(--cs-border-color);
    }

    // Each tree node in children
    >.tree-node {
      position: relative;

      // Horizontal connector line
      &::before {
        content: '';
        position: absolute;
        left: -21px;
        top: 12px;
        width: 12px;
        height: 1px;
        border-top: 1px dashed var(--cs-border-color);
      }
    }
  }
}
</style>
