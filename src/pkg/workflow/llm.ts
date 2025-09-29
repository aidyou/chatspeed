/**
 * LLM Service Module
 * Handles communication with the Large Language Model and parsing of its responses.
 */

import { invoke } from '@tauri-apps/api/core'
import type { ParsedLLMResponse, WorkflowMessage } from './types'

/**
 * Represents the request payload for an LLM chat completion call.
 */
export interface LLMRequest {
  providerId: number | string
  modelId: string
  messages: WorkflowMessage[]
  temperature?: number
  [key: string]: unknown // Allow additional properties for invoke
}

/**
 * Calls the backend to get a chat completion from the specified LLM.
 * @param request The request payload for the LLM.
 * @returns The raw string response from the LLM.
 */
export async function callLLM(request: LLMRequest): Promise<string> {
  // This assumes a backend command `execute_chat_completion` exists.
  // We will need to implement this command in Rust later.
  try {
    const response = await invoke('execute_chat_completion', request)
    return response as string
  } catch (error) {
    console.error('Error calling LLM:', error)
    throw new Error('Failed to get response from LLM.')
  }
}

/**
 * Parses the raw text response from an LLM to extract structured thought and action.
 * It looks for a JSON block enclosed in ```json ... ``` for the action.
 * @param responseText The raw text from the LLM.
 * @returns A structured object with thought and/or action.
 */
export function parseLLMResponse(responseText: string): ParsedLLMResponse {
  const result: ParsedLLMResponse = {}

  // Regular expression to find the action JSON block
  const actionRegex = /```json\n([\s\S]*?)\n```/
  const match = responseText.match(actionRegex)

  if (match?.[1]) {
    try {
      result.action = JSON.parse(match[1])
      // The text before the action block is considered the thought
      result.thought = responseText.substring(0, match.index).trim()
    } catch (error) {
      console.error('Failed to parse action JSON:', error)
      // If parsing fails, treat the whole response as a final answer
      result.finalAnswer = responseText
    }
  } else {
    // If no action block is found, the entire response is the final answer
    result.finalAnswer = responseText.trim()
  }

  return result
}
