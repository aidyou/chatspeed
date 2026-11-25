//! Terminal panel: Vue's docked terminal surface backed by an in-process VT.

use std::ops::Range;
use std::time::Duration;

use crate::Observer;
use alacritty_terminal::{
    event::VoidListener,
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point as CellPoint, Side},
    selection::{Selection, SelectionType},
    term::{
        cell::{Cell, Flags},
        Config, Term, TermMode,
    },
    vte::{
        ansi::{CursorShape, CursorStyle as VtCursorStyle, Processor},
        Parser as VteParser, Perform,
    },
};
use chatspeed_gpui::components::dialog::WindowExt;
use chatspeed_gpui::terminal::{self, Input, Output};
use gpui_kit::component::popover::Popover;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{h_flex, v_flex, ActiveTheme},
    *,
};
use rust_i18n::t;

use crate::ui::{icon::spin_phase, trigger, Icon};

// Keep the terminal comfortably readable independently of the surrounding UI.
const FONT_SIZE: f32 = 14.;
const LINE_HEIGHT: f32 = 20.;
const BEAM_WIDTH: f32 = 2.;
const DEFAULT_CURSOR: VtCursorStyle = VtCursorStyle {
    shape: CursorShape::Beam,
    blinking: true,
};

/// One terminal skin, in the shape the Vue terminal themes use.
///
/// The values mirror the `ayu` skin in `src/constants/terminalThemes.js`, so the
/// native dock shows the same colours as the web terminal for the same output.
struct TerminalPalette {
    background: u32,
    foreground: u32,
    /// The caret color. The dock always draws a beam, so the skin's block
    /// accent is unused.
    cursor: u32,
    selection: u32,
    selection_foreground: u32,
    /// Indices 0-7, then the bright 8-15.
    ansi: [u32; 16],
}

/// ayu dark (iTerm2-Color-Schemes `ghostty/Ayu`).
const AYU_DARK: TerminalPalette = TerminalPalette {
    background: 0x0b0e14,
    foreground: 0xbfbdb6,
    cursor: 0xe6b450,
    selection: 0x409fff,
    selection_foreground: 0x0b0e14,
    ansi: [
        0x11151c, 0xea6c73, 0x7fd962, 0xf9af4f, 0x53bdfa, 0xcda1fa, 0x90e1c6, 0xc7c7c7, 0x686868,
        0xf07178, 0xaad94c, 0xffb454, 0x59c2ff, 0xd2a6ff, 0x95e6cb, 0xffffff,
    ],
};

/// ayu light (iTerm2-Color-Schemes `ghostty/Ayu Light`).
const AYU_LIGHT: TerminalPalette = TerminalPalette {
    background: 0xf8f9fa,
    foreground: 0x5c6166,
    cursor: 0xffaa33,
    selection: 0x035bd6,
    selection_foreground: 0xf8f9fa,
    ansi: [
        0x000000, 0xea6c6d, 0x6cbf43, 0xeca944, 0x3199e1, 0x9e75c7, 0x46ba94, 0xbababa, 0x686868,
        0xf07171, 0x86b300, 0xf2ae49, 0x399ee6, 0xa37acc, 0x4cbf99, 0xd1d1d1,
    ],
};

fn terminal_palette(dark: bool) -> &'static TerminalPalette {
    if dark {
        &AYU_DARK
    } else {
        &AYU_LIGHT
    }
}

const HEADER_HEIGHT: f32 = 38.;
const PANEL_HEIGHT: f32 = 260.;
const FALLBACK_CELL_WIDTH: f32 = 8.;
/// The runtime creates an 80x24 PTY, so the local grid matches it.
const DEFAULT_COLUMNS: usize = 80;
const DEFAULT_ROWS: usize = 24;

/// Grid size. The VT wraps at the width it is told, so long lines stay inside
/// the panel instead of running off its right edge.
#[derive(Clone, Copy)]
pub struct TermSize {
    lines: usize,
    columns: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl TermSize {
    fn new(lines: usize, columns: usize) -> Self {
        Self {
            lines: lines.max(4),
            columns: columns.max(20),
        }
    }
}

fn cell_text(cell: &Cell) -> String {
    let mut text = cell.c.to_string();
    if let Some(marks) = cell.zerowidth() {
        text.extend(marks.iter().copied());
    }
    text
}

/// Captures the OSC 7 working-directory report the runtime's prompt hook emits.
///
/// The PTY splits writes at arbitrary byte boundaries, so a sequence can span
/// two chunks; `vte::Parser` carries that state across `advance` calls.
#[derive(Default)]
struct CwdReport {
    path: Option<String>,
}

impl Perform for CwdReport {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.first().copied() != Some(&b"7"[..]) {
            return;
        }
        let Some(uri) = params.get(1).and_then(|uri| std::str::from_utf8(uri).ok()) else {
            return;
        };
        let Some(rest) = uri.strip_prefix("file://") else {
            return;
        };
        // `file://host/path`: the path starts at the first slash after the host.
        let Some(slash) = rest.find('/') else {
            return;
        };
        self.path = Some(rest[slash..].to_string());
    }
}

/// Vue's tab title shows the directory's own name; `/a/b/c` reads `c`.
fn directory_name(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    std::path::Path::new(trimmed)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Config, Processor, Term, TermSize, VoidListener};

    #[test]
    fn rendered_cell_keeps_combining_marks_without_using_another_column() {
        let mut cell = super::Cell {
            c: 'e',
            ..Default::default()
        };
        cell.push_zerowidth('\u{301}');
        assert_eq!(super::cell_text(&cell), "e\u{301}");
    }

    #[test]
    fn tab_title_uses_the_shells_working_directory() {
        use super::{directory_name, CwdReport, VteParser};

        let mut parser = VteParser::new();
        let mut report = CwdReport::default();
        // The PTY can split an OSC sequence anywhere, including mid-prefix.
        parser.advance(&mut report, b"\x1b]7;file://localho");
        assert_eq!(report.path, None);
        parser.advance(&mut report, b"st/a/b/c\x07");
        assert_eq!(report.path.as_deref(), Some("/a/b/c"));

        assert_eq!(directory_name("/a/b/c"), "c");
        assert_eq!(directory_name("/a/b/c/"), "c");
        assert_eq!(directory_name("/"), "/");
    }

    #[test]
    fn terminal_palette_follows_the_ayu_skins() {
        // Values copied from the `ayu` skin in `src/constants/terminalThemes.js`.
        assert_eq!(super::terminal_palette(true).background, 0x0b0e14);
        assert_eq!(super::terminal_palette(true).foreground, 0xbfbdb6);
        assert_eq!(super::terminal_palette(true).ansi[1], 0xea6c73);
        assert_eq!(super::terminal_palette(false).background, 0xf8f9fa);
        assert_eq!(super::terminal_palette(false).foreground, 0x5c6166);
        assert_eq!(super::terminal_palette(false).ansi[1], 0xea6c6d);
    }

    #[test]
    fn terminal_starts_with_a_beam_and_honors_application_cursor_changes() {
        let config = Config {
            default_cursor_style: super::DEFAULT_CURSOR,
            ..Config::default()
        };
        let mut screen = Term::new(config, &TermSize::new(24, 80), VoidListener);
        assert_eq!(screen.renderable_content().cursor.shape, super::CursorShape::Beam);
        assert!(screen.cursor_style().blinking);

        let mut parser: Processor = Processor::new();
        parser.advance(&mut screen, b"\x1b[2 q");
        assert_eq!(screen.renderable_content().cursor.shape, super::CursorShape::Block);
        assert!(!screen.cursor_style().blinking);
        parser.advance(&mut screen, b"\x1b[4 q");
        assert_eq!(screen.renderable_content().cursor.shape, super::CursorShape::Underline);
    }

    #[test]
    fn terminal_parser_preserves_split_utf8_and_cursor_movement() {
        let size = TermSize::new(24, 80);
        let mut screen = Term::new(Config::default(), &size, VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut screen, b"abc\r\x1b[");
        parser.advance(&mut screen, b"2C\xc3");
        parser.advance(&mut screen, b"\xa9");
        let row: String = screen
            .renderable_content()
            .display_iter
            .take(3)
            .map(|cell| cell.c)
            .collect();
        assert_eq!(row, "abé");
    }

    #[test]
    fn reverse_video_keeps_the_last_glyph_visible_with_default_colors() {
        let mut screen = Term::new(Config::default(), &TermSize::new(24, 80), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut screen, b"ab\x1b[7mc\x1b[0m");
        let content = screen.renderable_content();
        let mut cells = content.display_iter;
        let normal = cells.next().expect("first character");
        let reversed = cells.nth(1).expect("last character");
        assert_eq!(reversed.c, 'c');
        assert!(reversed.flags.contains(super::Flags::INVERSE));
        assert_eq!(content.cursor.point.column.0, 3);

        let light = gpui_kit::rgb(0xf5f5f5).into();
        let dark = gpui_kit::rgb(0x202020).into();
        for (foreground, background) in [(light, dark), (dark, light)] {
            assert_eq!(
                super::cell_colors(&normal, content.colors, foreground, background, &super::AYU_DARK.ansi),
                (foreground, gpui_kit::transparent_black().into()),
            );
            assert_eq!(
                super::cell_colors(&reversed, content.colors, foreground, background, &super::AYU_DARK.ansi),
                (background, foreground),
            );
        }
    }

    #[test]
    fn terminal_preserves_suggestion_style_and_cursor() {
        use alacritty_terminal::{
            term::cell::Flags,
            vte::ansi::{Color, NamedColor},
        };
        let size = TermSize::new(24, 80);
        let mut screen = Term::new(Config::default(), &size, VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut screen, b"git\x1b[2;31m status\x1b[0m\x1b[7D");
        let content = screen.renderable_content();
        assert_eq!(content.cursor.point.column.0, 3);
        let suggestion = content.display_iter.skip(3).next().unwrap();
        assert!(suggestion.flags.contains(Flags::DIM));
        assert_eq!(suggestion.fg, Color::Named(NamedColor::Red));
        assert_ne!(
            super::terminal_color(
                suggestion.fg,
                content.colors,
                gpui_kit::transparent_black().into(),
                &super::AYU_DARK.ansi,
            ),
            gpui_kit::transparent_black().into()
        );
    }

    #[test]
    fn terminal_selection_copies_across_rows_in_drag_order() {
        use super::{CellPoint, Column, Line, Selection, SelectionType, Side};

        let mut screen = Term::new(Config::default(), &TermSize::new(24, 80), VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut screen, b"abc\r\ndef");
        let mut selection = Selection::new(
            SelectionType::Simple,
            CellPoint::new(Line(1), Column(0)),
            Side::Right,
        );
        selection.update(CellPoint::new(Line(0), Column(1)), Side::Left);
        screen.selection = Some(selection);
        assert_eq!(screen.selection_to_string().as_deref(), Some("bc\nd"));
    }

    #[test]
    fn resizing_keeps_the_cursor_inside_the_grid() {
        let mut screen = Term::new(Config::default(), &TermSize::new(24, 80), VoidListener);
        screen.resize(TermSize::new(10, 40));
        let content = screen.renderable_content();
        assert!(content.cursor.point.line.0 < 10);
    }
}

pub struct Panel {
    focus: FocusHandle,
    visible: bool,
    tabs: Vec<Entity<Tab>>,
    shells: Vec<(String, String)>,
    active: usize,
    /// Input handler that turns IME commits into PTY input.
    input: Option<Entity<TerminalInput>>,
    /// Bounds of the rows area from the last paint. The grid size, the wheel
    /// history, and the mouse mapping all derive from this one rect.
    scroll: ScrollHandle,
    /// Horizontal offset of the tab strip. Without it a long tab list is
    /// clipped at the header's right edge and the hidden tabs cannot be reached.
    tabs_scroll: ScrollHandle,
    /// Active tab to reveal on the next frame. Event handlers only know the new
    /// index; render applies it once the strip has laid out that width.
    reveal_tab: std::cell::Cell<Option<usize>>,
    /// Grid size (columns, rows) last applied to the tabs and the PTY.
    applied_size: std::cell::Cell<Option<(usize, usize)>>,
    /// Vue's fullscreen toggle; the dock keeps its header but uses most of the
    /// workflow column.
    maximized: bool,
    height: Pixels,
    /// Cell where the current drag began. A plain click only moves the caret,
    /// so it never leaves a one-cell selection block behind.
    selection_start: Option<(CellPoint, Side)>,
    resize_start: Option<(Pixels, Pixels)>,
}

pub struct Tab {
    screen: Term<VoidListener>,
    parser: Processor,
    /// Sniffs OSC 7 out of the same bytes the VT parses, so the tab title can
    /// follow the shell's working directory.
    cwd_parser: VteParser,
    cwd_report: CwdReport,
    /// Working directory last reported by the prompt.
    cwd: Option<String>,
    /// Shell name; stands in for the title until the prompt reports a directory.
    shell: Option<String>,
    status: &'static str,
    input: Option<tokio::sync::mpsc::Sender<Input>>,
    size: TermSize,
}

impl Tab {
    /// The directory's own name, so a tab reads as the folder it is working in;
    /// the shell name only appears until the prompt reports a directory.
    fn title(&self) -> String {
        self.cwd
            .as_deref()
            .map(directory_name)
            .or_else(|| self.shell.clone())
            .unwrap_or_else(|| t!("terminal").to_string())
    }

    /// Feeds one PTY chunk to the VT and to the directory sniffer. Both parsers
    /// keep their own split-sequence state.
    fn write_output(&mut self, bytes: &[u8]) {
        self.cwd_parser.advance(&mut self.cwd_report, bytes);
        if let Some(path) = self.cwd_report.path.take() {
            self.cwd = Some(path);
        }
        self.parser.advance(&mut self.screen, bytes);
    }
}

impl Panel {
    pub fn new(cx: &App) -> Self {
        Self {
            focus: cx.focus_handle(),
            visible: false,
            tabs: Vec::new(),
            shells: Vec::new(),
            active: 0,
            input: None,
            scroll: ScrollHandle::new(),
            tabs_scroll: ScrollHandle::new(),
            reveal_tab: std::cell::Cell::new(None),
            applied_size: std::cell::Cell::new(None),
            maximized: false,
            height: px(PANEL_HEIGHT),
            selection_start: None,
            resize_start: None,
        }
    }

    pub(crate) fn set_input(&mut self, input: Entity<TerminalInput>) {
        self.input = Some(input);
    }

    /// Removes a tab and keeps the active index inside the remaining list.
    /// Closing is confirmed by the caller so a stray click cannot kill a
    /// running command.
    pub(crate) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut App) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.visible = false;
        }
        self.active = (self.active - usize::from(index < self.active))
            .min(self.tabs.len().saturating_sub(1));
        if self.visible {
            if !self.tabs.is_empty() {
                self.reveal_tab.set(Some(self.active));
            }
            self.focus.focus(window, cx);
        }
    }

    /// Sends composed text (an IME commit) to the active shell.
    pub(crate) fn send_text(&mut self, text: &str, cx: &mut App) {
        let text = text.replace('\x1b', "");
        if text.is_empty() {
            return;
        }
        if let Some(tab) = self.tabs.get(self.active) {
            let bracketed = tab
                .read(cx)
                .screen
                .mode()
                .contains(TermMode::BRACKETED_PASTE);
            let payload = if bracketed {
                format!("\x1b[200~{text}\x1b[201~")
            } else {
                text
            };
            tab.update(cx, |tab, _| {
                // Typed text replaces any selection, as in a text field, and
                // returns the view from history to the live prompt.
                tab.screen.selection = None;
                tab.screen.scroll_display(Scroll::Bottom);
                send(tab, payload);
            });
        }
    }
}

/// Committed text from the platform input method reaches the shell here.
///
/// The terminal owns no editable text, but it still has to report the IME's
/// uncommitted preedit as marked text: the platform only routes Enter to the
/// composition commit while it believes text is marked.
pub struct TerminalInput {
    owner: WeakEntity<Observer>,
    /// The IME preedit, kept only while the composition is uncommitted.
    preedit: Option<String>,
}

impl TerminalInput {
    pub(crate) fn new(owner: WeakEntity<Observer>) -> Self {
        Self {
            owner,
            preedit: None,
        }
    }

    /// Whether the IME is composing; typed keys belong to it until it commits.
    pub(crate) fn composing(&self) -> bool {
        self.preedit.is_some()
    }

    fn commit(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        let _ = self.owner.update(cx, |this, cx| {
            this.terminal.send_text(text, cx);
            cx.notify();
        });
    }
}

impl EntityInputHandler for TerminalInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        // The preedit is the only text this handler owns; the platform reads it
        // back to place the candidate window.
        let utf16: Vec<u16> = self.preedit.as_ref()?.encode_utf16().collect();
        let start = range.start.min(utf16.len());
        let end = range.end.clamp(start, utf16.len());
        *adjusted_range = Some(start..end);
        Some(String::from_utf16_lossy(&utf16[start..end]))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.preedit
            .as_ref()
            .map(|preedit| 0..preedit.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        // A cancelled composition never becomes shell input.
        self.preedit = None;
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = None;
        self.commit(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A marked selection means the text is still a composition preview.
        // Without one, this call replaced the preedit with committed text.
        match new_selected_range {
            Some(_) => self.preedit = Some(new_text.to_string()),
            None => {
                self.preedit = None;
                self.commit(new_text, cx);
            }
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The IME anchors its candidate window to this rect. The terminal has no
        // text runs, so the caret cell is the only useful answer; without one
        // the candidate bar can only fall back to a window corner.
        let owner = self.owner.upgrade()?;
        let observer = owner.read(cx);
        let panel = &observer.terminal;
        let tab = panel.tabs.get(panel.active)?.read(cx);
        let content = tab.screen.renderable_content();
        let display_offset = i32::try_from(content.display_offset).ok()?;
        let row = usize::try_from(content.cursor.point.line.0 + display_offset).ok()?;
        if row >= tab.size.screen_lines() {
            return None;
        }
        let column = usize::from(content.cursor.point.column.0);
        let viewport = panel.scroll.bounds();
        if viewport.size.width <= px(0.) || viewport.size.height <= px(0.) {
            return None;
        }
        let width = cell_advance(window, cx);
        let origin = point(
            viewport.origin.x + px(column as f32 * f32::from(width)),
            viewport.origin.y + px(row as f32 * LINE_HEIGHT),
        );
        Some(Bounds::new(origin, Size::new(width, px(LINE_HEIGHT))))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

pub fn toggle(this: &mut Observer, cx: &mut Context<Observer>) {
    this.terminal.visible = !this.terminal.visible;
    if this.terminal.visible {
        // Opening the dock returns to the live prompt even if it was scrolled
        // back into history.
        if let Some(tab) = this.terminal.tabs.get(this.terminal.active) {
            tab.update(cx, |tab, _| tab.screen.scroll_display(Scroll::Bottom));
        }
    }
    if this.terminal.visible && this.terminal.tabs.is_empty() {
        add(this, None, cx);
    }
    cx.notify();
}

/// Cmd/Ctrl+J toggles the dock and Cmd/Ctrl+K clears the active tab.
///
/// Callers pass the raw keystroke so the same rules work from a window-level
/// interceptor, which sees keys even when no element holds focus.
pub fn shortcut(
    this: &mut Observer,
    key: &Keystroke,
    window: &mut Window,
    cx: &mut Context<Observer>,
) {
    let mods = key.modifiers;
    let command = if cfg!(target_os = "macos") {
        mods.platform && !mods.control
    } else {
        mods.control && !mods.platform
    };
    if !command || mods.alt || mods.shift {
        return;
    }
    match key.key.as_str() {
        "j" => {
            // A modal owns the keyboard; moving focus into the dock would take
            // its Escape/Enter handling away mid-edit. The shortcut is still
            // consumed so the platform does not beep.
            if !window.has_active_dialog(cx) {
                toggle(this, cx);
                if this.terminal.visible {
                    this.terminal.focus.focus(window, cx);
                }
            }
        }
        "k" if this.terminal.visible && this.terminal.focus.is_focused(window) => {
            if let Some(tab) = this.terminal.tabs.get(this.terminal.active) {
                tab.update(cx, |tab, _| clear_keep_input(tab));
            }
            cx.notify();
        }
        _ => return,
    }
    window.prevent_default();
    cx.stop_propagation();
}

fn add(this: &mut Observer, shell: Option<String>, cx: &mut Context<Observer>) {
    add_tab(this, shell, None, cx);
}

pub(crate) fn switch_shell(this: &mut Observer, path: String, cx: &mut Context<Observer>) {
    let replace = this.terminal.tabs.get(this.terminal.active).cloned();
    add_tab(this, Some(path), replace, cx);
}

fn add_tab(
    this: &mut Observer,
    shell: Option<String>,
    replace: Option<Entity<Tab>>,
    cx: &mut Context<Observer>,
) {
    let options = this.options.clone();
    let cwd = this.roots().first().cloned();
    // A configured default shell only applies while the runtime still offers it.
    let shell = shell.or_else(|| {
        let configured = this.terminal_default_shell.clone()?;
        this.terminal
            .shells
            .iter()
            .find(|(name, path)| path == &configured || name.eq_ignore_ascii_case(&configured))
            .map(|(_, path)| path.clone())
    });
    let scrolling_history = this.terminal_output_limit;
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    let (out, mut updates) = tokio::sync::mpsc::channel(64);
    // A new tab starts at the size the dock last rendered; that size drives the
    // PTY too, so the shell wraps to the visible rows instead of a hidden grid.
    let (columns, rows) = this
        .terminal
        .applied_size
        .get()
        .unwrap_or((DEFAULT_COLUMNS, DEFAULT_ROWS));
    let size = TermSize::new(rows, columns);
    let initial_cwd = cwd.clone();
    let tab = cx.new(move |_| Tab {
        screen: Term::new(
            Config {
                default_cursor_style: DEFAULT_CURSOR,
                scrolling_history,
                ..Config::default()
            },
            &size,
            VoidListener,
        ),
        parser: Processor::new(),
        cwd_parser: VteParser::new(),
        cwd_report: CwdReport::default(),
        cwd: initial_cwd,
        shell: None,
        status: "terminal_connecting",
        input: Some(tx),
        size,
    });
    let weak = tab.downgrade();
    this.terminal.active = this.terminal.tabs.len();
    this.terminal.tabs.push(tab);
    this.terminal.reveal_tab.set(Some(this.terminal.active));
    let launch = std::thread::Builder::new()
        .name("gpui-terminal".into())
        .spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let size = (columns as u16, rows as u16);
                runtime.block_on(terminal::run(options, cwd, shell, size, rx, out));
            }
        });
    if launch.is_err() {
        if let Some(tab) = this.terminal.tabs.last() {
            tab.update(cx, |tab, _| {
                tab.input = None;
                tab.status = "terminal_lost";
            });
        }
    }
    let replace = replace.map(|tab| tab.downgrade());
    cx.spawn(async move |owner, cx| {
        let mut replace = replace;
        while let Some(update) = updates.recv().await {
            if matches!(&update, Output::Ready(_)) && weak.upgrade().is_some() {
                if let Some(old) = replace.take().and_then(|tab| tab.upgrade()) {
                    let _ = owner.update(cx, |this, cx| {
                        let panel = &mut this.terminal;
                        if let Some(index) = panel.tabs.iter().position(|tab| tab == &old) {
                            panel.tabs.remove(index);
                            panel.active = (panel.active - usize::from(index < panel.active))
                                .min(panel.tabs.len().saturating_sub(1));
                            cx.notify();
                        }
                    });
                }
            }
            let update = match update {
                Output::Shells(shells) => {
                    if owner
                        .update(cx, |this, cx| {
                            this.terminal.shells = shells;
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }
                update => update,
            };
            if weak
                .update(cx, |tab, _| match update {
                    Output::Shells(_) => {}
                    Output::Ready(label) => {
                        tab.shell = Some(label);
                        tab.status = "";
                    }
                    Output::Bytes(bytes) => tab.write_output(&bytes),
                    Output::Exit => {
                        tab.input = None;
                        tab.status = "terminal_exited";
                    }
                    Output::Lost => {
                        tab.input = None;
                        tab.status = "terminal_lost";
                    }
                })
                .is_err()
                || owner.update(cx, |_, cx| cx.notify()).is_err()
            {
                return;
            }
        }
        let _ = weak.update(cx, |tab, _| {
            if tab.input.take().is_some() {
                tab.status = "terminal_lost";
            }
        });
        let _ = owner.update(cx, |_, cx| cx.notify());
    })
    .detach();
    cx.notify();
}

fn send(tab: &mut Tab, text: String) {
    if let Some(input) = &tab.input {
        if input.try_send(Input::Text(text)).is_err() {
            tab.status = "terminal_input_failed";
        }
    }
}

/// Vue's clear keeps the input block on screen and moves it to the first row,
/// instead of leaving an empty screen with a stale prompt.
fn clear_keep_input(tab: &mut Tab) {
    let content = tab.screen.renderable_content();
    let rows = tab.size.screen_lines();
    let cursor = content.cursor.point;
    let cursor_y = usize::try_from(cursor.line.0)
        .unwrap_or(0)
        .min(rows.saturating_sub(1));
    let cursor_x = usize::from(cursor.column.0);
    // A soft-wrapped row is flagged on the last cell of the row above it.
    // Displayed rows carry the display offset, so map back to the screen row.
    let display_offset = i32::try_from(content.display_offset).unwrap_or(0);
    let mut continues = vec![false; rows + 1];
    for cell in content.display_iter {
        if cell.flags.contains(Flags::WRAPLINE) {
            if let Ok(line) = usize::try_from(cell.point.line.0 + display_offset) {
                if let Some(next) = continues.get_mut(line + 1) {
                    *next = true;
                }
            }
        }
    }
    let mut block_top = cursor_y;
    while block_top > 0 && continues[block_top] {
        block_top -= 1;
    }
    let block_rows = cursor_y - block_top + 1;
    let mut sequence = String::new();
    if block_top > 0 {
        sequence.push_str(&format!(
            "\x1b[1;{}r\x1b[H\x1b[{}M\x1b[r",
            cursor_y + 1,
            block_top
        ));
    }
    if block_rows < rows {
        sequence.push_str(&format!("\x1b[{};1H\x1b[0J", block_rows + 1));
    }
    sequence.push_str("\x1b[3J");
    sequence.push_str(&format!("\x1b[{};{}H", block_rows, cursor_x + 1));
    tab.parser.advance(&mut tab.screen, sequence.as_bytes());
    tab.screen.scroll_display(Scroll::Bottom);
}

/// Shaped advance of one terminal cell, shared by painting and hit-testing.
///
/// The shaped advance is fractional, but the cell boxes are laid out on gpui's
/// rounded grid. Hit-testing has to use the same whole-pixel advance as the
/// boxes: a fractional advance drifts one character every few dozen columns,
/// which reads as the selection starting left of the pointer.
fn cell_advance(window: &Window, cx: &App) -> Pixels {
    let font = gpui_kit::font(cx.theme().mono_font_family.clone());
    let font_id = cx.text_system().resolve_font(&font);
    let measured = window.text_system().layout_width(font_id, px(FONT_SIZE), '0');
    if measured > px(0.) && f32::from(measured).is_finite() {
        px(f32::from(measured).round().max(1.))
    } else {
        px(FALLBACK_CELL_WIDTH)
    }
}

/// Fits the VT and the PTY to the rows area.
///
/// The grid must match the visible cells: otherwise the shell wraps to a size
/// the dock never shows, the extra rows are unreachable, and the VT scrollback
/// has nowhere to appear when the user scrolls back.
fn apply_grid_size(panel: &Panel, cx: &mut Context<Observer>, cell_width: Pixels) {
    let viewport = panel.scroll.bounds();
    if viewport.size.width <= px(0.) || viewport.size.height <= px(0.) {
        return;
    }
    let columns =
        (f32::from(viewport.size.width) / f32::from(cell_width)).floor() as usize;
    let rows = (f32::from(viewport.size.height) / LINE_HEIGHT).floor() as usize;
    let size = (columns.max(20), rows.max(4));
    if panel.applied_size.get() == Some(size) {
        return;
    }
    panel.applied_size.set(Some(size));
    for tab in &panel.tabs {
        tab.update(cx, |tab, _| {
            if (tab.size.columns, tab.size.lines) == size {
                return;
            }
            let grid = TermSize::new(size.1, size.0);
            tab.size = grid;
            tab.screen.resize(grid);
            if let Some(input) = &tab.input {
                let _ = input.try_send(Input::Resize(size.0 as u16, size.1 as u16));
            }
        });
    }
}

fn selection_position(
    viewport: Bounds<Pixels>,
    position: Point<Pixels>,
    cell_width: Pixels,
    tab: &Tab,
) -> (CellPoint, Side) {
    // The rows area shows the VT's current display lines: the top visible row
    // is content line `-display_offset`, so a click row maps back through it.
    let local = position - viewport.origin;
    let x = local.x / cell_width;
    let column = (x as usize).min(tab.size.columns - 1);
    let row = ((local.y / px(LINE_HEIGHT)) as usize).min(tab.size.lines - 1);
    let side = if x - (column as f32) < 0.5 { Side::Left } else { Side::Right };
    let line = row as i32 - tab.screen.renderable_content().display_offset as i32;
    (CellPoint::new(Line(line), Column(column)), side)
}

fn key(tab: &mut Tab, event: &KeyDownEvent, cx: &mut App) -> bool {
    let key = &event.keystroke;
    let mods = key.modifiers;
    if key.key == "c" && (mods.platform || mods.control) {
        if let Some(text) = tab.screen.selection_to_string() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            return false;
        }
        if mods.platform || mods.shift {
            return false;
        }
    }
    tab.screen.selection = None;
    if (mods.platform || (mods.control && mods.shift)) && key.key == "v" {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text.replace('\x1b', "");
            let text = if tab.screen.mode().contains(TermMode::BRACKETED_PASTE) {
                format!("\x1b[200~{text}\x1b[201~")
            } else {
                text
            };
            send(tab, text);
        }
        return true;
    }
    let cursor = if tab.screen.mode().contains(TermMode::APP_CURSOR) {
        "\x1bO"
    } else {
        "\x1b["
    };
    let value = match key.key.as_str() {
        "enter" => "\r".into(),
        "backspace" => "\x7f".into(),
        "tab" => "\t".into(),
        "escape" => "\x1b".into(),
        "up" => format!("{cursor}A"),
        "down" => format!("{cursor}B"),
        "right" => format!("{cursor}C"),
        "left" => format!("{cursor}D"),
        "home" => format!("{cursor}H"),
        "end" => format!("{cursor}F"),
        "delete" => "\x1b[3~".into(),
        k if mods.control
            && !mods.platform
            && k.len() == 1
            && k.as_bytes()[0].is_ascii_lowercase() =>
        {
            ((k.as_bytes()[0] - b'a' + 1) as char).to_string()
        }
        // IME commits arrive through `TerminalInput`, so only plain key presses
        // that already carry a character reach this branch.
        _ if !mods.control && !mods.platform => key.key_char.clone().unwrap_or_default(),
        _ => String::new(),
    };
    if !value.is_empty() {
        // Typing leaves any history view and returns to the live prompt.
        tab.screen.scroll_display(Scroll::Bottom);
        send(tab, value);
    }
    true
}

fn terminal_color(
    color: alacritty_terminal::vte::ansi::Color,
    colors: &alacritty_terminal::term::color::Colors,
    fallback: Hsla,
    ansi: &[u32; 16],
) -> Hsla {
    use alacritty_terminal::vte::ansi::Color;
    let index = match color {
        Color::Spec(rgb) => {
            return gpui_kit::rgb(
                (u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b),
            )
            .into()
        }
        Color::Named(name) => name as usize,
        Color::Indexed(index) => usize::from(index),
    };
    if let Some(rgb) = colors[index] {
        return gpui_kit::rgb(
            (u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b),
        )
        .into();
    }
    let rgb = match index {
        0..=15 => ansi[index],
        16..=231 => {
            let n = index - 16;
            let level = |v: usize| if v == 0 { 0 } else { 55 + 40 * v as u32 };
            (level(n / 36) << 16) | (level(n / 6 % 6) << 8) | level(n % 6)
        }
        232..=255 => (8 + 10 * (index as u32 - 232)) * 0x010101,
        _ => return fallback,
    };
    gpui_kit::rgb(rgb).into()
}

/// Default backgrounds are transparent only until they become ink for reverse video.
fn cell_colors(
    cell: &Cell,
    colors: &alacritty_terminal::term::color::Colors,
    foreground: Hsla,
    background: Hsla,
    ansi: &[u32; 16],
) -> (Hsla, Hsla) {
    let ink = terminal_color(cell.fg, colors, foreground, ansi);
    if cell.flags.contains(Flags::INVERSE) {
        (terminal_color(cell.bg, colors, background, ansi), ink)
    } else {
        (ink, terminal_color(cell.bg, colors, transparent_black().into(), ansi))
    }
}

pub(crate) fn expanded(this: &Observer) -> bool {
    this.terminal.visible && this.terminal.maximized
}

pub fn render(this: &Observer, window: &mut Window, cx: &mut Context<Observer>) -> Option<AnyElement> {
    let panel = &this.terminal;
    if !panel.visible {
        return None;
    }
    let max_height = (window.viewport_size().height - px(160.)).max(px(180.));
    if let Some((start_y, height)) = panel.resize_start {
        let owner = cx.entity().downgrade();
        window.on_mouse_event(move |event: &MouseMoveEvent, _, _, cx: &mut App| {
            if event.pressed_button == Some(MouseButton::Left) {
                let _ = owner.update(cx, |this, cx| {
                    this.terminal.height = (height + start_y - event.position.y).clamp(px(180.), max_height);
                    cx.notify();
                });
            }
        });
        let owner = cx.entity().downgrade();
        window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx: &mut App| {
            let _ = owner.update(cx, |this, cx| {
                if this.terminal.resize_start.take().is_some() { cx.notify(); }
            });
        });
    }
    let border = cx.theme().border;
    let background = cx.theme().background;
    let surface = cx.theme().muted;
    let secondary = cx.theme().muted_foreground;
    let primary = cx.theme().primary;
    // The canvas follows the ayu skin instead of the app surface so the ANSI
    // palette reads the way its author intended; the header keeps the chrome.
    // `terminal_color_scheme` can pin the skin's light or dark palette.
    let palette = terminal_palette(this.terminal_color_dark.unwrap_or(this.appearance_dark));
    let terminal_background: Hsla = rgb(palette.background).into();
    let terminal_foreground: Hsla = rgb(palette.foreground).into();
    let pressed = crate::ui::surfaces(this.appearance_dark).active;
    let active = panel.tabs.get(panel.active).cloned();

    // Match the font and shaped width used to paint the cells. The raw glyph
    // advance can differ from the platform's text layout metrics.
    let font = gpui_kit::font(cx.theme().mono_font_family.clone());
    let cell_width = cell_advance(window, cx);
    // Fit the grid to the rows area before drawing it. The PTY, the VT and the
    // drawn rows share one size, so the shell wraps at the panel edge and the
    // retained output lives in the VT scrollback rather than a hidden grid.
    apply_grid_size(panel, cx, cell_width);

    // Tabs reuse the chat-hub strip: rounded chips on the panel surface, the
    // active one filled with the muted chrome surface and its label in the
    // primary color. Sharing that component is what keeps the terminal from
    // looking like a second application docked inside the window.
    let mut tabs = h_flex()
        .id("terminal-tabs")
        .flex_1()
        .min_w_0()
        .h_full()
        .items_center()
        .gap(px(4.))
        .px(px(4.))
        .overflow_x_scroll()
        .track_scroll(&panel.tabs_scroll);
    for (index, tab) in panel.tabs.iter().enumerate() {
        let label = tab.read(cx).title();
        let selected = index == panel.active;
        tabs = tabs.child(
            h_flex()
                .id(("terminal-tab", index))
                .flex_none()
                .h(px(28.))
                .max_w(px(220.))
                .min_w_0()
                .items_center()
                .gap(px(4.))
                .rounded(px(8.))
                .when(selected, |row| row.bg(surface))
                .when(!selected, |row| row.hover(move |style| style.bg(surface)))
                .cursor_pointer()
                .child(
                    div()
                        .min_w_0()
                        .pl(px(8.))
                        .pr(px(4.))
                        .text_ellipsis()
                        .text_size(px(12.))
                        .when(selected, |text| {
                            text.text_color(primary).font_weight(FontWeight::MEDIUM)
                        })
                        .when(!selected, |text| text.text_color(secondary))
                        .child(label),
                )
                .child(
                    div()
                        .id(("terminal-close", index))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .mr(px(4.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(pressed))
                        .child(Icon::new("close").size(12.).color(secondary))
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            cx.stop_propagation();
                            if index < this.terminal.tabs.len() {
                                this.confirm = Some(crate::ui::confirm::ConfirmRequest {
                                    kind: crate::ui::confirm::ConfirmKind::CloseTerminal(index),
                                    title: t!("terminal_close").to_string(),
                                    message: t!("terminal_close_confirm").to_string(),
                                    confirm_label: t!("terminal_close").to_string(),
                                    danger: true,
                                });
                            }
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.terminal.active = index;
                    this.terminal.reveal_tab.set(Some(index));
                    this.terminal.focus.focus(window, cx);
                    cx.notify();
                })),
        );
    }
    if let Some(index) = panel.reveal_tab.take() {
        panel.tabs_scroll.scroll_to_item(index);
    }

    let mut controls = h_flex().flex_none().items_center().gap(px(2.)).px(px(8.));
    controls = controls.child(
        div()
            .id("terminal-add")
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(move |style| style.bg(surface))
            .child(Icon::new("add").size(14.).color(secondary))
            .on_click(cx.listener(|this, _, window, cx| {
                add(this, None, cx);
                this.terminal.focus.focus(window, cx);
            })),
    );

    // The shell selector mirrors Vue's dropdown: bash glyph, current shell, caret.
    let shells = panel.shells.clone();
    let current_shell = active
        .as_ref()
        .and_then(|tab| tab.read(cx).shell.clone())
        .or_else(|| shells.first().map(|(name, _)| name.clone()))
        .unwrap_or_else(|| t!("terminal").to_string());
    let observer_entity = cx.entity();
    if !shells.is_empty() {
        controls = controls.child(
            Popover::new("terminal-shell-menu")
                .trigger(trigger(
                    h_flex()
                        .id("terminal-shell")
                        .flex_none()
                        .max_w(px(150.))
                        .items_center()
                        .gap(px(4.))
                        .h(px(24.))
                        .px(px(6.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(surface))
                        .child(Icon::new("bash").size(14.).color(secondary))
                        .child(
                            div()
                                .min_w_0()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .child(current_shell.clone()),
                        )
                        .child(Icon::new("caret-down").size(12.).color(secondary)),
                ))
                .content(move |_, window, cx| {
                    let popover = cx.entity();
                    let mut list = v_flex().id("terminal-shell-list").w(px(180.)).gap(px(2.));
                    for (name, path) in shells.clone() {
                        let popover = popover.clone();
                        let observer_entity = observer_entity.clone();
                        let path = path.clone();
                        list = list.child(
                            h_flex()
                                .id(SharedString::from(format!("terminal-shell-{name}")))
                                .w_full()
                                .min_w_0()
                                .items_center()
                                .gap(px(8.))
                                .px(px(8.))
                                .py(px(4.))
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(|style| style.bg(cx.theme().muted))
                                .text_size(px(12.))
                                .child(
                                    Icon::new("bash")
                                        .size(12.)
                                        .color(cx.theme().muted_foreground),
                                )
                                .child(div().min_w_0().text_ellipsis().child(name))
                                .on_click(move |_, window, cx: &mut App| {
                                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                                    let path = path.clone();
                                    observer_entity.update(cx, |this, cx| {
                                        switch_shell(this, path, cx)
                                    });
                                }),
                        );
                    }
                    let _ = window;
                    list.into_any_element()
                })
                .into_any_element(),
        );
    }

    controls = controls
        .child(div().flex_none().w(px(1.)).h(px(16.)).bg(border))
        .child(
            div()
                .id("terminal-hide")
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(px(24.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(move |style| style.bg(surface))
                .child(Icon::new("minimize").size(14.).color(secondary))
                .on_click(cx.listener(|this, _, _, cx| toggle(this, cx))),
        )
        .child(
            div()
                .id("terminal-fullscreen")
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(px(24.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(move |style| style.bg(surface))
                .child(
                    Icon::new(if panel.maximized {
                        "fullscreen"
                    } else {
                        "fullscreen-off"
                    })
                    .size(14.)
                    .color(secondary),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.terminal.maximized = !this.terminal.maximized;
                    cx.notify();
                })),
        );

    let header = h_flex()
        .id("terminal-header")
        .w_full()
        .h(px(HEADER_HEIGHT))
        .flex_none()
        .items_center()
        .border_b_1()
        .border_color(border)
        .bg(background)
        .child(tabs)
        .child(controls);

    let focus = panel.focus.clone();
    let input_entity = panel.input.clone();
    let mut body = v_flex()
        .id("terminal-content")
        .relative()
        .track_focus(&focus)
        .flex_1()
        .min_h_0()
        .w_full()
        .px(px(8.))
        .py(px(8.))
        .bg(terminal_background)
        .font(font)
        .text_size(px(FONT_SIZE))
        .line_height(px(LINE_HEIGHT));

    if let Some(tab) = active {
        let status = tab.read(cx).status;
        if !status.is_empty() {
            body = body.child(
                div()
                    .text_size(px(12.))
                    .text_color(secondary)
                    .child(t!(status).to_string()),
            );
        }
        let state = tab.read(cx);
        let content = state.screen.renderable_content();
        let rows = state.size.screen_lines();
        let has_input = state.input.is_some();
        let cursor = content.cursor;
        // Displayed rows start at content line `-display_offset`; the cursor is
        // in content coordinates, so it only shows while its row is in view.
        let display_offset = i32::try_from(content.display_offset).unwrap_or(0);
        let cursor_row = usize::try_from(cursor.point.line.0 + display_offset)
            .ok()
            .filter(|row| *row < rows);
        let cursor_column = usize::from(cursor.point.column.0);
        let show_cursor =
            has_input && cursor.shape != CursorShape::Hidden && cursor_row.is_some();
        // Sample the 1s blink from the shared clock instead of using
        // `with_animation`: a repeating animation is pinned to phase zero under
        // the desktop's reduced-motion preference, which left the caret on
        // forever. Requesting the frame also keeps gpui-fast's cached scroll
        // layers repainting.
        let blinking = state.screen.cursor_style().blinking;
        let cursor_on = show_cursor && (!blinking || spin_phase(Duration::from_secs(1)) < 0.5);
        if show_cursor && blinking {
            window.request_animation_frame();
        }
        let cursor_color: Hsla = rgb(palette.cursor).into();
        let mut lines: Vec<Vec<AnyElement>> = (0..rows).map(|_| Vec::new()).collect();
        for cell in content.display_iter {
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let Ok(cell_row) = usize::try_from(cell.point.line.0 + display_offset) else {
                continue;
            };
            let Some(row) = lines.get_mut(cell_row) else {
                continue;
            };
            let (mut fg, mut bg) =
                cell_colors(&cell, content.colors, terminal_foreground, terminal_background, &palette.ansi);
            if cell.flags.contains(Flags::DIM) {
                fg = fg.opacity(0.55);
            }
            if content.selection.is_some_and(|range| range.contains(cell.point)) {
                bg = rgb(palette.selection).into();
                fg = rgb(palette.selection_foreground).into();
            }
            let cursor_here = cursor_on
                && Some(cell_row) == cursor_row
                && usize::from(cell.point.column.0) == cursor_column;
            // The caret is always a thin beam. Shells such as fish request a
            // block cursor, and a filled cell over the last typed character
            // reads like an autocomplete highlight instead of a caret.
            let width = if cell.flags.contains(Flags::WIDE_CHAR) {
                cell_width * 2.
            } else {
                cell_width
            };
            row.push(
                div()
                    .flex_none()
                    .w(width)
                    .h(px(LINE_HEIGHT))
                    .relative()
                    .text_color(fg)
                    .bg(bg)
                    .child(cell_text(&cell))
                    .when(cursor_here, |view| {
                        view.child(
                            div().absolute().top_0().bottom_0().left_0()
                                .w(px(BEAM_WIDTH)).bg(cursor_color),
                        )
                    })
                    .into_any_element(),
            );
        }
        let mut rows_view = v_flex()
            .id("terminal-rows")
            .w_full()
            .min_w_0()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&panel.scroll)
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                if let Some(tab) = this.terminal.tabs.get(this.terminal.active).cloned() {
                    let viewport = this.terminal.scroll.bounds();
                    // A new click drops the old selection; a drag is what creates
                    // the next one, so a click never highlights a single cell.
                    let start = tab.update(cx, |tab, _| {
                        tab.screen.selection = None;
                        selection_position(viewport, event.position, cell_width, tab)
                    });
                    this.terminal.selection_start = Some(start);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| {
                this.terminal.selection_start = None;
            }))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| {
                this.terminal.selection_start = None;
            }))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let Some((start, start_side)) = this.terminal.selection_start else {
                    return;
                };
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                if let Some(tab) = this.terminal.tabs.get(this.terminal.active).cloned() {
                    let viewport = this.terminal.scroll.bounds();
                    let dragging = tab.update(cx, |tab, _| {
                        let (point, side) =
                            selection_position(viewport, event.position, cell_width, tab);
                        // A press that never leaves its cell is a click, not a
                        // drag; it must not leave a one-cell block behind.
                        if tab.screen.selection.is_none() && point == start && side == start_side {
                            return false;
                        }
                        let selection = tab.screen.selection.get_or_insert_with(|| {
                            Selection::new(SelectionType::Simple, start, start_side)
                        });
                        selection.update(point, side);
                        true
                    });
                    if dragging {
                        cx.notify();
                    }
                }
            }))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                // The wheel walks the VT's retained output. The rows themselves
                // never scroll: their content is exactly the visible grid, and
                // history is addressed by the VT display offset. gpui reports an
                // upward wheel as a positive delta, and `Scroll::Delta(+)` moves
                // the display further into history.
                let delta = event.delta.pixel_delta(px(LINE_HEIGHT)).y;
                if delta == px(0.) {
                    return;
                }
                let lines = ((f32::from(delta) / LINE_HEIGHT).round() as i32).max(1);
                let lines = if delta > px(0.) { lines } else { -lines };
                if let Some(tab) = this.terminal.tabs.get(this.terminal.active) {
                    tab.update(cx, |tab, _| tab.screen.scroll_display(Scroll::Delta(lines)));
                    cx.notify();
                }
            }));
        for cells in lines {
            let row = h_flex()
                .flex_none()
                .w_full()
                .min_w_0()
                .h(px(LINE_HEIGHT))
                .whitespace_nowrap()
                .children(cells);
            rows_view = rows_view.child(row);
        }
        body = body
            .child(rows_view)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                // The IME owns typed keys until it commits. Its Enter commits
                // the preedit instead of running the command, so forwarding the
                // key here would send a bare CR to the shell.
                let composing = this
                    .terminal
                    .input
                    .as_ref()
                    .is_some_and(|input| input.read(cx).composing());
                if composing {
                    cx.stop_propagation();
                    return;
                }
                if let Some(tab) = this.terminal.tabs.get(this.terminal.active) {
                    tab.update(cx, |tab, cx| key(tab, event, cx));
                    // Tab doubles as the window's focus traversal key, so the
                    // shell would receive it and the caret would then jump to
                    // the composer. Consuming the default keeps it in the PTY.
                    if event.keystroke.key == "tab" {
                        window.prevent_default();
                    }
                }
                // Keys go to the shell, not to a selection.
                cx.stop_propagation();
                cx.notify();
            }));
    }

    // Registering here is what lets the platform IME deliver composed text; the
    // terminal keeps no editable text, so the handler only forwards commits.
    let paint_focus = focus.clone();
    let canvas_layer = canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            if let Some(input) = &input_entity {
                window.handle_input(
                    &paint_focus,
                    ElementInputHandler::new(bounds, input.clone()),
                    cx,
                );
            }
        },
    )
    .absolute()
    .left_0()
    .top_0()
    .size_full();

    let maximized = panel.maximized;
    let panel = v_flex()
        .relative()
        .min_h_0()
        .w_full()
        .border_t_1()
        .border_color(border)
        .bg(background)
        .child(header)
        .child(
            body.child(canvas_layer)
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    focus.focus(window, cx)
                }),
        );
    let panel = if maximized {
        panel.flex_1()
    } else {
        let height = this.terminal.height.min(max_height);
        panel.h(height).flex_none().child(div().absolute().top_0().left_0().right_0().h(px(5.))
            .id("terminal-resize").cursor(CursorStyle::ResizeUpDown).occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.terminal.resize_start = Some((event.position.y, height));
                cx.stop_propagation();
                cx.notify();
            })))
    };
    Some(panel.into_any_element())
}
