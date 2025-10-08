/**
 * The core ReAct workflow engine.
 */

import {
  addWorkflowMessage,
  createWorkflow,
  getWorkflowSnapshot,
  updateWorkflowStatus
} from './api'
import { callLLM } from './llm'
import { WorkflowStateMachine } from './stateMachine'
import { ToolRegistry } from './toolRegistry'
import { askUserTool } from './tools/askUser'
import { taskCompleteTool } from './tools/taskCompleteTool'
import { todoManagerTool } from './tools/todoManager'
import {
  WorkflowState,
  type Agent,
  type ConversationContext,
  type ParsedLLMResponse,
  type WorkflowMessage,
  type OmitWorkflowMessage
} from './types'

export class WorkflowEngine {
  public readonly stateMachine: WorkflowStateMachine
  public readonly toolRegistry: ToolRegistry
  public readonly sessionId: string
  private context: ConversationContext

  // Constructor is private to force instantiation via static methods.
  private constructor(agent: Agent, sessionId: string) {
    this.stateMachine = new WorkflowStateMachine()
    this.sessionId = sessionId

    // Register all available tools
    this.toolRegistry = new ToolRegistry()
    this.toolRegistry.register(askUserTool)
    this.toolRegistry.register(todoManagerTool)
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
    this.stateMachine.on('stateChanged', event => {
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
    // Implementation for planning mode will be detailed here
    this.stateMachine.transition('SKIP_APPROVAL')
  }

  private async handleThinkingState(): Promise<void> {
    const { actModel } = this.context.agent
    if (!actModel || !actModel.id) {
      throw new Error("Agent's action model is not configured.")
    }

    let thought = ''
    let finalContent = ''
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
          finalContent += chunk
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

          const tool = this.toolRegistry.get(toolName)
          if (!tool) {
            throw new Error(`Tool '${toolName}' not found in registry.`)
          }

          const needsApproval =
            tool.requiresApproval && !this.context.agent.autoApprove.includes(tool.id)
          this.stateMachine.setContext({ currentAction: action })

          if (needsApproval) {
            this.stateMachine.transition('NEED_APPROVAL')
          } else {
            this.stateMachine.transition('EXECUTE_TOOL')
          }
        },
        onDone: async () => {
          if (!actionCalled && finalContent) {
            await this.addMessage({
              sessionId: this.sessionId,
              role: 'assistant',
              message: finalContent
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

    const result = await this.toolRegistry.execute({
      toolId: name,
      parameters
    })

    const observationMessage = `Observation: ${JSON.stringify(result)}`

    await this.addMessage({
      sessionId: this.sessionId,
      role: 'tool',
      message: observationMessage,
      metadata: { tool: name, parameters, result }
    } as OmitWorkflowMessage)

    this.stateMachine.transition('TOOL_COMPLETE')
  }
}
