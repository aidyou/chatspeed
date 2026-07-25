import assert from 'node:assert/strict'
import test from 'node:test'

import { formatPlanApprovalMarkdown } from './planApproval.js'

test('submit_plan approval renders the structured acceptance contract as readable markdown', () => {
  const markdown = formatPlanApprovalMarkdown(
    {
      plan: '# Plan',
      acceptance_contract: {
        acceptance_criteria: [{ id: 'AC-1', description: 'Role is persisted' }],
        invariants: [{ id: 'INV-1', description: 'Primary agents remain unchanged' }],
        implementation_units: [
          {
            id: 'U-1',
            description: 'Add role storage',
            covers: ['AC-1', 'INV-1'],
            depends_on: [],
            files: ['src-tauri/src/db/agent.rs']
          }
        ],
        verification_items: [
          {
            id: 'V-1',
            description: 'Run the focused test',
            covers: ['AC-1', 'INV-1'],
            method: 'cargo test focused',
            expected_evidence: 'The test passes'
          }
        ],
        unresolved_blockers: []
      }
    },
    key => key.split('.').at(-1)
  )

  assert.match(markdown, /^# Plan/)
  assert.match(markdown, /acceptanceContract/)
  assert.match(markdown, /\*\*AC-1\*\*: Role is persisted/)
  assert.match(markdown, /\*\*U-1\*\*: Add role storage/)
  assert.match(markdown, /covers: `AC-1`, `INV-1`/)
  assert.match(markdown, /src-tauri\/src\/db\/agent\.rs/)
  assert.match(markdown, /method: cargo test focused/)
  assert.doesNotMatch(markdown, /"acceptance_criteria"/)
})
