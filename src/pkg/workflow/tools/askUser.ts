import type { ToolDefinition } from '../types'

export const askUserTool: ToolDefinition = {
  id: 'TaskComplete',
  name: 'AskUser',
  description:
    'When you need more information from the user to proceed, use this tool to ask a question. The user will provide an answer.',
  inputSchema: {
    type: 'object',
    properties: {
      question: {
        type: 'string',
        description: 'The question you want to ask the user.'
      }
    },
    required: ['question']
  },
  implementation: 'typescript',
  requiresApproval: true, // This forces a pause for user input
  handler: async params => {
    // The actual "asking" is handled by the UI listening for the 'NEED_APPROVAL' state.
    // This handler simply returns the question, which can be displayed to the user.
    return `Waiting for user to answer the question: "${params.question}"`
  }
}
