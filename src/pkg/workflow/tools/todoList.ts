import type { TodoItem, ToolDefinition } from '../types'
import { updateWorkflowTodoList } from '../api'

// Store todo lists per workflow
const workflowTodoLists: Map<string, TodoItem[]> = new Map()

/**
 * Get the todo list for a specific workflow
 * @param workflowId The workflow ID
 * @returns The todo list for the workflow
 */
export function getTodoListForWorkflow(workflowId: string): TodoItem[] {
  if (!workflowTodoLists.has(workflowId)) {
    workflowTodoLists.set(workflowId, [])
  }
  return workflowTodoLists.get(workflowId) ?? []
}

/**
 * Set the todo list for a specific workflow
 * @param workflowId The workflow ID
 * @param todos The todo list to set
 */
export function setTodoListForWorkflow(workflowId: string, todos: TodoItem[]): void {
  workflowTodoLists.set(workflowId, todos)
  // Persist the todo list to the database
  persistTodoList(workflowId, todos)
}

/**
 * Persist todo list to database
 * @param workflowId The workflow ID
 * @param todos The todo list to persist
 */
async function persistTodoList(workflowId: string, todos: TodoItem[]): Promise<void> {
  try {
    const todoListJson = JSON.stringify(todos)
    await updateWorkflowTodoList(workflowId, todoListJson)
  } catch (error) {
    console.error('Failed to persist todo list:', error)
  }
}

// Store the current workflow ID for todo manager
let currentWorkflowId: string | null = null

/**
 * Set the current workflow ID for the todo manager
 * @param workflowId The workflow ID
 */
export function setTodoListWorkflowId(workflowId: string): void {
  currentWorkflowId = workflowId
}

export const todoListTool: ToolDefinition = {
  id: 'TodoList',
  name: 'TodoList',
  description: `Use this tool to create and manage a structured task list for your current session. This helps you track progress, organize complex tasks, and demonstrate thoroughness to the user. It also helps the user understand the progress of the task and overall progress of their requests.

## When to Use This Tool
Use this tool proactively in these scenarios:

1. Complex multi-step tasks - When a task requires 3 or more distinct steps or actions
2. Non-trivial and complex tasks - Tasks that require careful planning or multiple operations
3. User explicitly requests todo list - When the user directly asks you to use the todo list
4. User provides multiple tasks - When users provide a list of things to be done (numbered or comma-separated)
5. After receiving new instructions - Immediately capture user requirements as todos
6. When you start working on a task - Mark it as in_progress BEFORE beginning work. Ideally you should only have one todo as in_progress at a time
7. After completing a task - Mark it as completed and add any new follow-up tasks discovered during implementation

## When NOT to Use This Tool

Skip using this tool when:
1. There is only a single, straightforward task
2. The task is trivial and tracking it provides no organizational benefit
3. The task can be completed in less than 3 trivial steps
4. The task is purely conversational or informational

## Task States and Management

1. **Task States**: Use these states to track progress:
   - pending: Task not yet started
   - in_progress: Currently working on (limit to ONE task at a time)
   - completed: Task finished successfully
   - data_missing: Failure due to insufficient data, automatically advancing to the next task
   - failed: Critical failure that halts the workflow

2. **Task Management**:
   - Update task status in real-time as you work
   - Mark tasks complete IMMEDIATELY after finishing (don't batch completions)
   - Only have ONE task in_progress at any time
   - Complete current tasks before starting new ones
   - Remove tasks that are no longer relevant from the list entirely

3. **Task Completion Requirements**:
   - ONLY mark a task as completed when you have FULLY accomplished it
   - If you encounter errors, blockers, or cannot finish, keep the task as in_progress
   - When blocked, create a new task describing what needs to be resolved
   - Never mark a task as completed if tests are failing, implementation is partial, or you encountered unresolved errors

4. **Task Breakdown**:
   - Create specific, actionable items
   - Break complex tasks into smaller, manageable steps
   - Use clear, descriptive task names

When in doubt, use this tool. Being proactive with task management demonstrates attentiveness and ensures you complete all requirements successfully.

## Examples of When to Use the Todo List

<example>
User: I need to plan and execute a product launch campaign. This includes market research, content creation, social media promotion, and performance tracking.
Assistant: I'll help you plan and execute this product launch campaign. Let me create a todo list to track all the necessary steps.
*Creates todo list with items like: 1) Conduct market research and competitor analysis, 2) Develop campaign messaging and content strategy, 3) Create promotional materials, 4) Set up social media campaigns, 5) Implement performance tracking system*
Let's start with conducting market research and competitor analysis.

<reasoning>
The assistant used the todo list because:
1. This is a complex multi-step project requiring different types of activities
2. The user provided multiple components that need to be coordinated
3. The todo list helps organize the campaign into manageable phases
4. This approach ensures all aspects of the launch are covered systematically
</reasoning>
</example>

<example>
User: Can you help me reorganize my home office? I need to declutter, set up better storage, improve lighting, and create an ergonomic workspace.
Assistant: I'll help you reorganize your home office for better productivity and comfort. Let me create a todo list to track this reorganization project.
*Creates todo list with items like: 1) Sort and declutter existing items, 2) Plan storage solutions and layout, 3) Improve lighting setup, 4) Arrange ergonomic workspace, 5) Test and adjust the new setup*
Let's start by sorting and decluttering your existing items.

<reasoning>
The assistant used the todo list because:
1. This is a multi-step home organization project with physical and planning components
2. The user listed several distinct areas that need attention
3. The todo list helps ensure a systematic approach to reorganization
4. This method prevents overlooking important aspects of the workspace setup
</reasoning>
</example>

## Examples of When NOT to Use the Todo List

<example>
User: What's the weather like today?
Assistant: I can check the current weather conditions for you. Let me get that information.

The current weather is sunny with a high of 75°F and low of 60°F, with light winds from the northwest.

<reasoning>
The assistant did not use the todo list because this is a simple information request that can be answered immediately with a single step. There's no need for task tracking or organization.
</reasoning>
</example>

<example>
User: Can you explain the difference between marketing and advertising?
Assistant: Marketing is the broader process of promoting, selling, and distributing products or services, while advertising is a specific component of marketing that involves paid promotional messages. Marketing includes market research, product development, pricing strategy, and distribution channels, whereas advertising focuses on creating and placing paid messages to reach target audiences.

<reasoning>
The assistant did not use the todo list because this is an informational explanation that doesn't require multiple steps or task management. The user is asking for knowledge, not for the assistant to perform a complex task.
</reasoning>
</example>`,

  inputSchema: {
    type: 'object',
    properties: {
      operation: {
        type: 'string',
        description: "The operation to perform: 'add', 'add_batch', 'list', or 'update'.",
        enum: ['add', 'add_batch', 'list', 'update']
      },
      id: {
        type: 'string',
        description: "The ID of the task to update. Required for 'update' operation."
      },
      title: {
        type: 'string',
        description: "The title of the new task. Required for 'add' operation."
      },
      titles: {
        type: 'array',
        description: "Array of task titles for batch add. Required for 'add_batch' operation.",
        items: {
          type: 'string'
        }
      },
      status: {
        type: 'string',
        description:
          "The new status of the task, required for 'update'. 'data_missing' signifies a failure due to insufficient data, automatically advancing to the next task. 'failed' signifies a critical failure that halts the workflow.",
        enum: ['pending', 'in_progress', 'completed', 'data_missing', 'failed']
      }
    },
    required: ['operation']
  },
  implementation: 'typescript',
  requiresApproval: false,
  handler: async params => {
    if (!currentWorkflowId) {
      throw new Error('Todo manager is not initialized with a workflow ID.')
    }

    const todos = getTodoListForWorkflow(currentWorkflowId)

    switch (params.operation) {
      case 'add': {
        if (!params.title || typeof params.title !== 'string')
          throw new Error("'title' is required and must be a string for 'add' operation.")
        const newTask: TodoItem = {
          id: `task_${Date.now()}`,
          title: params.title,
          description: params.title, // Simplified for now
          status: 'pending'
        }
        todos.push(newTask)
        // Persist the updated todo list
        setTodoListForWorkflow(currentWorkflowId, todos)
        return `Task '${newTask.title}' added with ID ${newTask.id}.`
      }

      case 'add_batch': {
        if (!params.titles || !Array.isArray(params.titles))
          throw new Error("'titles' is required and must be an array for 'add_batch' operation.")

        const newTasks: TodoItem[] = []
        for (const title of params.titles) {
          if (typeof title !== 'string') throw new Error("All items in 'titles' must be strings.")
          const newTask: TodoItem = {
            id: `task_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`,
            title: title,
            description: title, // Simplified for now
            status: 'pending'
          }
          newTasks.push(newTask)
          todos.push(newTask)
        }

        // Persist the updated todo list
        setTodoListForWorkflow(currentWorkflowId, todos)
        return `Added ${newTasks.length} tasks: ${newTasks.map(t => t.title).join(', ')}.`
      }

      case 'list': {
        return todos.length > 0 ? todos : []
      }

      case 'update': {
        if (!params.id || typeof params.id !== 'string') {
          throw new Error("'id' is required and must be a string for 'update' operation.")
        }
        if (
          !params.status ||
          !(
            typeof params.status === 'string' &&
            ['pending', 'in_progress', 'completed', 'data_missing', 'failed'].includes(
              params.status
            )
          )
        ) {
          throw new Error(
            "'status' is required and must be 'pending', 'in_progress', 'completed', 'data_missing', or 'failed' for 'update' operation."
          )
        }
        const taskIndex = todos.findIndex(t => t.id === params.id)
        if (taskIndex === -1) {
          return `Error: Task with ID '${params.id}' not found.`
        }
        const task = todos[taskIndex] as TodoItem
        task.status = params.status as
          | 'pending'
          | 'in_progress'
          | 'completed'
          | 'data_missing'
          | 'failed'
        // Persist the updated todo list
        setTodoListForWorkflow(currentWorkflowId, todos)
        return `Task '${task.title}' updated to status '${params.status}'.`
      }

      default:
        throw new Error(`Invalid operation: ${params.operation}`)
    }
  }
}
