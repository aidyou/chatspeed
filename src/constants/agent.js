export const AGENT_ROLE = Object.freeze({
  PRIMARY: 'primary',
  CHILD: 'child'
})

export const AGENT_ROLE_OPTIONS = Object.freeze([
  { labelKey: 'settings.agent.rolePrimary', value: AGENT_ROLE.PRIMARY },
  { labelKey: 'settings.agent.roleChild', value: AGENT_ROLE.CHILD }
])

export const SUB_AGENT_ROLE = Object.freeze({
  EXPLORER: 'explorer',
  FINAL_REVIEWER: 'final_reviewer'
})

export const SUB_AGENT_ROLE_OPTIONS = Object.freeze([
  { value: '', labelKey: 'settings.agent.subAgentRoleGeneral' },
  { value: SUB_AGENT_ROLE.EXPLORER, labelKey: 'settings.agent.subAgentRoleExplorer' },
  { value: SUB_AGENT_ROLE.FINAL_REVIEWER, labelKey: 'settings.agent.subAgentRoleFinalReviewer' }
])
