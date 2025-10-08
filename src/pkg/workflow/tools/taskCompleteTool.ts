import type { ToolDefinition } from '../types'

export const taskCompleteTool: ToolDefinition = {
  id: 'task_complete',
  name: 'TaskComplete',
  description:
    "Call this tool when you have fully addressed the user's request and the task is complete. Provide the final answer to the user in the `finalAnswer` parameter.",
  inputSchema: {
    type: 'object',
    properties: {
      finalAnswer: {
        type: 'string',
        description: 'The final, complete answer to be presented to the user.'
      }
    },
    required: ['finalAnswer']
  },
  implementation: 'typescript',
  requiresApproval: false, // This tool should not require approval
  handler: async params => {
    // This tool's logic is primarily handled by the engine's onAction callback.
    // The handler itself doesn't need to do much.
    return params.finalAnswer
  }
}
