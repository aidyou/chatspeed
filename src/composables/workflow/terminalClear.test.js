import assert from 'node:assert/strict'
import test from 'node:test'

import { terminalBlockTopRow, terminalClearSequence } from './terminalClear.js'

test('the retained block starts at the first row of the soft-wrapped input line', () => {
  // Rows 1 and 2 continue row 0; row 5 continues row 4; row 3 starts a block of its own.
  const continuationRows = new Set([1, 2, 5])
  assert.equal(terminalBlockTopRow(2, row => continuationRows.has(row)), 0)
  assert.equal(terminalBlockTopRow(5, row => continuationRows.has(row)), 4)
  assert.equal(terminalBlockTopRow(3, row => continuationRows.has(row)), 3)
  assert.equal(terminalBlockTopRow(0, () => true), 0)
})

test('clear moves the retained input line to the top row and erases the scrollback', () => {
  // Screen of 24 rows, single-line prompt on row 5 with the cursor in column 12.
  assert.equal(
    terminalClearSequence({ rows: 24, cursorX: 12, cursorY: 5, blockTopRow: 5 }),
    '\x1b[1;6r\x1b[H\x1b[5M\x1b[r\x1b[2;1H\x1b[0J\x1b[3J\x1b[1;13H'
  )
})

test('clear keeps a soft-wrapped input block together', () => {
  // Rows 3..5 belong to one wrapped command; the cursor sits on the block's last row.
  assert.equal(
    terminalClearSequence({ rows: 24, cursorX: 4, cursorY: 5, blockTopRow: 3 }),
    '\x1b[1;6r\x1b[H\x1b[3M\x1b[r\x1b[4;1H\x1b[0J\x1b[3J\x1b[3;5H'
  )
})

test('clear on the first row only erases the rows below it', () => {
  assert.equal(
    terminalClearSequence({ rows: 24, cursorX: 0, cursorY: 0, blockTopRow: 0 }),
    '\x1b[2;1H\x1b[0J\x1b[3J\x1b[1;1H'
  )
})

test('clear leaves a block that fills the screen in place and only drops the scrollback', () => {
  assert.equal(
    terminalClearSequence({ rows: 4, cursorX: 7, cursorY: 3, blockTopRow: 0 }),
    '\x1b[3J\x1b[4;8H'
  )
})