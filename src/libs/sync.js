import { invoke } from '@tauri-apps/api/core'

/**
 * Sends the current synchronization state to the backend.
 */
export const sendSyncState = (syncType, windowLabel, metadata = {}) => {
  invoke('sync_state', { syncType, windowLabel, metadata })
    .catch((err) => {
      console.error('sendSyncState error:', err);
    });
}