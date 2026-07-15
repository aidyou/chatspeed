/**
 * Test for MarkdownStreamParser
 * Verifies the fix for nested code blocks issue
 */

import assert from 'node:assert/strict'
import { performance } from 'node:perf_hooks'
import { MarkdownStreamParser } from './markdown-stream-parser.js'

// Test case 1: Code block with ``` inside (the reported bug)
const test1 = `1. **进入仓库设置**
   \`\`\`
   仓库主页 → Settings → Features → Issues → "Set up templates"
   \`\`\``

console.log('Test 1: Code block with ``` inside')
console.log('Input:', test1)
console.log('')

const parser1 = new MarkdownStreamParser()
const result1 = parser1.process(test1)

console.log('Parsed blocks:')
result1.forEach((block, i) => {
  console.log(`Block ${i}:`, block.type)
  console.log('Content:', block.content)
  console.log('')
})

// Expected:
// Block 0: paragraph - "1. **进入仓库设置**\n   "
// Block 1: code - "```\n   仓库主页 → Settings → Features → Issues → \"Set up templates\"\n   ```"

console.log('='.repeat(60))
console.log('')

// Test case 2: Multiple code blocks
const test2 = `Here is some text.

\`\`\`javascript
const x = \`\`\` // This is inside the code block
\`\`\`

More text here.

\`\`\`python
print("Another code block")
\`\`\``

console.log('Test 2: Multiple code blocks with ``` inside')
console.log('Input:', test2)
console.log('')

const parser2 = new MarkdownStreamParser()
const result2 = parser2.process(test2)

console.log('Parsed blocks:')
result2.forEach((block, i) => {
  console.log(`Block ${i}:`, block.type)
  console.log('Content preview:', block.content.substring(0, 50) + '...')
  console.log('')
})

// Expected:
// Block 0: paragraph - "Here is some text.\n\n"
// Block 1: code - "```javascript\nconst x = ``` // This is inside the code block\n```"
// Block 2: paragraph - "\nMore text here.\n\n"
// Block 3: code - "```python\nprint(\"Another code block\")\n```"

console.log('='.repeat(60))
console.log('')

// Test case 3: Math block with $$ inside
const test3 = `Some text before.

$$
x = 5 \\
y = \$\$ \\
z = 10
$$

Text after.`

console.log('Test 3: Math block with $$ inside')
console.log('Input:', test3)
console.log('')

const parser3 = new MarkdownStreamParser()
const result3 = parser3.process(test3)

console.log('Parsed blocks:')
result3.forEach((block, i) => {
  console.log(`Block ${i}:`, block.type)
  console.log('Content preview:', block.content.substring(0, 50) + '...')
  console.log('')
})

console.log('All visual examples completed!')

const incrementalParser = new MarkdownStreamParser()
let incrementalResult = []
for (const chunk of ['prefix\n\n`', '``js\nconst x = 1\n`', '``\n\n$', '$\nx = 1\n$', '$\ntail']) {
  incrementalResult = incrementalParser.process(chunk)
}
assert.deepEqual(incrementalResult, [
  { type: 'paragraph', content: 'prefix' },
  { type: 'code', content: '```js\nconst x = 1\n```' },
  { type: 'math', content: '$$\nx = 1\n$$' },
  { type: 'paragraph', content: 'tail' }
])

const extendedOpeningFenceParser = new MarkdownStreamParser()
extendedOpeningFenceParser.process('```')
const extendedOpeningFenceResult = extendedOpeningFenceParser.process('`\ncode\n````\n')
assert.deepEqual(extendedOpeningFenceResult, [
  { type: 'code', content: '````\ncode\n````' },
  { type: 'paragraph', content: '' }
])

const extendedClosingFenceParser = new MarkdownStreamParser()
extendedClosingFenceParser.process('```\ncode\n```')
extendedClosingFenceParser.process('`\nstill code\n```')
const extendedClosingFenceResult = extendedClosingFenceParser.process('\ntail')
assert.deepEqual(extendedClosingFenceResult, [
  { type: 'code', content: '```\ncode\n````\nstill code\n```' },
  { type: 'paragraph', content: 'tail' }
])

const longTextParser = new MarkdownStreamParser()
const longText = 'a'.repeat(200_000)
const startedAt = performance.now()
for (let offset = 0; offset < longText.length; offset += 100) {
  longTextParser.process(longText.slice(offset, offset + 100))
}
const durationMs = performance.now() - startedAt
assert.ok(durationMs < 500, `incremental parsing took ${durationMs.toFixed(1)}ms`)

console.log(`Incremental assertions passed in ${durationMs.toFixed(1)}ms`)
