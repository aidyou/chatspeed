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
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

/**
 * Calls the backend to get a streaming chat completion from the specified LLM.
 * It processes the stream and invokes the appropriate handlers.
 * Includes a retry mechanism with exponential backoff for transient errors.
 * @param request The request payload for the LLM.
 * @param handlers The callback handlers for stream events.
 * @returns A promise that resolves when the stream is complete or rejects on error.
 */
export async function callLLM(request: LLMRequest, handlers: LLMStreamHandlers): Promise<void> {
  const maxRetries = 10
  const initialDelay = 1000 // 1 second

  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    try {
      // The core logic is wrapped in a promise to work with the retry loop
      await new Promise<void>((resolve, reject) => {
        let unlisten: (() => void) | undefined

        const cleanupAndResolve = () => {
          unlisten?.()
          resolve()
        }

        // Rejects the promise, which will be caught by the outer try/catch of the loop
        const triggerRetryOrFailure = (error: object) => {
          unlisten?.()
          reject(error) // This reject is caught by the loop's catch block
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
                  let error: object = { status: 500, message: 'Unknown stream error' }
                  if (typeof payload?.chunk === 'string') {
                    try {
                      const j = JSON.parse(payload.chunk)
                      // Accommodate the new standard { "error": { "status": ..., "message": ... } }
                      if (j.error && j.error.status && j.error.message) {
                        error = {
                          status: j.error.status,
                          message: j.error.message
                        }
                      } else { // Fallback for older or different error structures
                         error = {
                           status: j.status || 500,
                           message: j.details || j.error || j.message || 'Failed to parse error object'
                         }
                      }
                    } catch {
                      error = { status: 500, message: payload.chunk || 'Unknown stream error' }
                    }
                  }
                  triggerRetryOrFailure(error)
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
                    triggerRetryOrFailure(error)
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
                    triggerRetryOrFailure(error)
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
            // This catches errors from the initial invoke call (e.g., network errors)
            triggerRetryOrFailure(e instanceof Error ? e : new Error('Failed to initiate LLM call'))
          }
        })()
      })

      // If the promise above resolves, it means success, so we can exit the loop.
      return
    } catch (error: any) {
      console.warn(`LLM call attempt ${attempt + 1} failed.`, error)

      const status = error?.status
      const isRetryable =
        status === 429 || (error instanceof Error && error.message?.includes('NetworkError')) // Add other retryable conditions here
      const isFatal = status != null && [401, 403, 404, 410].includes(status)

      if (isFatal) {
        console.error(`Fatal error (${status}) received from LLM. Aborting.`)
        if (handlers.onError)
          handlers.onError(error instanceof Error ? error : new Error(String(error)))
        throw error // Re-throw the fatal error to stop the entire workflow
      }

      if (isRetryable && attempt < maxRetries) {
        const delay = initialDelay * Math.pow(2, attempt)
        console.log(`Rate limit or network error. Retrying in ${delay}ms...`)
        await sleep(delay)
      } else if (attempt >= maxRetries) {
        console.error('LLM call failed after maximum retries.')
        if (handlers.onError) handlers.onError(error)
        throw new Error('LLM call failed after maximum retries.')
      } else {
        // For non-retryable errors that are not fatal, fail immediately
        if (handlers.onError) handlers.onError(error)
        throw error
      }
    }
  }
}
