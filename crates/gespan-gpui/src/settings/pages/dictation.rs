//! DictationSettings.tsx: the speech model, custom words and the shortcut.
use crate::{
    conversation::dictation::{DICTATE_COMMAND, model_spec, speech},
    input::{self, Input},
    settings::{
        Section, SettingsView,
        widgets::{primary_button, red, secondary_button},
    },
    theme::*,
};
use gespan_models::{DownloadError, ModelSpec, ModelStatus, Progress};
use gpui::{
    AnyElement, Context, Entity, FontWeight, Role, SharedString, Subscription, Task, div,
    prelude::*, px, relative,
};

const ATTRIBUTION: &str = "Speech recognition uses NVIDIA Parakeet TDT 0.6B (CC-BY-4.0), run with sherpa-onnx (Apache-2.0).";
const ALREADY_DOWNLOADING: &str = "The speech model is already downloading.";

#[derive(Default)]
pub(in crate::settings) struct DictationState {
    words: Option<Entity<Input>>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

fn size_label(bytes: u64) -> String {
    format!("{} MB", (bytes as f64 / 1_000_000.).round() as u64)
}

fn percent_of(downloaded: u64, total: u64) -> u64 {
    (downloaded * 100).checked_div(total).unwrap_or(0)
}

/// The status line under a model's name.
pub(in crate::settings) fn status_label(
    spec: &ModelSpec,
    status: ModelStatus,
    running: Option<Progress>,
) -> String {
    let state = match (running, status) {
        (Some(p), _) => format!("Downloading… {}%", percent_of(p.downloaded, p.total)),
        (None, ModelStatus::Ready) => "Downloaded".into(),
        (None, ModelStatus::Partial { downloaded, .. }) => format!(
            "Partly downloaded ({}%)",
            percent_of(downloaded, spec.total_size())
        ),
        (None, ModelStatus::Missing) => "Not downloaded".into(),
    };
    format!("{} · {state}", size_label(spec.total_size()))
}

impl SettingsView {
    /// Reads the models' status and keeps the page current while it shows.
    pub(in crate::settings) fn watch_dictation(&mut self, cx: &mut Context<Self>) {
        if self.section != Section::Dictation {
            self.dictation = DictationState::default();
            return;
        }
        let hub = speech(cx);
        hub.update(cx, |hub, cx| hub.refresh_models(cx));
        let words = cx.new(|cx| {
            let mut input = Input::multiline_field("Custom words", "useInboxUnseen\nrusqlite", cx);
            input.set_text(self.prefs.dictation_words.clone(), cx);
            input
        });
        self.dictation = DictationState {
            _subscriptions: vec![
                cx.observe(&hub, |_, _, cx| cx.notify()),
                cx.subscribe(&words, |this, input, _: &input::Changed, cx| {
                    let text = input.read(cx).content.to_string();
                    if this.prefs.dictation_words != text {
                        this.update(cx, |p| p.dictation_words = text);
                    }
                }),
            ],
            words: Some(words),
            ..Default::default()
        };
    }

    fn download_model(&mut self, spec: &'static ModelSpec, cx: &mut Context<Self>) {
        self.dictation.error = None;
        let Some(task) = speech(cx).update(cx, |hub, cx| hub.download(spec, cx)) else {
            self.dictation.error = Some(ALREADY_DOWNLOADING.into());
            cx.notify();
            return;
        };
        self.dictation._tasks.push(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) | Err(DownloadError::Cancelled) => {}
                    Err(error) => this.dictation.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn delete_model(
        &mut self,
        spec: &'static ModelSpec,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            "Delete speech model",
            Some(&format!(
                "Delete the {} speech model? It can be downloaded again later.",
                spec.label
            )),
            &["Delete", "Cancel"],
            cx,
        );
        self.dictation._tasks.push(cx.spawn(async move |this, cx| {
            if !matches!(answer.await, Ok(0)) {
                return;
            }
            let Ok(deleted) = this.update(cx, |this, cx| {
                this.dictation.error = None;
                speech(cx).update(cx, |hub, cx| hub.delete(spec, cx))
            }) else {
                return;
            };
            let result = deleted.await;
            let _ = this.update(cx, |this, cx| {
                this.dictation.error = result.err();
                cx.notify();
            });
        }));
    }

    pub(super) fn dictation_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let hub = speech(cx);
        let chosen = model_spec(&self.prefs.dictation_model).id;
        let hub_state = hub.read(cx);
        let models: Option<Vec<(&'static ModelSpec, ModelStatus, Option<Progress>)>> =
            hub_state.models().map(|models| {
                models
                    .iter()
                    .map(|&(spec, status)| (spec, status, hub_state.progress(spec.id)))
                    .collect()
            });
        let mut rows = Vec::new();
        match models {
            None => rows.push(super::super::widgets::note("Loading…")),
            Some(models) => {
                for (ix, (spec, status, running)) in models.into_iter().enumerate() {
                    let selected = chosen == spec.id;
                    let label = div()
                        .id(SharedString::from(format!("dictation-model-{}", spec.id)))
                        .debug_selector(move || format!("dictation-model-{}", spec.id))
                        .role(Role::RadioButton)
                        .aria_label(spec.label)
                        .aria_selected(selected)
                        .flex()
                        .items_center()
                        .gap_3()
                        .cursor_pointer()
                        .child(
                            div()
                                .size(px(14.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(if selected {
                                    accent_color()
                                } else {
                                    content(0.25)
                                })
                                .when(selected, |d| {
                                    d.child(div().size(px(6.)).rounded_full().bg(accent_color()))
                                }),
                        )
                        .child(div().min_w_0().child(spec.label))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.prefs.dictation_model != spec.id {
                                this.update(cx, |p| p.dictation_model = spec.id.into());
                            }
                        }));
                    let controls = match (running, status) {
                        (Some(progress), _) => vec![
                            div()
                                .h_1()
                                .w_20()
                                .overflow_hidden()
                                .rounded_full()
                                .bg(content(0.1))
                                .child(div().h_full().rounded_full().bg(accent_color()).w(
                                    relative(
                                        percent_of(progress.downloaded, progress.total) as f32
                                            / 100.,
                                    ),
                                ))
                                .into_any_element(),
                            secondary_button(
                                SharedString::from(format!("dictation-cancel-{}", spec.id)),
                                "Cancel",
                                false,
                                false,
                            )
                            .child("Cancel")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                speech(cx).read(cx).cancel_download(spec.id)
                            }))
                            .into_any_element(),
                        ],
                        (None, ModelStatus::Ready) => vec![
                            secondary_button(
                                SharedString::from(format!("dictation-delete-{}", spec.id)),
                                "Delete",
                                false,
                                false,
                            )
                            .child("Delete")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.delete_model(spec, window, cx)
                            }))
                            .into_any_element(),
                        ],
                        (None, status) => {
                            let text = if matches!(status, ModelStatus::Partial { .. }) {
                                "Resume"
                            } else {
                                "Download"
                            };
                            vec![
                                primary_button(
                                    SharedString::from(format!("dictation-download-{}", spec.id)),
                                    text,
                                    false,
                                )
                                .debug_selector(move || format!("dictation-download-{}", spec.id))
                                .child(text)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.download_model(spec, cx)
                                    }),
                                )
                                .into_any_element(),
                            ]
                        }
                    };
                    rows.push(self.row(
                        None,
                        ix == 0,
                        label,
                        Some(status_label(spec, status, running).into()),
                        controls,
                    ));
                }
            }
        }
        let error = self.dictation.error.clone();
        let model_group = div()
            .flex()
            .flex_col()
            .child(self.group(
                Some("dictation-model"),
                true,
                "Speech model",
                Some("Downloaded once to this device, and recognition runs here too.".into()),
                None,
                rows,
            ))
            .when_some(error, |d, error| {
                d.child(
                    div()
                        .id("dictation-settings-error")
                        .role(Role::Alert)
                        .mt_2()
                        .text_size(px(12.))
                        .text_color(red(1.))
                        .child(error),
                )
            })
            .into_any_element();
        let words = self.dictation.words.clone().map(|words| {
            div()
                .px_3()
                .py_2()
                .text_size(px(12.))
                .child(words)
                .into_any_element()
        });
        let shortcut = crate::settings::command_shortcut(&self.prefs, DICTATE_COMMAND)
            .unwrap_or_else(|| "Off".into());
        let shortcut_row = self.row(
            Some("dictation-shortcut"),
            true,
            "Shortcut",
            Some("In the composer, tap to start and tap again to stop, or hold to talk and release to stop. Change it under Keybindings.".into()),
            vec![
                div()
                    .debug_selector(|| "dictation-shortcut-keys".into())
                    .rounded_md()
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.05))
                    .px_2()
                    .py(px(2.))
                    .text_size(px(12.))
                    .text_color(content(0.7))
                    .child(shortcut)
                    .into_any_element(),
            ],
        );
        vec![
            model_group,
            self.group(
                Some("dictation-words"),
                false,
                "Custom words",
                Some("One per line. Names and identifiers to spell the way you write them, on top of the ones Gespan picks up from the project.".into()),
                None,
                words.into_iter().collect(),
            ),
            div()
                .pt_8()
                .child(shortcut_row)
                .into_any_element(),
            div()
                .pt_8()
                .text_size(px(11.))
                .line_height(px(17.875))
                .text_color(content(0.4))
                .font_weight(FontWeight::NORMAL)
                .child(ATTRIBUTION)
                .into_any_element(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_speech::catalog::{PARAKEET_V2_EN, PARAKEET_V3_MULTI};

    #[test]
    fn model_rows_read_like_the_desktop_app() {
        assert_eq!(
            status_label(&PARAKEET_V2_EN, ModelStatus::Ready, None),
            "661 MB · Downloaded"
        );
        assert_eq!(
            status_label(&PARAKEET_V3_MULTI, ModelStatus::Missing, None),
            "670 MB · Not downloaded"
        );
        let half = PARAKEET_V2_EN.total_size() / 2;
        assert_eq!(
            status_label(
                &PARAKEET_V2_EN,
                ModelStatus::Partial {
                    downloaded: half,
                    total: PARAKEET_V2_EN.total_size()
                },
                None
            ),
            "661 MB · Partly downloaded (49%)"
        );
        // A running download wins over the stored status, and an unknown
        // total reads 0% rather than dividing by zero.
        let running = Progress {
            downloaded: 5,
            total: 0,
        };
        assert_eq!(
            status_label(&PARAKEET_V2_EN, ModelStatus::Missing, Some(running)),
            "661 MB · Downloading… 0%"
        );
    }
}
