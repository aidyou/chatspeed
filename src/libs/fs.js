import { invoke } from '@tauri-apps/api/core'

/**
 * Convert file path to data URL
 * @param {string} path - file path
 * @returns {Promise<string | null>} - data URL or null
 */
export const imagePreview = async (path) => {
  try {
    return await invoke('image_preview', { imagePath:path });
  } catch (error) {
    console.error(error);
    return null;
  }
};


/**
 * Get file extension
 * @param {string} path - file path
 * @returns {string} - file extension
 */
export function getFileExtension(path) {
  const lastDotIndex = path.lastIndexOf('.');
  if (lastDotIndex === -1) {
    return '';
  }
  return path.substring(lastDotIndex + 1).toLowerCase();
}
