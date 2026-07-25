const appendContractItems = (parts, title, items, renderDetails) => {
  if (!Array.isArray(items) || !items.length) return
  parts.push('', `### ${title}`)
  items.forEach(item => {
    if (!item || typeof item !== 'object') return
    const id = String(item.id || '').trim()
    const description = String(item.description || '').trim()
    if (!id && !description) return
    parts.push(`- **${id || '-'}**: ${description}`)
    renderDetails?.(parts, item)
  })
}

const formatIdList = value =>
  Array.isArray(value) && value.length ? value.map(item => `\`${item}\``).join(', ') : ''

export const formatPlanApprovalMarkdown = (details, translate) => {
  if (!details || typeof details !== 'object' || typeof details.plan !== 'string') {
    return ''
  }

  const t = typeof translate === 'function' ? translate : key => key
  const parts = [details.plan.trim()]
  const contract = details.acceptance_contract
  if (!contract || typeof contract !== 'object') {
    return parts.join('\n')
  }

  parts.push('', `## ${t('workflow.approval.acceptanceContract')}`)
  appendContractItems(
    parts,
    t('workflow.approval.acceptanceCriteria'),
    contract.acceptance_criteria
  )
  appendContractItems(parts, t('workflow.approval.invariants'), contract.invariants)
  appendContractItems(
    parts,
    t('workflow.approval.implementationUnits'),
    contract.implementation_units,
    (target, item) => {
      const covers = formatIdList(item.covers)
      const dependencies = formatIdList(item.depends_on)
      const files = Array.isArray(item.files) ? item.files.filter(Boolean).join(', ') : ''
      if (covers) target.push(`  - ${t('workflow.approval.covers')}: ${covers}`)
      if (dependencies) {
        target.push(`  - ${t('workflow.approval.dependsOn')}: ${dependencies}`)
      }
      if (files) target.push(`  - ${t('workflow.approval.files')}: ${files}`)
    }
  )
  appendContractItems(
    parts,
    t('workflow.approval.verificationItems'),
    contract.verification_items,
    (target, item) => {
      const covers = formatIdList(item.covers)
      if (covers) target.push(`  - ${t('workflow.approval.covers')}: ${covers}`)
      if (item.method) target.push(`  - ${t('workflow.approval.method')}: ${item.method}`)
      if (item.expected_evidence) {
        target.push(
          `  - ${t('workflow.approval.expectedEvidence')}: ${item.expected_evidence}`
        )
      }
    }
  )
  appendContractItems(
    parts,
    t('workflow.approval.unresolvedBlockers'),
    contract.unresolved_blockers
  )

  return parts.join('\n')
}
