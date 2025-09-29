/**
 * The core ReAct workflow engine.
 * This class orchestrates the entire workflow, using the state machine and tool registry
 * to manage the agent's lifecycle of thought, action, and observation.
 */

import { callLLM, parseLLMResponse } from './llm'
import { WorkflowStateMachine } from './stateMachine'
import { ToolRegistry } from './toolRegistry'
import { askUserTool } from './tools/askUser'
import { todoManagerTool } from './tools/todoManager'
import type { Agent, ConversationContext, ParsedLLMResponse, WorkflowMessage } from './types'
import { WorkflowState } from './types'

export class WorkflowEngine {
  public readonly stateMachine: WorkflowStateMachine
  public readonly toolRegistry: ToolRegistry
  private context: ConversationContext
  private readonly sessionId: string
  private messageCounter: number

  constructor(agent: Agent) {
    this.stateMachine = new WorkflowStateMachine()
    this.toolRegistry = new ToolRegistry()
    this.sessionId = `sess_${Date.now()}`
    this.messageCounter = 0

    // Register built-in TS tools
    this.toolRegistry.register(askUserTool)
    this.toolRegistry.register(todoManagerTool)

    // TODO: Register Rust tools by fetching their definitions

    // Initialize the context with agent-specific information
    this.context = {
      agent,
      messages: [
        {
          id: this.getNextMessageId(),
          sessionId: this.sessionId,
          role: 'system',
          message: agent.systemPrompt,
          createdAt: new Date()
        }
      ],
      maxTokens: agent.maxContexts,
      totalTokens: 0, // Token calculation to be implemented
      systemPrompt: agent.systemPrompt // <-- Added this line
    }

    // Pass the initial context to the state machine
    this.stateMachine.setContext(this.context)

    this.setupEventHandlers()
  }

  private getNextMessageId(): number {
    this.messageCounter += 1
    return this.messageCounter
  }

  /**
   * Sets up handlers for events emitted by the state machine or other components.
   */
  private setupEventHandlers(): void {
    this.stateMachine.on('stateChanged', event => {
      console.log(`Workflow state changed to: ${event.payload.newState}`)
      // Run the main loop whenever the state changes
      this.run()
    })
  }

  /**
   * Starts the workflow based on the agent's type.
   * @param userQuery The initial query from the user.
   */
  public start(userQuery: string): void {
    if (this.stateMachine.getState() !== WorkflowState.IDLE) {
      console.warn('Workflow is already running.')
      return
    }

    // Add user query to context
    this.context.messages.push({
      id: this.getNextMessageId(),
      sessionId: this.sessionId,
      role: 'user',
      message: userQuery,
      createdAt: new Date()
    })

    // Trigger the first state transition
    if (this.context.agent.agentType === 'planning') {
      this.stateMachine.transition('START_PLANNING')
    } else {
      this.stateMachine.transition('START_EXECUTION')
    }
  }

  /**
   * The main run loop, triggered by state changes.
   */
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

        // Other states can be handled here as well
      }
    } catch (error: unknown) {
      console.error('Error during workflow run:', error)
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.stateMachine.transition('ERROR_OCCURRED', { error: errorMessage })
    }
  }

  /**
   * Handles the logic for the PLANNING state.
   */
  private async handlePlanningState(): Promise<void> {
    const { agent } = this.stateMachine.getContext() as ConversationContext
    const { planModel, planningPrompt } = agent
    if (!planModel || !planModel.id) {
      throw new Error("Agent's planning model is not configured.")
    }

    // Construct a planning-specific message history
    const planningMessages: WorkflowMessage[] = [
      {
        id: this.getNextMessageId(),
        sessionId: this.sessionId,
        role: 'system',
        message: planningPrompt || 'Please create a plan.',
        createdAt: new Date()
      },
      {
        id: this.getNextMessageId(),
        sessionId: this.sessionId,
        role: 'user',
        message: `Here is the user's request: ${
          (this.stateMachine.getContext() as ConversationContext).messages.find(
            m => m.role === 'user'
          )?.message || ''
        }`,
        createdAt: new Date()
      }
    ]

    const rawResponse = await callLLM({
      providerId: planModel.id,
      modelId: planModel.model,
      messages: planningMessages
    })

    // For now, we assume the plan is the raw response.
    // TODO: Implement robust plan parsing (e.g., from JSON or Markdown)
    const plan = rawResponse

    this.stateMachine.transition('PLAN_READY', { plan })
  }

  /**
   * Handles the logic for the THINKING state.
   */
  private async handleThinkingState(): Promise<void> {
    const { actModel } = (this.stateMachine.getContext() as ConversationContext).agent
    if (!actModel || !actModel.id) {
      throw new Error("Agent's action model is not configured.")
    }

    const rawResponse = await callLLM({
      providerId: actModel.id,
      modelId: actModel.model,
      messages: (this.stateMachine.getContext() as ConversationContext).messages
    })

    const parsed = parseLLMResponse(rawResponse)

    if (parsed.thought) {
      ;(this.stateMachine.getContext() as ConversationContext).messages.push({
        id: this.getNextMessageId(),
        sessionId: this.sessionId,
        role: 'assistant',
        message: parsed.thought,
        createdAt: new Date()
      })
    }

    if (parsed.action) {
      const tool = this.toolRegistry.get(parsed.action.tool)
      if (!tool) {
        throw new Error(`Tool '${parsed.action.tool}' not found in registry.`)
      }

      // Approval Hook Logic
      const needsApproval =
        tool.requiresApproval &&
        !(this.stateMachine.getContext() as ConversationContext).agent.autoApprove.includes(tool.id)

      this.stateMachine.setContext({ currentAction: parsed.action })

      if (needsApproval) {
        this.stateMachine.transition('NEED_APPROVAL')
      } else {
        this.stateMachine.transition('EXECUTE_TOOL')
      }
    } else if (parsed.finalAnswer) {
      ;(this.stateMachine.getContext() as ConversationContext).messages.push({
        id: this.getNextMessageId(),
        sessionId: this.sessionId,
        role: 'assistant',
        message: parsed.finalAnswer,
        createdAt: new Date()
      })
      this.stateMachine.transition('TASK_COMPLETE')
    } else {
      // If the model output is unclear, ask it to clarify or try again.
      throw new Error('LLM response was not a valid action or final answer.')
    }
  }

  /**
   * Handles the logic for the EXECUTING_TOOL state.
   */
  private async handleExecutingToolState(): Promise<void> {
    const { currentAction } = this.stateMachine.getContext() as {
      currentAction: ParsedLLMResponse['action']
    }
    if (!currentAction) {
      throw new Error('No action to execute.')
    }

    const { tool, parameters } = currentAction

    const result = await this.toolRegistry.execute({
      toolId: tool,
      parameters
    })

    // Create the observation message
    const observationMessage: string = `Observation: ${JSON.stringify(result)}`

    ;(this.stateMachine.getContext() as ConversationContext).messages.push({
      id: this.getNextMessageId(),
      sessionId: this.sessionId,
      role: 'tool',
      message: observationMessage,
      metadata: { tool, parameters, result },
      createdAt: new Date()
    })

    if (result.success) {
      this.stateMachine.transition('TOOL_COMPLETE')
    } else {
      this.stateMachine.transition('TOOL_FAILED', { error: result.error })
    }
  }
}
