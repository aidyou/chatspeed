//! Shared model configuration card.
//!
//! The composer's quick panel and the automation editor edit the same slot map,
//! so they render one component. Callers only differ in where the saved map
//! goes and whether a catalog refresh is still pending.

use chatspeed_gpui::components::{
    input::{Input, InputEvent, InputState},
    slider::{Slider, SliderEvent, SliderState, SliderValue},
    switch::Switch,
};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use serde_json::{json, Value};

use super::footer::{apply_model_choice, model_choices_from, ModelChoice};
use super::panels::{
    capsule_segments, check_row, empty_note, field_step, model_select_trigger, param_label,
};
use crate::ui::menu_popover::MenuPopover;

pub(crate) const MODEL_SLOTS: [&str; 5] = ["plan", "act", "utility", "lite", "vision"];

/// The numeric step shared by both context and output-token controls.
const TOKEN_STEP: u64 = 1000;

/// Editable slot map backing one model configuration card.
pub(crate) struct ModelConfigState {
    pub configs: Value,
    pub slot: String,
    pub proxy: bool,
    pub models: Vec<Value>,
    pub proxy_targets: Value,
    pub temperature_slider: Entity<SliderState>,
    pub context_input: Entity<InputState>,
    pub max_tokens_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl ModelConfigState {
    pub(crate) fn new(
        configs: Value,
        slot: impl Into<String>,
        models: Vec<Value>,
        proxy_targets: Value,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let slot = slot.into();
        let slot_config = configs.get(slot.as_str()).cloned().unwrap_or(Value::Null);
        let temperature = slot_config["temperature"].as_f64().unwrap_or(-0.1) as f32;
        let context = slot_config["contextSize"].as_u64().unwrap_or(128_000);
        let max_tokens = slot_config["maxTokens"].as_u64().unwrap_or(0);

        cx.new(|cx| {
            let temperature_slider = cx.new(|_| {
                SliderState::new()
                    .min(-0.1)
                    .max(2.0)
                    .step(0.1)
                    .default_value(temperature)
            });
            let context_input = cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder("128000");
                input.set_value(context.to_string(), window, cx);
                input
            });
            let max_tokens_input = cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder("0");
                input.set_value(max_tokens.to_string(), window, cx);
                input
            });

            let mut state = Self {
                configs,
                slot,
                proxy: is_proxy(&slot_config),
                models,
                proxy_targets,
                temperature_slider,
                context_input,
                max_tokens_input,
                _subscriptions: Vec::new(),
            };

            let slider_subscription = cx.subscribe_in(
                &state.temperature_slider,
                window,
                |this: &mut Self, _, event: &SliderEvent, _, cx| {
                    let (SliderEvent::Change(SliderValue::Single(value))
                    | SliderEvent::Release(SliderValue::Single(value))) = event
                    else {
                        return;
                    };
                    this.set_temperature(*value);
                    cx.notify();
                },
            );
            let context_subscription = cx.subscribe_in(
                &state.context_input,
                window,
                |this: &mut Self, input: &Entity<InputState>, event: &InputEvent, _, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let Ok(value) = input.read(cx).value().trim().parse::<u64>() else {
                        return;
                    };
                    this.set_context_size(value);
                    cx.notify();
                },
            );
            let max_tokens_subscription = cx.subscribe_in(
                &state.max_tokens_input,
                window,
                |this: &mut Self, input: &Entity<InputState>, event: &InputEvent, _, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let Ok(value) = input.read(cx).value().trim().parse::<u64>() else {
                        return;
                    };
                    this.set_max_tokens(value);
                    cx.notify();
                },
            );
            state._subscriptions = vec![
                slider_subscription,
                context_subscription,
                max_tokens_subscription,
            ];

            // The slider and inputs above were seeded from the requested slot,
            // but the stored map may still lack a full object for a new slot.
            state.ensure_slot(&state.slot.clone());
            state
        })
    }

    pub(crate) fn slot_config(&self) -> Value {
        self.configs
            .get(self.slot.as_str())
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn ensure_slot(&mut self, slot: &str) {
        if !self.configs[slot].is_object() {
            self.configs[slot] = json!({
                "id": 0,
                "model": "",
                "temperature": -0.1,
                "contextSize": 128000,
                "maxTokens": 0,
            });
        }
    }

    fn edit_slot(&mut self, slot: &str, edit: impl FnOnce(&mut Value)) {
        self.ensure_slot(slot);
        edit(&mut self.configs[slot]);
    }

    pub(crate) fn set_slot(&mut self, slot: impl Into<String>, window: &mut Window, cx: &mut App) {
        self.slot = slot.into();
        self.ensure_slot(&self.slot.clone());
        let config = self.slot_config();
        self.proxy = is_proxy(&config);
        let temperature = config["temperature"].as_f64().unwrap_or(-0.1) as f32;
        self.temperature_slider
            .update(cx, |state, cx| state.set_value(temperature, window, cx));
        self.sync_inputs(&config, window, cx);
    }

    pub(crate) fn set_source(&mut self, show_proxy: bool, window: &mut Window, cx: &mut App) {
        if self.proxy == show_proxy {
            return;
        }
        self.proxy = show_proxy;
        let slot = self.slot.clone();
        let config = self.slot_config();
        let first = if show_proxy {
            let group = self.proxy_targets.as_object().and_then(|groups| {
                groups.iter().find_map(|(group, aliases)| {
                    aliases
                        .as_object()
                        .is_some_and(|aliases| !aliases.is_empty())
                        .then(|| group.clone())
                })
            });
            group.and_then(|group| {
                let config = json!({"id": 0, "model": format!("{group}@")});
                model_choices_from(&self.models, &self.proxy_targets, &config)
                    .1
                    .into_iter()
                    .next()
            })
        } else {
            self.models.iter().find_map(|provider| {
                let provider_id = provider["id"].as_i64()?;
                let entry = provider["models"].as_array()?.first()?;
                let model = entry["id"].as_str()?.to_owned();
                Some(ModelChoice {
                    provider_id,
                    model,
                    name: entry["name"].as_str().unwrap_or_default().to_owned(),
                    target: Some(entry.clone()),
                })
            })
        };
        if let Some(choice) = first {
            self.apply_choice(&slot, &choice, window, cx);
        } else if !config.is_object() {
            self.ensure_slot(&slot);
        }
    }

    pub(crate) fn apply_choice(
        &mut self,
        slot: &str,
        choice: &ModelChoice,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.ensure_slot(slot);
        let next = apply_model_choice(&self.configs[slot], choice);
        self.configs[slot] = next.clone();
        let temperature = next["temperature"].as_f64().unwrap_or(-0.1) as f32;
        self.temperature_slider
            .update(cx, |state, cx| state.set_value(temperature, window, cx));
        self.sync_inputs(&next, window, cx);
    }

    pub(crate) fn step_context(&mut self, forward: bool, window: &mut Window, cx: &mut App) {
        let slot = self.slot.clone();
        self.edit_slot(&slot, |config| {
            step_token_field(config, "contextSize", 128_000, forward);
        });
        self.sync_current_inputs(window, cx);
    }

    pub(crate) fn step_max_tokens(&mut self, forward: bool, window: &mut Window, cx: &mut App) {
        let slot = self.slot.clone();
        self.edit_slot(&slot, |config| {
            step_token_field(config, "maxTokens", 0, forward);
        });
        self.sync_current_inputs(window, cx);
    }

    pub(crate) fn set_thinking(&mut self, enabled: bool) {
        let slot = self.slot.clone();
        self.edit_slot(&slot, |config| {
            config["thinking"] = if enabled {
                json!({"type": "enabled", "budgetTokens": 1024})
            } else {
                json!({"type": "disabled"})
            };
        });
    }

    pub(crate) fn set_reasoning_level(&mut self, level: &str) {
        let tokens = reasoning_budget(level);
        let slot = self.slot.clone();
        self.edit_slot(&slot, |config| {
            config["thinking"] = json!({
                "type": "enabled",
                "budgetTokens": tokens,
            });
        });
    }

    fn set_temperature(&mut self, value: f32) {
        let slot = self.slot.clone();
        self.edit_slot(&slot, |config| {
            config["temperature"] = json!(value.clamp(-0.1, 2.0));
        });
    }

    fn set_context_size(&mut self, value: u64) {
        let slot = self.slot.clone();
        self.edit_slot(&slot, |config| {
            config["contextSize"] = json!(value);
        });
    }

    fn set_max_tokens(&mut self, value: u64) {
        let slot = self.slot.clone();
        self.edit_slot(&slot, |config| {
            config["maxTokens"] = json!(value);
        });
    }

    fn sync_current_inputs(&mut self, window: &mut Window, cx: &mut App) {
        let config = self.slot_config();
        self.sync_inputs(&config, window, cx);
    }

    pub(crate) fn sync_inputs(&self, config: &Value, window: &mut Window, cx: &mut App) {
        let context = config["contextSize"].as_u64().unwrap_or(128_000);
        let max_tokens = config["maxTokens"].as_u64().unwrap_or(0);
        self.context_input.update(cx, |state, cx| {
            state.set_value(context.to_string(), window, cx)
        });
        self.max_tokens_input.update(cx, |state, cx| {
            state.set_value(max_tokens.to_string(), window, cx)
        });
    }
}

fn step_token_field(config: &mut Value, key: &str, default: u64, forward: bool) {
    let value = config[key].as_u64().unwrap_or(default);
    config[key] = json!(step_tokens(value, forward));
}

fn step_tokens(value: u64, forward: bool) -> u64 {
    if forward {
        value.saturating_add(TOKEN_STEP)
    } else {
        value.saturating_sub(TOKEN_STEP)
    }
}

fn is_proxy(config: &Value) -> bool {
    config["id"].as_i64().unwrap_or_default() == 0
        && config["model"]
            .as_str()
            .is_some_and(|model| model.contains('@'))
}

/// Token budget behind a reasoning capsule level.
///
/// The model card and the composer's quick selector both offer these levels, so
/// the mapping lives here instead of being repeated at each capsule.
pub(crate) fn reasoning_budget(level: &str) -> u32 {
    match level {
        "max" => 8192,
        "high" => 4096,
        "medium" => 2048,
        _ => 1024,
    }
}

/// The capsule level a stored budget rounds up to.
pub(crate) fn reasoning_level(budget: u64) -> &'static str {
    match budget {
        4097.. => "max",
        2049.. => "high",
        1025.. => "medium",
        _ => "low",
    }
}

/// The full card body, excluding the footer so each caller can commit the draft
/// to its own owner.
pub(crate) fn model_config_content(state: &Entity<ModelConfigState>, cx: &mut App) -> AnyElement {
    let (slot, slot_config, proxy, models, proxy_targets, slider) = {
        let state = state.read(cx);
        let slot_config = state.slot_config();
        (
            state.slot.clone(),
            slot_config,
            state.proxy,
            state.models.clone(),
            state.proxy_targets.clone(),
            state.temperature_slider.clone(),
        )
    };
    let current_model = slot_config["model"].as_str().unwrap_or_default().to_owned();
    let current_provider = slot_config["id"].as_i64().unwrap_or_default();
    let border = cx.theme().border;
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let surface = crate::ui::layers(cx.theme().is_dark());

    let mut tabs = h_flex()
        .w_full()
        .min_w_0()
        .gap(px(2.))
        .border_b_1()
        .border_color(border);
    for value in MODEL_SLOTS {
        let selected = slot == value;
        let target = value.to_owned();
        let state_for_tab = state.clone();
        tabs = tabs.child(
            h_flex()
                .id(SharedString::from(format!("model-slot-{value}")))
                .flex_none()
                .items_center()
                .justify_center()
                .px(px(12.))
                .py(px(8.))
                .border_b_2()
                .border_color(if selected {
                    primary
                } else {
                    primary.opacity(0.)
                })
                .cursor_pointer()
                .text_size(px(12.))
                .font_weight(if selected {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if selected { primary } else { muted })
                .child(value.to_uppercase())
                .on_click(move |_, window, cx| {
                    state_for_tab.update(cx, |this, cx| {
                        this.set_slot(target.clone(), window, cx);
                        cx.notify();
                    });
                }),
        );
    }

    let mut modes = h_flex()
        .flex_none()
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(4.))
        .border_1()
        // The provider/proxy switch is a well inside the card, like the
        // reasoning capsule below it.
        .bg(surface.track)
        .border_color(surface.hairline);
    for (show_proxy, key) in [
        (false, "model_source_provider"),
        (true, "model_source_proxy"),
    ] {
        let state_for_mode = state.clone();
        modes = modes.child(
            div()
                .id(("model-source", usize::from(show_proxy)))
                .flex_none()
                .px(px(8.))
                .py(px(3.))
                .rounded(px(3.))
                .cursor_pointer()
                .text_size(px(11.))
                .text_color(if proxy == show_proxy { primary } else { muted })
                .when(proxy == show_proxy, |tab| tab.bg(primary.opacity(0.12)))
                .hover(|style| style.bg(cx.theme().muted))
                .child(t!(key).to_string())
                .on_click(move |_, window, cx| {
                    state_for_mode.update(cx, |this, cx| {
                        this.set_source(show_proxy, window, cx);
                        cx.notify();
                    });
                }),
        );
    }

    let (group_label, choices) = model_choices_from(&models, &proxy_targets, &slot_config);
    let providers: Vec<(i64, String, ModelChoice)> = models
        .iter()
        .filter_map(|provider| {
            let provider_id = provider["id"].as_i64()?;
            let entry = provider["models"].as_array()?.first()?;
            let model = entry["id"].as_str()?.to_owned();
            Some((
                provider_id,
                provider["name"].as_str().unwrap_or_default().to_owned(),
                ModelChoice {
                    provider_id,
                    model,
                    name: entry["name"].as_str().unwrap_or_default().to_owned(),
                    target: Some(entry.clone()),
                },
            ))
        })
        .collect();
    let proxy_groups: Vec<String> = proxy_targets
        .as_object()
        .map(|groups| {
            groups
                .iter()
                .filter(|(_, aliases)| {
                    aliases
                        .as_object()
                        .is_some_and(|aliases| !aliases.is_empty())
                })
                .map(|(group, _)| group.clone())
                .collect()
        })
        .unwrap_or_default();

    let left_label = if proxy {
        group_label.clone()
    } else {
        providers
            .iter()
            .find(|(id, _, _)| *id == current_provider)
            .map(|(_, name, _)| name.clone())
            .unwrap_or_else(|| t!("model_placeholder").to_string())
    };
    let left_trigger = model_select_trigger("model-left-select", left_label, true, 120., cx);
    let left_select = if proxy {
        let state_for_select = state.clone();
        let slot_key = slot.clone();
        let groups = proxy_groups.clone();
        MenuPopover::new("model-group-popover", left_trigger, move |_, _, cx| {
            let popover = cx.entity();
            let mut list = v_flex().id("model-group-list").w(px(220.)).gap(px(2.));
            for group in groups.clone() {
                let popover = popover.clone();
                let state_for_item = state_for_select.clone();
                let slot_key = slot_key.clone();
                let selected = group == group_label;
                let item_group = group.clone();
                list = list.child(check_row(
                    format!("model-group-{group}"),
                    group,
                    String::new(),
                    selected,
                    move |_, window, cx| {
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                        state_for_item.update(cx, |this, cx| {
                            let config = json!({"id": 0, "model": format!("{item_group}@")});
                            if let Some(choice) =
                                model_choices_from(&this.models, &this.proxy_targets, &config)
                                    .1
                                    .into_iter()
                                    .next()
                            {
                                this.apply_choice(&slot_key, &choice, window, cx);
                            }
                            cx.notify();
                        });
                    },
                    cx,
                ));
            }
            list
        })
        .into_any_element()
    } else {
        let state_for_select = state.clone();
        let slot_key = slot.clone();
        let providers = providers.clone();
        MenuPopover::new("model-provider-popover", left_trigger, move |_, _, cx| {
            let popover = cx.entity();
            let mut list = v_flex().id("model-provider-list").w(px(220.)).gap(px(2.));
            for (provider_id, name, choice) in providers.clone() {
                let popover = popover.clone();
                let state_for_item = state_for_select.clone();
                let slot_key = slot_key.clone();
                let selected = provider_id == current_provider;
                list = list.child(check_row(
                    format!("model-provider-{provider_id}"),
                    name,
                    String::new(),
                    selected,
                    move |_, window, cx| {
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                        state_for_item.update(cx, |this, cx| {
                            this.apply_choice(&slot_key, &choice, window, cx);
                            cx.notify();
                        });
                    },
                    cx,
                ));
            }
            list
        })
        .into_any_element()
    };

    let right_label = choices
        .iter()
        .find(|choice| choice.model == current_model)
        .map(|choice| choice.name.clone())
        .unwrap_or_else(|| {
            if current_model.is_empty() {
                t!("model_placeholder").to_string()
            } else {
                current_model.clone()
            }
        });
    let right_trigger = model_select_trigger("model-right-select", right_label, true, 0., cx);
    let state_for_choices = state.clone();
    let slot_key = slot.clone();
    let all_choices = choices.clone();
    let right_select = MenuPopover::new("model-choice-popover", right_trigger, move |_, _, cx| {
        let popover = cx.entity();
        let mut list = v_flex()
            .id("model-choice-list")
            .w(px(240.))
            .max_h(px(280.))
            .overflow_y_scroll()
            .gap(px(2.));
        if all_choices.is_empty() {
            list = list.child(empty_note(t!("model_placeholder").to_string(), cx));
        }
        for choice in all_choices.clone() {
            let popover = popover.clone();
            let state_for_choice = state_for_choices.clone();
            let slot_key = slot_key.clone();
            let selected = choice.model == current_model;
            list = list.child(check_row(
                format!("model-choice-{}-{}", choice.provider_id, choice.model),
                choice.name.clone(),
                String::new(),
                selected,
                move |_, window, cx| {
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                    state_for_choice.update(cx, |this, cx| {
                        this.apply_choice(&slot_key, &choice, window, cx);
                        cx.notify();
                    });
                },
                cx,
            ));
        }
        list
    })
    .into_any_element();

    v_flex()
        .w_full()
        .min_w_0()
        .gap(px(16.))
        .px(px(16.))
        .pt(px(16.))
        .child(tabs)
        .child(
            v_flex()
                .w_full()
                .min_w_0()
                .gap(px(12.))
                .p(px(12.))
                .rounded(px(8.))
                .border_1()
                // Level 2 of the dialog ladder: the popover around this card is
                // level 1, and the segmented controls inside are level 3, so the
                // three steps stay distinguishable in both themes.
                .border_color(surface.hairline)
                .bg(surface.card)
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(foreground)
                                .child(format!(
                                    "{} {}",
                                    slot.to_uppercase(),
                                    t!("status_panel_models")
                                )),
                        )
                        .child(modes),
                )
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap(px(8.))
                        .child(left_select)
                        .child(right_select),
                )
                .child(model_params(state, &slot_config, &slider, cx)),
        )
        .into_any_element()
}

fn model_params(
    state: &Entity<ModelConfigState>,
    slot_config: &Value,
    slider: &Entity<SliderState>,
    cx: &mut App,
) -> AnyElement {
    let foreground = cx.theme().foreground;

    let temperature = slot_config["temperature"]
        .as_f64()
        .unwrap_or(-0.1)
        .clamp(-0.1, 2.0);
    let temperature_text = if temperature < 0.0 {
        t!("param_off").to_string()
    } else {
        format!("{temperature:.1}")
    };
    let temperature_row = h_flex()
        .w_full()
        .items_center()
        .gap(px(8.))
        .child(param_label(t!("model_temperature").to_string(), cx))
        .child(Slider::new(slider).flex_1().min_w(px(0.)))
        .child(
            div()
                .flex_none()
                .min_w(px(38.))
                .text_align(TextAlign::Right)
                .text_size(px(11.))
                .text_color(foreground)
                .child(temperature_text),
        );

    let state_for_context_minus = state.clone();
    let context_minus = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
        state_for_context_minus.update(cx, |this, cx| {
            this.step_context(false, window, cx);
            cx.notify();
        });
    };
    let state_for_context_plus = state.clone();
    let context_plus = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
        state_for_context_plus.update(cx, |this, cx| {
            this.step_context(true, window, cx);
            cx.notify();
        });
    };
    let state_for_max_minus = state.clone();
    let max_minus = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
        state_for_max_minus.update(cx, |this, cx| {
            this.step_max_tokens(false, window, cx);
            cx.notify();
        });
    };
    let state_for_max_plus = state.clone();
    let max_plus = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
        state_for_max_plus.update(cx, |this, cx| {
            this.step_max_tokens(true, window, cx);
            cx.notify();
        });
    };

    let (context_input, max_tokens_input) = {
        let state = state.read(cx);
        (state.context_input.clone(), state.max_tokens_input.clone())
    };
    let context_field = chatspeed_gpui::components::input::field(&context_input, cx)
        .suffix(
            h_flex()
                .flex_none()
                .items_center()
                .gap(px(2.))
                .child(field_step("model-context-minus", "−", context_minus, cx))
                .child(field_step("model-context-plus", "+", context_plus, cx)),
        );
    let max_field = Input::new(&max_tokens_input)
        .bg(crate::ui::field_bg(cx))
        .suffix(
            h_flex()
                .flex_none()
                .items_center()
                .gap(px(2.))
                .child(field_step("model-max-minus", "−", max_minus, cx))
                .child(field_step("model-max-plus", "+", max_plus, cx)),
        );

    let reasoning_enabled = slot_config["thinking"]["type"]
        .as_str()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("enabled"));
    let budget = slot_config["thinking"]["budgetTokens"]
        .as_u64()
        .unwrap_or(1024);
    let level = reasoning_level(budget);
    let state_for_thinking = state.clone();
    let thinking_switch = Switch::new("model-thinking")
        .checked(reasoning_enabled)
        .small()
        .on_click(move |checked, _, cx| {
            state_for_thinking.update(cx, |this, cx| {
                this.set_thinking(*checked);
                cx.notify();
            });
        });

    let mut reasoning = v_flex().w_full().min_w_0().gap(px(8.)).child(
        h_flex()
            .w_full()
            .items_center()
            .gap(px(4.))
            .child(param_label(t!("model_reasoning").to_string(), cx))
            .child(thinking_switch),
    );
    if reasoning_enabled {
        let state_for_level = state.clone();
        let levels = capsule_segments(
            "model-level",
            vec![
                ("low", t!("reasoning_low").to_string()),
                ("medium", t!("reasoning_medium").to_string()),
                ("high", t!("reasoning_high").to_string()),
                ("max", t!("reasoning_max").to_string()),
            ],
            level,
            move |value, _, cx| {
                state_for_level.update(cx, |this, cx| {
                    this.set_reasoning_level(value);
                    cx.notify();
                });
            },
            cx,
        );
        reasoning = reasoning.child(
            h_flex()
                .w_full()
                .items_center()
                .gap(px(8.))
                .child(param_label(t!("model_reasoning_level").to_string(), cx))
                .child(levels),
        );
    }

    v_flex()
        .id("model-params")
        .w_full()
        .min_w_0()
        .gap(px(8.))
        .p_3()
        .child(temperature_row)
        .child(
            h_flex()
                .w_full()
                .gap(px(16.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(4.))
                        .child(param_label(t!("model_context_size").to_string(), cx))
                        .child(context_field),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(4.))
                        .child(param_label(t!("model_max_tokens").to_string(), cx))
                        .child(max_field),
                ),
        )
        .child(reasoning)
        .into_any_element()
}

/// The shared Save/Cancel row. Callers only decide where the saved map goes.
pub(crate) fn model_config_footer(
    id_prefix: &'static str,
    save_enabled: bool,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_save: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let border = cx.theme().border;
    let primary = cx.theme().primary;
    let foreground = cx.theme().foreground;
    div()
        .w_full()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_end()
                .gap(px(12.))
                .px_3()
                .py_2()
                .child(
                    div()
                        .id(SharedString::from(format!("{id_prefix}-cancel")))
                        .flex_none()
                        .px(px(16.))
                        .py(px(4.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(border)
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(foreground)
                        .hover(|style| style.bg(cx.theme().muted))
                        .child(t!("action_cancel").to_string())
                        .on_click(on_cancel),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("{id_prefix}-save")))
                        .flex_none()
                        .px(px(16.))
                        .py(px(4.))
                        .rounded(px(8.))
                        .bg(if save_enabled {
                            primary
                        } else {
                            primary.opacity(0.5)
                        })
                        .text_size(px(13.))
                        .text_color(cx.theme().primary_foreground)
                        .when(save_enabled, |button| {
                            button
                                .cursor_pointer()
                                .hover(|style| style.opacity(0.9))
                                .on_click(on_save)
                        })
                        .child(t!("session_save").to_string()),
                ),
        )
        .into_any_element()
}

pub(crate) fn model_config_panel(
    state: &Entity<ModelConfigState>,
    footer: AnyElement,
    cx: &mut App,
) -> AnyElement {
    v_flex()
        .w_full()
        .child(model_config_content(state, cx))
        .child(footer)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{reasoning_budget, reasoning_level, step_token_field, step_tokens};
    use serde_json::json;

    #[::core::prelude::v1::test]
    fn reasoning_levels_round_trip_through_their_budgets() {
        for level in ["low", "medium", "high", "max"] {
            assert_eq!(reasoning_level(u64::from(reasoning_budget(level))), level);
        }
        assert_eq!(reasoning_level(0), "low");
        assert_eq!(reasoning_level(1025), "medium");
        assert_eq!(reasoning_level(2049), "high");
        assert_eq!(reasoning_level(4097), "max");
        assert_eq!(reasoning_level(8192), "max");
    }

    #[::core::prelude::v1::test]
    fn token_steps_are_one_thousand_with_no_upper_bound() {
        assert_eq!(step_tokens(0, false), 0);
        assert_eq!(step_tokens(0, true), 1000);
        assert_eq!(step_tokens(128_000, false), 127_000);
        assert_eq!(step_tokens(128_000, true), 129_000);
        assert_eq!(step_tokens(u64::MAX, true), u64::MAX);
    }

    #[::core::prelude::v1::test]
    fn token_fields_use_their_own_defaults_and_keep_zero_as_the_floor() {
        let mut config = json!({});
        step_token_field(&mut config, "contextSize", 128_000, true);
        step_token_field(&mut config, "maxTokens", 0, true);
        assert_eq!(config["contextSize"], 129_000);
        assert_eq!(config["maxTokens"], 1000);

        step_token_field(&mut config, "contextSize", 128_000, false);
        step_token_field(&mut config, "maxTokens", 0, false);
        assert_eq!(config["contextSize"], 128_000);
        assert_eq!(config["maxTokens"], 0);
    }
}
