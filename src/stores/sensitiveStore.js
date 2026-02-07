import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { sendSyncState } from '@/libs/sync';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';

const windowLabel = getCurrentWebviewWindow().label;

export const useSensitiveStore = defineStore('sensitive', () => {
  const config = ref({
    enabled: true,
    common_enabled: true,
    custom_blocklist: [],
    allowlist: []
  });

  const supportedFilters = ref([]);
  const isLoading = ref(false);
  const status = ref({ healthy: true, error: null });

  const fetchStatus = async (retryCount = 0) => {
    try {
      const result = await invoke('get_sensitive_status');
      status.value = result;
      return true;
    } catch (error) {
      console.error(`Failed to fetch sensitive status (retry ${retryCount}):`, error);
      status.value = { healthy: false, error: String(error) };
      
      // Retry if backend is still initializing (max 5 retries over 2.5 seconds)
      if (retryCount < 5) {
        await new Promise(resolve => setTimeout(resolve, 500));
        return fetchStatus(retryCount + 1);
      }
      return false;
    }
  };

  const fetchConfig = async () => {
    isLoading.value = true;
    try {
      const statusOk = await fetchStatus();
      if (!statusOk) {
        console.warn('Backend not ready, using default config');
        return;
      }
      
      const data = await invoke('get_sensitive_config');
      if (data) {
        config.value = {
          enabled: data.enabled ?? true,
          common_enabled: data.common_enabled ?? true,
          custom_blocklist: data.custom_blocklist || [],
          allowlist: data.allowlist || []
        };
      }
    } catch (error) {
      console.error('Failed to fetch sensitive config:', error);
    } finally {
      isLoading.value = false;
    }
  };

  const saveConfig = async () => {
    try {
      if (!status.value.healthy) {
        config.value.enabled = false;
      }
      await invoke('update_sensitive_config', { config: config.value });
      // Broadcast change to other windows
      sendSyncState('sensitive_config_changed', windowLabel, config.value);
    } catch (error) {
      console.error('Failed to update sensitive config:', error);
      throw error;
    }
  };

  const fetchSupportedFilters = async () => {
    if (!status.value.healthy) {
      supportedFilters.value = [];
      return;
    }
    try {
      supportedFilters.value = await invoke('get_supported_filters');
    } catch (error) {
      console.error('Failed to fetch supported filters:', error);
    }
  };

  return {
    config,
    supportedFilters,
    isLoading,
    status,
    fetchConfig,
    saveConfig,
    fetchSupportedFilters,
    fetchStatus
  };
});
