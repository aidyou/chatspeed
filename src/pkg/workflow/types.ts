/**
 * Core type definitions for the workflow system
 */

// Agent configuration
export interface Agent {
  id: string
  name: string
  description: string
  systemPrompt: string
  agentType: 'autonomous' | 'planning'
  planningPrompt?: string
  availableTools: string[]
  autoApprove: string[]
  planModel: { id: string; model: string }
  actModel: { id: string; model: string }
  visionModel: { id: string; model: string }
  maxContexts: number
  createdAt: Date
  updatedAt: Date
}

// Workflow state machine states
export enum WorkflowState {
  IDLE = 'idle',
  PLANNING = 'planning',
  WAITING_FOR_APPROVAL = 'waiting_for_approval',
  EXECUTING_TOOL = 'executing_tool',
  THINKING = 'thinking',
  PAUSED = 'paused',
  FINISHED = 'finished',
  ERROR = 'error'
}

// Workflow configuration
export interface Workflow {
  id: string
  title: string
  userQuery: string
  todoList: TodoItem[]
  status: 'pending' | 'running' | 'paused' | 'completed'
  agentId: string
  currentState: WorkflowState
  createdAt: Date
  updatedAt: Date
}

// Todo item structure
export interface TodoItem {
  id: string
  title: string
  description: string
  status: 'pending' | 'in_progress' | 'completed'
  dependencies?: string[]
  result?: unknown
}

// Message types
export interface WorkflowMessage {
  id: number
  sessionId: string
  role: 'assistant' | 'tool' | 'user' | 'system'
  message: string
  metadata?: Record<string, unknown>
  createdAt: Date
}

// Tool system types
export interface ToolDefinition {
  id: string
  name: string
  description: string
  parameters: Record<string, unknown> // JSON Schema
  implementation: 'rust' | 'typescript' | 'browser'
  requiresApproval: boolean
  category?: string
  // Handler for tools implemented in TypeScript/Browser
  handler?: (params: Record<string, unknown>) => Promise<unknown>
}

export interface ToolExecutionRequest {
  toolId: string
  parameters: Record<string, unknown>
  context?: unknown
}

export interface ToolExecutionResult {
  success: boolean
  result?: unknown
  error?: string
  metadata?: Record<string, unknown>
}

// Message queue types
export interface QueuedMessage {
  id: string
  sessionId: string
  role: 'user'
  message: string
  timestamp: Date
  priority: 'normal' | 'high' | 'interrupt'
}

export interface MessageQueue {
  sessionId: string
  messages: QueuedMessage[]
  isProcessing: boolean
}

// AI model response types
export interface ThoughtResponse {
  thought: string
  nextAction?: string
  reasoning?: string
}

export interface ActionResponse {
  tool: string
  parameters: Record<string, unknown>
  reasoning?: string
}

export interface PlanResponse {
  plan: TodoItem[]
  summary?: string
  estimatedSteps?: number
}

export interface ParsedLLMResponse {
  thought?: string
  action?: {
    tool: string
    parameters: Record<string, unknown>
  }
  finalAnswer?: string
}

// Context management types
export interface ConversationContext {
  messages: WorkflowMessage[]
  totalTokens: number
  maxTokens: number
  systemPrompt: string
  agent: Agent
}

// Approval request types
export interface ApprovalRequest {
  id: string
  workflowId: string
  tool: ToolDefinition
  parameters: Record<string, unknown>
  reason: string
  timestamp: Date
  status: 'pending' | 'approved' | 'denied'
}

// Event types for workflow system
export interface WorkflowEvent {
  type: string
  payload: unknown
  timestamp: Date
}

export interface StateChangeEvent extends WorkflowEvent {
  type: 'state_change'
  payload: {
    oldState: WorkflowState
    newState: WorkflowState
    reason?: string
  }
}

export interface ToolExecutionEvent extends WorkflowEvent {
  type: 'tool_execution'
  payload: {
    tool: string
    parameters: Record<string, unknown>
    result?: unknown
    error?: string
  }
}

export interface UserInteractionEvent extends WorkflowEvent {
  type: 'user_interaction'
  payload: {
    interactionType: 'message' | 'approval' | 'interrupt'
    data: unknown
  }
}
