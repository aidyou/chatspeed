<template>
  <el-container class="settings-container">
    <titlebar class="header-container" :show-maximize-button="false">
      <template #center>
        <el-menu mode="horizontal" :default-active="settingType" class="menu">
          <el-menu-item
            class="upperLayer"
            v-for="(item, index) in menuItems"
            :key="index"
            :index="item.id"
            @click="switchSetting(item.id)">
            <cs :name="item.icon" />
            <span>{{ item.label }}</span>
          </el-menu-item>
        </el-menu>
      </template>
    </titlebar>

    <el-main v-show="settingType === 'general'" class="main">
      <general />
    </el-main>

    <el-main v-show="settingType === 'model'" class="main">
      <model />
    </el-main>

    <el-main v-show="settingType === 'skill'" class="main">
      <skill />
    </el-main>

    <el-main v-show="settingType === 'privacy'" class="main">
      <privacy />
    </el-main>

    <el-main v-show="settingType === 'about'" class="main">
      <about />
    </el-main>
  </el-container>
</template>

<script setup>
import { ref, onMounted, computed } from 'vue'
import { useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'

import about from '@/components/setting/about.vue'
import general from '@/components/setting/general.vue'
import model from '@/components/setting/model.vue'
import skill from '@/components/setting/skill.vue'
import privacy from '@/components/setting/privacy.vue'
import titlebar from '@/components/window/titlebar.vue'

const { t } = useI18n()

// const settingType = ref('model')
// const settingLabel = ref(t(`settings.type.model`))
const settingType = ref('general')
const settingLabel = ref(t('settings.type.general'))
const menuItems = computed(() => [
  { label: t('settings.type.general'), icon: 'setting', id: 'general' },
  { label: t('settings.type.model'), icon: 'model', id: 'model' },
  { label: t('settings.type.skill'), icon: 'skill', id: 'skill' },
  { label: t('settings.type.privacy'), icon: 'privacy', id: 'privacy' },
  { label: t('settings.type.about'), icon: 'about', id: 'about' }
])

onMounted(async () => {
  // Switch the setting window to the user-defined type or default to 'general' if not set
  const route = useRoute()
  const queryType = route.params.type
  if (queryType) {
    const menuItem = menuItems.value.find(item => item.icon === queryType)
    if (menuItem) {
      settingType.value = menuItem.id
      settingLabel.value = menuItem.label
    }
  }
  console.log('settingType', settingType.value, route.params.type)
})

const switchSetting = id => {
  settingType.value = id
  settingLabel.value = t(`settings.type.${id}`)
}
</script>

<style lang="scss">
.settings-container {
  display: flex;
  flex-direction: column;
  height: 100vh;
  font-size: var(--cs-font-size);
  font-family: var(--cs-font-family);
  color: var(--cs-text-color-primary);
  border-radius: var(--cs-border-radius-md);

  .el-header {
    &.header-container {
      .menu {
        display: flex;
        justify-content: center;
        align-items: center;
        background: none;
        border-bottom: none;

        .el-menu-item {
          display: flex;
          flex-direction: column;
          align-items: center;
          justify-content: center;
          height: 50px;
          line-height: unset;
          margin-right: 1px;
          transition: none;
          border-radius: var(--cs-border-radius);

          &:last-child {
            margin-right: 0;
          }

          .cs {
            margin-right: 0;
            font-size: 18px !important;
          }

          span {
            font-size: var(--cs-font-size-xs);
            line-height: 1;
            color: var(--cs-text-color-secondary);
            transition: var(--el-transition-color);
          }
        }
      }

      .el-menu--horizontal {
        > .el-menu-item {
          &.is-active {
            border-bottom: none;
            // background-color: var(--cs-active-bg-color);

            .cs,
            span {
              color: var(--cs-color-primary);
            }
          }

          &:not(.is-disabled):focus,
          &:not(.is-disabled):hover {
            color: var(--cs-text-color-primary);
            background-color: transparent;
          }
        }
      }
    }
  }

  .main {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: var(--cs-space-xxs) var(--cs-space-md) var(--cs-space-md);
    display: flex;
    flex-direction: column;
  }
}
</style>
