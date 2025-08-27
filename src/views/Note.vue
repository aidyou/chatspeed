<template>
  <el-container class="note-container">
    <titlebar class="header-container">
      <template #left>
        <div class="icon-btn upperLayer" @click="onToggleSidebar">
          <cs name="sidebar" />
        </div>
      </template>
      <template #center>
        <div class="note-title" v-if="currentNote">
          {{ currentNote?.title }}
        </div>
      </template>
      <template #right>
        <div class="icon-btn upperLayer" @click="onNoteTrash" v-if="currentNote">
          <cs name="trash" />
        </div>
      </template>
    </titlebar>

    <div class="body">
      <el-aside class="sidebar" :style="{ width: sidebarWidth + 'px' }" v-show="!sidebarCollapsed">
        <el-tree :data="treeData" :props="defaultProps" :expand-on-click-node="true" :default-expand-all="true"
          :highlight-current="true" @node-click="onHandleNodeClick" @node-expand="onHandleNodeExpand"
          @node-collapse="onHandleNodeCollapse" node-key="id">
          <template #default="{ node, data }">
            <div class="tree-node" :class="{ 'is-tag': data.type === 'tag' }">
              <div class="label">
                <el-icon class="note-tree-icon">
                  <template v-if="data.type === 'tag'">
                    <FolderOpened v-if="data.expanded" />
                    <folder v-else />
                  </template>
                  <document v-else />
                </el-icon>
                {{ node.label }}<span v-if="data.type === 'tag'">({{ data.count }})</span>
              </div>
            </div>
          </template>
        </el-tree>
      </el-aside>
      <div class="resize-handle" @mousedown="handleResizeStart" v-show="!sidebarCollapsed"></div>
      <el-main ref="mainContent" class="main">
        <div v-if="currentNote" class="chat note-content">
          <markdown :content="currentNote.content" :reference="currentNote?.metadata?.reference || []"
            :reasoning="currentNote?.metadata?.reasoning || ''" />
          <div class="note-footer">
            <div class="note-meta">
              <span class="time">{{ formatTime(currentNote.updatedAt * 1000) }}</span>
              <div class="tags">
                <el-tag v-for="tag in currentNote.tags" :key="tag" size="small">
                  {{ tag }}
                </el-tag>
              </div>
            </div>
          </div>
        </div>
        <div v-else class="empty-state">
          <el-empty description="选择或创建一个笔记" />
        </div>
      </el-main>
    </div>
  </el-container>

  <el-dialog class="note-search-dialog" v-model="searchDialogVisible" :close-on-press-escape="false"
    :show-close="false">
    <el-input ref="searchInputRef" v-model="kw" :placeholder="$t('note.searchNotePlaceholder')" @input="onSearchNote" />
    <div class="note-list">
      <div class="note-item" v-for="note in searchResult" :key="note.id" @click="onSelectNote(note)">
        {{ note.title }}
      </div>
    </div>
  </el-dialog>
</template>

<script setup>
import { ref, onMounted, watch, nextTick, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { Document, Folder, FolderOpened } from '@element-plus/icons-vue'
import { listen } from '@tauri-apps/api/event'

import markdown from '@/components/chat/Markdown.vue'

import { csStorageKey } from '@/config/config'
import { formatTime } from '@/libs/util'
import { useNoteStore } from '@/stores/note'
import { csGetStorage, csSetStorage, showMessage } from '@/libs/util'

const { t } = useI18n()

const noteStore = useNoteStore()

// note search
const kw = ref('')
const searchResult = ref([])
const searchDialogVisible = ref(false)
const searchInputRef = ref(null)

const treeData = ref([])
const currentNote = ref(null)
const defaultProps = {
  label: 'label',
  children: 'nodes'
}
const mainContent = ref(null)

// sidebar
const sidebarCollapsed = ref(false)

const sidebarWidth = ref(220)
const isResizing = ref(false)
const startX = ref(0)
const startWidth = ref(0)

watch(
  () => noteStore.tags,
  () => {
    treeData.value = noteStore.tags.map(tag => ({
      id: tag.id,
      type: 'tag',
      label: tag.name,
      count: tag.note_count,
      nodes: []
    }))
  },
  { immediate: true }
)

watch(
  sidebarWidth,
  newWidth => {
    document.documentElement.style.setProperty('--sidebar-width', `${newWidth}px`)
  },
  { immediate: true }
)

onMounted(async () => {
  // init sidebar width and collapsed state
  const width = csGetStorage(csStorageKey.noteSidebarWidth)
  if (width) {
    sidebarWidth.value = Number(width)
  }
  sidebarCollapsed.value = csGetStorage(csStorageKey.noteSidebarCollapsed)

  // update tags
  noteStore.getTagList()

  // listen note_update event
  await listen('sync_state', event => {
    if (event.payload.windowLabel === noteStore.windowLabel) {
      return
    }
    if (event.payload.type === 'note_update') {
      noteStore.getTagList()
    }
  })
  // add keyboard shortcut ctrl|command + p handle
  document.addEventListener('keydown', e => {
    if (e.ctrlKey || e.metaKey) {
      if (e.key === 'b') {
        onToggleSidebar()
      } else if (e.key === 'p') {
        searchDialogVisible.value = true
        setTimeout(() => {
          searchInputRef.value?.focus()
        }, 300)
      }
    }
  })
})

onUnmounted(() => {
  document.removeEventListener('mousemove', handleResizeMove)
  document.removeEventListener('mouseup', handleResizeEnd)
})

const handleResizeStart = e => {
  isResizing.value = true
  startX.value = e.clientX
  startWidth.value = sidebarWidth.value
  document.body.style.cursor = 'col-resize'
  document.body.style.userSelect = 'none'

  // 添加全局事件监听
  document.addEventListener('mousemove', handleResizeMove)
  document.addEventListener('mouseup', handleResizeEnd)
}

const handleResizeMove = e => {
  if (!isResizing.value) return

  const deltaX = e.clientX - startX.value
  const newWidth = startWidth.value + deltaX

  // 限制最小和最大宽度
  sidebarWidth.value = Math.max(200, Math.min(500, newWidth))

  // 保存宽度到本地存储
  csSetStorage(csStorageKey.noteSidebarWidth, sidebarWidth.value)
}

const handleResizeEnd = () => {
  isResizing.value = false
  document.body.style.cursor = ''
  document.body.style.userSelect = ''

  // 移除全局事件监听
  document.removeEventListener('mousemove', handleResizeMove)
  document.removeEventListener('mouseup', handleResizeEnd)
}

const onHandleNodeClick = data => {
  if (data.type === 'tag') {
    if (typeof data.expanded === 'undefined') {
      data.expanded = true
    }
    noteStore.getNotes(data.id).then(res => {
      data.nodes = res.map(note => ({
        id: note.id,
        type: 'note',
        label: note.title
      }))
    })
  } else {
    noteStore.getNote(data.id).then(res => {
      currentNote.value = res
      // reset scroll
      nextTick(() => {
        if (mainContent.value) {
          mainContent.value.$el.scrollTop = 0
        }
      })
    })
  }
}

const onHandleNodeExpand = data => {
  data.expanded = true
}
const onHandleNodeCollapse = data => {
  data.expanded = false
}

const onToggleSidebar = () => {
  sidebarCollapsed.value = !sidebarCollapsed.value
  csSetStorage(csStorageKey.noteSidebarCollapsed, sidebarCollapsed.value)
}

const onNoteTrash = () => {
  if (!currentNote.value) return

  ElMessageBox.confirm(t('chat.noteDeleteConfirmContent'), t('chat.noteDeleteConfirmTitle'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel'),
    type: 'warning'
  }).then(() => {
    noteStore
      .deleteNote(currentNote.value.id)
      .then(() => {
        // remove note from tags and filter out empty tags
        treeData.value = treeData.value
          ?.map(tag => {
            if (tag.nodes.length > 0) {
              tag.nodes = tag.nodes.filter(note => note.id !== currentNote.value.id)
              tag.count = tag.nodes.length
            }
            return tag
          })
          .filter(tag => tag.count > 0)

        // clear current note and reset UI
        currentNote.value = null
        if (mainContent.value) {
          mainContent.value.scrollTop = 0
        }
      })
      .catch(error => {
        showMessage(t('chat.errorOnDeleteNote', { error }), 'error', 5000)
      })
  })
}

const onSearchNote = () => {
  if (!kw.value) {
    searchResult.value = []
    return
  }
  noteStore.searchNotes(kw.value).then(res => {
    searchResult.value = res
  })
}

const onSelectNote = note => {
  currentNote.value = note
  searchDialogVisible.value = false
}
</script>

<style lang="scss">
.note-container {
  height: 100vh;
  background-color: var(--cs-bg-color);
  color: var(--cs-text-color-primary);
  border-radius: var(--cs-border-radius-md);
  display: flex;
  flex-direction: column;

  .note-title {
    font-weight: bold;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .body {
    display: flex;
    flex: 1;
    overflow: hidden;

    .resize-handle {
      position: relative;
      width: 4px;
      background: var(--cs-bg-color-light);
      cursor: col-resize;
      transition: background-color 0.2s;
      z-index: 1;
      flex-shrink: 0;

      &:hover,
      &:active {
        background: var(--cs-bg-color-deep);
      }
    }

    .sidebar {
      flex-shrink: 0;
      border-right: 1px solid var(--cs-border-color);
      background-color: var(--cs-bg-color-deep);
      padding: var(--cs-space) var(--cs-space-xs);
      overflow-y: auto;
      background: var(--cs-bg-color);
      user-select: none;
      -moz-user-select: none;
      -webkit-user-select: none;

      .el-tree {
        background: none;
        user-select: none;
        -moz-user-select: none;
        -webkit-user-select: none;

        &.el-tree--highlight-current .el-tree-node.is-current>.el-tree-node__content {
          background-color: var(--cs-active-bg-color);
        }

        .el-tree-node__expand-icon {
          display: none;
        }
      }

      .tree-node {
        display: flex;
        align-items: center;

        &.is-tag {
          font-weight: 500;

          .label {
            padding-left: var(--cs-space-xs);
          }
        }

        .label {
          display: flex;
          align-items: center;
          gap: var(--cs-space-xxs);

          .note-tree-icon {
            font-size: var(--cs-font-size-md);
          }
        }
      }
    }

    .main {
      flex: 1;
      padding: var(--cs-space);
      overflow-y: auto;
      background-color: var(--cs-bg-color-light);

      .note-content {
        margin: 0 auto;

        .note-footer {
          margin: var(--cs-space) auto;

          .note-meta {
            display: flex;
            align-items: center;
            gap: var(--cs-space);
            color: var(--cs-text-color-secondary);

            .time {
              font-size: var(--cs-font-size-sm);
            }

            .tags {
              display: flex;
              gap: var(--cs-space-xxs);
            }
          }
        }
      }

      .empty-state {
        height: 100%;
        display: flex;
        align-items: center;
        justify-content: center;
      }
    }
  }
}

.note-search-dialog {
  display: flex;
  flex-direction: column;

  .el-dialog__header {
    display: none;
  }

  .note-list {
    flex: 1;
    max-height: 500px;
    overflow-y: scroll;

    .note-item {
      width: 100%;
      cursor: pointer;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      margin-top: var(--cs-space-xs);
      padding: var(--cs-space-xs) var(--cs-space-xs);

      &:hover {
        background-color: var(--cs-hover-bg-color);
      }
    }
  }
}
</style>
