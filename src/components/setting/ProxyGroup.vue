<template>
  <div class="proxy-group-container">
    <div class="card">
      <div class="title">
        <span>{{ $t('settings.proxyGroup.title') }}</span>
        <el-tooltip :content="$t('settings.proxyGroup.addGroup')" placement="top">
          <span class="icon" @click="openAddDialog">
            <cs name="add" />
          </span>
        </el-tooltip>
      </div>

      <div class="list">
        <template v-if="proxyGroupStore.list.length > 0">
          <div v-for="group in proxyGroupStore.list" :key="group.id" class="item">
            <div class="label">
              <Avatar :size="36" :text="group.name" />
              <div class="label-text">
                {{ group.name }}
                <small>{{ group.description }}</small>
              </div>
            </div>

            <div class="value">
              <el-tooltip :content="$t('settings.proxyGroup.editGroup')" placement="top">
                <span class="icon" @click="openEditDialog(group)">
                  <cs name="edit" size="16px" color="secondary" />
                </span>
              </el-tooltip>
              <el-tooltip :content="$t('settings.proxyGroup.deleteGroup')" placement="top">
                <span class="icon" @click="handleDeleteGroup(group.id)">
                  <cs name="trash" size="16px" color="secondary" />
                </span>
              </el-tooltip>
            </div>
          </div>
        </template>
        <template v-else>
          <div class="empty-state">
            {{ $t('settings.proxyGroup.noGroupsFound') }}
            <el-button type="primary" @click="openAddDialog" size="small">
              <cs name="add" />{{ $t('settings.proxyGroup.addNow') }}
            </el-button>
          </div>
        </template>
      </div>

      <el-dialog
        v-model="dialogVisible"
        :title="
          isEditing ? $t('settings.proxyGroup.editTitle') : $t('settings.proxyGroup.addTitle')
        "
        width="600px"
        align-center
        @closed="resetForm"
        class="proxy-group-edit-dialog"
        :show-close="false"
        :close-on-click-modal="false"
        :close-on-press-escape="false">
        <div class="form-container">
          <el-form
            :model="currentGroup"
            label-width="auto"
            ref="proxyGroupFormRef"
            style="padding-top: 10px">
            <el-form-item
              :label="$t('settings.proxyGroup.form.name')"
              prop="name"
              :rules="[
                { required: true, message: $t('settings.proxyGroup.validation.nameRequired') }
              ]">
              <el-input
                v-model="currentGroup.name"
                :placeholder="$t('settings.proxyGroup.form.namePlaceholder')" />
            </el-form-item>
            <el-form-item :label="$t('settings.proxyGroup.form.description')" prop="description">
              <el-input
                v-model="currentGroup.description"
                type="textarea"
                :rows="2"
                :placeholder="$t('settings.proxyGroup.form.descriptionPlaceholder')" />
            </el-form-item>
            <el-form-item
              :label="$t('settings.proxyGroup.form.promptInjection')"
              prop="prompt_injection">
              <el-select
                v-model="currentGroup.prompt_injection"
                :placeholder="$t('settings.proxyGroup.form.promptInjectionPlaceholder')">
                <el-option :label="$t('settings.proxyGroup.promptInjection.off')" value="off" />
                <el-option
                  :label="$t('settings.proxyGroup.promptInjection.enhance')"
                  value="enhance" />
                <el-option
                  :label="$t('settings.proxyGroup.promptInjection.replace')"
                  value="replace" />
              </el-select>
            </el-form-item>
            <el-form-item :label="$t('settings.proxyGroup.form.promptText')" prop="prompt_text">
              <el-input
                v-model="currentGroup.prompt_text"
                type="textarea"
                :rows="4"
                :placeholder="$t('settings.proxyGroup.form.promptTextPlaceholder')" />
            </el-form-item>
            <el-form-item :label="$t('settings.proxyGroup.form.toolFilter')" prop="tool_filter">
              <el-input
                v-model="currentGroup.tool_filter"
                type="textarea"
                :rows="3"
                :placeholder="$t('settings.proxyGroup.form.toolFilterPlaceholder')" />
            </el-form-item>
            <el-form-item :label="$t('settings.proxyGroup.form.temperature')" prop="temperature">
              <div class="temperature-wrap">
                <el-tooltip
                  :content="$t('settings.proxyGroup.form.temperaturePlaceholder')"
                  placement="top">
                  <el-input-number
                    v-model="currentGroup.temperature"
                    :min="-1"
                    :max="1"
                    :step="0.1" />
                </el-tooltip>
                <el-slider
                  v-model="currentGroup.temperature"
                  :min="-1"
                  :max="1"
                  :step="0.1"
                  style="width: 65%" />
              </div>
            </el-form-item>
            <el-form-item :label="$t('settings.proxyGroup.form.disabled')" prop="disabled">
              <el-switch v-model="currentGroup.disabled" />
            </el-form-item>
          </el-form>
        </div>
        <template #footer>
          <span class="dialog-footer">
            <el-button @click="dialogVisible = false">{{ $t('common.cancel') }}</el-button>
            <el-button type="primary" @click="handleGroupConfigSubmit" :loading="formLoading">
              {{ $t('common.confirm') }}
            </el-button>
          </span>
        </template>
      </el-dialog>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useProxyGroupStore } from '@/stores/proxy_group'
import { ElMessageBox } from 'element-plus'
import { showMessage } from '@/libs/util'

const { t } = useI18n()
const proxyGroupStore = useProxyGroupStore()

const dialogVisible = ref(false)
const isEditing = ref(false)
const formLoading = ref(false)
const proxyGroupFormRef = ref(null)

const initialGroupState = () => ({
  id: null,
  name: '',
  description: '',
  prompt_injection: 'off',
  prompt_text: '',
  tool_filter: '',
  temperature: -1,
  disabled: false
})

const currentGroup = ref(initialGroupState())

onMounted(() => {
  proxyGroupStore.getList()
})

const openAddDialog = () => {
  isEditing.value = false
  currentGroup.value = initialGroupState()
  dialogVisible.value = true
}

const openEditDialog = group => {
  isEditing.value = true
  currentGroup.value = { ...group }
  dialogVisible.value = true
}

const resetForm = () => {
  currentGroup.value = initialGroupState()
  isEditing.value = false
  if (proxyGroupFormRef.value) {
    proxyGroupFormRef.value.resetFields()
  }
  formLoading.value = false
}

const handleGroupConfigSubmit = async () => {
  if (!proxyGroupFormRef.value) return
  await proxyGroupFormRef.value.validate(async valid => {
    if (valid) {
      formLoading.value = true
      try {
        if (isEditing.value) {
          await proxyGroupStore.update(currentGroup.value)
          showMessage(t('settings.proxyGroup.updateSuccess'), 'success')
        } else {
          await proxyGroupStore.add(currentGroup.value)
          showMessage(t('settings.proxyGroup.addSuccess'), 'success')
        }
        dialogVisible.value = false
      } catch (error) {
        showMessage(t('settings.proxyGroup.saveFailed', { error: error.message || error }), 'error')
      } finally {
        formLoading.value = false
      }
    }
  })
}

const handleDeleteGroup = id => {
  ElMessageBox.confirm(
    t('settings.proxyGroup.deleteConfirmText'),
    t('settings.proxyGroup.deleteConfirmTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'warning'
    }
  )
    .then(async () => {
      try {
        await proxyGroupStore.remove(id)
        showMessage(t('settings.proxyGroup.deleteSuccess'), 'success')
      } catch (error) {
        showMessage(
          t('settings.proxyGroup.deleteFailed', { error: error.message || error }),
          'error'
        )
      }
    })
    .catch(() => {})
}
</script>

<style lang="scss" scoped>
.proxy-group-container {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-lg);
}

.label-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-weight: 500;
  color: var(--cs-text-color);
  small {
    color: var(--cs-text-color-secondary);
    font-size: var(--cs-font-size-xs);
  }
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--cs-space-sm);
  color: var(--cs-text-color-secondary);
  margin: var(--cs-space-lg) auto;
  text-align: center;

  .el-button {
    display: flex;
    align-items: center;
    gap: var(--cs-space-xxs);
    .cs {
      font-size: 1.1em;
    }
  }
}

.form-container {
  max-height: calc(70vh - 120px);

  .temperature-wrap {
    display: flex;
    flex-direction: row;
    flex: 1;
    gap: var(--cs-space-md);
    box-sizing: border-box;
    padding-right: var(--cs-space-sm);
  }
}

.proxy-group-edit-dialog {
  :deep(.el-dialog__body) {
    padding-top: 0px;
    padding-bottom: 0px;
  }
  :deep(.el-dialog__footer) {
    padding-top: var(--cs-space-sm);
  }
}
</style>
