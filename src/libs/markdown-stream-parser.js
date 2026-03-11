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
  }

  reset() {
    this.buffer = ''
    this.blocks.length = 0
    this.state = 'normal'
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
    const codeIdx = this.buffer.indexOf('```')
    const mathIdx = this.buffer.indexOf('$$')
    const paraIdx = this.buffer.indexOf('\n\n')

    // Find the first occurrence among potential markers
    const found = [
      { idx: codeIdx, type: 'code', len: 3 },
      { idx: mathIdx, type: 'math', len: 2 },
      { idx: paraIdx, type: 'para', len: 2 }
    ].filter(f => f.idx !== -1).sort((a, b) => a.idx - b.idx)

    if (found.length === 0) return false

    const first = found[0]

    // CRITICAL FIX: Code and Math blocks MUST start at the beginning of a line
    if (first.type === 'code' || first.type === 'math') {
      const isAtLineStart = first.idx === 0 || this.buffer[first.idx - 1] === '\n'
      if (!isAtLineStart) {
        // Not a real block start, treat the content up to marker as paragraph and continue
        const content = this.buffer.substring(0, first.idx + first.len)
        this.blocks.push({ type: 'paragraph', content })
        this.buffer = this.buffer.substring(first.idx + first.len)
        return true
      }
    }

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

    this.state = first.type === 'code' ? 'in_code_block' : 'in_math_block'
    return true
  }

  parseCodeBlock() {
    // Search for closing ``` starting from index 3 to skip the opening one
    const idx = this.buffer.indexOf('```', 3)
    if (idx === -1) return false

    // Check if closing fence is valid (at start of line or following a newline)
    const isAtLineStart = idx === 0 || this.buffer[idx - 1] === '\n'
    
    // We allow closing even if not at line start to be more permissive with messy LLM output,
    // but standard MD prefers line start. 
    // Optimization: If there's content followed by ```, we consider it the end.
    const blockEnd = idx + 3
    const codeContent = this.buffer.substring(0, blockEnd)
    this.blocks.push({ type: 'code', content: codeContent })

    this.buffer = this.buffer.substring(blockEnd)
    if (this.buffer.startsWith('\n')) {
      this.buffer = this.buffer.substring(1)
    }
    this.state = 'normal'
    return true
  }

  parseMathBlock() {
    // Search for closing $$ starting from index 2
    const idx = this.buffer.indexOf('$$', 2)
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
