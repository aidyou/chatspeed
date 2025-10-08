/**
 * LLM Service Module
 * Handles communication with the Large Language Model and parsing of its responses.
 */

import { invoke } from '@tauri-apps/api/core'
import { emit, listen } from '@tauri-apps/api/event'
import type {
  ChatResponse,
  LLMStreamHandlers,
  ParsedLLMResponse,
  ToolCalls,
  WorkflowMessage
} from './types'
import { LLMResponseType } from './types'

/**
 * Represents the request payload for an LLM chat completion call.
 */
export interface LLMRequest extends Record<string, unknown> {
  providerId: number
  modelId: string
  messages: WorkflowMessage[]
  temperature?: number
  availableTools?: string[]
  tsTools?: object[]
}

/**
 * Type guard to check if an object conforms to the ToolCalls interface.
 * @param obj The object to check.
 * @returns True if the object is a valid ToolCalls object.
 */
function isToolCalls(obj: unknown): obj is ToolCalls {
  if (obj == null || typeof obj !== 'object') {
    return false
  }
  if (!('function' in obj) || obj.function == null || typeof obj.function !== 'object') {
    return false
  }
  return 'name' in obj.function && 'arguments' in obj.function
}

/**
 * Calls the backend to get a streaming chat completion from the specified LLM.
 * It processes the stream and invokes the appropriate handlers.
 * @param request The request payload for the LLM.
 * @param handlers The callback handlers for stream events.
 * @returns A promise that resolves when the stream is complete or rejects on error.
 */
export async function callLLM(request: LLMRequest, handlers: LLMStreamHandlers): Promise<void> {
  return new Promise((resolve, reject) => {
    let unlisten: (() => void) | undefined

    const cleanupAndResolve = () => {
      unlisten?.()
      resolve()
    }

    const cleanupAndReject = (error: object) => {
      unlisten?.()
      if (handlers.onError) handlers.onError(error)
      reject(error)
    }

    ;(async () => {
      try {
        const streamId: string = await invoke('workflow_chat_completion', request)

        let reasoning = ''
        let content = ''
        unlisten = await listen(`workflow_stream://${streamId}`, event => {
          const payload = event.payload as ChatResponse

          switch (payload.type) {
            case LLMResponseType.Error: {
              // const error = new Error(payload.chunk || 'Unknown stream error')
              let error: object = {}
              if (typeof payload?.chunk === 'string') {
                try {
                  const j = JSON.parse(payload.chunk)
                  error = {
                    status: j.status || 'N/A',
                    message: j.details || j.error || 'Unknown stream error'
                  }
                } catch {
                  error = { status: 'N/A', message: payload.chunk || 'Unknown stream error' }
                }
              }
              cleanupAndReject(error)
              break
            }
            case LLMResponseType.Finished: {
              if (handlers.onDone) handlers?.onDone({ reasoning, content })
              cleanupAndResolve()
              break
            }
            case LLMResponseType.ToolCalls: {
              let parsedChunk: unknown
              try {
                parsedChunk =
                  typeof payload.chunk === 'string' ? JSON.parse(payload.chunk) : payload.chunk
              } catch (e: unknown) {
                const errorMessage =
                  e instanceof Error
                    ? e.message
                    : 'Unknown error occurred while parsing tool calls JSON'
                const error = new Error(`Failed to parse tool calls JSON: ${errorMessage}`)
                cleanupAndReject(error)
                return
              }

              if (isToolCalls(parsedChunk)) {
                const action: ToolCalls = parsedChunk // Type is now guaranteed
                if (handlers.onAction) {
                  handlers.onAction({
                    name: action.function?.name,
                    arguments: action.function?.arguments
                  } as ParsedLLMResponse['action'])
                }
              } else {
                const error = new Error('Invalid ToolCalls structure received')
                cleanupAndReject(error)
              }
              break
            }

            case LLMResponseType.Reasoning: {
              reasoning += payload.chunk
              if (handlers.onReasoning) handlers.onReasoning(payload.chunk)
              break
            }

            case LLMResponseType.Text: {
              content += payload.chunk
              if (handlers.onContent) handlers.onContent(payload.chunk)
              break
            }
          }
        })

        // Signal to the backend that the listener is ready
        await emit('frontend_ready_for_stream', { streamId })
      } catch (e: unknown) {
        const error = e instanceof Error ? e : new Error('Failed to initiate LLM call')
        cleanupAndReject(error)
      }
    })()
  })
}
