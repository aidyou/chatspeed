/**
 * The core ReAct workflow engine.
 */

import {
  addWorkflowMessage,
  createWorkflow,
  getWorkflowSnapshot,
  updateWorkflowStatus,
  workflowCallTool
} from './api'
import { callLLM } from './llm'
import {
  FATAL_ERROR,
  INVALID_PARAMS_ERROR,
  networkErrorMaxRetries,
  networkErrorRetry,
  TODO_MANAGER_SUCCESS,
  TODO_USAGE_REMINDER,
  unexpectedError,
  WEB_FETCH_INSUFFICIENT_CONTENT,
  WEB_FETCH_SUCCESS,
  WEB_SEARCH_NO_RESULTS,
  WEB_SEARCH_SUCCESS
} from './reminders'
import { WorkflowStateMachine } from './stateMachine'
import { ToolRegistry } from './toolRegistry'
import { askUserTool } from './tools/askUser'
import { taskCompleteTool } from './tools/taskCompleteTool'
import { setTodoListWorkflowId, todoListTool } from './tools/todoList'
import { webAnalyticsTool, setWebAnalyticsContext } from './tools/webAnalytics'
import {
  type Agent,
  type ConversationContext,
  type OmitWorkflowMessage,
  type ParsedLLMResponse,
  type StateChangeEvent,
  type WorkflowMessage,
  WorkflowState
} from './types'

// Define ToolError locally to avoid modifying shared types.ts
interface ToolError {
  type: string
  message: string
}

export class WorkflowEngine {
  public readonly stateMachine: WorkflowStateMachine
  public readonly toolRegistry: ToolRegistry
  public readonly sessionId: string
  private context: ConversationContext
  private stepsSinceTodo: number = 0
  private readonly maxRetries: number = 3

  // Constructor is private to force instantiation via static methods.
  private constructor(agent: Agent, sessionId: string) {
    this.stateMachine = new WorkflowStateMachine()
    this.sessionId = sessionId

    // Set the workflow ID for the todo manager
    setTodoListWorkflowId(sessionId)

    // Register all available tools
    this.toolRegistry = new ToolRegistry()
    
    // Set up WebAnalytics context and register the tool
    setWebAnalyticsContext({ providerId: agent.actModel.id, modelId: agent.actModel.model })
    this.toolRegistry.register(webAnalyticsTool)
    
    this.toolRegistry.register(askUserTool)
    this.toolRegistry.register(todoListTool)
    this.toolRegistry.register(taskCompleteTool)

    this.context = {
      agent,
      messages: [],
      maxTokens: agent.maxContexts,
      totalTokens: 0, // Token calculation to be implemented in callLLM.onDone
      systemPrompt: agent.systemPrompt
    }

    this.setupEventHandlers()
  }

  /**
   * Creates and starts a new workflow.
   * @param agent The agent to use.
   * @param userQuery The initial user query.
   * @returns A promise that resolves with the initialized WorkflowEngine instance.
   */
  public static async startNew(agent: Agent, userQuery: string): Promise<WorkflowEngine> {
    const workflow = await createWorkflow(userQuery, agent.id)
    const engine = new WorkflowEngine(agent, workflow.id)

    // Create and persist initial user message
    const userMessage: OmitWorkflowMessage = {
      sessionId: engine.sessionId,
      role: 'user',
      message: userQuery
    }
    await engine.addMessage(userMessage)

    // Start the workflow
    const transitionEvent = agent.agentType === 'planning' ? 'START_PLANNING' : 'START_EXECUTION'
    engine.stateMachine.transition(transitionEvent)

    return engine
  }

  /**
   * Loads an existing workflow from the database.
   * @param agent The agent associated with the workflow.
   * @param workflowId The ID of the workflow to load.
   * @returns A promise that resolves with the loaded WorkflowEngine instance.
   */
  public static async load(agent: Agent, workflowId: string): Promise<WorkflowEngine> {
    const snapshot = await getWorkflowSnapshot(workflowId)
    const engine = new WorkflowEngine(agent, workflowId)

    // Restore context and state
    engine.context.messages = snapshot.messages.map(
      m =>
        ({
          id: m.id,
          sessionId: m.sessionId,
          role: m.role as 'system' | 'assistant' | 'tool' | 'user',
          message: m.message,
          metadata: m.metadata,
          // Ensure date objects are correctly parsed if stored as strings
          createdAt: m.createdAt ? new Date(m.createdAt) : new Date()
        }) as WorkflowMessage
    )
    engine.stateMachine.setState(snapshot.workflow.status as WorkflowState)

    return engine
  }

  /**
   * Resumes a paused workflow.
   */
  public resume(): void {
    if (this.stateMachine.isInState(WorkflowState.PAUSED)) {
      this.stateMachine.transition('RESUME')
    } else {
      console.warn('Workflow can only be resumed from a paused state.')
    }
  }

  private async addMessage(message: OmitWorkflowMessage): Promise<void> {
    const newApiMessage = await addWorkflowMessage(message)

    if (newApiMessage.id === null || newApiMessage.id === undefined) {
      throw new Error('Backend did not return an ID for the new message.')
    }

    this.context.messages.push({
      id: newApiMessage.id,
      sessionId: newApiMessage.sessionId,
      role: newApiMessage.role as 'system' | 'assistant' | 'tool' | 'user',
      message: newApiMessage.message,
      ...(newApiMessage.metadata && { metadata: newApiMessage.metadata }),
      createdAt: newApiMessage.createdAt ? new Date(newApiMessage.createdAt) : new Date()
    })
  }

  private setupEventHandlers(): void {
    this.stateMachine.on('stateChanged', (data?: unknown) => {
      const event = data as StateChangeEvent
      console.log(`Workflow state changed: ${event.payload.oldState} -> ${event.payload.newState}`)
      // Persist status change
      updateWorkflowStatus(this.sessionId, event.payload.newState).catch(console.error)

      // Avoid re-triggering if the state was just set by the load method
      if (event.payload.newState !== this.stateMachine.getState()) {
        this.run()
      }
    })
  }

  private async run(): Promise<void> {
    const currentState = this.stateMachine.getState()

    try {
      switch (currentState) {
        case WorkflowState.PLANNING:
          await this.handlePlanningState()
          break
        case WorkflowState.THINKING:
          await this.handleThinkingState()
          break
        case WorkflowState.EXECUTING_TOOL:
          await this.handleExecutingToolState()
          break
      }
    } catch (error: unknown) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.stateMachine.transition('ERROR_OCCURRED', { error: errorMessage })
    }
  }

  private async handlePlanningState(): Promise<void> {
    const { agent, messages } = this.context
    const { planModel, planningPrompt } = agent

    if (agent.agentType !== 'planning' || !planModel?.id || !planningPrompt) {
      // This should not happen if the state transition is correct, but as a safeguard:
      console.warn('Agent is not configured for planning, skipping planning phase.')
      this.stateMachine.transition('SKIP_APPROVAL')
      return
    }

    // Use a compatible method to find the last user message
    const userQuery = [...messages]
      .reverse()
      .find((m: WorkflowMessage) => m.role === 'user')?.message
    if (!userQuery) {
      throw new Error('Cannot start planning without a user query.')
    }

    // Construct messages for the planning call. The type is inferred to be more
    // flexible than OmitWorkflowMessage to allow for the 'system' role.
    const messagesForLLM = [
      {
        sessionId: this.sessionId,
        role: 'system' as const,
        message: planningPrompt
      },
      {
        sessionId: this.sessionId,
        role: 'user' as const,
        message: userQuery
      }
    ]

    let planContent = ''

    await callLLM(
      {
        providerId: planModel.id,
        modelId: planModel.model,
        messages: messagesForLLM,
        // No tools are provided in the planning phase
        availableTools: [],
        tsTools: []
      },
      {
        onContent: chunk => {
          planContent += chunk
        },
        onDone: async () => {
          if (!planContent) {
            throw new Error('LLM did not return a plan.')
          }

          try {
            // Assuming the LLM returns a JSON string representing the todoList
            const todoList = JSON.parse(planContent)

            // Save the plan to the context and wait for user approval
            this.stateMachine.setContext({ plan: todoList, approvalType: 'plan' })

            await this.addMessage({
              sessionId: this.sessionId,
              role: 'assistant',
              message: `I have created a plan to address your request. Please review and approve it.\n\n\`\`\`json\n${JSON.stringify(
                todoList,
                null,
                2
              )}\n\`\`\``,
              metadata: { todoList }
            })

            this.stateMachine.transition('NEED_APPROVAL')
          } catch (e) {
            console.error('Failed to parse plan from LLM response:', e)
            // Add the raw response for debugging and ask for clarification
            await this.addMessage({
              sessionId: this.sessionId,
              role: 'assistant',
              message: `I was unable to create a structured plan. Here is the raw response I generated:\n\n${planContent}`
            })
            this.stateMachine.transition('ERROR_OCCURRED', {
              error: 'Failed to parse plan from LLM.'
            })
          }
        },
        onError: error => {
          throw error
        }
      }
    )
  }

  private async handleThinkingState(): Promise<void> {
    const { actModel } = this.context.agent
    if (!actModel || !actModel.id) {
      throw new Error("Agent's action model is not configured.")
    }

    let thought = ''
    let content = ''
    let actionCalled = false

    const messagesForLLM: WorkflowMessage[] = [
      {
        // id, createdAt are not needed for the LLM call
        sessionId: this.sessionId,
        role: 'system',
        message: this.context.agent.systemPrompt
      } as OmitWorkflowMessage,
      ...this.context.messages
    ]

    await callLLM(
      {
        providerId: actModel.id,
        modelId: actModel.model,
        messages: messagesForLLM,
        availableTools: this.toolRegistry.getToolNames(),
        tsTools: this.toolRegistry.getToolDeclarations()
      },
      {
        onReasoning: chunk => {
          thought += chunk
          console.log('AI Thought:', chunk)
        },
        onContent: chunk => {
          content += chunk
        },
        onAction: async action => {
          actionCalled = true
          if (thought) {
            await this.addMessage({
              sessionId: this.sessionId,
              role: 'assistant',
              message: thought
            } as OmitWorkflowMessage)
          }

          const toolName = action?.name
          if (!toolName) {
            throw new Error('Action name is required but was not provided.')
          }

          if (toolName === taskCompleteTool.name) {
            const finalAnswer = (action.arguments?.finalAnswer as string) || 'Task is complete.'
            await this.addMessage({
              sessionId: this.sessionId,
              role: 'assistant',
              message: finalAnswer
            } as OmitWorkflowMessage)
            this.stateMachine.transition('TASK_COMPLETE')
            return
          }

          // Check if tool is in TypeScript registry first
          const tsTool = this.toolRegistry.get(toolName)

          if (tsTool) {
            // TypeScript tool found
            const needsApproval =
              tsTool.requiresApproval && !this.context.agent.autoApprove.includes(tsTool.id)
            this.stateMachine.setContext({ currentAction: action })

            if (needsApproval) {
              this.stateMachine.transition('NEED_APPROVAL')
            } else {
              this.stateMachine.transition('EXECUTE_TOOL')
            }
          } else {
            // Try to execute as Rust tool via workflow_call_tool
            this.stateMachine.setContext({ currentAction: action })
            this.stateMachine.transition('EXECUTE_TOOL')
          }
        },
        onDone: async () => {
          if (!actionCalled && content) {
            await this.addMessage({
              sessionId: this.sessionId,
              role: 'assistant',
              message: content
            } as OmitWorkflowMessage)
            this.stateMachine.transition('TASK_COMPLETE')
          }
        },
        onError: error => {
          throw error
        }
      }
    )
  }

  private async handleExecutingToolState(): Promise<void> {
    const { currentAction } = this.stateMachine.getContext() as {
      currentAction: ParsedLLMResponse['action']
    }
    if (!currentAction) {
      throw new Error('No action to execute.')
    }

    const { name, arguments: parameters } = currentAction

    try {
      // First try to execute TypeScript tool
      const tsTool = this.toolRegistry.get(name)

      if (tsTool) {
        // TypeScript tool found, execute it
        const result = await this.toolRegistry.execute({
          toolId: name,
          parameters
        })
        await this.handleToolResult(name, parameters, result)
      } else {
        // Try to execute as Rust tool via workflow_call_tool
        try {
          const result = await workflowCallTool(name, parameters)
          await this.handleToolResult(name, parameters, result)
        } catch (error) {
          await this.handleToolResult(name, parameters, error)
        }
      }
    } catch (error) {
      await this.handleToolResult(name, parameters, error)
    }
  }

  private async handleToolResult(
    toolName: string,
    parameters: Record<string, unknown>,
    result: unknown
  ): Promise<void> {
    // Type guard to check if the result is a ToolError
    const isToolError = (res: unknown): res is ToolError => {
      return typeof res === 'object' && res !== null && 'type' in res && 'message' in res
    }

    if (result instanceof Error || isToolError(result)) {
      const error = isToolError(result)
        ? result
        : { type: 'UnknownError', message: (result as Error).message }
      await this.handleErrorObservation(toolName, parameters, error)
    } else {
      await this.handleSuccessObservation(toolName, parameters, result)
    }
  }

  private async handleSuccessObservation(
    toolName: string,
    parameters: Record<string, unknown>,
    result: unknown
  ): Promise<void> {
    let observationMessage: string
    let systemReminder: string | undefined

    this.stepsSinceTodo++

    switch (toolName) {
      case 'WebSearch': {
        // Assuming result is { results: any[] }
        const searchResult = result as { results?: unknown[] }
        if (!searchResult.results || searchResult.results.length === 0) {
          systemReminder = WEB_SEARCH_NO_RESULTS
        } else {
          systemReminder = WEB_SEARCH_SUCCESS
        }
        observationMessage = `Observation: ${JSON.stringify(result)}`
        break
      }

      case 'WebFetch': {
        // Assuming result is { content: string }
        const fetchResult = result as { content?: string }
        const contentLength = fetchResult.content?.length || 0
        if (contentLength < 100) {
          systemReminder = WEB_FETCH_INSUFFICIENT_CONTENT
        } else {
          systemReminder = WEB_FETCH_SUCCESS
        }
        observationMessage = `Observation: ${JSON.stringify(result)}`
        break
      }

      case 'TodoList': {
        this.stepsSinceTodo = 0
        systemReminder = TODO_MANAGER_SUCCESS

        // Check for failed status and stop workflow execution
        const todoResult = result as { message: string; updated_todos?: any[] }
        if (todoResult.updated_todos) {
          const lastTodo = todoResult.updated_todos[todoResult.updated_todos.length - 1]
          if (lastTodo?.status === 'failed') {
            systemReminder += ' Task failed - workflow execution stopped.'
            await this.addMessage({
              sessionId: this.sessionId,
              role: 'tool',
              message: `Observation: ${JSON.stringify(result)}\n${systemReminder}`,
              metadata: { tool: toolName, parameters, result }
            } as OmitWorkflowMessage)
            this.stateMachine.transition('ERROR_OCCURRED', {
              error: `Task failed: ${lastTodo.title}`
            })
            return
          }
        }

        observationMessage = `Observation: ${JSON.stringify(result)}`
        break
      }

      default:
        observationMessage = `Observation: ${JSON.stringify(result)}`
        break
    }

    if (this.stepsSinceTodo > 3) {
      const todoReminder = TODO_USAGE_REMINDER
      observationMessage = `${observationMessage}\n${todoReminder}`
      this.stepsSinceTodo = 0 // Reset after reminding
    }

    if (systemReminder) {
      observationMessage = `${observationMessage}\n${systemReminder}`
    }

    await this.addMessage({
      sessionId: this.sessionId,
      role: 'tool',
      message: observationMessage,
      metadata: { tool: toolName, parameters, result }
    } as OmitWorkflowMessage)

    this.stateMachine.transition('TOOL_COMPLETE')
  }

  private async handleErrorObservation(
    toolName: string,
    parameters: Record<string, unknown>,
    error: ToolError
  ): Promise<void> {
    const context = this.stateMachine.getContext() as {
      currentAction?: {
        retryCount?: number
      } & ParsedLLMResponse['action']
    }
    const currentRetry = (context.currentAction?.retryCount || 0) as number

    let errorMessage: string
    let shouldRetry = false

    const errorType = error.type || 'UnknownError'

    switch (errorType) {
      case 'InvalidParams':
        errorMessage = INVALID_PARAMS_ERROR
        break

      case 'NetworkError':
      case 'Timeout':
        if (currentRetry < this.maxRetries) {
          shouldRetry = true
          errorMessage = networkErrorRetry(currentRetry + 1, this.maxRetries)
        } else {
          errorMessage = networkErrorMaxRetries(this.maxRetries)
        }
        break

      case 'Fatal':
        errorMessage = FATAL_ERROR
        break

      default:
        errorMessage = unexpectedError(error.message)
        break
    }

    await this.addMessage({
      sessionId: this.sessionId,
      role: 'tool',
      message: errorMessage,
      metadata: { tool: toolName, parameters, error }
    } as OmitWorkflowMessage)

    if (shouldRetry) {
      this.stateMachine.setContext({
        ...context,
        currentAction: { ...context.currentAction, retryCount: currentRetry + 1 }
      })
      this.stateMachine.transition('EXECUTE_TOOL')
    } else {
      this.stateMachine.transition('ERROR_OCCURRED', { error: error.message })
    }
  }
}
