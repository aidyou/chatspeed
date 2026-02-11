import { tool, jsonSchema } from 'ai'
import type { ToolDefinition } from './types'
import type { ToolRegistry } from './toolRegistry'

/**
 * Tool Adapter Module
 * Converts Chatspeed's legacy ToolDefinition into Vercel AI SDK compatible tool objects.
 */

/**
 * Wraps a legacy ToolDefinition into an AI SDK tool.
 *
 * @param toolDef - The original tool definition containing description and JSON schema.
 * @param registry - The registry instance to handle actual execution.
 * @returns A standard tool object for use with generateText/streamText.
 */
export function adaptToSdkTool(toolDef: ToolDefinition, registry: ToolRegistry) {
  return tool({
    description: toolDef.description,
    // Use jsonSchema helper as our legacy tools already use JSON Schema
    parameters: jsonSchema(toolDef.inputSchema),
    execute: async params => {
      const result = await registry.execute({
        toolId: toolDef.id,
        parameters: params as Record<string, unknown>
      })

      if (!result.success) {
        // Rethrow error so the AI SDK can handle it as a tool failure
        throw new Error(result.error || 'Unknown tool execution error')
      }

      return result.result
    }
  })
}

/**
 * Batch converts all tools from a registry.
 *
 * @param registry - The tool registry instance.
 * @returns A record of tools mapped by their names.
 */
export function getAllSdkTools(registry: ToolRegistry) {
  const sdkTools: Record<string, any> = {}
  const allTools = registry.getAll()

  for (const toolDef of allTools) {
    // We use the tool's name as the key for the AI model to identify it
    sdkTools[toolDef.name] = adaptToSdkTool(toolDef, registry)
  }

  return sdkTools
}
