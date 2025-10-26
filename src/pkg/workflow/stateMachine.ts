/**
 * State machine for managing workflow lifecycle
 */

import type { StateChangeEvent } from './types'
import { WorkflowState } from './types'

// Simple EventEmitter implementation for browser compatibility
class EventEmitter {
  private events: Record<string, Array<(data?: unknown) => void>> = {}

  on(event: string, listener: (data?: unknown) => void): void {
    if (!this.events[event]) {
      this.events[event] = []
    }
    this.events[event].push(listener)
  }

  emit(event: string, data?: unknown): void {
    if (this.events[event]) {
      this.events[event].forEach(listener => listener(data))
    }
  }
}

export interface StateTransition {
  from: WorkflowState | WorkflowState[]
  to: WorkflowState
  event: string
  guard?: (context: unknown) => boolean
  action?: (context: unknown) => void | Promise<void>
}

export class WorkflowStateMachine extends EventEmitter {
  private currentState: WorkflowState
  private transitions: Map<string, StateTransition[]>
  private context: unknown
  private stateHistory: Array<{ state: WorkflowState; timestamp: Date }>

  constructor(initialState: WorkflowState = WorkflowState.IDLE) {
    super()
    this.currentState = initialState
    this.transitions = new Map()
    this.context = {}
    this.stateHistory = [{ state: initialState, timestamp: new Date() }]
    this.defineTransitions()
  }

  /**
   * Define all valid state transitions
   */
  private defineTransitions(): void {
    // From IDLE
    this.addTransition({
      from: WorkflowState.IDLE,
      to: WorkflowState.PLANNING,
      event: 'START_PLANNING'
    })

    this.addTransition({
      from: WorkflowState.IDLE,
      to: WorkflowState.THINKING,
      event: 'START_EXECUTION'
    })

    // From PLANNING
    this.addTransition({
      from: WorkflowState.PLANNING,
      to: WorkflowState.WAITING_FOR_APPROVAL,
      event: 'PLAN_READY'
    })

    this.addTransition({
      from: WorkflowState.PLANNING,
      to: WorkflowState.THINKING,
      event: 'SKIP_APPROVAL'
    })

    this.addTransition({
      from: WorkflowState.PLANNING,
      to: WorkflowState.ERROR,
      event: 'PLANNING_FAILED'
    })

    // From WAITING_FOR_APPROVAL
    this.addTransition({
      from: WorkflowState.WAITING_FOR_APPROVAL,
      to: WorkflowState.THINKING,
      event: 'APPROVAL_GRANTED'
    })

    this.addTransition({
      from: WorkflowState.WAITING_FOR_APPROVAL,
      to: WorkflowState.IDLE,
      event: 'APPROVAL_DENIED'
    })

    // From THINKING
    this.addTransition({
      from: WorkflowState.THINKING,
      to: WorkflowState.EXECUTING_TOOL,
      event: 'EXECUTE_TOOL'
    })

    this.addTransition({
      from: WorkflowState.THINKING,
      to: WorkflowState.WAITING_FOR_APPROVAL,
      event: 'NEED_APPROVAL'
    })

    this.addTransition({
      from: WorkflowState.THINKING,
      to: WorkflowState.FINISHED,
      event: 'TASK_COMPLETE'
    })

    this.addTransition({
      from: WorkflowState.THINKING,
      to: WorkflowState.PAUSED,
      event: 'PAUSE'
    })

    // From EXECUTING_TOOL
    this.addTransition({
      from: WorkflowState.EXECUTING_TOOL,
      to: WorkflowState.THINKING,
      event: 'TOOL_COMPLETE'
    })

    this.addTransition({
      from: WorkflowState.EXECUTING_TOOL,
      to: WorkflowState.ERROR,
      event: 'TOOL_FAILED'
    })

    this.addTransition({
      from: WorkflowState.EXECUTING_TOOL,
      to: WorkflowState.WAITING_FOR_APPROVAL,
      event: 'NEED_APPROVAL'
    })

    // From PAUSED
    this.addTransition({
      from: WorkflowState.PAUSED,
      to: WorkflowState.THINKING,
      event: 'RESUME'
    })

    this.addTransition({
      from: WorkflowState.PAUSED,
      to: WorkflowState.IDLE,
      event: 'RESET'
    })

    // From ERROR
    this.addTransition({
      from: WorkflowState.ERROR,
      to: WorkflowState.IDLE,
      event: 'RESET'
    })

    this.addTransition({
      from: WorkflowState.ERROR,
      to: WorkflowState.THINKING,
      event: 'RETRY'
    })

    // From FINISHED
    this.addTransition({
      from: WorkflowState.FINISHED,
      to: WorkflowState.THINKING,
      event: 'TASK_CONTINUE'
    })

    // Global transitions (can happen from any state)
    this.addTransition({
      from: [
        WorkflowState.PLANNING,
        WorkflowState.THINKING,
        WorkflowState.EXECUTING_TOOL,
        WorkflowState.WAITING_FOR_APPROVAL
      ],
      to: WorkflowState.PAUSED,
      event: 'PAUSE'
    })

    this.addTransition({
      from: [
        WorkflowState.PLANNING,
        WorkflowState.THINKING,
        WorkflowState.EXECUTING_TOOL,
        WorkflowState.WAITING_FOR_APPROVAL,
        WorkflowState.PAUSED
      ],
      to: WorkflowState.ERROR,
      event: 'ERROR_OCCURRED'
    })
  }

  /**
   * Add a state transition
   */
  private addTransition(transition: StateTransition): void {
    if (!this.transitions.has(transition.event)) {
      this.transitions.set(transition.event, [])
    }
    // This is safe because we just ensured the array exists.
    const eventTransitions = this.transitions.get(transition.event)
    if (eventTransitions) {
      eventTransitions.push(transition)
    }
  }

  /**
   * Trigger a state transition
   */
  public async transition(event: string, payload?: unknown): Promise<boolean> {
    const transitions = this.transitions.get(event)
    if (!transitions) {
      console.warn(`No transitions defined for event: ${event}`)
      return false
    }

    for (const transition of transitions) {
      const fromStates = Array.isArray(transition.from) ? transition.from : [transition.from]

      if (fromStates.includes(this.currentState)) {
        // Check guard condition if present
        if (transition.guard && !transition.guard(this.context)) {
          continue
        }

        const oldState = this.currentState
        this.currentState = transition.to
        this.stateHistory.push({ state: this.currentState, timestamp: new Date() })

        // Execute action if present
        if (transition.action) {
          await transition.action(this.context)
        }

        // Emit state change event
        const stateChangeEvent: StateChangeEvent = {
          type: 'state_change',
          payload: {
            oldState,
            newState: this.currentState,
            reason: event
          },
          timestamp: new Date()
        }

        this.emit('stateChanged', stateChangeEvent, payload)
        return true
      }
    }

    console.warn(`Invalid transition: ${event} from state ${this.currentState}`)
    return false
  }

  /**
   * Get current state
   */
  public getState(): WorkflowState {
    return this.currentState
  }

  /**
   * Directly sets the state. Should only be used when restoring from a snapshot.
   */
  public setState(newState: WorkflowState): void {
    this.currentState = newState
  }

  /**
   * Set context data
   */
  public setContext(context: unknown): void {
    if (typeof context === 'object' && context !== null) {
      this.context = {
        ...(this.context as Record<string, unknown>),
        ...(context as Record<string, unknown>)
      }
    } else {
      console.warn('Attempted to set context with a non-object value:', context)
    }
  }

  /**
   * Get context data
   */
  public getContext(): unknown {
    return this.context
  }

  /**
   * Get state history
   */
  public getHistory(): Array<{ state: WorkflowState; timestamp: Date }> {
    return [...this.stateHistory]
  }

  /**
   * Check if current state is one of the given states
   */
  public isInState(...states: WorkflowState[]): boolean {
    return states.includes(this.currentState)
  }

  /**
   * Check if transition is valid from current state
   */
  public canTransition(event: string): boolean {
    const transitions = this.transitions.get(event)
    if (!transitions) return false

    return transitions.some(t => {
      const fromStates = Array.isArray(t.from) ? t.from : [t.from]
      return fromStates.includes(this.currentState) && (!t.guard || t.guard(this.context))
    })
  }

  /**
   * Reset state machine
   */
  public reset(): void {
    this.currentState = WorkflowState.IDLE
    this.context = {}
    this.stateHistory = [{ state: WorkflowState.IDLE, timestamp: new Date() }]
    this.emit('stateChanged', {
      type: 'state_change',
      payload: {
        oldState: this.currentState,
        newState: WorkflowState.IDLE,
        reason: 'RESET'
      },
      timestamp: new Date()
    })
  }
}
