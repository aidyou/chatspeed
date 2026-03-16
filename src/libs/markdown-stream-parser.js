/**
 * @class MarkdownStreamParser
 * @description A stateful parser for incrementally processing streaming Markdown text.
 * Fixed to be robust against missing newlines before closing fences and inline markers.
 */
export class MarkdownStreamParser {
  constructor() {
    this.buffer = ''
    this.blocks = []
    this.state = 'normal' // 'normal', 'in_code_block', 'in_math_block'
    this.fenceLength = 0 // Track the length of opening fence (for code blocks)
  }

  reset() {
    this.buffer = ''
    this.blocks.length = 0
    this.state = 'normal'
    this.fenceLength = 0
  }

  /**
   * Processes an incoming chunk of text and returns an array of parsed block objects.
   */
  process(chunk) {
    this.buffer += chunk

    let keepProcessing = true
    while (keepProcessing) {
      switch (this.state) {
        case 'normal':
          keepProcessing = this.parseNormal()
          break
        case 'in_code_block':
          keepProcessing = this.parseCodeBlock()
          break
        case 'in_math_block':
          keepProcessing = this.parseMathBlock()
          break
        default:
          keepProcessing = false
      }
    }

    // The rest of the buffer is the current, incomplete block
    const currentBlock = { type: this.stateToType(), content: this.buffer }

    // Return a combined view of finalized blocks and the current one
    return [...this.blocks, currentBlock]
  }

  end() {
    if (this.buffer.trim()) {
      this.blocks.push({ type: this.stateToType(), content: this.buffer })
    }
    const result = [...this.blocks]
    this.reset()
    return result
  }

  stateToType() {
    switch (this.state) {
      case 'in_code_block': return 'code'
      case 'in_math_block': return 'math'
      default: return 'paragraph'
    }
  }

  // Internal parsing methods

  parseNormal() {
    // Find code block fence (3 or more backticks)
    let codeIdx = -1
    let codeLen = 0
    for (let i = 0; i <= this.buffer.length - 3; i++) {
      if (this.buffer[i] === '`' && this.buffer[i + 1] === '`' && this.buffer[i + 2] === '`') {
        // Count the number of backticks
        let len = 3
        while (i + len < this.buffer.length && this.buffer[i + len] === '`') {
          len++
        }
        // Check if at line start
        const isAtLineStart = i === 0 || this.buffer[i - 1] === '\n'
        if (isAtLineStart) {
          codeIdx = i
          codeLen = len
          break
        }
      }
    }

    // Find math block fence (2 dollar signs)
    let mathIdx = -1
    for (let i = 0; i <= this.buffer.length - 2; i++) {
      if (this.buffer[i] === '$' && this.buffer[i + 1] === '$') {
        const isAtLineStart = i === 0 || this.buffer[i - 1] === '\n'
        if (isAtLineStart) {
          mathIdx = i
          break
        }
      }
    }

    const paraIdx = this.buffer.indexOf('\n\n')

    // Find the first occurrence among potential markers
    const found = [
      { idx: codeIdx, type: 'code', len: codeLen },
      { idx: mathIdx, type: 'math', len: 2 },
      { idx: paraIdx, type: 'para', len: 2 }
    ].filter(f => f.idx !== -1).sort((a, b) => a.idx - b.idx)

    if (found.length === 0) return false

    const first = found[0]

    // Push preceding content as a paragraph
    const contentBefore = this.buffer.substring(0, first.idx)
    if (contentBefore.trim()) {
      this.blocks.push({ type: 'paragraph', content: contentBefore })
    }
    this.buffer = this.buffer.substring(first.idx)

    if (first.type === 'para') {
      this.buffer = this.buffer.substring(2) // Skip the \n\n
      return true
    }

    if (first.type === 'code') {
      this.fenceLength = first.len // Remember the fence length
      this.state = 'in_code_block'
    } else {
      this.state = 'in_math_block'
    }
    return true
  }

  parseCodeBlock() {
    // Search for closing fence with the SAME length as opening fence
    // Start searching after the opening fence
    const startIdx = this.fenceLength

    let idx = -1
    for (let i = startIdx; i <= this.buffer.length - this.fenceLength; i++) {
      // Check if we found enough backticks
      let match = true
      for (let j = 0; j < this.fenceLength; j++) {
        if (this.buffer[i + j] !== '`') {
          match = false
          break
        }
      }

      if (match) {
        // Check if there are MORE backticks after (which would be a longer fence, not ours)
        if (i + this.fenceLength < this.buffer.length && this.buffer[i + this.fenceLength] === '`') {
          // This is a longer fence, skip it
          continue
        }

        // Check if at line start
        const isAtLineStart = i === 0 || this.buffer[i - 1] === '\n'
        if (isAtLineStart) {
          idx = i
          break
        }
      }
    }

    if (idx === -1) return false

    const blockEnd = idx + this.fenceLength
    const codeContent = this.buffer.substring(0, blockEnd)
    this.blocks.push({ type: 'code', content: codeContent })

    this.buffer = this.buffer.substring(blockEnd)
    if (this.buffer.startsWith('\n')) {
      this.buffer = this.buffer.substring(1)
    }
    this.state = 'normal'
    this.fenceLength = 0
    return true
  }

  parseMathBlock() {
    // Search for closing $$ starting from index 2
    let idx = this.buffer.indexOf('$$', 2)
    if (idx === -1) return false

    // CRITICAL FIX: Keep searching until we find a valid closing fence
    // A valid closing fence MUST be at the start of a line (after \n or at index 0)
    while (idx !== -1) {
      const isAtLineStart = idx === 0 || this.buffer[idx - 1] === '\n'
      if (isAtLineStart) {
        // Found a valid closing fence
        break
      }
      // Not a valid closing fence, search for the next one
      idx = this.buffer.indexOf('$$', idx + 2)
    }

    // No valid closing fence found
    if (idx === -1) return false

    const blockEnd = idx + 2
    const mathContent = this.buffer.substring(0, blockEnd)
    this.blocks.push({ type: 'math', content: mathContent })

    this.buffer = this.buffer.substring(blockEnd)
    if (this.buffer.startsWith('\n')) {
      this.buffer = this.buffer.substring(1)
    }
    this.state = 'normal'
    return true
  }
}
