import { FrontendAppError, invokeWrapper } from '@/libs/tauri'
import { defineStore } from 'pinia'
import { reactive } from 'vue'

export const useProxyGroupStore = defineStore('proxy_group', () => {
  const list = reactive([])

  const getList = async () => {
    try {
      const items = await invokeWrapper('proxy_group_list')
      list.splice(0, list.length, ...items)
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
    getList,
    add,
    update,
    remove
  }
})
