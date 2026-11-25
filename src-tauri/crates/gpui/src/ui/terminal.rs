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
    vte::ansi::{CursorShape, CursorStyle as VtCursorStyle, Processor},
};
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
/// The caret's amber and the ink that stays readable on it: the ayu skin's
/// cursor pair in `src/constants/terminalThemes.js`. The amber reads on both the
/// light and dark terminal surfaces, and the dark accent keeps a glyph visible
/// when the block sits on a character.
const CURSOR_COLOR: u32 = 0xe6b450;
const CURSOR_ACCENT: u32 = 0x0b0e14;
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
                super::cell_colors(&normal, content.colors, foreground, background),
                (foreground, gpui_kit::transparent_black().into()),
            );
            assert_eq!(
                super::cell_colors(&reversed, content.colors, foreground, background),
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
                gpui_kit::transparent_black().into()
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
    scroll: ScrollHandle,
    /// Whether the view keeps the cursor line visible as output arrives.
    follow: bool,
    /// Vue's fullscreen toggle; the dock keeps its header but uses most of the
    /// workflow column.
    maximized: bool,
    height: Pixels,
    selection_drag: bool,
    resize_start: Option<(Pixels, Pixels)>,
}

pub struct Tab {
    screen: Term<VoidListener>,
    parser: Processor,
    label: Option<String>,
    status: &'static str,
    input: Option<tokio::sync::mpsc::Sender<Input>>,
    size: TermSize,
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
            follow: true,
            maximized: false,
            height: px(PANEL_HEIGHT),
            selection_drag: false,
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
            tab.update(cx, |tab, _| send(tab, payload));
        }
        // Typing always returns the view to the prompt line.
        self.follow = true;
    }

    fn near_bottom(&self) -> bool {
        let max = self.scroll.max_offset();
        let offset = self.scroll.offset();
        (max.y + offset.y) <= px(48.)
    }
}

/// Committed text from the platform input method reaches the shell here; the
/// terminal owns no editable text, so every range query answers empty.
pub struct TerminalInput {
    owner: WeakEntity<Observer>,
}

impl TerminalInput {
    pub(crate) fn new(owner: WeakEntity<Observer>) -> Self {
        Self { owner }
    }
}

impl EntityInputHandler for TerminalInput {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
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
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.owner.update(cx, |this, cx| {
            this.terminal.send_text(text, cx);
            cx.notify();
        });
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // A composition preview is not shell input; only the commit above is.
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
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
    if this.terminal.visible && this.terminal.tabs.is_empty() {
        add(this, None, cx);
    }
    cx.notify();
}

pub fn shortcut(
    this: &mut Observer,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<Observer>,
) {
    let key = &event.keystroke;
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
            toggle(this, cx);
            if this.terminal.visible {
                this.terminal.focus.focus(window, cx);
            }
        }
        "k" if this.terminal.visible && this.terminal.focus.is_focused(window) => {
            if let Some(tab) = this.terminal.tabs.get(this.terminal.active) {
                tab.update(cx, |tab, _| clear_keep_input(tab));
            }
            this.terminal.follow = true;
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
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    let (out, mut updates) = tokio::sync::mpsc::channel(64);
    let size = TermSize::new(DEFAULT_ROWS, DEFAULT_COLUMNS);
    let tab = cx.new(|_| Tab {
        screen: Term::new(
            Config {
                default_cursor_style: DEFAULT_CURSOR,
                ..Config::default()
            },
            &size,
            VoidListener,
        ),
        parser: Processor::new(),
        label: None,
        status: "terminal_connecting",
        input: Some(tx),
        size,
    });
    let weak = tab.downgrade();
    this.terminal.active = this.terminal.tabs.len();
    this.terminal.tabs.push(tab);
    let launch = std::thread::Builder::new()
        .name("gpui-terminal".into())
        .spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                runtime.block_on(terminal::run(options, cwd, shell, rx, out));
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
                        tab.label = Some(label);
                        tab.status = "";
                    }
                    Output::Bytes(bytes) => tab.parser.advance(&mut tab.screen, &bytes),
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
                || owner
                    .update(cx, |_, cx| cx.notify())
                    .is_err()
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
    let mut continues = vec![false; rows + 1];
    for cell in content.display_iter {
        if cell.flags.contains(Flags::WRAPLINE) {
            if let Ok(line) = usize::try_from(cell.point.line.0) {
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

fn selection_position(
    scroll: &ScrollHandle,
    position: Point<Pixels>,
    cell_width: Pixels,
    tab: &Tab,
) -> (CellPoint, Side) {
    let local = position - scroll.bounds().origin - scroll.offset();
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
        send(tab, value);
    }
    true
}

fn terminal_color(
    color: alacritty_terminal::vte::ansi::Color,
    colors: &alacritty_terminal::term::color::Colors,
    fallback: Hsla,
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
    const ANSI: [u32; 16] = [
        0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666,
        0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
    ];
    let rgb = match index {
        0..=15 => ANSI[index],
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
) -> (Hsla, Hsla) {
    let ink = terminal_color(cell.fg, colors, foreground);
    if cell.flags.contains(Flags::INVERSE) {
        (terminal_color(cell.bg, colors, background), ink)
    } else {
        (ink, terminal_color(cell.bg, colors, transparent_black().into()))
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
    let foreground = cx.theme().foreground;
    let secondary = cx.theme().muted_foreground;
    let hover = crate::ui::surfaces(this.appearance_dark).hover;
    let active = panel.tabs.get(panel.active).cloned();

    // Match the font and shaped width used to paint the cells. The raw glyph
    // advance can differ from the platform's text layout metrics.
    let font = gpui_kit::font(cx.theme().mono_font_family.clone());
    let font_id = cx.text_system().resolve_font(&font);
    let measured_width = window.text_system().layout_width(font_id, px(FONT_SIZE), '0');
    let cell_width = if measured_width > px(0.) && f32::from(measured_width).is_finite() {
        measured_width
    } else {
        px(FALLBACK_CELL_WIDTH)
    };

    let mut tabs = h_flex().flex_1().min_w_0().h_full().overflow_hidden();
    for (index, tab) in panel.tabs.iter().enumerate() {
        let label = tab
            .read(cx)
            .label
            .clone()
            .unwrap_or_else(|| t!("terminal").to_string());
        let selected = index == panel.active;
        tabs = tabs.child(
            h_flex()
                .id(("terminal-tab", index))
                .flex_none()
                .h_full()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .border_r_1()
                .border_color(border)
                .bg(if selected { background } else { surface })
                .text_size(px(12.))
                .text_color(if selected { foreground } else { secondary })
                .cursor_pointer()
                .child(div().min_w_0().text_ellipsis().child(label))
                .child(
                    div()
                        .id(("terminal-close", index))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
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
                    this.terminal.focus.focus(window, cx);
                    cx.notify();
                })),
        );
    }

    let mut controls = h_flex().flex_none().items_center().gap(px(4.)).px(px(8.));
    controls = controls.child(
        div()
            .id("terminal-add")
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .p(px(4.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
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
        .and_then(|tab| tab.read(cx).label.clone())
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
                        .p(px(4.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
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
                .p(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
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
                .p(px(4.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
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
        .bg(surface)
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
        .bg(background)
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
        let cursor_row = usize::try_from(cursor.point.line.0).unwrap_or(0);
        let cursor_column = usize::from(cursor.point.column.0);
        let show_cursor = has_input && cursor.shape != CursorShape::Hidden && cursor_row < rows;
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
        let cursor_color: Hsla = rgb(CURSOR_COLOR).into();
        let cursor_accent: Hsla = rgb(CURSOR_ACCENT).into();
        let mut lines: Vec<Vec<AnyElement>> = (0..rows).map(|_| Vec::new()).collect();
        for cell in content.display_iter {
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let Ok(line) = usize::try_from(cell.point.line.0) else {
                continue;
            };
            let Some(row) = lines.get_mut(line) else {
                continue;
            };
            let (mut fg, mut bg) = cell_colors(&cell, content.colors, foreground, background);
            if cell.flags.contains(Flags::DIM) {
                fg = fg.opacity(0.55);
            }
            if content.selection.is_some_and(|range| range.contains(cell.point)) {
                bg = cx.theme().primary;
                fg = cx.theme().primary_foreground;
            }
            let cursor_here = cursor_on
                && line == cursor_row
                && usize::from(cell.point.column.0) == cursor_column;
            // Leave the glyph readable when the cursor covers the whole cell.
            if cursor_here && cursor.shape == CursorShape::Block {
                bg = cursor_color;
                fg = cursor_accent;
            }
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
                    .when(cursor_here, |view| match cursor.shape {
                        CursorShape::Beam => view.child(
                            div().absolute().top_0().bottom_0().left_0()
                                .w(px(BEAM_WIDTH)).bg(cursor_color),
                        ),
                        CursorShape::Underline => view.child(
                            div().absolute().bottom_0().left_0().right_0()
                                .h(px(1.)).bg(cursor_color),
                        ),
                        CursorShape::HollowBlock => view.child(
                            div().absolute().inset_0().border_1().border_color(cursor_color),
                        ),
                        CursorShape::Block | CursorShape::Hidden => view,
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
                    let scroll = &this.terminal.scroll;
                    tab.update(cx, |tab, _| {
                        let (point, side) = selection_position(scroll, event.position, cell_width, tab);
                        tab.screen.selection = Some(Selection::new(SelectionType::Simple, point, side));
                    });
                    this.terminal.selection_drag = true;
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| {
                this.terminal.selection_drag = false;
            }))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| {
                this.terminal.selection_drag = false;
            }))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if !this.terminal.selection_drag || event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                if let Some(tab) = this.terminal.tabs.get(this.terminal.active).cloned() {
                    let scroll = &this.terminal.scroll;
                    let changed = tab.update(cx, |tab, _| {
                        let (point, side) = selection_position(scroll, event.position, cell_width, tab);
                        if let Some(selection) = &mut tab.screen.selection {
                            selection.update(point, side);
                            true
                        } else {
                            false
                        }
                    });
                    if changed {
                        this.terminal.follow = false;
                        cx.notify();
                    }
                }
            }))
            .on_scroll(cx.listener(|this, _: &ScrollEvent, _, cx| {
                let follow = this.terminal.near_bottom();
                if follow != this.terminal.follow {
                    this.terminal.follow = follow;
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
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if let Some(tab) = this.terminal.tabs.get(this.terminal.active) {
                    this.terminal.follow = tab.update(cx, |tab, cx| key(tab, event, cx));
                }
                // Follow the cursor after typing, but not when copying a selection.
                cx.stop_propagation();
                cx.notify();
            }));
        // The cursor row is a direct child of the scroll view, so the scroll
        // handle can keep exactly that line in view instead of jumping to the
        // bottom of the 24-row grid (which is mostly empty at startup).
        if panel.follow && cursor_row < rows {
            panel.scroll.scroll_to_item(cursor_row);
        }
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
