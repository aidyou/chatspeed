export const AGENT_ROLE = Object.freeze({
  PRIMARY: 'primary',
  CHILD: 'child'
})

export const AGENT_ROLE_OPTIONS = Object.freeze([
  { labelKey: 'settings.agent.rolePrimary', value: AGENT_ROLE.PRIMARY },
  { labelKey: 'settings.agent.roleChild', value: AGENT_ROLE.CHILD }
])
