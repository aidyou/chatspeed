import type { TodoItem, ToolDefinition } from '../types'

// This is a simplified in-memory implementation for now.
// In a real application, this might interact with the engine's context or a database.
const todoList: TodoItem[] = []

export const todoManagerTool: ToolDefinition = {
  id: 'todo_manager',
  name: 'TodoManager',
  description:
    'A tool to manage a list of tasks. You can add, list, and update the status of tasks.',
  parameters: {
    type: 'object',
    properties: {
      operation: {
        type: 'string',
        description: "The operation to perform: 'add', 'list', or 'update'.",
        enum: ['add', 'list', 'update']
      },
      task_id: {
        type: 'string',
        description: "The ID of the task to update. Required for 'update' operation."
      },
      task_title: {
        type: 'string',
        description: "The title of the new task. Required for 'add' operation."
      },
      new_status: {
        type: 'string',
        description: "The new status of the task. Required for 'update' operation.",
        enum: ['pending', 'in_progress', 'completed']
      }
    },
    required: ['operation']
  },
  implementation: 'typescript',
  requiresApproval: false,
  handler: async params => {
    switch (params.operation) {
      case 'add': {
        if (!params.task_title || typeof params.task_title !== 'string')
          throw new Error("'task_title' is required and must be a string for 'add' operation.")
        const newTask: TodoItem = {
          id: `task_${Date.now()}`,
          title: params.task_title,
          description: params.task_title, // Simplified for now
          status: 'pending'
        }
        todoList.push(newTask)
        return `Task '${newTask.title}' added with ID ${newTask.id}.`
      }

      case 'list': {
        return todoList.length > 0 ? todoList : 'The todo list is empty.'
      }

      case 'update': {
        if (!params.task_id || typeof params.task_id !== 'string') {
          throw new Error("'task_id' is required and must be a string for 'update' operation.")
        }
        if (
          !params.new_status ||
          !(
            typeof params.new_status === 'string' &&
            ['pending', 'in_progress', 'completed'].includes(params.new_status)
          )
        ) {
          throw new Error(
            "'new_status' is required and must be 'pending', 'in_progress', or 'completed' for 'update' operation."
          )
        }
        const taskIndex = todoList.findIndex(t => t.id === params.task_id)
        if (taskIndex === -1) {
          return `Error: Task with ID '${params.task_id}' not found.`
        }
        // Type assertion after validation
        todoList[taskIndex].status = params.new_status as 'pending' | 'in_progress' | 'completed'
        return `Task '${todoList[taskIndex].title}' updated to status '${params.new_status}'.`
      }

      default:
        throw new Error(`Invalid operation: ${params.operation}`)
    }
  }
}
