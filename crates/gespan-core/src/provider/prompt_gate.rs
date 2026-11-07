//! The turn's workspace baseline records while its provider starts. Each
//! transport runs its handshake as usual (initialize, session/new, the tool
//! mount, settings), and the receive loop keeps reading, ticking deadlines
//! and handling Stop; only the step that sends the prompt waits until the
//! baseline is recorded, so nothing can edit the workspace before it.
//!
//! A held step is taken again when the baseline is recorded, rather than its
//! frame being queued, so any acknowledgement deadline starts at the real send.
use super::*;

pub(super) enum HeldPrompt {
    Claude,
    Acp,
    Pi,
    /// Codex's `turn/start`, built from the `thread/start` reply. It has no
    /// acknowledgement deadline of its own.
    Codex(Value),
}

impl Protocol {
    /// The baseline is still recording.
    pub(super) fn baseline_pending(&self) -> bool {
        self.request
            .settings
            .baseline
            .as_ref()
            .is_some_and(|baseline| !baseline.recorded())
    }

    /// Called where a transport would send its prompt: `true` means the step
    /// was held and must not send now.
    pub(super) fn hold_prompt(&mut self, step: HeldPrompt) -> bool {
        if !self.baseline_pending() {
            return false;
        }
        self.held_prompt = Some(step);
        true
    }

    pub(super) fn prompt_held(&self) -> bool {
        self.held_prompt.is_some()
    }

    /// Sends the held prompt once the baseline is recorded.
    pub(super) fn release_prompt(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        if self.baseline_pending() {
            return Ok(());
        }
        match self.held_prompt.take() {
            None => Ok(()),
            Some(HeldPrompt::Claude) => self.claude_prompt(stdin),
            Some(HeldPrompt::Acp) => self.acp_prompt_step(stdin),
            Some(HeldPrompt::Pi) => self.pi_prompt_step(stdin),
            Some(HeldPrompt::Codex(frame)) => write_json(stdin, frame),
        }
    }
}

/// The hold at the protocol level, without processes, so it runs on every
/// platform. The process tests (`baseline_tests`) cover the receive loop.
#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::checkpoint::Baseline;

    /// A baseline that records once the returned sender fires.
    pub(in crate::provider) fn pending() -> (Baseline, mpsc::Sender<()>) {
        let (release, released) = mpsc::channel::<()>();
        let baseline = Baseline::record(move || {
            let _ = released.recv();
        });
        (baseline, release)
    }

    pub(in crate::provider) fn recorded(baseline: &Baseline, release: &mpsc::Sender<()>) {
        release.send(()).unwrap();
        baseline.finish();
    }

    pub(in crate::provider) fn protocol(kind: Kind, baseline: &Baseline) -> Protocol {
        Protocol::new(Request {
            kind,
            binary: "unused".into(),
            cwd: "/project".into(),
            prompt: "the user's message".into(),
            provider_id: None,
            model: None,
            settings: Settings {
                baseline: Some(baseline.clone()),
                ..Default::default()
            },
        })
    }

    /// Every frame written so far.
    pub(in crate::provider) fn sent(out: &[u8]) -> Vec<Value> {
        out.split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect()
    }

    fn methods(out: &[u8]) -> Vec<String> {
        sent(out)
            .iter()
            .filter_map(|frame| frame["method"].as_str().map(str::to_owned))
            .collect()
    }

    fn mentions_prompt(out: &[u8]) -> bool {
        String::from_utf8_lossy(out).contains("the user's message")
    }

    #[test]
    fn claude_holds_its_prompt_until_the_baseline_is_recorded() {
        let (baseline, release) = pending();
        let mut p = protocol(Kind::Claude, &baseline);
        let mut out = Vec::new();
        p.frame(
            json!({"type":"control_response","response":{"subtype":"success","request_id":"native-init"}}),
            &mut out,
        )
        .unwrap();
        assert!(p.prompt_held());
        assert!(!mentions_prompt(&out));
        assert!(
            !p.prompt_delivered(),
            "Hold must not stop an undelivered prompt"
        );
        // Too early: nothing goes out.
        p.release_prompt(&mut out).unwrap();
        assert!(!mentions_prompt(&out));

        recorded(&baseline, &release);
        p.release_prompt(&mut out).unwrap();
        assert!(!p.prompt_held());
        let user: Vec<_> = sent(&out)
            .into_iter()
            .filter(|f| f["type"] == "user")
            .collect();
        assert_eq!(user.len(), 1);
        assert!(mentions_prompt(&out));
        // Released once: a later release sends nothing more.
        p.release_prompt(&mut out).unwrap();
        assert_eq!(sent(&out).iter().filter(|f| f["type"] == "user").count(), 1);
    }

    #[test]
    fn kiro_sets_up_its_session_but_holds_session_prompt() {
        let (baseline, release) = pending();
        let mut p = protocol(Kind::Kiro, &baseline);
        let mut out = Vec::new();
        p.acp_start(&mut out).unwrap();
        p.frame(
            json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{}}}),
            &mut out,
        )
        .unwrap();
        p.frame(
            json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":"kiro-session"}}),
            &mut out,
        )
        .unwrap();
        // Settings requests the session needs are answered as they come.
        for _ in 0..8 {
            if p.prompt_held() {
                break;
            }
            let last = sent(&out).pop().unwrap();
            p.frame(
                json!({"jsonrpc":"2.0","id":last["id"],"result":{}}),
                &mut out,
            )
            .unwrap();
        }
        assert!(p.prompt_held(), "{:?}", methods(&out));
        assert!(methods(&out).contains(&"session/new".to_owned()));
        assert!(!methods(&out).contains(&"session/prompt".to_owned()));
        // The session is set up; a stderr sign-in line is no longer startup.
        assert!(!p.acp_starting());

        recorded(&baseline, &release);
        p.release_prompt(&mut out).unwrap();
        let last = sent(&out).pop().unwrap();
        assert_eq!(last["method"], "session/prompt");
        assert_eq!(last["params"]["sessionId"], "kiro-session");
        assert!(mentions_prompt(&out));
    }

    #[test]
    fn codex_starts_its_thread_but_holds_turn_start() {
        let (baseline, release) = pending();
        let mut p = protocol(Kind::Codex, &baseline);
        let mut out = Vec::new();
        p.frame(json!({"id":1,"result":{}}), &mut out).unwrap();
        assert!(methods(&out).contains(&"thread/start".to_owned()));
        p.frame(
            json!({"id":2,"result":{"thread":{"id":"codex-thread"}}}),
            &mut out,
        )
        .unwrap();
        assert!(p.prompt_held());
        assert!(!methods(&out).contains(&"turn/start".to_owned()));

        recorded(&baseline, &release);
        p.release_prompt(&mut out).unwrap();
        let last = sent(&out).pop().unwrap();
        assert_eq!(last["method"], "turn/start");
        assert_eq!(last["params"]["threadId"], "codex-thread");
    }

    #[test]
    fn a_recorded_or_absent_baseline_never_holds() {
        let mut p = protocol(Kind::Claude, &Baseline::ready());
        let mut out = Vec::new();
        p.claude_prompt(&mut out).unwrap();
        assert!(!p.prompt_held());
        assert!(mentions_prompt(&out));

        let mut p = protocol(Kind::Claude, &Baseline::ready());
        p.request.settings.baseline = None;
        let mut out = Vec::new();
        p.claude_prompt(&mut out).unwrap();
        assert!(!p.prompt_held());
        assert!(mentions_prompt(&out));
    }
}
