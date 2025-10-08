/**
 * API module for interacting with the backend workflow commands.
 */

import { invoke } from '@tauri-apps/api/core'

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
  createdAt: string
  updatedAt: string
}

export interface WorkflowMessage {
  id?: number
  sessionId: string
  role: string
  message: string
  metadata?: Record<string, unknown> | null
  createdAt?: string
}

export interface WorkflowSnapshot {
  workflow: Workflow
  messages: WorkflowMessage[]
}

// =================================================
//  API Functions
// =================================================

export const createWorkflow = (userQuery: string, agentId: string): Promise<Workflow> => {
  return invoke('create_workflow', { userQuery, agentId })
}

export const addWorkflowMessage = (
  message: Omit<WorkflowMessage, 'id' | 'createdAt'>
): Promise<WorkflowMessage> => {
  return invoke('add_workflow_message', { ...message })
}

export const updateWorkflowStatus = (workflowId: string, status: string): Promise<void> => {
  return invoke('update_workflow_status', { workflowId, status })
}

export const getWorkflowSnapshot = (workflowId: string): Promise<WorkflowSnapshot> => {
  return invoke('get_workflow_snapshot', { workflowId })
}

export const listWorkflows = (): Promise<Workflow[]> => {
  return invoke('list_workflows')
}
