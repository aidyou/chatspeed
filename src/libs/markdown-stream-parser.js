/**
 * @class MarkdownStreamParser
 * @description A stateful parser for incrementally processing streaming Markdown text.
 * It splits the text into logical blocks like paragraphs, code blocks, and math blocks
 * without re-parsing the entire text on each new chunk. This is highly efficient for
 * rendering large, streaming AI responses.
 *
 * @example
 * const parser = new MarkdownStreamParser();
 *
 * // In your streaming handler
 * onData(chunk) {
 *   const blocks = parser.process(chunk);
 *   // 'blocks' is an array of block objects, e.g.,
 *   // [{ type: 'code', lang: 'js', content: '...' }, { type: 'paragraph', content: '...' }]
 *   // The last block in the array may be incomplete as it's still being streamed.
 *   render(blocks);
 * }
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
   * @param {string} chunk - The new chunk of text from the stream.
   * @returns {Array<Object>} An array of block objects.
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

  /**
   * Finalizes any remaining content in the buffer.
   * Call this when the stream is finished.
   * @returns {Array<Object>} The complete array of all blocks.
   */
  end() {
    if (this.buffer.trim()) {
      this.blocks.push({ type: this.stateToType(), content: this.buffer })
    }
    this.buffer = ''
    return this.blocks
  }

  stateToType() {
    switch (this.state) {
      case 'in_code_block':
        return 'code'
      case 'in_math_block':
        return 'math'
      default:
        return 'paragraph'
    }
  }

  // Internal parsing methods based on state

  parseNormal() {
    const codeBlockStartIndex = this.buffer.indexOf('```')
    const mathBlockStartIndex = this.buffer.indexOf('$$')
    const paragraphEndIndex = this.buffer.indexOf('\n\n')

    const indices = [codeBlockStartIndex, mathBlockStartIndex, paragraphEndIndex].filter(
      i => i !== -1
    )

    if (indices.length === 0) {
      return false // Not enough content to form a block, wait for more chunks
    }

    const firstIndex = Math.min(...indices)

    // If a paragraph break comes first, or is the only thing found
    if (firstIndex === paragraphEndIndex) {
      const paragraphContent = this.buffer.substring(0, paragraphEndIndex)
      if (paragraphContent.trim()) {
        this.blocks.push({ type: 'paragraph', content: paragraphContent })
      }
      this.buffer = this.buffer.substring(paragraphEndIndex + 2)
      return true
    }

    // If a block marker comes first
    if (firstIndex === codeBlockStartIndex || firstIndex === mathBlockStartIndex) {
      const contentBefore = this.buffer.substring(0, firstIndex)
      if (contentBefore.trim()) {
        this.blocks.push({ type: 'paragraph', content: contentBefore })
      }
      this.buffer = this.buffer.substring(firstIndex)
      this.state = firstIndex === codeBlockStartIndex ? 'in_code_block' : 'in_math_block'
      return true
    }

    return false
  }

  parseCodeBlock() {
    const endIndex = this.buffer.indexOf('\n```')
    if (endIndex !== -1) {
      const blockEnd = endIndex + 4 // Include the closing ```
      const codeContent = this.buffer.substring(0, blockEnd)
      this.blocks.push({ type: 'code', content: codeContent })

      this.buffer = this.buffer.substring(blockEnd)
      // Consume potential newline after closing fence
      if (this.buffer.startsWith('\n')) {
        this.buffer = this.buffer.substring(1)
      }
      this.state = 'normal'
      return true
    }
    return false
  }

  parseMathBlock() {
    const endIndex = this.buffer.indexOf('$$')
    // Ensure it's not the opening $$ if the buffer just started
    if (endIndex > 1) {
      const blockEnd = endIndex + 2 // Include the closing $$
      const mathContent = this.buffer.substring(0, blockEnd)
      this.blocks.push({ type: 'math', content: mathContent })

      this.buffer = this.buffer.substring(blockEnd)
      // Consume potential newline after closing fence
      if (this.buffer.startsWith('\n')) {
        this.buffer = this.buffer.substring(1)
      }
      this.state = 'normal'
      return true
    }
    return false
  }
}
