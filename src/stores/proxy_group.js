import { defineStore } from 'pinia'
import { reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export const useProxyGroupStore = defineStore('proxy_group', () => {
  const list = reactive([])

  const getList = async () => {
    const items = await invoke('proxy_group_list')
    list.splice(0, list.length, ...items)
    return list
  }

  const add = async item => {
    item.id = 0
    await invoke('proxy_group_add', { item })
      .then(id => {
        item.id = id
        list.push(item)
        return id
      })
      .catch(error => {
        console.error('Failed to add proxy group:', error)
      })
  }

  const update = async item => {
    await invoke('proxy_group_update', { item }).then(() => {
      const index = list.findIndex(x => x.id === item.id)
      if (index !== -1) {
        list.splice(index, 1, item)
      }
    })
  }

  const remove = async id => {
    await invoke('proxy_group_delete', { id }).then(() => {
      const index = list.findIndex(x => x.id === id)
      if (index !== -1) {
        list.splice(index, 1)
      }
    })
  }

  return {
    list,
    getList,
    add,
    update,
    remove
  }
})
