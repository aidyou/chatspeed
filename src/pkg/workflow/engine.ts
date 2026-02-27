import { streamText, generateText, type ModelMessage } from 'ai'
import { z } from 'zod'
import {
  addWorkflowMessage,
  createWorkflow,
  getWorkflowSnapshot,
  getWorkflowSessionKey,
  updateWorkflowStatus
} from './api'
import { createChatspeedModel } from './llm'
import { WorkflowStateMachine } from './stateMachine'
import { ToolRegistry } from './toolRegistry'
import { getAllSdkTools } from './toolAdapter'
import { askUserTool } from './tools/askUser'
import { taskCompleteTool } from './tools/taskCompleteTool'
import { setTodoListForWorkflow, setTodoListWorkflowId, todoListTool } from './tools/todoList'
import { webAnalyticsTool, setWebAnalyticsContext } from './tools/webAnalytics'
import { useWorkflowStore } from '@/stores/workflow'
import {
  type Agent,
  type ConversationContext,
  type OmitWorkflowMessage,
  WorkflowState
} from './types'

/**
 * WorkflowEngine
 *
 * High-performance ReAct engine powered by Vercel AI SDK.
 * Supports Autonomous and Planning modes with local persistence.
 */
export class WorkflowEngine {
  public readonly stateMachine: WorkflowStateMachine
  public readonly toolRegistry: ToolRegistry
  public readonly sessionId: string
  private context: ConversationContext
  private readonly proxyPort: number
  private readonly sessionKey: string
  private currentStepIndex: number = 0

  /**
   * Private constructor to ensure initialization via static factory methods.
   */
  private constructor(agent: Agent, sessionId: string, proxyPort: number, sessionKey: string) {
    this.stateMachine = new WorkflowStateMachine()
    this.sessionId = sessionId
    this.proxyPort = proxyPort
    this.sessionKey = sessionKey

    // Initialize tools
    setTodoListWorkflowId(sessionId)
    this.toolRegistry = new ToolRegistry()

    setWebAnalyticsContext({ providerId: agent.actModel.id, modelId: agent.actModel.model })
    this.toolRegistry.register(webAnalyticsTool)
    this.toolRegistry.register(askUserTool)
    this.toolRegistry.register(todoListTool)
    this.toolRegistry.register(taskCompleteTool)

    this.context = {
      agent,
      messages: [],
      sdkMessages: [],
      maxTokens: agent.maxContexts,
      totalTokens: 0,
      systemPrompt: agent.systemPrompt
    }

    this.setupEventHandlers()
    this.startMessageProcessor()
  }

  private unwatch: (() => void) | null = null

  /**
   * Starts a background loop to process messages from the shared workflow store queue.
   */
  private startMessageProcessor(): void {
    const workflowStore = useWorkflowStore()

    // Watch for new messages in the queue
    this.unwatch = workflowStore.$subscribe((_mutation: any, state: any) => {
      if (state.messageQueue.length > 0 && state.currentWorkflowId === this.sessionId) {
        this.processMessageQueue()
      }
    })
  }

  /**
   * Clean up resources
   */
  public destroy(): void {
    if (this.unwatch) {
      this.unwatch()
      this.unwatch = null
    }
  }

  private async processMessageQueue(): Promise<void> {
    const workflowStore = useWorkflowStore()

    const queuedMessages = [...workflowStore.messageQueue]
    // Clear the queue immediately to avoid double processing
    workflowStore.messageQueue = []

    for (const msg of queuedMessages) {
      await this.addMessage({
        sessionId: this.sessionId,
        role: msg.role,
        message: msg.message,
        stepIndex: this.currentStepIndex
      })
    }

    // Trigger transition based on current state
    const currentState = this.stateMachine.getState()
    if (currentState === WorkflowState.ERROR) {
      this.stateMachine.transition('RETRY')
    } else if (currentState === WorkflowState.COMPLETED) {
      this.stateMachine.transition('TASK_CONTINUE')
    } else if (currentState === WorkflowState.IDLE) {
      this.stateMachine.transition('START_EXECUTION')
    } else if (currentState === WorkflowState.PAUSED) {
      this.stateMachine.transition('RESUME')
    }
  }

  /**
   * Static factory to start a brand new workflow.
   *
   * @param agent - The agent configuration.
   * @param userQuery - Initial query from user.
   * @param proxyPort - The local CCProxy port.
   */
  public static async startNew(
    agent: Agent,
    userQuery: string,
    proxyPort: number
  ): Promise<WorkflowEngine> {
    const response = await createWorkflow(userQuery, agent.id)
    const engine = new WorkflowEngine(agent, response.workflow.id, proxyPort, response.sessionKey)

    const userMessage: OmitWorkflowMessage = {
      sessionId: engine.sessionId,
      role: 'user',
      message: userQuery,
      stepIndex: 0
    }
    await engine.addMessage(userMessage)

    const transitionEvent = agent.agentType === 'planning' ? 'START_PLANNING' : 'START_EXECUTION'
    engine.stateMachine.transition(transitionEvent)

    return engine
  }

  /**
   * Static factory to load an existing workflow from DB.
   *
   * @param agent - The agent configuration.
   * @param workflowId - Existing workflow session ID.
   * @param proxyPort - The local CCProxy port.
   */
  public static async load(
    agent: Agent,
    workflowId: string,
    proxyPort: number
  ): Promise<WorkflowEngine> {
    const [snapshot, sessionKey] = await Promise.all([
      getWorkflowSnapshot(workflowId),
      getWorkflowSessionKey(workflowId)
    ])
    const engine = new WorkflowEngine(agent, workflowId, proxyPort, sessionKey)

    engine.context.messages = snapshot.messages.map(m => ({
      id: m.id,
      sessionId: m.sessionId,
      role: m.role as any,
      message: m.message,
      metadata: m.metadata,
      stepType: m.stepType as any,
      stepIndex: m.stepIndex || 0,
      createdAt: m.createdAt ? new Date(m.createdAt) : new Date()
    }))

    // Determine current step index
    if (engine.context.messages.length > 0) {
      engine.currentStepIndex = Math.max(...engine.context.messages.map(m => m.stepIndex || 0))
    }

    // Convert to SDK compatible messages
    engine.syncSdkMessages()

    engine.stateMachine.setState(snapshot.workflow.status as WorkflowState)
    return engine
  }

  /**
   * Syncs internal message list to AI SDK ModelMessage format.
   * Handles restoration of tool calls and results from metadata.
   */
  private syncSdkMessages(): void {
    const sdkMsgs: ModelMessage[] = []

    for (const m of this.context.messages) {
      if (m.role === 'tool') {
        const toolCallId = (m.metadata?.toolCallId as string) || `call_${m.id}`
        const toolName = (m.metadata?.tool as string) || 'unknown_tool'
        const result = m.metadata?.result ?? m.message

        sdkMsgs.push({
          role: 'tool',
          content: [
            {
              type: 'tool-result',
              toolCallId: toolCallId,
              toolName: toolName,
              result: result
            }
          ]
        })
      } else if (m.role === 'assistant') {
        const content: any[] = []
        if (m.message && m.message.trim() !== '') {
          content.push({ type: 'text', text: m.message })
        }

        if (m.metadata?.toolCalls) {
          const toolCalls = (m.metadata.toolCalls as any[]).map(tc => ({
            type: 'tool-call',
            toolCallId: tc.id,
            toolName: tc.name,
            args: tc.arguments
          }))
          content.push(...toolCalls)
        }

        // AI SDK assistant message must have content
        if (content.length > 0) {
          sdkMsgs.push({
            role: 'assistant',
            content: content
          })
        }
      } else {
        // system or user
        sdkMsgs.push({
          role: m.role as 'user' | 'system',
          content: m.message
        })
      }
    }

    this.context.sdkMessages = sdkMsgs
  }

  /**
   * Persists a message to the backend and updates local context.
   */
  private async addMessage(message: OmitWorkflowMessage): Promise<void> {
    const newApiMessage = await addWorkflowMessage(message)

    this.context.messages.push({
      id: newApiMessage.id!,
      sessionId: newApiMessage.sessionId,
      role: newApiMessage.role as any,
      message: newApiMessage.message,
      metadata: newApiMessage.metadata || undefined,
      stepType: newApiMessage.stepType as any,
      stepIndex: newApiMessage.stepIndex || 0,
      createdAt: newApiMessage.createdAt ? new Date(newApiMessage.createdAt) : new Date()
    })

    this.syncSdkMessages()
  }

  private setupEventHandlers(): void {
    this.stateMachine.on('stateChanged', (data?: any) => {
      const event = data
      const newState = event.payload.newState as WorkflowState

      // Map internal state machine states to standard database status strings
      let dbStatus: string
      switch (newState) {
        case WorkflowState.COMPLETED:
          dbStatus = 'completed'
          break
        case WorkflowState.ERROR:
          dbStatus = 'error'
          break
        case WorkflowState.PAUSED:
          dbStatus = 'paused'
          break
        case WorkflowState.IDLE:
          dbStatus = 'pending'
          break
        default:
          dbStatus = 'running'
          break
      }

      updateWorkflowStatus(this.sessionId, dbStatus).catch(console.error)

      // Trigger run() when entering states that require active processing
      if (newState === WorkflowState.PLANNING || newState === WorkflowState.THINKING) {
        this.run()
      }
    })
  }

  /**
   * Main execution loop based on current state.
   */
  private async run(): Promise<void> {
    const currentState = this.stateMachine.getState()
    try {
      switch (currentState) {
        case WorkflowState.PLANNING:
          await this.handlePlanning()
          break
        case WorkflowState.THINKING:
          await this.handleAutonomous()
          break
      }
    } catch (error: any) {
      this.stateMachine.transition('ERROR_OCCURRED', { error: error.message })
    }
  }

  /**
   * Planning Mode: Generates a structured Todo List using generateText with output setting.
   */
  private async handlePlanning(): Promise<void> {
    const { agent } = this.context
    const model = createChatspeedModel(
      agent.planModel.model,
      this.proxyPort,
      this.sessionKey,
      this.sessionId,
      agent.planModel.id
    )

    try {
      const { output: object } = await generateText({
        model,
        system: agent.planningPrompt || 'Generate a plan.',
        prompt: this.context.messages[this.context.messages.length - 1].message,
        output: {
          schema: z.object({
            plan: z.array(
              z.object({
                id: z.string(),
                title: z.string(),
                description: z.string().optional(),
                status: z.enum(['pending', 'in_progress', 'completed', 'failed'])
              })
            )
          })
        }
      })

      // Update the global todo manager so UI can pick it up
      setTodoListForWorkflow(this.sessionId, object.plan as any)

      await this.addMessage({
        sessionId: this.sessionId,
        role: 'assistant',
        message: `I have created a plan to address your request. Please review and approve it.\n\n\`\`\`json\n${JSON.stringify(
          object.plan,
          null,
          2
        )}\n\`\`\``,
        metadata: { todoList: object.plan },
        stepType: 'think',
        stepIndex: this.currentStepIndex
      })

      this.stateMachine.transition('PLAN_READY')
    } catch (error: any) {
      console.error('Planning error:', error)
      this.stateMachine.transition('ERROR_OCCURRED', { error: error.message || String(error) })
      throw error
    }
  }

  /**
   * Autonomous Mode: Standard ReAct loop with tools and approval interception.
   */
  private async handleAutonomous(): Promise<void> {
    const { agent } = this.context
    const model = createChatspeedModel(
      agent.actModel.model,
      this.proxyPort,
      this.sessionKey,
      this.sessionId,
      agent.actModel.id
    )
    const sdkTools = getAllSdkTools(this.toolRegistry)

    try {
      let hasReceivedContent = false
      let hasReceivedTools = false

      const result = streamText({
        model,
        system: agent.systemPrompt,
        messages: this.context.sdkMessages,
        tools: sdkTools,
        maxSteps: 15,
        onStepFinish: async ({ text, toolCalls, toolResults }) => {
          this.currentStepIndex++

          if (text) hasReceivedContent = true
          if (toolCalls.length > 0) hasReceivedTools = true

          // 1. Check if any tool call requires approval
          for (const call of toolCalls) {
            const toolDef = this.toolRegistry.get(call.toolName)
            const needsApproval = toolDef?.requiresApproval && !agent.autoApprove.includes(toolDef.id)

            if (needsApproval) {
              this.stateMachine.setContext({
                currentAction: {
                  name: call.toolName,
                  arguments: call.args,
                  toolCallId: call.toolCallId
                }
              })
              this.stateMachine.transition('NEED_APPROVAL')
            }
          }

          // 2. Persist assistant message with tool calls metadata
          if (text || toolCalls.length > 0) {
            await this.addMessage({
              sessionId: this.sessionId,
              role: 'assistant',
              message: text || '',
              metadata: {
                toolCalls: toolCalls.map(tc => ({
                  id: tc.toolCallId,
                  name: tc.toolName,
                  arguments: tc.args
                }))
              },
              stepType: toolCalls.length > 0 ? 'act' : 'think',
              stepIndex: this.currentStepIndex
            })
          }

          // 3. Persist tool results
          for (const res of toolResults) {
            await this.addMessage({
              sessionId: this.sessionId,
              role: 'tool',
              message: `Observation from ${res.toolName}: ${JSON.stringify(res.result)}`,
              metadata: {
                tool: res.toolName,
                parameters: res.args,
                result: res.result,
                toolCallId: res.toolCallId
              },
              stepType: 'observe',
              stepIndex: this.currentStepIndex
            })
          }
        }
      })

      // Consume the stream to trigger execution
      for await (const chunk of result.textStream) {
        if (chunk) hasReceivedContent = true
      }

      await result.finishPromise

      // If we are still in THINKING state, but haven't received anything at all,
      // it's likely a silent failure (e.g. 404 handled internally by the provider)
      if (this.stateMachine.getState() === WorkflowState.THINKING) {
        if (!hasReceivedContent && !hasReceivedTools) {
          throw new Error('No response received from the model. Please check your configuration.')
        }
        this.stateMachine.transition('TASK_COMPLETE')
      }
    } catch (error: any) {
      console.error('Autonomous execution error:', error)
      this.stateMachine.transition('ERROR_OCCURRED', { error: error.message || String(error) })
      throw error
    }
  }

  public resume(): void {
    if (this.stateMachine.isInState(WorkflowState.PAUSED)) {
      this.stateMachine.transition('RESUME')
    }
  }
}
