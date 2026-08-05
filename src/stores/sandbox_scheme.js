import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/core'
import { ref } from 'vue'

export const useSandboxSchemeStore = defineStore('sandbox-scheme', () => {
  const schemes = ref([])
  const loading = ref(false)

  const fetchSchemes = async () => {
    loading.value = true
    try {
      schemes.value = await invoke('get_sandbox_schemes')
    } finally {
      loading.value = false
    }
  }

  return { schemes, loading, fetchSchemes }
})
