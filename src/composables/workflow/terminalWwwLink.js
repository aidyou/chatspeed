// Bare `www.`-prefixed host detection for the workflow terminal.
//
// ghostty-web's UrlRegexProvider only matches URLs that carry an explicit scheme, so a host written
// as `www.example.com` in command output stays plain text. This provider adds those bare www hosts
// as links without disturbing scheme URLs, which the scheme provider already owns.

// A www host is `www.` followed by at least one label and a letter TLD (so `www.example.com` and
// `www.example.com.cn` match, while `www.com` or `www.example` do not). The negative lookbehind
// rejects a preceding word character, dot, colon, dash, or slash so `http://www.example.com` and
// `subwww.example.com` are not matched here; the lookahead stops before trailing punctuation. Hosts
// without a leading `www.` (for example `abc.def.eft.com`) never match, which is intentional.
const WWW_HOST_PATTERN =
  /(?<![A-Za-z0-9_.:/-])www\.(?:[A-Za-z0-9_-]+\.)+[A-Za-z]{2,}(?![A-Za-z0-9_-])/gi

// Convert one buffer row to text with one character per cell, so a regex index equals the terminal
// column. This mirrors ghostty-web's own URL provider and keeps wide or control cells aligned.
const rowToText = line => {
  let text = ''
  for (let x = 0; x < line.length; x += 1) {
    const codepoint = line.getCell(x)?.getCodepoint() ?? 0
    text += codepoint === 0 || codepoint < 32 ? ' ' : String.fromCodePoint(codepoint)
  }
  return text
}

// Return the www hosts on a single line as { text, start, end } with an inclusive end column.
export function findWwwHosts(lineText) {
  const hosts = []
  if (!lineText) return hosts
  // The shared pattern keeps lastIndex between global exec calls; reset before each scan and the
  // loop runs to exhaustion, so the field always returns to 0 for the next caller.
  WWW_HOST_PATTERN.lastIndex = 0
  let match = WWW_HOST_PATTERN.exec(lineText)
  while (match !== null) {
    hosts.push({ text: match[0], start: match.index, end: match.index + match[0].length - 1 })
    match = WWW_HOST_PATTERN.exec(lineText)
  }
  return hosts
}

// Build a ghostty-web ILinkProvider that highlights bare www hosts. openLink receives the modifier
// event and the matched host text so the caller decides how to resolve the scheme.
export function createWwwLinkProvider(terminal, openLink) {
  return {
    provideLinks(y, callback) {
      const line = terminal.buffer.active.getLine(y)
      if (!line) {
        callback(undefined)
        return
      }
      const hosts = findWwwHosts(rowToText(line))
      if (!hosts.length) {
        callback(undefined)
        return
      }
      callback(
        hosts.map(host => ({
          text: host.text,
          range: { start: { x: host.start, y }, end: { x: host.end, y } },
          activate: event => openLink(event, host.text)
        }))
      )
    },
    dispose() {}
  }
}
