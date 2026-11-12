//! The two-click review setup sheet: one sentence of inline choices, a
//! GitHub readiness line, and Turn on / Customize… / Cancel. What it saves
//! is `review_draft` of exactly the values the sentence shows.
use super::*;
use gespan_core::{
    automations::review_setup::{ReviewSetupInput, WhichPrs, find_review_automation, review_draft},
    review_policy::{ReviewResults, supported_results},
};
use std::path::{Path, PathBuf};

const GITHUB_CLI_URL: &str = "https://cli.github.com";
const ANY_REPO: &str = "any repo in this project";
const CHOOSE_REPO: &str = "Choose a repository…";

const WHICH: [(WhichPrs, &str, &str); 3] = [
    (
        WhichPrs::ReviewRequested,
        "review-requested",
        "asks for your review",
    ),
    (WhichPrs::Opened, "opened", "is opened"),
    (
        WhichPrs::OpenedAndDrafts,
        "opened-and-drafts",
        "is opened, drafts too",
    ),
];
/// Shown beneath Results whenever it is Post, in the sheet and the editor.
pub(super) const POST_WARNING: &str = "Posted reviews skip your approval. A pull request can try to make the agent quote local files in its review.";

/// The warning under a Results choice, only for Post.
pub(super) fn post_warning(results: ReviewResults) -> Option<&'static str> {
    (results == ReviewResults::Post).then_some(POST_WARNING)
}

pub(super) const RESULTS: [(ReviewResults, &str, &str); 3] = [
    (
        ReviewResults::Gespan,
        "gespan",
        "keeps the review in Gespan",
    ),
    (
        ReviewResults::Draft,
        "draft",
        "drafts a review for you to approve",
    ),
    (ReviewResults::Post, "post", "posts the review on the PR"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    Checking,
    NotInstalled,
    SignedOut,
    Connected,
    NoGithubRemote,
    Remote,
}

/// Where the project stands with GitHub. A remote project is never ready,
/// since the check and the sign-in are both local.
pub(crate) fn readiness(
    installed: bool,
    authenticated: bool,
    repos: &[String],
    remote: bool,
    chosen_repo: Option<&str>,
) -> Readiness {
    if remote {
        Readiness::Remote
    } else if !installed {
        Readiness::NotInstalled
    } else if !authenticated {
        Readiness::SignedOut
    } else if repos.is_empty() && chosen_repo.is_none() {
        Readiness::NoGithubRemote
    } else {
        Readiness::Connected
    }
}

pub(crate) fn can_turn_on(readiness: Readiness) -> bool {
    readiness == Readiness::Connected
}

/// The repository the sentence starts on: the project's only one, else any.
pub(crate) fn default_repo(repos: &[String]) -> Option<String> {
    match repos {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// The Results the harness can enforce.
pub(crate) fn results_for(harness: &str) -> &'static [ReviewResults] {
    supported_results(Kind::from_id(harness))
}

/// What the readiness check found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GithubCheck {
    pub installed: bool,
    pub authenticated: bool,
    pub repos: Vec<String>,
    pub remote: bool,
}

/// Runs the readiness check for a project folder. Blocking; called off the
/// UI thread. Tests swap it so they never run `gh`.
pub(crate) type GithubProbe = Arc<dyn Fn(&str) -> GithubCheck + Send + Sync>;

/// The real check: Settings → Inbox's `gh` status and the project's GitHub
/// remotes. A remote project is not asked anything.
pub(crate) fn github_probe() -> GithubProbe {
    Arc::new(|cwd: &str| {
        let path = Path::new(cwd);
        if gespan_core::files::remote::is_remote(path) {
            return GithubCheck {
                remote: true,
                ..Default::default()
            };
        }
        // GitHub's status reads only the project folder, not the data folder.
        let status = gespan_core::inbox::Service {
            data_dir: PathBuf::new(),
            cwd: path.to_path_buf(),
        }
        .status("github")
        .unwrap_or_default();
        GithubCheck {
            installed: status.installed,
            authenticated: status.authenticated,
            repos: gespan_core::inbox::github_repositories(path).unwrap_or_default(),
            remote: false,
        }
    })
}

#[derive(Clone)]
pub(crate) struct ReviewSetup {
    pub input: ReviewSetupInput,
    pub readiness: Readiness,
    pub repos: Vec<String>,
    /// Which readiness check is current; an older one's answer is dropped.
    pub generation: u64,
    check: Option<GithubCheck>,
}

fn which_label(which: WhichPrs) -> &'static str {
    WHICH
        .iter()
        .find(|(value, ..)| *value == which)
        .map_or("", |(.., label)| label)
}

pub(super) fn results_from_id(id: &str) -> Option<ReviewResults> {
    RESULTS
        .iter()
        .find(|(_, value, _)| *value == id)
        .map(|(results, ..)| *results)
}

pub(super) fn results_label(results: ReviewResults) -> &'static str {
    RESULTS
        .iter()
        .find(|(value, ..)| *value == results)
        .map_or("", |(.., label)| label)
}

fn model_label(input: &ReviewSetupInput) -> String {
    let kind = Kind::from_id(&input.harness);
    let model = gespan_core::store::native_model(&input.model, &input.harness);
    cached_model_name(kind, model.as_deref()).to_string()
}

impl Automations {
    /// The "Review pull requests" card: the sheet, or the project's existing
    /// review automation in the editor so there is never a second one.
    pub(super) fn open_review_setup(&mut self, cx: &mut Context<Self>) {
        if let Some(existing) = find_review_automation(&self.automations, &self.cwd) {
            let id = existing.id.clone();
            self.select(id, cx);
            return;
        }
        let preferences = cx.try_global::<crate::settings::Preferences>();
        let (kind, model) = crate::installed_providers::new_session_choice(
            preferences.unwrap_or(&Default::default()),
            Some(&self.cwd),
            cx,
        );
        let input = ReviewSetupInput {
            cwd: self.cwd.clone(),
            repo: None,
            which: WhichPrs::ReviewRequested,
            harness: kind.id().into(),
            model: model.unwrap_or_default(),
            results: ReviewResults::Gespan,
        };
        self.review_generation += 1;
        self.review_setup = Some(ReviewSetup {
            input,
            readiness: Readiness::Checking,
            repos: Vec::new(),
            generation: self.review_generation,
            check: None,
        });
        self.choice = None;
        self.models = None;
        self.trigger_menu = None;
        self.project_picker = None;
        self.error = None;
        self.check_github(cx);
        cx.notify();
    }

    /// The project the open review sheet is for.
    pub(crate) fn review_cwd(&self) -> Option<&str> {
        self.review_setup
            .as_ref()
            .map(|setup| setup.input.cwd.as_str())
    }

    /// Runs the readiness check again, after a sign-in or "Check again".
    pub(crate) fn recheck_github(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = &mut self.review_setup else {
            return;
        };
        self.review_generation += 1;
        setup.generation = self.review_generation;
        setup.readiness = Readiness::Checking;
        self.check_github(cx);
        cx.notify();
    }

    fn check_github(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = &self.review_setup else {
            return;
        };
        let generation = setup.generation;
        let cwd = setup.input.cwd.clone();
        let probe = self.github_probe.clone();
        let work = cx.background_executor().spawn(async move { probe(&cwd) });
        cx.spawn(async move |this, cx| {
            let check = work.await;
            let _ = this.update(cx, |this, cx| this.github_checked(generation, check, cx));
        })
        .detach();
    }

    pub(super) fn github_checked(
        &mut self,
        generation: u64,
        check: GithubCheck,
        cx: &mut Context<Self>,
    ) {
        let Some(setup) = &mut self.review_setup else {
            return;
        };
        if setup.generation != generation {
            return;
        }
        let first = setup.check.is_none();
        setup.repos = check.repos.clone();
        let stale = setup
            .input
            .repo
            .as_ref()
            .is_some_and(|repo| !setup.repos.contains(repo));
        if first || stale {
            setup.input.repo = default_repo(&setup.repos);
        }
        setup.check = Some(check);
        refresh_readiness(setup);
        cx.notify();
    }

    /// The Model phrase's pick. Results falls back to Gespan when the new
    /// harness can't enforce the one chosen.
    pub(super) fn set_review_model(
        &mut self,
        kind: Kind,
        model: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(setup) = &mut self.review_setup else {
            return;
        };
        setup.input.harness = kind.id().into();
        setup.input.model = model.unwrap_or_default();
        if !results_for(&setup.input.harness).contains(&setup.input.results) {
            setup.input.results = ReviewResults::Gespan;
        }
        cx.notify();
    }

    pub(super) fn apply_setup_choice(&mut self, field: Choice, value: String) {
        let Some(setup) = &mut self.review_setup else {
            return;
        };
        match field {
            Choice::SetupRepo => {
                setup.input.repo = (!value.is_empty()).then_some(value);
                refresh_readiness(setup);
            }
            Choice::SetupWhich => {
                if let Some((which, ..)) = WHICH.iter().find(|(_, id, _)| *id == value) {
                    setup.input.which = *which;
                }
            }
            Choice::SetupResults => {
                if let Some((results, ..)) = RESULTS.iter().find(|(_, id, _)| *id == value)
                    && results_for(&setup.input.harness).contains(results)
                {
                    setup.input.results = *results;
                }
            }
            _ => {}
        }
        self.choice = None;
    }

    fn setup_options(&self, field: &Choice) -> Vec<(String, String)> {
        let Some(setup) = &self.review_setup else {
            return Vec::new();
        };
        match field {
            Choice::SetupRepo if setup.repos.is_empty() => Vec::new(),
            Choice::SetupRepo => std::iter::once((String::new(), ANY_REPO.to_string()))
                .chain(setup.repos.iter().map(|repo| (repo.clone(), repo.clone())))
                .collect(),
            Choice::SetupWhich => WHICH
                .iter()
                .map(|(_, id, label)| (id.to_string(), label.to_string()))
                .collect(),
            Choice::SetupResults => RESULTS
                .iter()
                .filter(|(results, ..)| results_for(&setup.input.harness).contains(results))
                .map(|(_, id, label)| (id.to_string(), label.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Saves and enables the automation the sentence describes, then shows
    /// it selected, as saving from the editor does.
    pub(super) fn turn_on_review(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = &self.review_setup else {
            return;
        };
        if !can_turn_on(setup.readiness) {
            return;
        }
        let draft = review_draft(&setup.input);
        self.upsert(draft, cx);
    }

    /// The full editor on the sheet's draft, unsaved.
    fn customize_review(&mut self, cx: &mut Context<Self>) {
        let Some(setup) = self.review_setup.take() else {
            return;
        };
        self.selected = None;
        self.runs.clear();
        self.set_editor(review_draft(&setup.input), false, cx);
    }

    fn repo_label(&self) -> String {
        let Some(setup) = &self.review_setup else {
            return String::new();
        };
        match (&setup.input.repo, setup.readiness) {
            (Some(repo), _) => repo.clone(),
            (None, Readiness::NoGithubRemote) => CHOOSE_REPO.into(),
            (None, _) => ANY_REPO.into(),
        }
    }

    /// The sentence as it reads on screen.
    pub(super) fn review_sentence(&self) -> String {
        let Some(setup) = &self.review_setup else {
            return String::new();
        };
        format!(
            "When a pull request in {} {}, Gespan reviews it with {}, without changing any files, and {}.",
            self.repo_label(),
            which_label(setup.input.which),
            model_label(&setup.input),
            results_label(setup.input.results),
        )
    }

    pub(super) fn readiness_message(&self) -> &'static str {
        match self.review_setup.as_ref().map(|setup| setup.readiness) {
            None => "",
            Some(Readiness::Checking) => "Checking GitHub…",
            Some(Readiness::NotInstalled) => "GitHub CLI isn't installed.",
            Some(Readiness::SignedOut) => "Sign in to GitHub to continue.",
            Some(Readiness::Connected) => "GitHub connected ✓",
            Some(Readiness::NoGithubRemote) => {
                "This project isn't on GitHub. Use Customize… to pick repositories."
            }
            Some(Readiness::Remote) => "Quick setup isn't available for remote projects yet.",
        }
    }

    /// One inline dropdown of the sentence, read out as the whole choice.
    fn phrase(
        &self,
        id: &'static str,
        name: &'static str,
        label: String,
        field: Option<Choice>,
        suffix: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let anchor = AnchorSlot::default();
        let captured = anchor.clone();
        let phrase = div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Button)
            .aria_label(format!("{name}: {label}"))
            .relative()
            .h(px(28.))
            .px(px(8.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(content(1.))
            .bg(crate::glass::fill_strong())
            .border_1()
            .border_color(crate::glass::edge())
            .hover(|s| s.bg(content(0.12)))
            .when(field.is_none(), |d| d.child(self.model_anchor.probe()))
            .when(field.is_some(), |d| d.child(anchor.probe()))
            .child(label)
            .child(icon("chevron-down", 12.).text_color(content(0.45)))
            .on_click(cx.listener(move |this, _, window, cx| match &field {
                None => this.choose_model(window, cx),
                Some(field) => {
                    let options = this.setup_options(field);
                    if options.is_empty() {
                        return;
                    }
                    this.choice = Some(ChoiceMenu {
                        field: field.clone(),
                        options,
                        anchor: captured.clone(),
                    });
                    this.models = None;
                    cx.notify();
                }
            }));
        // Punctuation sits against its phrase, never wrapped apart from it.
        div()
            .flex()
            .items_center()
            .child(phrase)
            .when(!suffix.is_empty(), |d| d.child(suffix))
            .into_any_element()
    }

    pub(super) fn render_review_setup(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(setup) = &self.review_setup else {
            return div().into_any_element();
        };
        let words = |text: &str| {
            text.split_whitespace()
                .map(|word| div().child(word.to_string()).into_any_element())
                .collect::<Vec<_>>()
        };
        let mut sentence = div()
            .id("review-setup-sentence")
            .debug_selector(|| "review-setup-sentence".into())
            .role(Role::Group)
            .aria_label(self.review_sentence())
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(5.))
            .gap_y(px(6.))
            .text_size(px(16.))
            .line_height(px(26.))
            .text_color(content(0.75));
        // With no repositories there is nothing to choose, so the phrase is
        // plain text, not a dropdown that opens onto nothing.
        let repo = if setup.repos.is_empty() {
            div()
                .id("review-setup-repo-text")
                .debug_selector(|| "review-setup-repo-text".into())
                .px(px(4.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(content(0.6))
                .child(self.repo_label())
                .into_any_element()
        } else {
            self.phrase(
                "review-setup-repo",
                "Repository",
                self.repo_label(),
                Some(Choice::SetupRepo),
                "",
                cx,
            )
        };
        sentence = sentence
            .children(words("When a pull request in"))
            .child(repo)
            .child(self.phrase(
                "review-setup-which",
                "Which pull requests",
                which_label(setup.input.which).into(),
                Some(Choice::SetupWhich),
                ",",
                cx,
            ))
            .children(words("Gespan reviews it with"))
            .child(self.phrase(
                "review-setup-model",
                "Model",
                model_label(&setup.input),
                None,
                ",",
                cx,
            ))
            .children(words("without changing any files, and"))
            .child(self.phrase(
                "review-setup-results",
                "Results",
                results_label(setup.input.results).into(),
                Some(Choice::SetupResults),
                ".",
                cx,
            ));
        let readiness = setup.readiness;
        let ready = can_turn_on(readiness);
        let cwd = setup.input.cwd.clone();
        let mut line = div()
            .id("review-setup-readiness")
            .debug_selector(|| "review-setup-readiness".into())
            .role(Role::Status)
            .aria_label(self.readiness_message())
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .text_color(if ready { content(0.75) } else { content(0.6) })
            .child(self.readiness_message());
        let small = |id: &'static str, label: &'static str| {
            button(id, label)
                .debug_selector(move || id.into())
                .h(px(24.))
                .px(px(10.))
        };
        if readiness == Readiness::NotInstalled {
            line = line.child(
                small("review-setup-get-gh", "Get GitHub CLI")
                    .on_click(|_, _, cx| cx.open_url(GITHUB_CLI_URL)),
            );
        }
        if readiness == Readiness::SignedOut {
            line = line.child(small("review-setup-sign-in", "Sign in").on_click(
                cx.listener(move |_, _, _, cx| cx.emit(Event::SignInGithub { cwd: cwd.clone() })),
            ));
        }
        if matches!(readiness, Readiness::NotInstalled | Readiness::SignedOut) {
            line = line.child(
                small("review-setup-check-again", "Check again")
                    .on_click(cx.listener(|this, _, _, cx| this.recheck_github(cx))),
            );
        }
        let disabled = !ready || self.saving;
        let actions = div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(
                primary_button(
                    "review-setup-turn-on",
                    if self.saving {
                        "Turning on…"
                    } else {
                        "Turn on"
                    },
                )
                .debug_selector(|| "review-setup-turn-on".into())
                .when(disabled, |d| d.opacity(0.4))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !disabled {
                        this.turn_on_review(cx);
                    }
                })),
            )
            .child(
                button("review-setup-customize", "Customize…")
                    .debug_selector(|| "review-setup-customize".into())
                    .on_click(cx.listener(|this, _, _, cx| this.customize_review(cx))),
            )
            .child(
                ghost_button("review-setup-cancel", "Cancel")
                    .debug_selector(|| "review-setup-cancel".into())
                    .on_click(cx.listener(|this, _, _, cx| this.show_gallery(cx))),
            );
        div()
            .id("review-setup-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .id("review-setup")
                    .debug_selector(|| "review-setup".into())
                    .role(Role::Dialog)
                    .aria_label("Review pull requests")
                    .w_full()
                    .max_w(px(680.))
                    .mx_auto()
                    .px(px(36.))
                    .pt(px(26.))
                    .pb(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(layout::tile(
                                36.,
                                layout::icon_glyph("git-pull-request", 16.),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(
                                        crate::glass::eyebrow("New automation")
                                            .text_color(content(0.55)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(22.))
                                            .line_height(px(28.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child("Review pull requests"),
                                    ),
                            ),
                    )
                    // The sentence and what it means sit on one glass card.
                    .child(
                        crate::glass::card()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .child(sentence)
                            .children(post_warning(setup.input.results).map(|text| {
                                div()
                                    .id("review-setup-post-warning")
                                    .debug_selector(|| "review-setup-post-warning".into())
                                    .role(Role::Note)
                                    .aria_label(text)
                                    .w_full()
                                    .text_size(px(13.))
                                    .line_height(px(18.))
                                    .text_color(content(0.6))
                                    .child(text)
                            }))
                            .child(
                                line.pt(px(12.))
                                    .border_t_1()
                                    .border_color(crate::glass::hairline()),
                            ),
                    )
                    .child(actions),
            )
            .into_any_element()
    }
}

fn refresh_readiness(setup: &mut ReviewSetup) {
    if let Some(check) = &setup.check {
        setup.readiness = readiness(
            check.installed,
            check.authenticated,
            &check.repos,
            check.remote,
            setup.input.repo.as_deref(),
        );
    }
}
