use crate::{
    settings::{Section, SettingsEvent, SettingsView},
    theme::*,
};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};

impl SettingsView {
    /// Which teams' issues reach the inbox, the Inbox's Teams filter here.
    pub(super) fn linear_teams(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let teams = self.ctx.linear_teams.clone();
        div()
            .debug_selector(|| "settings-linear-teams".into())
            .px_4()
            .py_3()
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Teams"),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .child("Unchecked teams stay out of the inbox."),
            )
            .child(div().mt_2().flex().flex_col().gap(px(2.)).children(
                teams.into_iter().enumerate().map(|(ix, (team, shown))| {
                    let id = team.id.clone();
                    div()
                        .id(SharedString::from(format!("settings-linear-team-{ix}")))
                        .debug_selector(move || format!("settings-linear-team-{ix}"))
                        .role(gpui::Role::CheckBox)
                        .aria_label(team.name.clone())
                        .aria_toggled(if shown {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .h(px(28.))
                        .px_2()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(13.))
                        .cursor_pointer()
                        .hover(|s| s.bg(content(0.05)))
                        .child(div().flex_1().min_w_0().truncate().child(team.name))
                        .when(!team.key.is_empty(), |d| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(content(0.4))
                                    .child(team.key),
                            )
                        })
                        .when(shown, |d| d.child(icon("check", 14.)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SettingsEvent::ToggleLinearTeam(id.clone()))
                        }))
                }),
            ))
            .into_any_element()
    }

    /// Settings › Pull requests: which repositories Pull requests lists, review requests from
    /// other repos, how it opens, Notify me and bot folding. Hosts are
    /// connected in Connections.
    pub(super) fn inbox_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(inbox) = self.ctx.inbox.clone() else {
            return vec![
                div()
                    .debug_selector(|| "settings-git-loading".into())
                    .px_4()
                    .py_3()
                    .text_size(px(12.))
                    .text_color(content(0.55))
                    .child("Loading your repos…")
                    .into_any_element(),
            ];
        };
        let git = inbox.read(cx).git_settings();
        let search_input = inbox.read(cx).repo_search_input();
        let pointer = div()
            .debug_selector(|| "settings-git-connections".into())
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.5))
            .text_color(content(0.62))
            .child("What the Pull requests page lists and how it opens. Hosts are connected in")
            .child(
                div()
                    .id("settings-open-connections")
                    .debug_selector(|| "settings-open-connections".into())
                    .role(gpui::Role::Link)
                    .aria_label("Open Connections")
                    .cursor_pointer()
                    .text_color(accent_color())
                    .child("Connections.")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.select_section(Section::Services, cx);
                        cx.emit(SettingsEvent::Section(Section::Services));
                    })),
            )
            .into_any_element();

        let mut repo_rows: Vec<AnyElement> = Vec::new();
        if git.repos.is_empty() {
            repo_rows.push(
                div()
                    .debug_selector(|| "settings-git-no-repos".into())
                    .px_4()
                    .py_3()
                    .text_size(px(12.))
                    .text_color(content(0.55))
                    .child(if git.loading {
                        "Finding your projects\u{2019} repos…"
                    } else {
                        "Open a project with a GitHub, GitLab or Azure DevOps remote, or add a repo below."
                    })
                    .into_any_element(),
            );
        }
        for (ix, row) in git.repos.iter().enumerate() {
            let host = gespan_core::inbox::provider_label(&row.provider);
            let control = if row.added {
                let (provider, repo) = (row.provider.clone(), row.repo.clone());
                let inbox = inbox.clone();
                crate::glass::button(
                    SharedString::from(format!("settings-git-remove-{ix}")),
                    "Remove",
                    crate::glass::Button::Ghost,
                )
                .debug_selector(move || format!("settings-git-remove-{ix}"))
                .role(gpui::Role::Button)
                .aria_label(format!("Remove {}", row.repo))
                .h(px(28.))
                .on_click(move |_, _, cx| {
                    inbox.update(cx, |inbox, cx| {
                        inbox.remove_added_repo(&provider, &repo, cx)
                    })
                })
                .into_any_element()
            } else {
                let on = !row.hidden;
                let (provider, repo) = (row.provider.clone(), row.repo.clone());
                let inbox = inbox.clone();
                div()
                    .id(SharedString::from(format!("settings-git-repo-{ix}")))
                    .debug_selector(move || format!("settings-git-repo-{ix}"))
                    .role(gpui::Role::Switch)
                    .aria_label(format!("Show {} in Pull requests", row.repo))
                    .aria_toggled(if on {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .relative()
                    .flex_shrink_0()
                    .rounded_full()
                    .h(px(18.))
                    .w(px(30.))
                    .cursor_pointer()
                    .bg(if on { accent_color() } else { content(0.16) })
                    .child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .left(px(if on { 14. } else { 2. }))
                            .size(px(14.))
                            .rounded_full()
                            .bg(gpui::rgb(0xffffff)),
                    )
                    .on_click(move |_, _, cx| {
                        inbox.update(cx, |inbox, cx| {
                            inbox.set_repo_hidden(&provider, &repo, on, cx)
                        })
                    })
                    .into_any_element()
            };
            repo_rows.push(self.row(
                None,
                ix == 0,
                row.repo.clone(),
                Some(format!("{} · {host}", row.source).into()),
                vec![control],
            ));
        }
        let hits = git.hits.iter().enumerate().map(|(ix, hit)| {
            let inbox = inbox.clone();
            let chosen = hit.clone();
            div()
                .id(SharedString::from(format!("settings-git-hit-{ix}")))
                .debug_selector(move || format!("settings-git-hit-{ix}"))
                .role(gpui::Role::Button)
                .aria_label(format!("Add {}", hit.repo))
                .h(px(30.))
                .px_2()
                .rounded_md()
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .hover(|s| s.bg(content(0.05)))
                .text_size(px(12.5))
                .child(icon("plus", 13.).text_color(accent_color()))
                .child(div().flex_1().min_w_0().truncate().child(hit.repo.clone()))
                .child(
                    div()
                        .text_color(content(0.45))
                        .child(gespan_core::inbox::provider_label(&hit.provider)),
                )
                .on_click(move |_, _, cx| {
                    let hit = chosen.clone();
                    inbox.update(cx, |inbox, cx| inbox.add_searched_repo(hit, cx))
                })
                .into_any_element()
        });
        repo_rows.push(
            div()
                .debug_selector(|| "settings-git-add-repo".into())
                .px(px(16.))
                .py(px(12.))
                .border_t_1()
                .border_color(content(0.06))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    crate::glass::field()
                        .child(
                            icon(if git.searching { "loader" } else { "search" }, 14.)
                                .text_color(content(0.55)),
                        )
                        .child(div().flex_1().min_w_0().child(search_input)),
                )
                .children(hits)
                .when_some(git.search_error.clone(), |d, error| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .text_color(crate::glass::Tone::Bad.ink())
                            .child(error),
                    )
                })
                .into_any_element(),
        );
        let repos = self.glass_group(
            Some("git-repos"),
            "Your repos",
            Some("Pull requests lists only these: the remotes of the projects you have open in Gespan, plus any you add. The rest of your org is never fetched.".into()),
            None,
            None,
            repo_rows,
        );

        let code_review = self.code_review_group(&git, &inbox, cx);
        let pref_toggle =
            |this: &Self,
             id: &'static str,
             label: &'static str,
             on: bool,
             cx: &mut Context<Self>,
             change: fn(&mut gespan_core::inbox::queue::Prefs, bool)| {
                let inbox = inbox.clone();
                this.toggle(id, label, on, false, cx, move |_, on, cx| {
                    inbox.update(cx, |inbox, cx| {
                        inbox.change_git_prefs(|p| change(p, on), cx)
                    })
                })
            };
        let other = self.glass_group(
            Some("git-other-repos"),
            "Other repos",
            None,
            None,
            None,
            vec![self.row(
                Some("git-other-repos-row"),
                true,
                "Show review requests from other repos",
                Some("One search for reviews asked of you anywhere you have access. They show folded under the list, with Add repo.".into()),
                vec![pref_toggle(self, "git-other-repos", "Show review requests from other repos", git.other_repos, cx, |p, on| p.other_repos = on)],
            )],
        );
        let opens = {
            use gespan_core::inbox::queue::View;
            let inbox = inbox.clone();
            self.select(
                "git-opens-on",
                "Opens on",
                git.opens_on.id(),
                View::TABS
                    .into_iter()
                    .map(|view| {
                        (
                            view.id().to_string(),
                            SharedString::from(view.label()),
                            None,
                        )
                    })
                    .collect(),
                Some(160.),
                cx,
                move |_, value, cx| {
                    let view = View::TABS
                        .into_iter()
                        .find(|view| view.id() == value)
                        .unwrap_or_default();
                    inbox.update(cx, |inbox, cx| {
                        inbox.change_git_prefs(|p| p.opens_on = view, cx)
                    })
                },
            )
        };
        let opening = self.glass_group(
            Some("git-opening"),
            "Opening Pull requests",
            None,
            None,
            None,
            vec![
                self.row(
                    Some("git-opens-on-row"),
                    true,
                    "Opens on",
                    Some("The view Pull requests shows first, 25 per page.".into()),
                    vec![opens],
                ),
                self.row(
                    Some("git-keep-filters-row"),
                    false,
                    "Keep filters for each view",
                    Some("The filters you last used in a view come back after a restart.".into()),
                    vec![pref_toggle(
                        self,
                        "git-keep-filters",
                        "Keep filters for each view",
                        git.keep_filters,
                        cx,
                        |p, on| p.keep_filters = on,
                    )],
                ),
                self.row(
                    Some("git-notify-row"),
                    false,
                    "Notify me",
                    Some("When my review is requested, or checks fail on my pull request.".into()),
                    vec![pref_toggle(
                        self,
                        "git-notify",
                        "Notify me",
                        git.notify,
                        cx,
                        |p, on| p.notify = on,
                    )],
                ),
                self.row(
                    Some("git-fold-bots-row"),
                    false,
                    "Fold bot pull requests",
                    Some("dependabot and renovate go to the Bots view.".into()),
                    vec![pref_toggle(
                        self,
                        "git-fold-bots",
                        "Fold bot pull requests",
                        git.fold_bots,
                        cx,
                        |p, on| p.fold_bots = on,
                    )],
                ),
            ],
        );
        vec![
            pointer,
            repos,
            code_review,
            other,
            opening,
            self.project_notification_settings(cx),
        ]
    }

    /// Settings › Git › Code review: each GitHub and GitLab repo, the one
    /// folder that reviews it, and its review switch. Turning a repo on makes
    /// its system code review; the rest of its settings live in Automations.
    fn code_review_group(
        &self,
        git: &crate::inbox::GitSettings,
        inbox: &gpui::Entity<crate::inbox::Inbox>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        if git.review_repos.is_empty() {
            rows.push(
                div()
                    .debug_selector(|| "settings-review-no-repos".into())
                    .px_4()
                    .py_3()
                    .text_size(px(12.))
                    .text_color(content(0.55))
                    .child("Open a project with a GitHub or GitLab remote to review its pull requests.")
                    .into_any_element(),
            );
        }
        for (ix, repo) in git.review_repos.iter().enumerate() {
            let review = self.ctx.code_reviews.iter().find(|r| r.repo == repo.repo);
            let on = review.is_some_and(|r| r.enabled);
            let (host, name) = repo
                .repo
                .split_once(':')
                .unwrap_or(("", repo.repo.as_str()));
            let host = gespan_core::inbox::provider_label(host);
            let description: SharedString = match (&repo.folder, repo.folders.len()) {
                (_, 0) => format!("{host} · needs a project folder to review in").into(),
                (Some(folder), _) => format!("{host} · reviews in {}", folder_name(folder)).into(),
                (None, n) => {
                    format!("{host} · {n} folders point here: pick the one that reviews").into()
                }
            };
            let mut controls: Vec<AnyElement> = Vec::new();
            // Two clones: one chip per folder, the picked one lit.
            if repo.folders.len() > 1 {
                let chips = repo.folders.iter().enumerate().map(|(fx, folder)| {
                    let picked = repo.folder.as_deref() == Some(folder.as_str());
                    let (key, folder) = (repo.repo.clone(), folder.clone());
                    let inbox = inbox.clone();
                    crate::glass::chip(
                        SharedString::from(format!("settings-review-folder-{ix}-{fx}")),
                        folder_name(&folder),
                        picked,
                    )
                    .debug_selector(move || format!("settings-review-folder-{ix}-{fx}"))
                    .role(gpui::Role::Button)
                    .aria_label(format!("Review {name} in {folder}"))
                    .aria_selected(picked)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        inbox.update(cx, |inbox, cx| {
                            inbox.pick_review_folder(key.clone(), folder.clone(), cx)
                        });
                        // A review that runs moves with the pick.
                        if on {
                            cx.emit(SettingsEvent::CodeReview {
                                repo: key.clone(),
                                folder: folder.clone(),
                                on: true,
                            });
                        }
                    }))
                    .into_any_element()
                });
                controls.push(div().flex().gap(px(6.)).children(chips).into_any_element());
            }
            if let Some(review) = review {
                let id = review.id.clone();
                controls.push(
                    crate::glass::button(
                        SharedString::from(format!("settings-review-open-{ix}")),
                        "Open",
                        crate::glass::Button::Ghost,
                    )
                    .debug_selector(move || format!("settings-review-open-{ix}"))
                    .role(gpui::Role::Button)
                    .aria_label(format!("Open the code review of {name} in Automations"))
                    .h(px(28.))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(SettingsEvent::OpenAutomation(id.clone()))
                    }))
                    .into_any_element(),
                );
            }
            // A review that exists keeps its switch while the folder pick is
            // open again (a second clone appeared), so it can still be turned
            // off here. With its project closed the repo has no row; Automations
            // still pauses it.
            if let Some(folder) = repo
                .folder
                .clone()
                .or_else(|| review.map(|review| review.cwd.clone()))
            {
                let key = repo.repo.clone();
                controls.push(
                    div()
                        .id(SharedString::from(format!("settings-review-{ix}")))
                        .debug_selector(move || format!("settings-review-{ix}"))
                        .role(gpui::Role::Switch)
                        .aria_label(format!("Review pull requests of {name}"))
                        .aria_toggled(if on {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .relative()
                        .flex_shrink_0()
                        .rounded_full()
                        .h(px(18.))
                        .w(px(30.))
                        .cursor_pointer()
                        .bg(if on { accent_color() } else { content(0.16) })
                        .child(
                            div()
                                .absolute()
                                .top(px(2.))
                                .left(px(if on { 14. } else { 2. }))
                                .size(px(14.))
                                .rounded_full()
                                .bg(gpui::rgb(0xffffff)),
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SettingsEvent::CodeReview {
                                repo: key.clone(),
                                folder: folder.clone(),
                                on: !on,
                            })
                        }))
                        .into_any_element(),
                );
            }
            rows.push(self.row(None, ix == 0, name.to_string(), Some(description), controls));
        }
        self.glass_group(
            Some("git-code-review"),
            "Code review",
            Some("Gespan reviews a repo in the one project folder its remote points at. Turning a repo on makes its review: a system automation you can tune in Automations but not delete.".into()),
            None,
            None,
            rows,
        )
    }
}

/// A folder's last part, as projects are named.
fn folder_name(path: &str) -> String {
    std::path::Path::new(path.trim_end_matches('/'))
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.into())
}
