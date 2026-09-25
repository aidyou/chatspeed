// Local terminal clear used by the workflow terminal panel.
//
// ghostty-web only offers CSI 2J + CSI H, which wipes the row holding the input cursor. The panel
// therefore builds the clear itself to keep the xterm behaviour the terminal had before the
// ghostty-web migration: the input block survives as the first screen row while the rest of the
// screen and the scrollback history are erased.

/**
 * Row of the first line of the soft-wrapped block that ends at `cursorRow`.
 *
 * A long prompt or command can soft-wrap over several rows; the whole block is the "input line".
 * `isRowContinuation` receives a row and answers whether that row continues the row above it, which
 * is what ghostty-web reports from `IBufferLine.isWrapped` despite its xterm-style wording.
 */
export const terminalBlockTopRow = (cursorRow, isRowContinuation) => {
  let topRow = cursorRow
  while (topRow > 0 && isRowContinuation(topRow)) topRow -= 1
  return topRow
}

/**
 * Escape sequence that erases everything except the input block and moves the block to the top row,
 * leaving the cursor on the block's last row at its original column.
 *
 * Rows above the block are dropped with a scroll region limited to the cursor row, so the retained
 * block keeps its cells and styling instead of being reprinted.
 */
export const terminalClearSequence = ({ rows, cursorX, cursorY, blockTopRow }) => {
  const blockRows = cursorY - blockTopRow + 1
  let sequence = ''
  if (blockTopRow > 0) {
    sequence += `\x1b[1;${cursorY + 1}r\x1b[H\x1b[${blockTopRow}M\x1b[r`
  }
  if (blockRows < rows) {
    sequence += `\x1b[${blockRows + 1};1H\x1b[0J`
  }
  // Saved lines live outside the screen, so the scrollback is erased after the row shuffle.
  sequence += '\x1b[3J'
  return `${sequence}\x1b[${blockRows};${cursorX + 1}H`
}