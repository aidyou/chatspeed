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
 * Copy original image to local static server and return source URL
 * @param {string} path - file path
 * @returns {Promise<string | null>} - source URL or null
 */
export const imageSourceUrl = async (path) => {
  try {
    return await invoke('image_source_url', { imagePath: path })
  } catch (error) {
    console.error(error)
    return null
  }
}


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
