<template>
  <div class="file-selector">
    <div v-if="selectPath" class="preview">
      <template v-if="props.type == 'image'">
        <div v-for="(item, index) in previewFiles" :key="index">
          <div class="image" :style="imgStyle">
            <div class="icon" v-if="item.isPreview">
              <cs name="trash" @click="onDeleteImage(index)" />
            </div>
            <img :src="item.path" />
          </div>
        </div>
        <div v-if="loading">
          <div class="image" :style="imgStyle">
            <div class="icon">
              <cs name="loading" size="20px" class="cs-spin" />
            </div>
          </div>
        </div>
      </template>
      <template v-else>
        <div v-for="(item, index) in previewFiles" :key="index" class="flex items-center">
          <div class="file"><cs name="file" color="primary" size="20px" /> {{ item.path }}</div>
        </div>
      </template>
    </div>
    <div
      class="upload-btn"
      :class="{ image: props.type == 'image' }"
      :style="imgStyle"
      @click="selectFile"
      v-if="selectPath.length == 0 || props.multiple">
      <cs :name="props.type == 'image' ? 'upload-image' : 'add'" size="22px" />
    </div>
  </div>
</template>
<script setup>
import { computed, ref } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { pictureDir, documentDir } from '@tauri-apps/api/path'

import { imagePreview } from '@/libs/fs'

const props = defineProps({
  type: {
    type: String,
    default: 'image',
    validator: value => ['image', 'document'].includes(value),
  },
  imgWidth: {
    type: Number,
    default: 36,
  },
  imgHeight: {
    type: Number,
    default: 36,
  },
  multiple: {
    type: Boolean,
    default: false,
  },
  defaultPath: {
    type: String,
    default: '',
  },
})

const emit = defineEmits(['fileChanged'])

const allowedTypes = {
  image: {
    name: 'Images',
    extensions: ['png', 'jpg', 'jpeg'],
  },
  document: {
    name: 'Documents',
    extensions: ['doc', 'docx', 'pdf', 'txt', 'md'],
  },
}

const loading = ref(false) // Indicates if the file is currently being loaded
const selectPath = ref([]) // User-selected file paths
const FileUrls = ref([]) // URLs for the preview images or file names

const imgStyle = computed(() => {
  if (props.type == 'image') {
    return {
      width: props.imgWidth + 'px',
      height: props.imgHeight + 'px',
    }
  }
  return {}
})

/**
 * Compute the preview files based on selected files or default path
 */
const previewFiles = computed(() => {
  // Create preview objects for selected files
  // Use default path if no files are selected
  return FileUrls.value.length > 0
    ? FileUrls.value.map(x => ({ isPreview: true, path: x }))
    : props.defaultPath
    ? [{ isPreview: false, path: props.defaultPath }]
    : []
})

/**
 * Function to open file dialog and select files
 */
const selectFile = async () => {
  try {
    const selected = await open({
      multiple: props.multiple,
      filters: [allowedTypes[props.type]], // Filter files based on type
      defaultPath: props.type === 'image' ? await pictureDir() : await documentDir(),
    })

    if (selected) {
      if (typeof selected === 'string') {
        selectPath.value = [selected] // Set selected path
        loading.value = true
        FileUrls.value =
          props.type === 'image' ? [await imagePreview(selected)] : [getFileName(selected)] // Generate preview URL or file name
        console.log(FileUrls.value)
      } else {
        selectPath.value = selected // Set selected paths
        loading.value = true
        FileUrls.value = selected.map(async path => {
          return props.type === 'image' ? await imagePreview(path) : getFileName(path) // Generate preview URLs or file names for multiple files
        })
      }
      loading.value = false
      emit('fileChanged', selectPath.value) // Emit event with selected paths
    } else {
      selectPath.value = [] // Reset selected paths if no files are chosen
      FileUrls.value = []
      emit('fileChanged', props.defaultPath ? [props.defaultPath] : []) // Emit event with default path if no files are selected
    }
  } catch (error) {
    ElMessage.error(t('common.uploadFailed')) // Show error message if file selection fails
  }
}

/**
 * Get file name from path
 * @param {string} path - file path
 * @returns {string} - file name
 */
const getFileName = path => {
  return path.split('/').pop() // Extract the file name from the path
}

const onDeleteImage = index => {
  FileUrls.value.splice(index, 1) // Remove the file URL from the array
  selectPath.value.splice(index, 1) // Remove the selected path from the array
}

// =================================================
// Expose
// =================================================

const reset = () => {
  loading.value = false // Reset loading state
  selectPath.value = [] // Clear selected paths
  FileUrls.value = [] // Clear file URLs
}
defineExpose({
  reset, // Expose reset function
})
</script>

<style scoped lang="scss">
.file-selector {
  display: flex;
  align-items: center;

  .upload-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;

    &.image {
      margin-left: var(--cs-space);
      border-radius: 50%;
      border: 1px dashed #ccc;

      &:hover {
        color: var(--cs-color-primary);
        border-color: var(--cs-color-primary);
      }
    }
  }

  .preview {
    display: flex;
    flex-wrap: wrap;
    gap: var(--cs-space-sm);

    &:last-child {
      margin-right: 0;
    }

    .image {
      border-radius: var(--cs-border-radius);
      overflow: hidden;
      position: relative;

      img {
        width: 100%;
        height: 100%;
        border-radius: var(--cs-border-radius);
      }

      .icon {
        position: absolute;
        top: 0;
        bottom: 0;
        right: 0;
        left: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 10;

        &:hover {
          background-color: var(--cs-bg-color-opacity);
          color: var(--cs-text-color-primary);
        }

        .cs {
          cursor: pointer;
        }
      }

      img {
        width: 100%;
        height: 100%;
        object-fit: cover;
      }
    }
  }
}
</style>
