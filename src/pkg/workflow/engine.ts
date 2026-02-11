import { streamText, generateText, type ModelMessage } from 'ai'
import { z } from 'zod'
import {
  addWorkflowMessage,
  createWorkflow,
  getWorkflowSnapshot,
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

  /**
   * Private constructor to ensure initialization via static factory methods.
   */
  private constructor(agent: Agent, sessionId: string, proxyPort: number) {
    this.stateMachine = new WorkflowStateMachine()
    this.sessionId = sessionId
    this.proxyPort = proxyPort

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
    const workflow = await createWorkflow(userQuery, agent.id)
    const engine = new WorkflowEngine(agent, workflow.id, proxyPort)

    const userMessage: OmitWorkflowMessage = {
      sessionId: engine.sessionId,
      role: 'user',
      message: userQuery
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
    const snapshot = await getWorkflowSnapshot(workflowId)
    const engine = new WorkflowEngine(agent, workflowId, proxyPort)

    engine.context.messages = snapshot.messages.map(m => ({
      id: m.id,
      sessionId: m.sessionId,
      role: m.role as any,
      message: m.message,
      metadata: m.metadata,
      createdAt: m.createdAt ? new Date(m.createdAt) : new Date()
    }))

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
      if (m.role === 'tool' && m.metadata?.tool) {
        const toolCallId = (m.metadata.toolCallId as string) || `call_${m.id}`

        sdkMsgs.push({
          role: 'tool',
          content: [
            {
              type: 'tool-result',
              toolCallId: toolCallId,
              toolName: m.metadata.tool as string,
              result: m.metadata.result
            }
          ]
        })
      } else if (m.role === 'assistant' && m.metadata?.toolCalls) {
        sdkMsgs.push({
          role: 'assistant',
          content: [
            { type: 'text', text: m.message },
            ...(m.metadata.toolCalls as any[]).map(tc => ({
              type: 'tool-call',
              toolCallId: tc.id,
              toolName: tc.name,
              args: tc.arguments
            }))
          ]
        })
      } else {
        sdkMsgs.push({
          role: m.role as 'user' | 'assistant' | 'system',
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
      createdAt: newApiMessage.createdAt ? new Date(newApiMessage.createdAt) : new Date()
    })

    this.syncSdkMessages()
  }

  private setupEventHandlers(): void {
    this.stateMachine.on('stateChanged', (data?: any) => {
      const event = data
      updateWorkflowStatus(this.sessionId, event.payload.newState).catch(console.error)

      if (event.payload.newState !== this.stateMachine.getState()) {
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
    const model = createChatspeedModel(agent.planModel.model, this.proxyPort)

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
      metadata: { todoList: object.plan }
    })

    this.stateMachine.transition('NEED_APPROVAL')
  }

  /**
   * Autonomous Mode: Standard ReAct loop with tools and approval interception.
   */
  private async handleAutonomous(): Promise<void> {
    const { agent } = this.context
    const model = createChatspeedModel(agent.actModel.model, this.proxyPort)
    const sdkTools = getAllSdkTools(this.toolRegistry)

    const result = streamText({
      model,
      system: agent.systemPrompt,
      messages: this.context.sdkMessages,
      tools: sdkTools,
      maxSteps: 15,
      onStepFinish: async ({ text, toolCalls, toolResults }) => {
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
            }
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
            }
          })
        }
      }
    })

    // Consume the stream to trigger execution
    for await (const _ of result.textStream) {
      // Stream can be used for real-time UI updates
    }

    await result.finishPromise
    if (this.stateMachine.getState() === WorkflowState.THINKING) {
      this.stateMachine.transition('TASK_COMPLETE')
    }
  }

  public resume(): void {
    if (this.stateMachine.isInState(WorkflowState.PAUSED)) {
      this.stateMachine.transition('RESUME')
    }
  }
}
