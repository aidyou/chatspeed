import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'

import { sendSyncState } from '@/libs/sync'


export const useNoteStore = defineStore('note', () => {
  const label = getCurrentWebviewWindow().label;
  // Used to store all note tags. Each tag,
  // in addition to containing tag information,
  // also includes a list of all notes, structured as follows:
  // [{
  //     id: 1,
  //     name: '',
  //     note_count: 0,
  //     notes: []
  // }]
  const tags = ref([])
  /**
   * Gets a list of all note tags.
   */
  const getTagList = () => {
    invoke('get_tags').then(result => {
      if (!result) {
        tags.value = []
        return
      }
      tags.value = result.map(x => ({
        id: x.id,
        name: x.name,
        note_count: x.noteCount,
        notes: [],
      }))
      console.log(tags.value)
    })
  }

  /**
   * Creates a new note with the specified details and syncs the state across windows.
   * @param {string} title - The title of the note.
   * @param {string} content - The content of the note.
   * @param {number} [conversationId] - Optional ID of the associated conversation.
   * @param {number} [messageId] - Optional ID of the associated message.
   * @param {string} tags - Comma-separated list of tags.
   * @returns {Promise<void>} A promise that resolves when the note is created and state is synced.
   */
  const addNote = (title, content, conversationId, messageId, tags, metadata) => {
    tags = tags.replace('，', ',').split(',').map(x => x.trim()).filter(x => x != '')
    console.log(metadata)
    return new Promise((resolve, reject) => invoke('add_note', { title, content, conversationId, messageId, tags, metadata })
      .then(() => {
        sendSyncState('note_update', label)
        resolve()
      }).catch(reject))
  }

  /**
   * Gets a specific note by its ID.
   * @param {number} id - The ID of the note to retrieve.
   * @returns {Promise<Object>} A promise that resolves to the note object.
   */
  const getNote = (id) => {
    return invoke('get_note', { id })
  }

  /**
   * Gets all notes associated with a specific tag.
   * @param {number} tagId - The ID of the tag to filter notes by.
   * @returns {Promise<Array>} A promise that resolves to an array of notes.
   */
  const getNotes = (tagId) => {
    return invoke('get_notes', { tagId })
  }

  /**
   * Searches for notes based on a keyword.
   * @param {string} kw - The keyword to search for in note titles.
   * @returns {Promise<Array>} A promise that resolves to an array of matching notes.
   */
  const searchNotes = (kw) => {
    return invoke('search_notes', { kw })
  }

  /**
   * Deletes a note by its ID.
   * @param {number} id - The ID of the note to delete.
   * @returns {Promise<void>} A promise that resolves when the note is deleted.
   */
  const deleteNote = (id) => {
    return invoke('delete_note', { id })
  }

  return {
    label,
    tags,
    getTagList,
    addNote,
    getNote,
    getNotes,
    deleteNote,
    searchNotes
  }
})
