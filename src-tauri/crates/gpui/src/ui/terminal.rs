use crate::Observer;
use alacritty_terminal::{
    event::VoidListener,
    grid::Dimensions,
    term::{Config, Term, TermMode},
    vte::ansi::Processor,
};
use chatspeed_gpui::terminal::{self, Input, Output};
use gpui_kit::{
    component::{h_flex, v_flex, ActiveTheme},
    *,
};
use rust_i18n::t;

struct Size;

#[cfg(test)]
mod tests {
    use super::{Config, Processor, Size, Term, VoidListener};

    #[test]
    fn terminal_parser_preserves_split_utf8_and_cursor_movement() {
        let mut screen = Term::new(Config::default(), &Size, VoidListener);
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
    fn terminal_preserves_suggestion_style_and_cursor() {
        use alacritty_terminal::{term::cell::Flags, vte::ansi::{Color, NamedColor}};
        let mut screen = Term::new(Config::default(), &Size, VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut screen, b"git\x1b[2;31m status\x1b[0m\x1b[7D");
        let content = screen.renderable_content();
        assert_eq!(content.cursor.point.column.0, 3);
        let suggestion = content.display_iter.skip(3).next().unwrap();
        assert!(suggestion.flags.contains(Flags::DIM));
        assert_eq!(suggestion.fg, Color::Named(NamedColor::Red));
        assert_ne!(super::terminal_color(suggestion.fg, content.colors, gpui_kit::transparent_black().into()), gpui_kit::transparent_black().into());
    }
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        24
    }
    fn screen_lines(&self) -> usize {
        24
    }
    fn columns(&self) -> usize {
        80
    }
}

pub struct Panel {
    focus: FocusHandle,
    visible: bool,
    tabs: Vec<Entity<Tab>>,
    shells: Vec<(String, String)>,
    active: usize,
}

pub struct Tab {
    screen: Term<VoidListener>,
    parser: Processor,
    label: Option<String>,
    status: &'static str,
    input: Option<tokio::sync::mpsc::Sender<Input>>,
}

impl Panel {
    pub fn new(cx: &App) -> Self {
        Self {
            focus: cx.focus_handle(),
            visible: false,
            tabs: Vec::new(),
            shells: Vec::new(),
            active: 0,
        }
    }
}

pub fn toggle(this: &mut Observer, cx: &mut Context<Observer>) {
    this.terminal.visible = !this.terminal.visible;
    if this.terminal.visible && this.terminal.tabs.is_empty() {
        add(this, None, cx);
    }
    cx.notify();
}

pub fn shortcut(this: &mut Observer, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Observer>) {
    let key = &event.keystroke;
    let mods = key.modifiers;
    let command = if cfg!(target_os = "macos") { mods.platform && !mods.control } else { mods.control && !mods.platform };
    if !command || mods.alt || mods.shift { return; }
    match key.key.as_str() {
        "j" => {
            toggle(this, cx);
            if this.terminal.visible { this.terminal.focus.focus(window, cx); }
        }
        "k" if this.terminal.visible && this.terminal.focus.is_focused(window) => {
            if let Some(tab) = this.terminal.tabs.get(this.terminal.active) {
                tab.update(cx, |tab, _| {
                    tab.parser.advance(&mut tab.screen, b"\x1b[2J\x1b[3J");
                });
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

fn add_tab(this: &mut Observer, shell: Option<String>, replace: Option<Entity<Tab>>, cx: &mut Context<Observer>) {
    let options = this.options.clone();
    let cwd = this.roots().first().cloned();
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    let (out, mut updates) = tokio::sync::mpsc::channel(64);
    let tab = cx.new(|_| Tab {
        screen: Term::new(Config::default(), &Size, VoidListener),
        parser: Processor::new(),
        label: None,
        status: "terminal_connecting",
        input: Some(tx),
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
                    if owner.update(cx, |this, cx| { this.terminal.shells = shells; cx.notify(); }).is_err() { return; }
                    continue;
                }
                update => update,
            };
            if weak
                .update(cx, |tab, _| match update {
                    Output::Shells(_) => {},
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

fn key(tab: &mut Tab, event: &KeyDownEvent, cx: &App) {
    let key = &event.keystroke;
    let mods = key.modifiers;
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
        return;
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
        _ if !mods.control && !mods.platform => key.key_char.clone().unwrap_or_default(),
        _ => String::new(),
    };
    if !value.is_empty() {
        send(tab, value);
    }
}

fn terminal_color(color: alacritty_terminal::vte::ansi::Color, colors: &alacritty_terminal::term::color::Colors, fallback: Hsla) -> Hsla {
    use alacritty_terminal::vte::ansi::Color;
    let index = match color {
        Color::Spec(rgb) => return gpui_kit::rgb((u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b)).into(),
        Color::Named(name) => name as usize,
        Color::Indexed(index) => usize::from(index),
    };
    if let Some(rgb) = colors[index] {
        return gpui_kit::rgb((u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b)).into();
    }
    const ANSI: [u32; 16] = [0x000000,0xcd3131,0x0dbc79,0xe5e510,0x2472c8,0xbc3fbc,0x11a8cd,0xe5e5e5,0x666666,0xf14c4c,0x23d18b,0xf5f543,0x3b8eea,0xd670d6,0x29b8db,0xffffff];
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

pub fn render(this: &Observer, cx: &mut Context<Observer>) -> Option<AnyElement> {
    let panel = &this.terminal;
    if !panel.visible {
        return None;
    }
    let active = panel.tabs.get(panel.active).cloned();
    let focus = panel.focus.clone();
    let border = cx.theme().border;
    let muted = cx.theme().muted;
    let mut header = h_flex()
        .gap_2()
        .px_2()
        .py_1()
        .child(t!("terminal").to_string());
    for (index, tab) in panel.tabs.iter().enumerate() {
        let label = tab
            .read(cx)
            .label
            .clone()
            .unwrap_or_else(|| t!("terminal").to_string());
        header = header.child(
            h_flex()
                .gap_1()
                .child(
                    div()
                        .id(("terminal-tab", index))
                        .px_1()
                        .cursor_pointer()
                        .bg(if index == panel.active {
                            muted
                        } else {
                            transparent_black()
                        })
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.terminal.active = index;
                            this.terminal.focus.focus(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id(("terminal-close", index))
                        .cursor_pointer()
                        .child("×")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if index < this.terminal.tabs.len() {
                                this.terminal.tabs.remove(index);
                                if this.terminal.tabs.is_empty() {
                                    this.terminal.visible = false;
                                }
                                let active = this.terminal.active;
                                this.terminal.active = (active - usize::from(index < active))
                                    .min(this.terminal.tabs.len().saturating_sub(1));
                            }
                            cx.notify();
                        })),
                ),
        );
    }
    header = header.children(panel.shells.iter().enumerate().map(|(index, (name, path))| {
        let path = path.clone();
        div().id(("terminal-shell", index)).cursor_pointer().px_1().child(name.clone())
            .on_click(cx.listener(move |this, _, window, cx| {
                let replace = this.terminal.tabs.get(this.terminal.active).cloned();
                add_tab(this, Some(path.clone()), replace, cx);
                this.terminal.focus.focus(window, cx);
            }))
    }));
    header = header
        .child(
            div()
                .id("terminal-add")
                .cursor_pointer()
                .child("+")
                .on_click(cx.listener(|this, _, window, cx| {
                    add(this, None, cx);
                    this.terminal.focus.focus(window, cx);
                })),
        )
        .child(div().flex_1())
        .child(
            div()
                .id("terminal-hide")
                .cursor_pointer()
                .child("−")
                .on_click(cx.listener(|this, _, _, cx| toggle(this, cx))),
        );
    let mut body = v_flex()
        .id("terminal-content")
        .track_focus(&focus)
        .flex_1()
        .min_h_0()
        .overflow_scroll()
        .px_2()
        .font_family("monospace")
        .text_size(px(12.));
    if let Some(tab) = active {
        let state = tab.read(cx);
        if !state.status.is_empty() {
            body = body.child(t!(state.status).to_string());
        }
        let content = state.screen.renderable_content();
        let cursor = content.cursor;
        let mut rows: Vec<Vec<AnyElement>> = (0..24).map(|_| Vec::new()).collect();
        for cell in content.display_iter {
            use alacritty_terminal::term::cell::Flags;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let Some(row) = usize::try_from(cell.point.line.0).ok().and_then(|line| rows.get_mut(line)) else {
                continue;
            };
            let theme = cx.theme();
            let mut fg = terminal_color(cell.fg, content.colors, theme.foreground);
            let mut bg = terminal_color(cell.bg, content.colors, transparent_black().into());
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = fg.opacity(0.55);
            }
            let caret = cell.point == cursor.point && cursor.shape != alacritty_terminal::vte::ansi::CursorShape::Hidden && state.input.is_some();
            if caret {
                bg = theme.foreground;
                fg = theme.background;
            }
            row.push(div().flex_none().text_color(fg).bg(bg).child(cell.c.to_string()).into_any_element());
        }
        body = body
            .children(rows.into_iter().map(|line| h_flex().flex_none().whitespace_nowrap().children(line)))
            .on_key_down(cx.listener(move |_, event, _, cx| {
                tab.update(cx, |tab, cx| key(tab, event, cx));
                cx.stop_propagation();
                cx.notify();
            }));
    }
    Some(
        v_flex()
            .h(px(260.))
            .max_h(relative(0.5))
            .min_h_0()
            .w_full()
            .border_t_1()
            .border_color(border)
            .child(header)
            .child(body.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                focus.focus(window, cx)
            }))
            .into_any_element(),
    )
}
