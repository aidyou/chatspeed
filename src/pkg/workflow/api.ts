/**
 * API module for interacting with the backend workflow commands.
 */

import { invoke } from '@tauri-apps/api/core'
import type { ToolExecutionResult } from './types'

// =================================================
//  Interfaces
// =================================================

export interface Workflow {
  id: string
  title: string | null
  userQuery: string
  todoList: string | null
  status: string
  agentId: string
  allowedPaths?: string[] | null
  createdAt: string
  updatedAt: string
}

export interface WorkflowMessage {
  id?: number
  sessionId: string
  role: string
  message: string
  metadata?: Record<string, unknown> | null
  stepType?: string | null
  stepIndex?: number
  createdAt?: string
}

export interface WorkflowSnapshot {
  workflow: Workflow
  messages: WorkflowMessage[]
}

export interface WorkflowResponse {
  workflow: Workflow
  sessionKey: string
}

// =================================================
//  API Functions
// =================================================

export const createWorkflow = (
  userQuery: string,
  agentId: string,
  allowedPaths: string[] | null = null
): Promise<string> => {
  const id = `session_${Date.now()}`
  return invoke('create_workflow', {
    workflow: {
      id,
      userQuery,
      agentId,
      status: 'pending',
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      allowedPaths
    }
  })
}

export const addWorkflowMessage = (
  message: Omit<WorkflowMessage, 'id' | 'createdAt'>
): Promise<WorkflowMessage> => {
  return invoke('add_workflow_message', {
    sessionId: message.sessionId,
    role: message.role,
    message: message.message,
    metadata: message.metadata,
    stepType: message.stepType || null,
    stepIndex: message.stepIndex || 0
  })
}

export const updateWorkflowStatus = (workflowId: string, status: string): Promise<void> => {
  return invoke('update_workflow_status', { workflowId, status })
}

export const getWorkflowSnapshot = (workflowId: string): Promise<WorkflowSnapshot> => {
  return invoke('get_workflow_snapshot', { workflowId })
}

export const getWorkflowSessionKey = (workflowId: string): Promise<string> => {
  return invoke('get_workflow_session_key', { workflowId })
}

export const updateWorkflowTodoList = (workflowId: string, todoList: string): Promise<void> => {
  return invoke('update_workflow_todo_list', { workflowId, todoList })
}

export const listWorkflows = (): Promise<Workflow[]> => {
  return invoke('list_workflows')
}

export const workflowCallTool = (
  toolName: string,
  args?: Record<string, unknown>
): Promise<ToolExecutionResult> => {
  return invoke('workflow_call_tool', { toolName, arguments: args })
}
