// Terminal colour skins offered in Settings -> General -> Terminal.
//
// ghostty resolves terminal colours when a terminal is created, so every skin ships both a dark and
// a light palette and the terminal picks the one matching the resolved light/dark scheme. Entries
// 0-15 are the ANSI palette that drives `ls`, diff, and other coloured output; the remaining fields
// are the canvas colours the renderer draws with.
//
// Values are copied from the upstream theme definitions named in `source`; the default skin keeps
// the application's own `--cs-terminal-*` tokens and therefore carries no palette.
// The wasm configuration treats 0x000000 as "use the built-in default", so a scheme whose black is
// pure black (Ayu Light) falls back to that default there.

export const DEFAULT_TERMINAL_SKIN = 'default'

export const TERMINAL_SKINS = Object.freeze([
  {
    id: DEFAULT_TERMINAL_SKIN,
    labelKey: 'settings.general.terminalSkinDefault'
  },
  {
    id: 'ayu',
    labelKey: 'settings.general.terminalSkinAyu',
    source: 'ayu dark and ayu light (iTerm2-Color-Schemes ghostty/Ayu, ghostty/Ayu Light)',
    palettes: {
      dark: {
        background: '#0b0e14',
        foreground: '#bfbdb6',
        cursor: '#e6b450',
        cursorAccent: '#0b0e14',
        selectionBackground: '#409fff',
        selectionForeground: '#0b0e14',
        black: '#11151c',
        red: '#ea6c73',
        green: '#7fd962',
        yellow: '#f9af4f',
        blue: '#53bdfa',
        magenta: '#cda1fa',
        cyan: '#90e1c6',
        white: '#c7c7c7',
        brightBlack: '#686868',
        brightRed: '#f07178',
        brightGreen: '#aad94c',
        brightYellow: '#ffb454',
        brightBlue: '#59c2ff',
        brightMagenta: '#d2a6ff',
        brightCyan: '#95e6cb',
        brightWhite: '#ffffff'
      },
      light: {
        background: '#f8f9fa',
        foreground: '#5c6166',
        cursor: '#ffaa33',
        cursorAccent: '#f8f9fa',
        selectionBackground: '#035bd6',
        selectionForeground: '#f8f9fa',
        black: '#000000',
        red: '#ea6c6d',
        green: '#6cbf43',
        yellow: '#eca944',
        blue: '#3199e1',
        magenta: '#9e75c7',
        cyan: '#46ba94',
        white: '#bababa',
        brightBlack: '#686868',
        brightRed: '#f07171',
        brightGreen: '#86b300',
        brightYellow: '#f2ae49',
        brightBlue: '#399ee6',
        brightMagenta: '#a37acc',
        brightCyan: '#4cbf99',
        brightWhite: '#d1d1d1'
      }
    }
  },
  {
    id: 'catppuccin',
    labelKey: 'settings.general.terminalSkinCatppuccin',
    source: 'catppuccin mocha and latte (catppuccin/ghostty catppuccin-mocha, catppuccin-latte)',
    palettes: {
      dark: {
        background: '#1e1e2e',
        foreground: '#cdd6f4',
        cursor: '#f5e0dc',
        cursorAccent: '#11111b',
        selectionBackground: '#353749',
        selectionForeground: '#cdd6f4',
        black: '#45475a',
        red: '#f38ba8',
        green: '#a6e3a1',
        yellow: '#f9e2af',
        blue: '#89b4fa',
        magenta: '#f5c2e7',
        cyan: '#94e2d5',
        white: '#a6adc8',
        brightBlack: '#585b70',
        brightRed: '#f38ba8',
        brightGreen: '#a6e3a1',
        brightYellow: '#f9e2af',
        brightBlue: '#89b4fa',
        brightMagenta: '#f5c2e7',
        brightCyan: '#94e2d5',
        brightWhite: '#bac2de'
      },
      light: {
        background: '#eff1f5',
        foreground: '#4c4f69',
        cursor: '#dc8a78',
        cursorAccent: '#eff1f5',
        selectionBackground: '#d8dae1',
        selectionForeground: '#4c4f69',
        black: '#5c5f77',
        red: '#d20f39',
        green: '#40a02b',
        yellow: '#df8e1d',
        blue: '#1e66f5',
        magenta: '#ea76cb',
        cyan: '#179299',
        white: '#acb0be',
        brightBlack: '#6c6f85',
        brightRed: '#d20f39',
        brightGreen: '#40a02b',
        brightYellow: '#df8e1d',
        brightBlue: '#1e66f5',
        brightMagenta: '#ea76cb',
        brightCyan: '#179299',
        brightWhite: '#bcc0cc'
      }
    }
  },
  {
    id: 'tokyo-night',
    labelKey: 'settings.general.terminalSkinTokyoNight',
    source: 'tokyo night and tokyo night day (folke/tokyonight.nvim extras/ghostty)',
    palettes: {
      dark: {
        background: '#1a1b26',
        foreground: '#c0caf5',
        cursor: '#c0caf5',
        selectionBackground: '#283457',
        selectionForeground: '#c0caf5',
        black: '#15161e',
        red: '#f7768e',
        green: '#9ece6a',
        yellow: '#e0af68',
        blue: '#7aa2f7',
        magenta: '#bb9af7',
        cyan: '#7dcfff',
        white: '#a9b1d6',
        brightBlack: '#414868',
        brightRed: '#ff899d',
        brightGreen: '#9fe044',
        brightYellow: '#faba4a',
        brightBlue: '#8db0ff',
        brightMagenta: '#c7a9ff',
        brightCyan: '#a4daff',
        brightWhite: '#c0caf5'
      },
      light: {
        background: '#e1e2e7',
        foreground: '#3760bf',
        cursor: '#3760bf',
        selectionBackground: '#b7c1e3',
        selectionForeground: '#3760bf',
        black: '#b4b5b9',
        red: '#f52a65',
        green: '#587539',
        yellow: '#8c6c3e',
        blue: '#2e7de9',
        magenta: '#9854f1',
        cyan: '#007197',
        white: '#6172b0',
        brightBlack: '#a1a6c5',
        brightRed: '#ff4774',
        brightGreen: '#5c8524',
        brightYellow: '#a27629',
        brightBlue: '#358aff',
        brightMagenta: '#a463ff',
        brightCyan: '#007ea8',
        brightWhite: '#3760bf'
      }
    }
  }
])

// Returns the palette a skin uses for the resolved light/dark scheme, or undefined when the skin
// follows the application's own terminal tokens.
export const terminalSkinPalette = (skinId, dark) => {
  const palettes = TERMINAL_SKINS.find(skin => skin.id === skinId)?.palettes
  return palettes ? palettes[dark ? 'dark' : 'light'] : undefined
}