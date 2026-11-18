use crate::orchestrator::remote::{
    agent_name, deliver_when_idle, read_from, remote_id, remote_members, start_on, wait_on, Remote,
    Target,
};
use crate::orchestrator::roster::Status;
use std::sync::Mutex;

/// A herdr that records what it is asked and answers from a script.
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<String>>,
    status: Mutex<Vec<&'static str>>,
    refuse: Option<&'static str>,
    /// Who each `agent_in` finds in the pane, in turn; "fixer" when empty,
    /// `None` for no agent at all.
    occupants: Mutex<Vec<Option<&'static str>>>,
}

fn fixer() -> Target {
    Target {
        pane: "w1:p2".into(),
        name: Some("fixer".into()),
    }
}

impl Fake {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn log(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl Remote for Fake {
    fn split_pane(&self, cwd: &str) -> Result<String, String> {
        self.log(format!("split {cwd}"));
        Ok("w1:p2".into())
    }
    fn create_worktree(&self, cwd: &str, base: Option<&str>) -> Result<(String, String), String> {
        self.log(format!("worktree {cwd} {}", base.unwrap_or("-")));
        Ok((
            "/home/dev/.herdr/worktrees/app/otter".into(),
            "w2:p1".into(),
        ))
    }
    fn start_agent(&self, name: &str, kind: &str, pane: &str) -> Result<(), String> {
        self.log(format!("start {name} {kind} {pane}"));
        Ok(())
    }
    fn prompt(&self, target: &str, text: &str) -> Result<(), String> {
        if let Some(code) = self.refuse {
            return Err(code.into());
        }
        self.log(format!("prompt {target} {text}"));
        Ok(())
    }
    fn wait_settled(&self, target: &str, _timeout_ms: u64) -> Result<String, String> {
        self.log(format!("wait {target}"));
        let mut statuses = self.status.lock().unwrap();
        Ok(if statuses.is_empty() {
            "idle".into()
        } else {
            statuses.remove(0).into()
        })
    }
    fn read(&self, target: &str) -> Result<String, String> {
        self.log(format!("read {target}"));
        Ok("All green.".into())
    }
    fn agent_in(&self, pane: &str) -> Result<Option<String>, String> {
        self.log(format!("who {pane}"));
        let mut occupants = self.occupants.lock().unwrap();
        Ok(if occupants.is_empty() {
            Some("fixer".into())
        } else {
            occupants.remove(0).map(str::to_owned)
        })
    }
}

#[test]
fn a_remote_start_in_the_checkout_splits_a_pane_starts_and_prompts() {
    let fake = Fake::default();
    let started = start_on(
        &fake,
        "claude",
        "fix-build",
        "/home/dev/app",
        false,
        None,
        "Fix the build",
    )
    .unwrap();
    assert_eq!(started.pane_id, "w1:p2");
    assert_eq!(started.cwd, "/home/dev/app");
    assert_eq!(
        fake.calls(),
        [
            "split /home/dev/app",
            "start fix-build claude w1:p2",
            "prompt fix-build Fix the build",
        ]
    );
}

#[test]
fn a_remote_start_in_a_worktree_runs_in_its_new_pane() {
    let fake = Fake::default();
    let started = start_on(
        &fake,
        "codex",
        "notes",
        "/home/dev/app",
        true,
        Some("main"),
        "Write notes",
    )
    .unwrap();
    assert_eq!(started.cwd, "/home/dev/.herdr/worktrees/app/otter");
    assert_eq!(
        fake.calls(),
        [
            "worktree /home/dev/app main",
            "start notes codex w2:p1",
            "prompt notes Write notes",
        ]
    );
}

#[test]
fn a_message_waits_for_the_agent_to_settle_and_never_interrupts() {
    let fake = Fake::default();
    deliver_when_idle(&fake, &fixer(), "Rebase on main", 60_000).unwrap();
    assert_eq!(
        fake.calls(),
        [
            "who w1:p2",
            "wait w1:p2",
            "who w1:p2",
            "prompt w1:p2 Rebase on main"
        ],
        "checked before the wait and again before typing"
    );
}

#[test]
fn a_message_to_an_agent_waiting_on_its_user_is_not_sent() {
    let fake = Fake {
        status: Mutex::new(vec!["blocked"]),
        ..Default::default()
    };
    let err = deliver_when_idle(&fake, &fixer(), "go", 60_000).unwrap_err();
    assert!(err.contains("waiting on its user"), "{err}");
    assert!(
        !fake.calls().iter().any(|c| c.starts_with("prompt")),
        "nothing typed into its dialog"
    );
}

#[test]
fn a_refused_prompt_is_reported() {
    let fake = Fake {
        refuse: Some("agent_blocked"),
        ..Default::default()
    };
    let err = deliver_when_idle(&fake, &fixer(), "go", 60_000).unwrap_err();
    assert!(err.contains("agent_blocked"), "{err}");
}

#[test]
fn reading_returns_the_agents_recent_output() {
    let fake = Fake::default();
    assert_eq!(read_from(&fake, &fixer()).unwrap(), "All green.");
    assert_eq!(fake.calls(), ["who w1:p2", "read w1:p2"]);
}

#[test]
fn nothing_reaches_a_pane_whose_agent_was_replaced() {
    let fake = Fake {
        occupants: Mutex::new(vec![Some("intruder"), Some("intruder"), Some("intruder")]),
        ..Default::default()
    };
    for err in [
        deliver_when_idle(&fake, &fixer(), "go", 60_000).unwrap_err(),
        read_from(&fake, &fixer()).unwrap_err(),
        wait_on(&fake, &fixer(), 1_000).unwrap_err(),
    ] {
        assert!(err.contains("another agent"), "{err}");
    }
    assert_eq!(
        fake.calls(),
        ["who w1:p2", "who w1:p2", "who w1:p2"],
        "nothing else asked"
    );
}

#[test]
fn nothing_reaches_a_pane_whose_agent_exited() {
    let fake = Fake {
        occupants: Mutex::new(vec![None]),
        ..Default::default()
    };
    let err = read_from(&fake, &fixer()).unwrap_err();
    assert!(err.contains("no longer runs"), "{err}");
}

#[test]
fn an_agent_replaced_while_the_message_waited_gets_nothing() {
    let fake = Fake {
        occupants: Mutex::new(vec![Some("fixer"), Some("intruder")]),
        ..Default::default()
    };
    let err = deliver_when_idle(&fake, &fixer(), "go", 60_000).unwrap_err();
    assert!(err.contains("another agent"), "{err}");
    assert!(
        !fake.calls().iter().any(|c| c.starts_with("prompt")),
        "{:?}",
        fake.calls()
    );
}

#[test]
fn an_agent_listed_without_a_name_needs_only_to_still_be_there() {
    let fake = Fake {
        occupants: Mutex::new(vec![Some("renamed")]),
        ..Default::default()
    };
    let anyone = Target {
        pane: "w1:p2".into(),
        name: None,
    };
    assert_eq!(wait_on(&fake, &anyone, 1_000).unwrap(), "idle");
}

#[test]
fn agent_names_fit_herdrs_rules() {
    for (title, want) in [
        ("Fix the build", "fix-the-build"),
        ("  Résumé: v2!! ", "r-sum-v2"),
        ("123 go", "a-123-go"),
        ("", "gespan"),
    ] {
        assert_eq!(agent_name(title), want, "{title:?}");
    }
    let long = agent_name(&"word ".repeat(20));
    assert!(long.len() <= 32, "{long}");
    let valid = |name: &str| {
        name.starts_with(|c: char| c.is_ascii_lowercase())
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    };
    assert!(valid(&long));
}

#[test]
fn a_remote_agents_id_is_stable_negative_and_distinct() {
    let a = remote_id("m1", "w1:p1");
    assert_eq!(a, remote_id("m1", "w1:p1"));
    assert!(a < 0, "never a local session's id");
    assert_ne!(a, remote_id("m2", "w1:p1"));
    assert_ne!(a, remote_id("m1", "w1:p2"));
}

#[test]
fn remote_agents_in_a_folder_of_the_same_name_join_the_roster() {
    use herdr_link::{Agent, AgentStatus};
    let agent = |pane: &str, cwd: &str, status| Agent {
        terminal_id: format!("t-{pane}"),
        pane_id: pane.into(),
        workspace_id: "w1".into(),
        tab_id: "w1:t1".into(),
        name: Some("fixer".into()),
        agent: Some("claude".into()),
        title: Some("fix the build".into()),
        agent_status: status,
        completion_seq: None,
        cwd: Some(cwd.into()),
    };
    let agents = vec![
        agent("w1:p1", "/home/dev/gespan", AgentStatus::Working),
        agent(
            "w1:p2",
            "/home/dev/gespan/crates/core",
            AgentStatus::Blocked,
        ),
        agent("w1:p3", "/home/dev/other", AgentStatus::Idle),
    ];
    let members = remote_members("m1", "devbox", &agents, "gespan", "/local/gespan/.git");
    assert_eq!(
        members.len(),
        2,
        "a checkout named like this repository, or a folder in it"
    );
    assert_eq!(members[1].1.status, Status::WaitingOnUser);
    let (id, member, target) = &members[0];
    assert_eq!(*id, remote_id("m1", "w1:p1"));
    assert_eq!(target, "w1:p1");
    assert_eq!(member.status, Status::Working);
    assert_eq!(member.harness, "claude on devbox");
    assert_eq!(member.goal, "fix the build");
    assert!(member.repo.is_some(), "shares the caller's repository key");
    assert!(
        !member.remote,
        "listed like a sibling, not skipped as a remote:// chat"
    );
}

#[test]
fn a_remote_cwd_that_climbs_out_of_the_repository_is_not_a_sibling() {
    use herdr_link::{Agent, AgentStatus};
    let agents = vec![Agent {
        terminal_id: "t".into(),
        pane_id: "w1:p1".into(),
        workspace_id: "w1".into(),
        tab_id: "w1:t1".into(),
        name: None,
        agent: Some("claude".into()),
        title: None,
        agent_status: AgentStatus::Idle,
        completion_seq: None,
        cwd: Some("/home/dev/gespan/../../etc".into()),
    }];
    assert!(remote_members("m1", "devbox", &agents, "gespan", "/r/.git").is_empty());
}
