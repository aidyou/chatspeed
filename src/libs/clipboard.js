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
 * Prefer browser clipboard API for user-triggered copy actions.
 * Falls back to Rust-based clipboard implementation via arboard when needed.
 * This avoids Linux/X11 ownership warnings from very short-lived native clipboard instances.
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
  if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text)
      return
    } catch (error) {
      console.warn('Browser clipboard write failed, falling back to native clipboard:', error)
    }
  }

  return await invoke('write_clipboard', { text })
}
