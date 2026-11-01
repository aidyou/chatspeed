export const DEFAULT_AGENT_RESOURCE_LIMITS = Object.freeze({
  enabled: true,
  totalMemoryMb: 8192,
  maxConcurrentTasks: 4,
  taskMemoryMb: 4096,
  taskMaxProcesses: 128
})

export function validateAgentResourceLimits(limits) {
  const inRange = (value, minimum, maximum) =>
    Number.isInteger(value) && value >= minimum && value <= maximum
  return typeof limits?.enabled === 'boolean' &&
    inRange(limits.totalMemoryMb, 128, 1048576) &&
    inRange(limits.taskMemoryMb, 128, limits.totalMemoryMb) &&
    inRange(limits.maxConcurrentTasks, 1, 64) &&
    inRange(limits.taskMaxProcesses, 1, 4096)
}
