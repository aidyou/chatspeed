/**
 * Test for enhanced code block fence support (>=3 backticks)
 */

import { MarkdownStreamParser } from './markdown-stream-parser.js'

console.log('='.repeat(70))
console.log('Test 1: 4 backticks containing 3 backticks (nested code blocks)')
console.log('='.repeat(70))

const test1 = '````\nabc\n```js\nlet x=\'y\';\n```\ndef\n````'

console.log('Input:')
console.log(test1)
console.log('')

const parser1 = new MarkdownStreamParser()
const result1 = parser1.process(test1)

console.log('Parsed blocks:')
result1.forEach((block, i) => {
  console.log(`\n--- Block ${i} (${block.type}) ---`)
  console.log(block.content)
})

console.log('\n' + '='.repeat(70))
console.log('Test 2: 5 backticks containing 4 backticks')
console.log('='.repeat(70))

const test2 = '`````\nHere is some text with 4 backticks:\n````\ncode inside\n````\nAnd more text\n`````'

console.log('Input:')
console.log(test2)
console.log('')

const parser2 = new MarkdownStreamParser()
const result2 = parser2.process(test2)

console.log('Parsed blocks:')
result2.forEach((block, i) => {
  console.log(`\n--- Block ${i} (${block.type}) ---`)
  console.log(block.content)
})

console.log('\n' + '='.repeat(70))
console.log('Test 3: Regular 3-backtick code block')
console.log('='.repeat(70))

const test3 = 'Some text before.\n\n```javascript\nconst x = 123;\n```\n\nText after.'

console.log('Input:')
console.log(test3)
console.log('')

const parser3 = new MarkdownStreamParser()
const result3 = parser3.process(test3)

console.log('Parsed blocks:')
result3.forEach((block, i) => {
  console.log(`\n--- Block ${i} (${block.type}) ---`)
  console.log(block.content)
})

console.log('\n' + '='.repeat(70))
console.log('Test 4: Code block with varying backticks inside')
console.log('='.repeat(70))

const test4 = '````javascript\n// This is a code block with 4 backticks\n// Inside we can have:\n// ``` - 3 backticks\n// ````` - 5 backticks\n// And the code block still continues\n````'

console.log('Input:')
console.log(test4)
console.log('')

const parser4 = new MarkdownStreamParser()
const result4 = parser4.process(test4)

console.log('Parsed blocks:')
result4.forEach((block, i) => {
  console.log(`\n--- Block ${i} (${block.type}) ---`)
  console.log(block.content)
})

console.log('\n' + '='.repeat(70))
console.log('Test 5: Multiple code blocks with different fence lengths')
console.log('='.repeat(70))

const test5 = 'Some intro text.\n\n```\nSimple 3-backtick block\n```\n\nMiddle text.\n\n````\n4-backtick block\nwith ``` inside\n````\n\nEnd text.'

console.log('Input:')
console.log(test5)
console.log('')

const parser5 = new MarkdownStreamParser()
const result5 = parser5.process(test5)

console.log('Parsed blocks:')
result5.forEach((block, i) => {
  console.log(`\n--- Block ${i} (${block.type}) ---`)
  const preview = block.content.length > 100
    ? block.content.substring(0, 100) + '...'
    : block.content
  console.log(preview)
})

console.log('\n' + '='.repeat(70))
console.log('All tests completed!')
console.log('='.repeat(70))