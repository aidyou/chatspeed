import i18n from '@/i18n';
import { FrontendAppError, invokeWrapper } from '@/libs/tauri';
import { defineStore } from 'pinia';
import { ref } from 'vue';

import { csStorageKey } from '@/config/config';
import { csGetStorage, csSetStorage, isEmpty } from '@/libs/util';

let isConversationLoading = false

/**
 * useChatStore defines a store for managing chat messages.
 * It includes state for the list of chat messages and related operations.
 */
export const useChatStore = defineStore('chat', () => {
  const conversations = ref([])
  /**
   * Loads all conversations from the database and updates the state.
   * This function is called to refresh the list of conversations.
   */
  const loadConversations = () => {
    if (isConversationLoading) {
      return
    }
    isConversationLoading = true
    invokeWrapper('get_all_conversations')
      .then((result) => {
        console.log('conversations', result);
        // Assuming result is an array of conversations
        conversations.value = isEmpty(result) ? [] : [...result];
      })
      .catch((error) => {
        if (error instanceof FrontendAppError) {
          console.error(`Error loading conversations: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Error loading conversations:', error);
        }
      })
      .finally(() => {
        isConversationLoading = false
      });
  }

  const currentConversationId = ref(csGetStorage(csStorageKey.currentConversationId, 0))
  /**
   * Retrieves the current conversation ID from storage or creates a new one if none exists.
   * @returns {Promise<number>} A promise that resolves to the current conversation ID.
   */
  const getCurrentConversationId = () => {
    return new Promise((resolve, reject) => {
      if (currentConversationId.value) {
        resolve(currentConversationId.value)
      } else {
        createConversation().then((conversation) => {
          setCurrentConversationId(conversation.id)
          resolve(conversation.id)
        }).catch((error) => {
          if (error instanceof FrontendAppError) {
            console.error(`Error getting current conversation ID: ${error.toFormattedString()}`, error.originalError);
          } else {
            console.error('Error getting current conversation ID:', error);
          }
          reject(error)
        })
      }
    })
  }

  /**
   * Sets the current conversation ID in storage and updates the state.
   * @param {number} id - The ID of the conversation to set as current.
   */
  const setCurrentConversationId = (id) => {
    csSetStorage(csStorageKey.currentConversationId, id)
    currentConversationId.value = id
  }

  /**
   * Creates a new conversation with a generated title and adds it to the state.
   * @returns {Promise<Conversation>} A promise that resolves to the created conversation object.
   */
  const createConversation = () => {
    return new Promise((resolve, reject) => {
      let maxId = 0
      if (conversations.value.length > 0) {
        conversations.value.forEach(conversation => {
          if (conversation.id > maxId) {
            maxId = conversation.id
          }
        })
      }
      const title = `${i18n.global.t(`chat.conversation`)} ${maxId + 1}`;
      invokeWrapper('add_conversation', { title }).then((conversationId) => {
        if (conversationId) {
          setCurrentConversationId(conversationId)
          const conversation = { id: conversationId, title, isFavorite: false, createdAt: new Date().toLocaleString() }
          conversations.value.unshift(conversation)

          // clear messages
          messages.value.length = 0

          resolve(conversation)
        } else {
          reject(new Error('Failed to create conversation: No ID returned'))
        }
      }).catch((error) => {
        if (error instanceof FrontendAppError) {
          console.error(`Error creating conversation: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Error creating conversation:', error);
        }
        reject(error)
      })
    });
  }

  /**
   * Updates the favorite status of a conversation.
   * @param {number} id - The ID of the conversation to update.
   * @param {string} title - The new title of the conversation.
   * @param {boolean} isFavorite - The new favorite status.
   * @returns {Promise<void>} A promise that resolves when the update is complete.
   */
  const updateConversation = (id, title, isFavorite) => {
    return new Promise((resolve, reject) => {
      invokeWrapper('update_conversation', { id, title: title || null, isFavorite: isFavorite || null }).then(() => {
        const conversationToUpdate = conversations.value.find(conversation => conversation.id === id);
        if (conversationToUpdate) {
          if (title) {
            conversationToUpdate.title = title;
          }
          if (isFavorite !== undefined && isFavorite !== null) {
            conversationToUpdate.isFavorite = isFavorite;
          }
        }
        resolve()
      }).catch((error) => {
        if (error instanceof FrontendAppError) {
          console.error(`Error updating conversation favorite: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Error updating conversation favorite:', error);
        }
        reject(error)
      })
    })
  }

  /**
   * Deletes a conversation by its ID and updates the state.
   * @param {number} id - The ID of the conversation to delete.
   * @returns {Promise<void>} A promise that resolves when the deletion is complete.
   */
  const deleteConversation = (id) => {
    return new Promise((resolve, reject) => {
      invokeWrapper('delete_conversation', { id }).then(() => {
        conversations.value = conversations.value.filter((conversation) => conversation.id !== id)
        setCurrentConversationId(conversations.value[0]?.id || 0)
        resolve()
      }).catch((error) => {
        if (error instanceof FrontendAppError) {
          console.error(`Error deleting conversation: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Error deleting conversation:', error);
        }
        reject(error)
      })
    })
  }

  let isMessagesLoading = false
  const messages = ref([])
  /**
   * Loads messages for a specific conversation from the database.
   * @param {number} conversationId - The ID of the conversation to load messages for.
   */
  const loadMessages = async (conversationId, windowLabel) => {
    if (isMessagesLoading) {
      return
    }
    isMessagesLoading = true
    messages.value.length = 0
    return new Promise((resolve, reject) => {
      invokeWrapper('get_messages_for_conversation', { conversationId, windowLabel }).then(() => {
        console.log('loadMessages', windowLabel)
        resolve()
      }).catch((error) => {
        if (error instanceof FrontendAppError) {
          console.error(`Error loading messages: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Error loading messages:', error);
        }
        reject(error)
      })
        .finally(() => {
          isMessagesLoading = false
        })
    })
  }

  /**
   * Appends messages to the state.
   * @param {Object} message - The message to append.
   */
  const appendMessage = (message) => {
    if (isEmpty(message)) {
      return
    }
    messages.value = [...messages.value, message]
  }

  /**
   * Adds a new message to a conversation and updates the state.
   * If the messageId is provided, it do nothing.
   *
   * @param {number} conversationId - The ID of the conversation to add the message to.
   * @param {string} role - The role of the sender (e.g., 'user', 'bot').
   * @param {string} content - The content of the message.
   * @param {object} metadata - The metadata of the message.
   * @param {number} messageId - The ID of the message to update.
   * @returns {Promise<number>} A promise that resolves to the ID of the added message.
   */
  const addChatMessage = (conversationId, role, content, metadata = {}, messageId = null) => {
    return new Promise((resolve, reject) => {
      if (messageId && messages.value.length > 0) {
        console.log('resend message:', messages.value[messages.value.length - 1]?.id, messageId)
        if (messages.value[messages.value.length - 1].id === messageId) {
          return resolve(messageId)
        }
      }
      invokeWrapper('add_message', { conversationId, role, content, metadata })
        .then((result) => {
          let messageId = result
          let finalContent = content

          // Check if result is an array [id, content] (new format with filtering)
          if (Array.isArray(result) && result.length === 2) {
            messageId = result[0]
            finalContent = result[1]
          }

          messages.value = [...messages.value, {
            id: messageId,
            conversationId,
            role,
            content: finalContent,
            metadata
          }]
          resolve(messageId)
        })
        .catch((error) => {
          if (error instanceof FrontendAppError) {
            console.error(`Error adding message: ${error.toFormattedString()}`, error.originalError);
          } else {
            console.error('Error adding message:', error);
          }
          reject(error)
        })
    })
  }

  /**
   * Deletes a message by its ID and updates the state.
   * @param {Array<number>} ids - The IDs of the messages to delete.
   * @returns {Promise<void>} A promise that resolves when the deletion is complete.
   */
  const deleteMessage = (id) => {
    console.debug('delete message', id)
    return new Promise((resolve, reject) => {
      invokeWrapper('delete_message', { id }).then(() => {
        messages.value = messages.value.filter((message) => !id.includes(message.id))
        resolve()
      }).catch((error) => {
        if (error instanceof FrontendAppError) {
          console.error(`Error deleting message: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Error deleting message:', error);
        }
        reject(error)
      })
    })
  }

  const clearContext = () => {
    return new Promise((resolve, reject) => {
      // if the last message is context cleared or messages is empty, then do nothing
      if (messages.value.length === 0) return resolve()

      const lastMessage = messages.value[messages.value.length - 1]
      if (lastMessage.metadata?.contextCleared) return resolve()

      lastMessage.metadata = { ...lastMessage?.metadata, contextCleared: true }
      invokeWrapper('update_message_metadata', { id: lastMessage.id, metadata: lastMessage.metadata }).then(() => {
        lastMessage.metadata.contextCleared = true
        resolve()
      }).catch((error) => {
        if (error instanceof FrontendAppError) {
          console.error(`Error clearing context: ${error.toFormattedString()}`, error.originalError);
        } else {
          console.error('Error clearing context:', error);
        }
        reject(error)
      })
    })
  }

  return {
    conversations,
    loadConversations,
    currentConversationId,
    getCurrentConversationId,
    setCurrentConversationId,
    createConversation,
    deleteConversation,
    updateConversation,
    messages,
    appendMessage,
    loadMessages,
    addChatMessage,
    deleteMessage,
    clearContext
  }
});
