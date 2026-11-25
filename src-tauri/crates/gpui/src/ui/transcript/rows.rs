//! Shared row dispatch and stateless rows for the main and child transcripts.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use std::time::{Duration, Instant};

use crate::{accent, danger, pill, plain_text, ui::Icon, warning, MsgScope, Observer};
use chatspeed_gpui::presentation::{ActivityView, AlertKind, AlertView, Row};

pub(crate) fn latest_active_group(rows: &[Row], running: bool) -> Option<&str> {
    if !running {
        return None;
    }
    rows.iter()
        .rev()
        .find_map(|row| match row {
            Row::Activity(group) => Some(group),
            _ => None,
        })
        .filter(|group| group.ongoing)
        .map(|group| group.key.as_str())
}

const SWEEP_PERIOD: Duration = Duration::from_secs(3);
const SWEEP_HALF_WIDTH: f32 = 0.384;

fn sweep_center(elapsed: Duration) -> f32 {
    let phase = (elapsed.as_nanos() % SWEEP_PERIOD.as_nanos()) as f32
        / SWEEP_PERIOD.as_nanos() as f32;
    // Vue sweeps during the first half of the cycle, then rests off the right edge.
    let t = (phase * 2.).min(1.);
    -0.7 + 2.4 * t * t * (3. - 2. * t)
}

#[derive(IntoElement)]
struct ActivitySweep {
    id: SharedString,
    summary: SharedString,
    color: Hsla,
}

impl RenderOnce for ActivitySweep {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let started = *window.use_keyed_state(self.id, cx, |_, _| Instant::now()).read(cx);
        SweepText {
            text: StyledText::new(self.summary),
            started,
            highlight: self.color,
        }
    }
}

/// Keep GPUI in charge of shaping and ellipsis; tint only the glyphs it laid out.
struct SweepText {
    text: StyledText,
    started: Instant,
    highlight: Hsla,
}

#[cfg(not(feature = "fast"))]
impl IntoElement for SweepText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SweepText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.text.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text.prepaint(id, inspector, bounds, layout, window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        if bounds.size.width <= px(0.) {
            return;
        }
        // Request from paint so GPUI Fast marks the transcript tile animated.
        window.request_animation_frame();
        let center = sweep_center(self.started.elapsed());
        let foreground = window.text_style().color;
        let layout = self.text.layout();
        let height = layout.line_height();
        let mut y = bounds.top();
        for wrapped in layout.line_layouts() {
            let line = &wrapped.unwrapped_layout;
            let baseline = y + (height - line.ascent - line.descent) / 2. + line.ascent;
            for run in &line.runs {
                for glyph in &run.glyphs {
                    let x = glyph.position.x;
                    let origin = point(bounds.left() + x, baseline + glyph.position.y);
                    if glyph.is_emoji {
                        let _ = window.paint_emoji(origin, run.font_id, glyph.id, line.font_size);
                    } else {
                        let position = x / bounds.size.width;
                        let strength = (1. - (position - center).abs() / SWEEP_HALF_WIDTH).max(0.);
                        let color = foreground.blend(self.highlight.opacity(strength));
                        let _ = window.paint_glyph(origin, run.font_id, glyph.id, line.font_size, color);
                    }
                }
            }
            y += wrapped.size(height).height;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{latest_active_group, sweep_center, ActivityView, Duration, Row, SWEEP_HALF_WIDTH, SWEEP_PERIOD};

    #[test]
    fn sweep_crosses_the_text_then_rests_offscreen() {
        let start = sweep_center(Duration::ZERO);
        let middle = sweep_center(Duration::from_millis(750));
        let end = sweep_center(Duration::from_millis(1500));
        assert!(start + SWEEP_HALF_WIDTH < 0.);
        assert!((middle - 0.5).abs() < 0.0001);
        assert!(end - SWEEP_HALF_WIDTH > 1.);
        assert_eq!(sweep_center(Duration::from_millis(2250)), end);
        assert_eq!(sweep_center(SWEEP_PERIOD), start);
    }

    #[test]
    fn sweep_stays_on_the_last_group_and_never_jumps_back() {
        let group = |key: &str, ongoing| {
            Row::Activity(ActivityView {
                key: key.into(),
                icon: "tool",
                summary: String::new(),
                thought_count: 0,
                error_count: 0,
                ongoing,
                items: Vec::new(),
            })
        };
        let rows = [
            group("first", true),
            group("latest", true),
            group("finished", false),
            Row::Divider,
        ];
        assert_eq!(latest_active_group(&rows[..2], true), Some("latest"));
        assert_eq!(latest_active_group(&rows, true), None);
        assert_eq!(latest_active_group(&rows, false), None);
        assert_eq!(latest_active_group(&rows[2..], true), None);
    }
}

/// Shared dispatch for the window transcript and nested delegated transcripts.
pub(crate) fn row(
    window: &Observer,
    row: &Row,
    active_group: Option<&str>,
    scope: MsgScope,
    cx: &mut Context<Observer>,
) -> AnyElement {
    match row {
        Row::User(view) => window.user_row(view, scope, cx),
        Row::Assistant(view) => window.assistant_row(view, scope, cx),
        Row::Activity(group) => window.activity_row(
            group,
            active_group == Some(group.key.as_str()),
            scope,
            cx,
        ),
        Row::Tool(tool) => window.tool_row(tool, scope, cx),
        Row::SubAgent(card) => window.sub_agent_row(card, cx),
        Row::FinishTask(view) => window.finish_task_row(view, cx),
        Row::Snapshot(snapshot) => window.snapshot_row(snapshot, cx),
        Row::Alert(alert) => alert_row(alert),
        Row::Divider => divider_row(cx),
    }
}

pub(crate) fn activity_group(
    group: &ActivityView,
    shimmer: bool,
    items: Option<Vec<AnyElement>>,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    // The press hugs the icon and the summary; the rest of the line stays inert.
    let mut header = h_flex()
        .id(SharedString::from(group.key.clone()))
        .group("activity-header")
        .flex_none()
        .max_w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .h(px(26.))
        .px(px(4.))
        .rounded(px(4.))
        .cursor_pointer();
    let mut title = h_flex()
        .relative()
        .overflow_hidden()
        .h(px(22.))
        .min_w_0()
        .gap_2()
        .child(Icon::new(group.icon).size(15.).color(muted));
    if group.thought_count > 0 {
        title = title.child(pill(
            format!("{} x{}", t!("thought"), group.thought_count),
            accent(),
            Some(accent().opacity(0.12)),
        ));
    }
    if group.error_count > 0 {
        // Vue tints group errors into the background instead of shouting in red.
        title = title.child(
            div()
                .flex_none()
                .px_2()
                .py(px(1.))
                .rounded(px(4.))
                .bg(cx.theme().border.opacity(0.35))
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(format!("{} x{}", t!("error"), group.error_count)),
        );
    }
    let summary = group.summary.trim().replace(['\r', '\n'], " ");
    let summary = if shimmer && !cx.reduce_motion() {
        ActivitySweep {
            id: format!("sweep-{}", group.key).into(),
            summary: summary.into(),
            color: cx.theme().primary.blend(cx.theme().success.opacity(0.18)),
        }
        .into_any_element()
    } else {
        crate::ui::chrome_label(summary).into_any_element()
    };
    title = title.child(
        div()
            .flex_shrink_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(if shimmer { cx.theme().foreground } else { muted })
            .child(summary),
    );
    header = header
        .child(title)
        .child(
            div()
                .id("activity-caret")
                .flex_none()
                .ml(px(7.))
                .invisible()
                .group_hover("activity-header", |style| style.visible())
                .child(crate::ui::rotating_icon(
                    format!("activity-{}", group.key),
                    "caret-right",
                    14.,
                    muted,
                    if items.is_some() { 90. } else { 0. },
                )),
        )
        .on_click(on_toggle);
    v_flex()
        .w_full()
        .min_w_0()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .pl(px(2.))
                .child(header),
        )
        .when_some(items, |body, items| {
            body.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .pl_4()
                    .pt_1()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .children(items),
            )
        })
        .into_any_element()
}

fn alert_row(view: &AlertView) -> AnyElement {
    let is_warning = view.kind == AlertKind::Warning;
    let (icon, color) = if is_warning {
        ("warning", warning())
    } else {
        ("error", danger())
    };
    // Vue renders an `el-alert` with `show-icon`: the icon is a fixed left
    // column and everything else sits in one right column, so the body starts
    // at the title text instead of under the icon. Element Plus tints the
    // description of a light alert with the alert color too, which is why the
    // details read red rather than as ordinary text here.
    let mut content = v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(4.))
        .child(
            div()
                .w_full()
                .min_w_0()
                .text_size(px(14.))
                .line_height(px(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(view.title.clone()),
        );
    if !view.body.is_empty() {
        content = content.child(
            div()
                .w_full()
                .min_w_0()
                .text_size(px(13.))
                .text_color(color)
                .child(plain_text(&view.body, color)),
        );
    }
    h_flex()
        .w_full()
        .min_w_0()
        .items_start()
        .gap(px(8.))
        // Vue pills a warning and keeps a normal radius on an error.
        .rounded(if is_warning { px(9999.) } else { px(8.) })
        .border_1()
        .border_color(color.opacity(0.45))
        .bg(color.opacity(0.08))
        .px_4()
        .py_3()
        .child(Icon::new(icon).size(20.).color(color))
        .child(content)
        .into_any_element()
}

pub(super) fn divider_row(cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .py_3()
        .child(div().flex_1().h(px(1.)).bg(cx.theme().border))
        .child(
            div()
                .flex_none()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .child(t!("clear_context_divider").to_string()),
        )
        .child(div().flex_1().h(px(1.)).bg(cx.theme().border))
        .into_any_element()
}
