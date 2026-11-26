//! The Git page on screen: views and their counts, bots folded, 25 to a
//! page, arrivals held behind the banner, the Filters dropdown kept per view
//! and across restarts, saved views, review requests from other repos, the
//! empty state, the review bar, Polish's card, notifications and Settings ›
//! Git's repositories. No host is reached: the snapshot is the test's.
use super::*;
use gespan_core::inbox::{CheckSummary, RepoHit, Review, Status, Verdict, queue};
use gpui::{
    Bounds, Modifiers, TestAppContext, VisualTestContext, WindowBounds, WindowHandle,
    WindowOptions, point, px, size,
};

fn store() -> Database {
    let store = gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap();
    std::sync::Arc::new(std::sync::Mutex::new(store))
}

fn pr(repo: &str, number: i64, author: &str) -> Item {
    Item {
        provider: "github".into(),
        kind: "pr".into(),
        repo: repo.into(),
        number,
        state: "open".into(),
        title: format!("Change {number}"),
        author: author.into(),
        viewer: "me".into(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        updated_at: format!(
            "2026-11-26T{:02}:{:02}:00Z",
            10 + number / 60 % 10,
            number % 60
        ),
        ..Default::default()
    }
}
fn asked(repo: &str, number: i64) -> Item {
    Item {
        requested_reviewers: vec!["me".into()],
        ..pr(repo, number, "devon")
    }
}

fn open(
    store: Database,
    cx: &mut TestAppContext,
    setup: impl FnOnce(&mut Inbox) + 'static,
) -> WindowHandle<Inbox> {
    cx.update(|cx| cx.set_global(crate::settings::Preferences::default()));
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1300.), px(860.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let mut inbox =
                        Inbox::new_inner(store, vec![], "/missing/git".into(), false, cx);
                    inbox.offline = Offline::Instant;
                    inbox.snapshot.connections.insert(
                        "github".into(),
                        Status {
                            connected: true,
                            ..Default::default()
                        },
                    );
                    setup(&mut inbox);
                    // A refresh (after a review, say) finds the same items.
                    inbox.fetched = inbox.snapshot.clone();
                    inbox
                })
            },
        )
        .unwrap()
    });
    cx.run_until_parked();
    window
}
fn visual(window: WindowHandle<Inbox>, cx: &mut TestAppContext) -> VisualTestContext {
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual
}
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}
fn click(visual: &mut VisualTestContext, selector: &str) {
    let selector = leak(selector.to_string());
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    visual.simulate_click(bounds.center(), Modifiers::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
}
fn shown(visual: &mut VisualTestContext, selector: &str) -> bool {
    visual.debug_bounds(leak(selector.to_string())).is_some()
}
fn row(item: &Item) -> String {
    format!("inbox-item-{}", item.key())
}
fn visible(window: WindowHandle<Inbox>, visual: &mut VisualTestContext) -> Vec<i64> {
    let mut numbers: Vec<i64> = window
        .update(visual, |inbox, _, cx| {
            inbox
                .visible_items(cx)
                .iter()
                .map(|item| item.number)
                .collect()
        })
        .unwrap();
    numbers.sort();
    numbers
}

#[gpui::test]
fn the_tabs_count_their_views_and_bots_fold_behind_show_bots(cx: &mut TestAppContext) {
    let items = vec![
        asked("acme/web", 1),
        asked("acme/web", 2),
        pr("acme/web", 3, "me"),
        pr("acme/web", 4, "lena"),
        pr("acme/web", 5, "app/dependabot"),
        pr("acme/web", 6, "renovate[bot]"),
    ];
    let window = open(store(), cx, move |inbox| inbox.snapshot.items = items);
    let mut visual = visual(window, cx);
    // Needs me opens first: what involves you, never the bots.
    assert_eq!(visible(window, &mut visual), vec![1, 2, 3]);
    let counts = window
        .update(&mut visual, |inbox, _, _| {
            queue::View::TABS.map(|view| inbox.state.prefs.count(&inbox.snapshot.items, view))
        })
        .unwrap();
    assert_eq!(counts, [2, 1, 4], "Needs me counts only what waits on you");
    click(&mut visual, "git-view-all");
    assert_eq!(visible(window, &mut visual), vec![1, 2, 3, 4]);
    assert!(shown(&mut visual, "git-list-foot"));
    click(&mut visual, "git-show-bots");
    assert_eq!(visible(window, &mut visual), vec![5, 6]);
    assert!(
        shown(&mut visual, "git-current-view"),
        "the Bots view is named"
    );
    assert!(!shown(&mut visual, "git-show-bots"));
    click(&mut visual, "git-view-mine");
    assert_eq!(visible(window, &mut visual), vec![3]);
}

#[gpui::test]
fn the_list_pages_25_at_a_time(cx: &mut TestAppContext) {
    let items: Vec<Item> = (1..=60).map(|n| asked("acme/web", n)).collect();
    let first = items.clone();
    let window = open(store(), cx, move |inbox| inbox.snapshot.items = items);
    let mut visual = visual(window, cx);
    let on_screen = |visual: &mut VisualTestContext| {
        first
            .iter()
            .filter(|item| shown(visual, &row(item)))
            .count()
    };
    assert_eq!(on_screen(&mut visual), 25);
    assert!(shown(&mut visual, "git-page-range"));
    assert!(shown(&mut visual, "git-page-2"), "three pages");
    assert!(!shown(&mut visual, "git-page-3"));
    click(&mut visual, "git-page-next");
    assert_eq!(
        window
            .update(&mut visual, |inbox, _, _| inbox.page)
            .unwrap(),
        1
    );
    assert_eq!(on_screen(&mut visual), 25);
    click(&mut visual, "git-page-2");
    assert_eq!(on_screen(&mut visual), 10, "the last page holds the rest");
    // A new search starts over on the first page.
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox
                .search
                .update(cx, |input, cx| input.set_text("Change".into(), cx))
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        window
            .update(&mut visual, |inbox, _, _| inbox.page)
            .unwrap(),
        0
    );
}

#[gpui::test]
fn arrivals_wait_behind_the_banner_instead_of_moving_the_list(cx: &mut TestAppContext) {
    let first = vec![asked("acme/web", 1), asked("acme/web", 2)];
    let window = open(store(), cx, move |inbox| inbox.snapshot.items = first);
    let mut visual = visual(window, cx);
    assert!(!shown(&mut visual, "git-new-banner"));
    let newcomer = asked("acme/web", 3);
    let key = row(&newcomer);
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.snapshot.items.insert(0, newcomer);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(shown(&mut visual, "git-new-banner"));
    assert!(!shown(&mut visual, &key), "held back");
    assert_eq!(visible(window, &mut visual), vec![1, 2]);
    click(&mut visual, "git-new-banner");
    assert!(!shown(&mut visual, "git-new-banner"));
    assert!(shown(&mut visual, &key));
}

#[gpui::test]
fn filters_are_kept_per_view_and_after_a_restart(cx: &mut TestAppContext) {
    let db = store();
    let items = || {
        let mut gitlab = asked("acme/platform", 2);
        gitlab.provider = "gitlab".into();
        vec![asked("acme/web", 1), gitlab]
    };
    let first = items();
    let window = open(db.clone(), cx, move |inbox| inbox.snapshot.items = first);
    let mut visual = visual(window, cx);
    click(&mut visual, "git-filters");
    click(&mut visual, "git-filter-host");
    click(&mut visual, "git-filter-host-gitlab");
    assert_eq!(visible(window, &mut visual), vec![2]);
    assert!(shown(&mut visual, "git-filters-clear"));
    // Mine has its own (empty) filters; Needs me keeps GitLab.
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.set_git_view("all".into(), cx)
        })
        .unwrap();
    assert_eq!(visible(window, &mut visual), vec![1, 2]);
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.set_git_view("needs_me".into(), cx)
        })
        .unwrap();
    assert_eq!(visible(window, &mut visual), vec![2]);
    drop(visual);

    let again = items();
    let reopened = open(db, cx, move |inbox| inbox.snapshot.items = again);
    let mut second = VisualTestContext::from_window(reopened.into(), cx);
    second.run_until_parked();
    assert_eq!(
        visible(reopened, &mut second),
        vec![2],
        "the filter survives a restart"
    );
}

#[gpui::test]
fn added_filters_narrow_and_remove_cleanly(cx: &mut TestAppContext) {
    let mut failing = asked("acme/web", 1);
    failing.check_summary = CheckSummary {
        total: 2,
        passed: 1,
        failed: 1,
        ..Default::default()
    };
    let passing = Item {
        check_summary: CheckSummary {
            total: 2,
            passed: 2,
            ..Default::default()
        },
        ..asked("acme/web", 2)
    };
    let window = open(store(), cx, move |inbox| {
        inbox.snapshot.items = vec![failing, passing]
    });
    let mut visual = visual(window, cx);
    click(&mut visual, "git-filters");
    click(&mut visual, "git-add-filter");
    click(&mut visual, "git-add-filter-checks");
    click(&mut visual, "git-filter-checks-failing");
    assert_eq!(visible(window, &mut visual), vec![1]);
    click(&mut visual, "git-filter-remove-checks");
    assert_eq!(visible(window, &mut visual), vec![1, 2]);
    assert!(!shown(&mut visual, "git-filter-checks"));
}

#[gpui::test]
fn save_as_view_names_the_filters_and_opens_them(cx: &mut TestAppContext) {
    let mut gitlab = asked("acme/platform", 2);
    gitlab.provider = "gitlab".into();
    let window = open(store(), cx, move |inbox| {
        inbox.snapshot.items = vec![asked("acme/web", 1), gitlab]
    });
    let mut visual = visual(window, cx);
    click(&mut visual, "git-filters");
    click(&mut visual, "git-filter-host");
    click(&mut visual, "git-filter-host-gitlab");
    click(&mut visual, "git-save-as-view");
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.view_name.update(cx, |input, cx| {
                input.set_text("Platform reviews".into(), cx)
            })
        })
        .unwrap();
    click(&mut visual, "git-view-save");
    let (view, saved) = window
        .update(&mut visual, |inbox, _, _| {
            (
                inbox.state.prefs.view.clone(),
                inbox.state.prefs.saved.clone(),
            )
        })
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "Platform reviews");
    assert_eq!(view, format!("saved:{}", saved[0].id));
    assert!(shown(&mut visual, "git-current-view"));
    // Needs me itself is untouched by changes in the saved view.
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.change_filters(|f| f.hosts.clear(), cx);
            inbox.set_git_view("needs_me".into(), cx);
        })
        .unwrap();
    assert_eq!(visible(window, &mut visual), vec![2]);
}

#[gpui::test]
fn review_requests_from_other_repos_fold_under_the_list_and_add_their_repo(
    cx: &mut TestAppContext,
) {
    let elsewhere = Item {
        other_repo: true,
        ..asked("acme/docs", 9)
    };
    let key = row(&elsewhere);
    let window = open(store(), cx, move |inbox| {
        inbox.snapshot.items = vec![asked("acme/web", 1), elsewhere]
    });
    let mut visual = visual(window, cx);
    assert!(shown(&mut visual, "git-other-repos"));
    assert!(!shown(&mut visual, &key), "folded");
    assert_eq!(
        visible(window, &mut visual),
        vec![1],
        "not part of the list"
    );
    click(&mut visual, "git-other-repos-show");
    assert!(shown(&mut visual, &key));
    click(&mut visual, "git-other-repos-add");
    click(&mut visual, "git-add-repo-0");
    let added = window
        .update(&mut visual, |inbox, _, _| {
            inbox.state.prefs.added_repos.clone()
        })
        .unwrap();
    assert_eq!(
        added,
        vec![queue::AddedRepo {
            provider: "github".into(),
            repo: "acme/docs".into()
        }]
    );
    // Switched off in Settings › Git: the fold goes.
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.change_git_prefs(|p| p.other_repos = false, cx)
        })
        .unwrap();
    visual.refresh().unwrap();
    assert!(!shown(&mut visual, "git-other-repos"));
}

#[gpui::test]
fn nothing_needs_you_shows_its_way_out_and_the_connected_hosts(cx: &mut TestAppContext) {
    let window = open(store(), cx, |inbox| {
        inbox.snapshot.items = vec![pr("acme/web", 1, "me")];
        inbox.snapshot.repos = vec![model::ProjectRepo {
            provider: "github".into(),
            repo: "acme/web".into(),
            project_path: "/missing/git".into(),
        }];
    });
    let mut visual = visual(window, cx);
    // Your own pull request waits on its reviewers, not on you, but it is
    // still in Needs me: select nothing to see the empty side.
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.snapshot.items.clear();
            inbox.deselect(cx);
        })
        .unwrap();
    visual.refresh().unwrap();
    assert!(shown(&mut visual, "inbox-empty-list"));
    assert!(shown(&mut visual, "git-empty-mine"));
    assert!(shown(&mut visual, "git-hosts"));
    assert!(
        shown(&mut visual, "git-connect-gitlab"),
        "an unconnected host offers Connect"
    );
    assert!(!shown(&mut visual, "git-connect-github"));
    click(&mut visual, "git-empty-all");
    assert_eq!(
        window
            .update(&mut visual, |inbox, _, _| inbox.state.prefs.view.clone())
            .unwrap(),
        "all"
    );
}

#[gpui::test]
fn the_review_bar_submits_each_verdict_and_needs_words_for_all_but_approve(
    cx: &mut TestAppContext,
) {
    let theirs = asked("acme/web", 1);
    let mine = pr("acme/web", 2, "me");
    let (theirs_key, mine_key) = (theirs.key(), mine.key());
    let window = open(store(), cx, move |inbox| {
        inbox.snapshot.items = vec![theirs, mine];
    });
    let mut visual = visual(window, cx);
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.select(theirs_key.clone(), cx);
            inbox.set_tab(Tab::Code, cx);
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(shown(&mut visual, "git-review-bar"));
    click(&mut visual, "git-review-request-changes");
    let error = window
        .update(&mut visual, |inbox, _, _| inbox.review.error.clone())
        .unwrap();
    assert_eq!(
        error.as_deref(),
        Some("Write a comment to request changes.")
    );
    click(&mut visual, "git-review-approve");
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.review_body.update(cx, |input, cx| {
                input.set_text("Please add the index".into(), cx)
            })
        })
        .unwrap();
    click(&mut visual, "git-review-request-changes");
    let submitted = window
        .update(&mut visual, |inbox, _, _| {
            inbox.submitted_reviews.lock().unwrap().clone()
        })
        .unwrap();
    assert_eq!(
        submitted,
        vec![
            (theirs_key.clone(), Verdict::Approve, String::new(), vec![]),
            (
                theirs_key.clone(),
                Verdict::RequestChanges,
                "Please add the index".into(),
                vec![]
            )
        ]
    );
    let (body, notice) = window
        .update(&mut visual, |inbox, _, cx| {
            (
                inbox.review_body.read(cx).content.to_string(),
                inbox.review.notice.clone(),
            )
        })
        .unwrap();
    assert!(body.is_empty(), "a sent review clears its draft");
    assert_eq!(notice.as_deref(), Some("Changes requested."));
    // Your own pull request can be commented on, never approved.
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.select(mine_key.clone(), cx);
            inbox.set_tab(Tab::Code, cx);
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(shown(&mut visual, "git-review-bar"));
    assert!(shown(&mut visual, "git-review-comment"));
    assert!(!shown(&mut visual, "git-review-approve"));
    assert!(!shown(&mut visual, "git-review-request-changes"));
}

#[gpui::test]
fn line_comments_in_files_go_with_the_review_on_their_lines(cx: &mut TestAppContext) {
    use gespan_core::inbox::{LineComment, Side};
    let theirs = asked("acme/web", 1);
    let key = theirs.key();
    let window = open(store(), cx, move |inbox| {
        inbox.snapshot.items = vec![theirs]
    });
    let mut visual = visual(window, cx);
    let comment = |line, side, body: &str| LineComment {
        path: "docs/hackathon.md".into(),
        line,
        side,
        body: body.into(),
    };
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.select(key.clone(), cx);
            inbox.set_tab(Tab::Code, cx);
            // What the diff raises on "Add to review".
            inbox.diff_view.update(cx, |_, cx| {
                cx.emit(crate::diff_view::DiffEvent::LineComment(comment(
                    100,
                    Side::New,
                    "test comment",
                )));
                cx.emit(crate::diff_view::DiffEvent::LineComment(comment(
                    7,
                    Side::Old,
                    "why?",
                )));
                cx.emit(crate::diff_view::DiffEvent::LineComment(comment(
                    9,
                    Side::New,
                    "never mind",
                )));
            });
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(shown(&mut visual, "git-review-drafts"));
    assert!(shown(&mut visual, "git-review-draft-2"));
    click(&mut visual, "git-review-draft-remove-2");
    assert!(!shown(&mut visual, "git-review-draft-2"));
    // Line comments alone make a comment review: no summary needed.
    click(&mut visual, "git-review-comment");
    let submitted = window
        .update(&mut visual, |inbox, _, _| {
            inbox.submitted_reviews.lock().unwrap().clone()
        })
        .unwrap();
    assert_eq!(
        submitted,
        vec![(
            key.clone(),
            Verdict::Comment,
            String::new(),
            vec![
                comment(100, Side::New, "test comment"),
                comment(7, Side::Old, "why?")
            ]
        )]
    );
    assert!(
        !shown(&mut visual, "git-review-drafts"),
        "sent drafts are cleared"
    );
    // Nothing reaches the general discussion's composer.
    let discussion = window
        .update(&mut visual, |inbox, _, cx| {
            inbox.comment.read(cx).content.to_string()
        })
        .unwrap();
    assert!(discussion.is_empty());
}

#[gpui::test]
fn a_polished_comment_is_offered_to_use_or_keep(cx: &mut TestAppContext) {
    let theirs = asked("acme/web", 1);
    let key = theirs.key();
    let window = open(store(), cx, move |inbox| {
        inbox.snapshot.items = vec![theirs]
    });
    let mut visual = visual(window, cx);
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.select(key.clone(), cx);
            inbox.set_tab(Tab::Code, cx);
            inbox.review_body.update(cx, |input, cx| {
                input.set_text("this is wrong, add the index".into(), cx)
            });
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    click(&mut visual, "git-tone-friendly");
    assert_eq!(
        window
            .update(&mut visual, |inbox, _, _| inbox.review.tone)
            .unwrap(),
        model::polish::Tone::Friendly
    );
    let offer = |visual: &mut VisualTestContext| {
        window
            .update(visual, |inbox, _, cx| {
                inbox.review.polished = Some((
                    model::polish::Tone::Friendly,
                    "Good catch, thanks. Could you add an index?".into(),
                ));
                cx.notify();
            })
            .unwrap();
        visual.refresh().unwrap();
    };
    offer(&mut visual);
    assert!(shown(&mut visual, "git-polished"));
    click(&mut visual, "git-polish-keep");
    assert!(!shown(&mut visual, "git-polished"));
    let body = |visual: &mut VisualTestContext| {
        window
            .update(visual, |inbox, _, cx| {
                inbox.review_body.read(cx).content.to_string()
            })
            .unwrap()
    };
    assert_eq!(body(&mut visual), "this is wrong, add the index");
    offer(&mut visual);
    click(&mut visual, "git-polish-use");
    assert_eq!(
        body(&mut visual),
        "Good catch, thanks. Could you add an index?"
    );
    assert!(!shown(&mut visual, "git-polished"));
}

#[gpui::test]
fn your_pull_request_with_failing_checks_opens_on_checks(cx: &mut TestAppContext) {
    let mut failing = pr("acme/web", 1, "me");
    failing.check_summary = CheckSummary {
        total: 3,
        passed: 2,
        failed: 1,
        ..Default::default()
    };
    let other = asked("acme/web", 2);
    let (failing_key, other_key) = (failing.key(), other.key());
    let window = open(store(), cx, move |inbox| {
        inbox.snapshot.items = vec![failing, other]
    });
    let mut visual = visual(window, cx);
    let tab = |visual: &mut VisualTestContext, key: &str| {
        window
            .update(visual, |inbox, _, cx| {
                inbox.select(key.to_string(), cx);
                inbox.tab
            })
            .unwrap()
    };
    assert!(tab(&mut visual, &failing_key) == Tab::Checks);
    assert!(tab(&mut visual, &other_key) == Tab::Summary);
}

#[gpui::test]
fn only_pull_requests_that_start_waiting_on_you_notify(cx: &mut TestAppContext) {
    let window = open(store(), cx, |inbox| {
        inbox.snapshot.items = vec![asked("acme/web", 1)];
        inbox.state.feed.seen.seeded = true;
    });
    let notices = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let sink = notices.clone();
    let inbox = window.root(cx).unwrap();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&inbox, move |_, event: &Event, _| {
            if let Event::ReviewNotice { body, .. } = event {
                sink.borrow_mut().push(body.clone());
            }
        })
    });
    let land = |cx: &mut TestAppContext, items: Vec<Item>| {
        window
            .update(cx, |inbox, _, cx| {
                inbox.snapshot.items = items;
                inbox.notify_review_queue(cx);
            })
            .unwrap();
        cx.run_until_parked();
    };
    land(cx, vec![asked("acme/web", 1)]);
    assert!(notices.borrow().is_empty(), "the first refresh only seeds");
    let mut blocked = pr("acme/web", 2, "me");
    blocked.reviews = vec![Review {
        login: "mira".into(),
        state: "changes_requested".into(),
    }];
    land(
        cx,
        vec![
            asked("acme/web", 1),
            blocked.clone(),
            pr("acme/web", 3, "me"),
        ],
    );
    assert_eq!(
        *notices.borrow(),
        vec!["mira requested changes".to_string()]
    );
    land(cx, vec![asked("acme/web", 1), blocked.clone()]);
    assert_eq!(notices.borrow().len(), 1, "still waiting is not news");
    window
        .update(cx, |inbox, _, cx| {
            inbox.change_git_prefs(|p| p.notify = false, cx)
        })
        .unwrap();
    land(
        cx,
        vec![asked("acme/web", 1), blocked, asked("acme/web", 4)],
    );
    assert_eq!(notices.borrow().len(), 1, "Notify me is off");
}

#[gpui::test]
fn settings_lists_your_repos_hides_them_and_adds_one_found_by_search(cx: &mut TestAppContext) {
    let window = open(store(), cx, |inbox| {
        inbox.snapshot.items = vec![asked("acme/web", 1), asked("acme/api", 2)];
        inbox.snapshot.repos = vec![
            model::ProjectRepo {
                provider: "github".into(),
                repo: "acme/web".into(),
                project_path: "/work/web".into(),
            },
            model::ProjectRepo {
                provider: "github".into(),
                repo: "acme/api".into(),
                project_path: "/work/api".into(),
            },
            model::ProjectRepo {
                provider: "linear".into(),
                repo: "ENG".into(),
                project_path: String::new(),
            },
        ];
        inbox.canned_repo_hits = Some(vec![
            RepoHit {
                provider: "github".into(),
                repo: "acme/web".into(),
                description: String::new(),
            },
            RepoHit {
                provider: "github".into(),
                repo: "acme/website".into(),
                description: String::new(),
            },
        ]);
    });
    let mut visual = visual(window, cx);
    let settings = |visual: &mut VisualTestContext| {
        window
            .update(visual, |inbox, _, _| inbox.git_settings())
            .unwrap()
    };
    let rows = settings(&mut visual).repos;
    assert_eq!(
        rows.iter()
            .map(|r| (r.repo.as_str(), r.source.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("acme/web", "From project web"),
            ("acme/api", "From project api")
        ],
        "trackers are not repos"
    );
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.set_repo_hidden("github", "acme/api", true, cx)
        })
        .unwrap();
    assert_eq!(
        visible(window, &mut visual),
        vec![1],
        "a hidden repo leaves the list"
    );
    assert!(settings(&mut visual).repos[1].hidden);
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox
                .repo_search_input
                .update(cx, |input, cx| input.set_text("acme/web".into(), cx))
        })
        .unwrap();
    visual.run_until_parked();
    let hits = settings(&mut visual).hits;
    assert_eq!(
        hits.iter().map(|h| h.repo.as_str()).collect::<Vec<_>>(),
        vec!["acme/website"],
        "repos you already follow are not offered again"
    );
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.add_searched_repo(hits[0].clone(), cx)
        })
        .unwrap();
    let rows = settings(&mut visual).repos;
    let added = rows.iter().find(|r| r.repo == "acme/website").unwrap();
    assert!(added.added && added.source == "Added by hand");
    window
        .update(&mut visual, |inbox, _, cx| {
            inbox.remove_added_repo("github", "acme/website", cx)
        })
        .unwrap();
    assert!(
        settings(&mut visual)
            .repos
            .iter()
            .all(|r| r.repo != "acme/website")
    );
}
