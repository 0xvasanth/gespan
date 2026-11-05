use super::*;

fn accounts(cx: &mut gpui::TestAppContext) -> Entity<Accounts> {
    let store = Arc::new(std::sync::Mutex::new(
        gespan_core::store::Store::open(Path::new(":memory:")).unwrap(),
    ));
    cx.new(|cx| Accounts::new(PathBuf::from("/fixture"), Book::default(), store, cx))
}
fn chat(kind: Kind) -> (SignIn, std::sync::mpsc::Receiver<String>) {
    let (codes, received) = std::sync::mpsc::channel();
    (
        SignIn {
            kind,
            profile: Profile::default(),
            action: AuthAction::SignIn,
            surface: Surface::Chat,
            cancel: Arc::new(AtomicBool::new(false)),
            complete: false,
            error: None,
            adding: None,
            prompt: accounts::LoginPrompt {
                url: Some("https://auth.example.test/device".into()),
                code: Some("ABCD-EFGH".into()),
                wants_code: true,
            },
            codes: Some(codes),
        },
        received,
    )
}

#[gpui::test]
fn a_cancelled_chat_sign_in_stops_the_cli_and_offers_sign_in_again(cx: &mut gpui::TestAppContext) {
    let view = accounts(cx);
    let (signin, _codes) = chat(Kind::Codex);
    let cancel = signin.cancel.clone();
    view.update(cx, |accounts, cx| {
        accounts.signin = Some(signin);
        let state = accounts
            .chat_sign_in(Kind::Codex, accounts::DEFAULT)
            .unwrap();
        assert!(state.running());
        assert_eq!(state.prompt.code.as_deref(), Some("ABCD-EFGH"));
        // Another provider's chat sees a sign-in in progress, not its own.
        assert!(
            accounts
                .chat_sign_in(Kind::Claude, accounts::DEFAULT)
                .is_none()
        );
        assert!(accounts.auth_busy_elsewhere(Kind::Claude, accounts::DEFAULT));
        assert!(!accounts.auth_busy_elsewhere(Kind::Codex, accounts::DEFAULT));

        accounts.cancel_chat_auth(Kind::Codex, accounts::DEFAULT, cx);
        let state = accounts
            .chat_sign_in(Kind::Codex, accounts::DEFAULT)
            .unwrap();
        assert!(!state.running());
        assert_eq!(state.error.as_deref(), Some("Sign-in cancelled"));
        assert!(!accounts.auth_busy_elsewhere(Kind::Claude, accounts::DEFAULT));
        assert!(!accounts.send_chat_code(Kind::Codex, accounts::DEFAULT, "LATE".into()));
    });
    assert!(cancel.load(Ordering::Relaxed));
}
#[gpui::test]
fn pasted_codes_reach_only_the_waiting_cli(cx: &mut gpui::TestAppContext) {
    let view = accounts(cx);
    let (signin, codes) = chat(Kind::Claude);
    view.update(cx, |accounts, _| {
        accounts.signin = Some(signin);
        assert!(!accounts.send_chat_code(Kind::Claude, accounts::DEFAULT, "   ".into()));
        assert!(!accounts.send_chat_code(Kind::Codex, accounts::DEFAULT, "abc".into()));
        assert!(accounts.send_chat_code(Kind::Claude, accounts::DEFAULT, " abc#state \n".into()));
    });
    assert_eq!(codes.try_iter().collect::<Vec<_>>(), ["abc#state"]);
}
#[gpui::test]
fn a_failed_chat_sign_in_reports_why_and_frees_the_button(cx: &mut gpui::TestAppContext) {
    let view = accounts(cx);
    let (mut signin, _codes) = chat(Kind::Kiro);
    signin.error = Some("Kiro CLI still reports that you are signed out. Please try again.".into());
    view.update(cx, |accounts, cx| {
        accounts.signin = Some(signin);
        let state = accounts
            .chat_sign_in(Kind::Kiro, accounts::DEFAULT)
            .unwrap();
        assert!(!state.running());
        assert!(state.error.unwrap().contains("still reports"));
        // Cancelling a finished attempt changes nothing.
        accounts.cancel_chat_auth(Kind::Kiro, accounts::DEFAULT, cx);
        assert!(
            accounts
                .chat_sign_in(Kind::Kiro, accounts::DEFAULT)
                .unwrap()
                .error
                .unwrap()
                .contains("still reports")
        );
        assert!(!accounts.auth_busy_elsewhere(Kind::Kiro, accounts::DEFAULT));
    });
}
