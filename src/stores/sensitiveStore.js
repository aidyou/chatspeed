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

  const fetchConfig = async () => {
    isLoading.value = true;
    try {
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
      await invoke('update_sensitive_config', { config: config.value });
      // Broadcast change to other windows
      sendSyncState('sensitive_config_changed', windowLabel, config.value);
    } catch (error) {
      console.error('Failed to update sensitive config:', error);
      throw error;
    }
  };

  const fetchSupportedFilters = async () => {
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
    fetchConfig,
    saveConfig,
    fetchSupportedFilters
  };
});
