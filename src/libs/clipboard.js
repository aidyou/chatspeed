import { invoke } from '@tauri-apps/api/core'

/**
 * Read text content from system clipboard
 * 
 * Uses Rust-based clipboard implementation via arboard for cross-platform support.
 * This avoids browser clipboard API limitations and permission issues.
 * 
 * @returns {Promise<string>} Clipboard text content
 * @throws {Error} If clipboard access fails or content cannot be read
 * 
 * @example
 * try {
 *   const text = await readClipboard()
 *   console.log('Clipboard content:', text)
 * } catch (error) {
 *   console.error('Failed to read clipboard:', error)
 * }
 */
export const readClipboard = async () => {
  return await invoke('read_clipboard')
}

/**
 * Write text content to system clipboard
 * 
 * Uses Rust-based clipboard implementation via arboard for cross-platform support.
 * This avoids browser clipboard API limitations and permission issues.
 * 
 * @param {string} text - The text content to write to clipboard
 * @returns {Promise<void>} Resolves when text is written successfully
 * @throws {Error} If clipboard access fails or content cannot be written
 * 
 * @example
 * try {
 *   await writeClipboard('Hello, World!')
 *   console.log('Text copied to clipboard')
 * } catch (error) {
 *   console.error('Failed to write to clipboard:', error)
 * }
 */
export const writeClipboard = async (text) => {
  return await invoke('write_clipboard', { text })
}