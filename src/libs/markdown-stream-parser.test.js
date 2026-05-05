/**
 * Test for MarkdownStreamParser
 * Verifies the fix for nested code blocks issue
 */

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

console.log('All tests completed!')