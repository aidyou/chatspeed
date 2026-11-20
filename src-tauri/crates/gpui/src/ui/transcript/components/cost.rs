//! Cost analysis shown in completion rows and delegated-task cards.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::*;
use rust_i18n::t;

use crate::{accent, warning, MONO_FONT};
use chatspeed_gpui::presentation::{format_duration, grouped_number, CostTotals, CostView};

fn format_cost(value: Option<f64>) -> String {
    match value {
        Some(cost) => format!("${cost:.6}"),
        None => t!("cost_unpriced").to_string(),
    }
}

fn format_rate(value: Option<f64>) -> String {
    match value {
        Some(rate) => format!("${rate:.4} / M"),
        None => t!("cost_unpriced_rate").to_string(),
    }
}

fn format_percent(value: f64) -> String {
    format!("{value:.2}%")
}

/// A bordered summary value inside the cost card.
fn cost_chip(label: String, accent: Hsla, cx: &App) -> AnyElement {
    div()
        .flex_none()
        .min_h(px(26.))
        .px_2()
        .py(px(2.))
        .rounded(px(8.))
        .border_1()
        .border_color(accent.opacity(0.22))
        .bg(cx.theme().background.opacity(0.92))
        .text_size(px(12.))
        .text_color(cx.theme().foreground)
        .child(label)
        .into_any_element()
}

/// Money values use the accent color, monospace, and a heavier weight.
fn cost_amount(value: Option<f64>, accent: Hsla) -> AnyElement {
    div()
        .flex_none()
        .font_family(MONO_FONT)
        .text_size(px(12.))
        .font_weight(FontWeight::BOLD)
        .text_color(accent)
        .child(format_cost(value))
        .into_any_element()
}

pub(crate) fn cost_card(cost: &CostView, cx: &App) -> AnyElement {
    // Mirrors `.workflow-cost-analysis`: an accent rail, a left-to-right
    // accent wash over the muted surface, and chip-style summary values.
    let accent: Hsla = accent().into();
    let mut body = v_flex()
        .flex_1()
        .min_w_0()
        .gap_3()
        .px(px(20.))
        .py(px(16.))
        .bg(linear_gradient(
            90.,
            linear_color_stop(accent.opacity(0.13), 0.),
            linear_color_stop(accent.opacity(0.), 0.42),
        ))
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(cost_chip(format_duration(cost.duration_ms), accent, cx))
                .child(cost_chip(
                    format!(
                        "{} {}",
                        grouped_number(cost.with_sub_agents.total_tokens),
                        t!("cost_tokens")
                    ),
                    accent,
                    cx,
                )),
        )
        .child(cost_row(
            t!("cost_self").to_string(),
            &cost.self_usage,
            true,
            cx,
        ));
    if cost.has_sub_agents {
        body = body.child(cost_row(
            t!("cost_with_sub_agents").to_string(),
            &cost.with_sub_agents,
            false,
            cx,
        ));
    }
    if !cost.models.is_empty() {
        let mut models = v_flex().w_full().min_w_0().gap_2().pt_2();
        for model in &cost.models {
            let mut tokens = h_flex().w_full().flex_wrap().gap_x_3().gap_y_1();
            for (label, value) in [
                (t!("cost_input").to_string(), model.input_tokens),
                (t!("cost_cache").to_string(), model.cache_tokens),
                (t!("cost_output").to_string(), model.output_tokens),
            ] {
                tokens = tokens.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(cx.theme().foreground)
                        .child(format!("{label} {}", grouped_number(value))),
                );
            }
            if let Some(rate) = model.cache_hit_rate {
                tokens = tokens.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(cx.theme().foreground)
                        .child(format!(
                            "{} {}",
                            t!("cost_cache_hit_rate"),
                            format_percent(rate)
                        )),
                );
            }
            let mut rates = h_flex()
                .w_full()
                .flex_wrap()
                .gap_x_3()
                .gap_y_1()
                .opacity(0.86);
            for (label, rate) in [
                (t!("cost_input").to_string(), model.input_per_million),
                (t!("cost_cache").to_string(), model.cache_per_million),
                (t!("cost_output").to_string(), model.output_per_million),
            ] {
                rates = rates.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(cx.theme().foreground)
                        .child(format!("{label} {}", format_rate(rate))),
                );
            }
            if let Some(multiplier) = model.multiplier {
                rates = rates.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(cx.theme().foreground)
                        .child(format!("×{multiplier}")),
                );
            }
            models = models.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .rounded(px(12.))
                    .bg(cx.theme().background.opacity(0.92))
                    .px(px(16.))
                    .py_2()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_baseline()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().foreground)
                                    .child(model.backend_model.clone()),
                            )
                            .child(cost_amount(model.estimated_cost, accent)),
                    )
                    .child(tokens)
                    .child(rates),
            );
        }
        body = body.child(
            models
                .border_t_1()
                .border_dashed()
                .border_color(accent.opacity(0.28)),
        );
    }
    if cost.is_partial {
        body = body.child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(warning())
                .child(t!("cost_partial").to_string()),
        );
    }
    // Vue paints the rail with `inset 3px 0 0`, which CSS clips to the
    // border radius. GPUI clips children to a square rect, so the rail
    // squared off the card's left corners; the accent wash and border
    // carry the same emphasis without breaking the radius.
    h_flex()
        .relative()
        .w_full()
        .min_w_0()
        .rounded(px(12.))
        .border_1()
        .border_color(accent.opacity(0.24))
        .bg(cx.theme().muted)
        .overflow_hidden()
        .child(body)
        .into_any_element()
}

fn cost_row(label: String, totals: &CostTotals, primary: bool, cx: &App) -> AnyElement {
    let accent: Hsla = accent().into();
    let color = if primary {
        cx.theme().foreground
    } else {
        cx.theme().muted_foreground
    };
    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_baseline()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(label),
                )
                .child(cost_amount(totals.estimated_cost, accent)),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(color)
                .opacity(0.86)
                .child(format_rate(totals.effective_cost_per_million)),
        )
        .into_any_element()
}
