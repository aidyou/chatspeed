/**
 * Tool Registry and Dispatcher
 * Manages all available tools and handles their execution.
 */

import { invoke } from '@tauri-apps/api/core'
import type { ToolDefinition, ToolExecutionRequest, ToolExecutionResult } from './types'

export class ToolRegistry {
  private tools: Map<string, ToolDefinition> = new Map()

  /**
   * Registers a new tool.
   * @param tool The tool definition to register.
   */
  public register(tool: ToolDefinition): void {
    if (this.tools.has(tool.id)) {
      console.warn(`Tool with id '${tool.id}' is already registered. Overwriting.`)
    }
    this.tools.set(tool.id, tool)
  }

  /**
   * Registers multiple tools at once.
   * @param tools An array of tool definitions.
   */
  public registerAll(tools: ToolDefinition[]): void {
    for (const tool of tools) {
      this.register(tool)
    }
  }

  /**
   * Retrieves a tool definition by its ID.
   * @param toolId The ID of the tool to retrieve.
   * @returns The tool definition or undefined if not found.
   */
  public get(toolId: string): ToolDefinition | undefined {
    return this.tools.get(toolId)
  }

  /**
   * Gets a list of all registered tools.
   * @returns An array of all tool definitions.
   */
  public getAll(): ToolDefinition[] {
    return Array.from(this.tools.values())
  }

  /**
   * Gets the names of all registered tools.
   * @returns An array of tool names.
   */
  public getToolNames(): string[] {
    return Array.from(this.tools.values()).map(tool => tool.name)
  }

  /**
   * Gets the declarations of all registered tools in a format suitable for the LLM.
   * @returns An array of tool declaration objects.
   */
  public getToolDeclarations(): object[] {
    return Array.from(this.tools.values()).map(tool => ({
      name: tool.name,
      description: tool.description,
      inputSchema: tool.inputSchema
    }))
  }

  /**
   * Executes a tool based on its implementation type (Unified Dispatcher).
   * @param request The tool execution request.
   * @returns A promise that resolves with the execution result.
   */
  public async execute(request: ToolExecutionRequest): Promise<ToolExecutionResult> {
    const tool = this.get(request.toolId)

    if (!tool) {
      return {
        success: false,
        error: `Tool with id '${request.toolId}' not found.`
      }
    }

    try {
      let result: unknown
      switch (tool.implementation) {
        case 'rust':
          // Assumes the Rust command name matches the tool ID
          result = await invoke(tool.id, request.parameters)
          break

        case 'typescript':
        case 'browser':
          if (!tool.handler) {
            throw new Error(
              `Tool '${tool.id}' is a '${tool.implementation}' type but has no handler.`
            )
          }
          result = await tool.handler(request.parameters)
          break

        default:
          throw new Error(`Unsupported tool implementation type: ${tool.implementation}`)
      }

      return { success: true, result }
    } catch (e: unknown) {
      console.error(`Error executing tool '${tool.id}':`, e)
      const errorMessage = e instanceof Error ? e.message : String(e)
      return { success: false, error: errorMessage }
    }
  }
}
