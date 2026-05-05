/**
 * Clear test demonstrating the bug
 */

import { MarkdownStreamParser } from './markdown-stream-parser.js'

// The real bug: ``` inside code block being treated as closing fence
const testInput = `\`\`\`javascript
// This is a code block
const example = \`\`\` // <- this is inside the code block!
console.log(example)
\`\`\`

Some text after.`

console.log('Test: Code block containing ``` inside')
console.log('Input:')
console.log(testInput)
console.log('')

const parser = new MarkdownStreamParser()
const result = parser.process(testInput)

console.log('Parsed blocks:')
result.forEach((block, i) => {
  console.log(`\n--- Block ${i} (${block.type}) ---`)
  console.log(block.content)
})

console.log('\n' + '='.repeat(60))
console.log('\nExpected:')
console.log('Block 0 (code): Should contain ALL lines between first ``` and last ```')
console.log('Block 1 (paragraph): "Some text after."')
console.log('')
console.log('If you see multiple blocks inside the code, that\'s the BUG!')