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
    const id = await invoke('proxy_group_add', { item })
    await getList()
    return id
  }

  const update = async item => {
    await invoke('proxy_group_update', { item })
    await getList()
  }

  const remove = async id => {
    await invoke('proxy_group_delete', { id })
    await getList()
  }

  return {
    list,
    getList,
    add,
    update,
    remove
  }
})
