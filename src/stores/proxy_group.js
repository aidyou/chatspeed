import { FrontendAppError, invokeWrapper } from '@/libs/tauri'
import { defineStore } from 'pinia'
import { reactive, ref } from 'vue'

export const useProxyGroupStore = defineStore('proxy_group', () => {
  const list = reactive([])
  const activeGroup = ref('default')

  const getList = async () => {
    try {
      const items = await invokeWrapper('proxy_group_list')
      list.splice(0, list.length, ...items)
      await getActiveGroup()
      return list
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`Failed to list proxy groups: ${error.toFormattedString()}`, error.originalError);
      } else {
        console.error('Failed to list proxy groups:', error);
      }
      throw error;
    }
  }

  const getActiveGroup = async () => {
    try {
      activeGroup.value = await invokeWrapper('get_active_proxy_group')
    } catch (error) {
      console.error('Failed to get active proxy group:', error)
    }
  }

  const setActiveGroup = async name => {
    try {
      await invokeWrapper('set_active_proxy_group', { name })
      activeGroup.value = name
    } catch (error) {
      console.error('Failed to set active proxy group:', error)
      throw error
    }
  }

  const add = async item => {
    item.id = 0
    try {
      const id = await invokeWrapper('proxy_group_add', { item })
      item.id = id
      list.push(item)
      return id
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`Failed to add proxy group: ${error.toFormattedString()}`, error.originalError);
      } else {
        console.error('Failed to add proxy group:', error);
      }
      throw error;
    }
  }

  const update = async item => {
    try {
      await invokeWrapper('proxy_group_update', { item })
      const index = list.findIndex(x => x.id === item.id)
      if (index !== -1) {
        list.splice(index, 1, item)
      }
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`Failed to update proxy group: ${error.toFormattedString()}`, error.originalError);
      } else {
        console.error('Failed to update proxy group:', error);
      }
      throw error;
    }
  }

  const batchUpdate = async payload => {
    try {
      await invokeWrapper('proxy_group_batch_update', payload)
      await getList() // Refresh the list after batch update
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`Failed to batch update proxy groups: ${error.toFormattedString()}`, error.originalError);
      } else {
        console.error('Failed to batch update proxy groups:', error);
      }
      throw error;
    }
  }

  const remove = async id => {
    try {
      await invokeWrapper('proxy_group_delete', { id })
      const index = list.findIndex(x => x.id === id)
      if (index !== -1) {
        list.splice(index, 1)
      }
    } catch (error) {
      if (error instanceof FrontendAppError) {
        console.error(`Failed to delete proxy group: ${error.toFormattedString()}`, error.originalError);
      } else {
        console.error('Failed to delete proxy group:', error);
      }
      throw error;
    }
  }

  return {
    list,
    activeGroup,
    getList,
    getActiveGroup,
    setActiveGroup,
    add,
    update,
    batchUpdate,
    remove
  }
})
