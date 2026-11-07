//! Per-session credentials. Only a SHA-256 hash of a token is ever stored, and
//! a grant carries what its session may reach, read afresh on every request.
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    fmt,
    path::PathBuf,
    time::{Duration, Instant},
};

/// Tokens of ended sessions remembered for a clear error, oldest dropped first.
const REMEMBERED: usize = 1024;

/// What one agent session may reach. The app sets it; an agent never can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub session: i64,
    /// The folder this session works in.
    pub workspace: PathBuf,
    pub toolkits: BTreeSet<String>,
    /// Ids of the servers this session may reach.
    pub servers: BTreeSet<String>,
}

impl Grant {
    pub fn allows(&self, toolkit: &str) -> bool {
        self.toolkits.contains(toolkit)
    }
}

/// The secret an agent presents. Held only long enough to hand it over, and
/// never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    fn mint() -> Self {
        let bytes: [u8; 32] = rand::random();
        Self(hex(&bytes))
    }

    pub fn secret(&self) -> &str {
        &self.0
    }

    /// The value of an `Authorization` header.
    pub fn bearer(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Token(..)")
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hash(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

/// Why a request was turned away. Each reads as the error an agent shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Invalid,
    Ended,
    Expired,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "This tool credential is not valid",
            Self::Ended => "This session has ended, so its tools are no longer available",
            Self::Expired => "This tool credential expired while idle; start a new turn",
        })
    }
}

struct Live {
    grant: Grant,
    used: Instant,
    /// A `_meta` key and the value every tool call must carry in it.
    bound: Option<(String, serde_json::Value)>,
}

/// Live tokens by hash, each session holding at most one.
pub(crate) struct Grants {
    idle: Duration,
    live: HashMap<[u8; 32], Live>,
    sessions: HashMap<i64, [u8; 32]>,
    ended: HashMap<[u8; 32], Refusal>,
    order: VecDeque<[u8; 32]>,
}

impl Grants {
    pub(crate) fn new(idle: Duration) -> Self {
        Self {
            idle,
            live: HashMap::new(),
            sessions: HashMap::new(),
            ended: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// A new process gets a new token. The process it replaces, if any, loses
    /// its own.
    pub(crate) fn mint(&mut self, grant: Grant) -> Token {
        self.issue(grant, None)
    }

    /// A token bound from its first moment, so no call is served unbound
    /// between handing it over and binding it.
    pub(crate) fn mint_bound(
        &mut self,
        grant: Grant,
        key: &str,
        value: serde_json::Value,
    ) -> Token {
        self.issue(grant, Some((key.to_owned(), value)))
    }

    fn issue(&mut self, grant: Grant, bound: Option<(String, serde_json::Value)>) -> Token {
        self.revoke(grant.session, Refusal::Ended);
        let token = Token::mint();
        let key = hash(token.secret());
        self.sessions.insert(grant.session, key);
        self.live.insert(
            key,
            Live {
                grant,
                used: Instant::now(),
                bound,
            },
        );
        token
    }

    /// From now on the session's tool calls must carry `value` under `key` in
    /// their `_meta`. Returns whether the session had a live credential.
    pub(crate) fn bind(&mut self, session: i64, key: &str, value: serde_json::Value) -> bool {
        let Some(live) = self
            .sessions
            .get(&session)
            .and_then(|hashed| self.live.get_mut(hashed))
        else {
            return false;
        };
        live.bound = Some((key.to_owned(), value));
        true
    }

    /// What a session's tool calls must carry, if anything.
    pub(crate) fn bound(&self, session: i64) -> Option<(String, serde_json::Value)> {
        self.sessions
            .get(&session)
            .and_then(|key| self.live.get(key))
            .and_then(|live| live.bound.clone())
    }

    pub(crate) fn revoke(&mut self, session: i64, why: Refusal) -> bool {
        let Some(key) = self.sessions.remove(&session) else {
            return false;
        };
        self.live.remove(&key);
        self.remember(key, why);
        true
    }

    /// Ends this token if it is still its session's, and leaves a newer one
    /// alone. Returns whether it ended.
    pub(crate) fn withdraw(&mut self, session: i64, token: &Token) -> bool {
        self.sessions.get(&session) == Some(&hash(token.secret()))
            && self.revoke(session, Refusal::Ended)
    }

    fn remember(&mut self, key: [u8; 32], why: Refusal) {
        if self.order.len() >= REMEMBERED {
            if let Some(oldest) = self.order.pop_front() {
                self.ended.remove(&oldest);
            }
        }
        self.order.push_back(key);
        self.ended.insert(key, why);
    }

    /// The grant behind a presented secret, as of now. Using it keeps it alive.
    pub(crate) fn admit(&mut self, secret: &str) -> Result<Grant, Refusal> {
        let key = hash(secret);
        let Some(live) = self.live.get_mut(&key) else {
            return Err(self.ended.get(&key).copied().unwrap_or(Refusal::Invalid));
        };
        if live.used.elapsed() >= self.idle {
            let session = live.grant.session;
            self.revoke(session, Refusal::Expired);
            return Err(Refusal::Expired);
        }
        live.used = Instant::now();
        Ok(live.grant.clone())
    }

    /// A running turn keeps its credential alive between tool calls.
    pub(crate) fn touch(&mut self, session: i64) {
        if let Some(live) = self
            .sessions
            .get(&session)
            .and_then(|key| self.live.get_mut(key))
        {
            live.used = Instant::now();
        }
    }

    /// Replaces what a live session may reach. Returns whether that changed it.
    pub(crate) fn update(
        &mut self,
        session: i64,
        toolkits: BTreeSet<String>,
        servers: BTreeSet<String>,
    ) -> bool {
        let Some(live) = self
            .sessions
            .get(&session)
            .and_then(|key| self.live.get_mut(key))
        else {
            return false;
        };
        if live.grant.toolkits == toolkits && live.grant.servers == servers {
            return false;
        }
        live.grant.toolkits = toolkits;
        live.grant.servers = servers;
        true
    }

    /// Sessions that may reach this server now.
    pub(crate) fn holders(&self, server: &str) -> Vec<i64> {
        self.live
            .values()
            .filter(|live| live.grant.servers.contains(server))
            .filter(|live| live.used.elapsed() < self.idle)
            .map(|live| live.grant.session)
            .collect()
    }

    /// The servers a live session may reach.
    pub(crate) fn servers_of(&self, session: i64) -> BTreeSet<String> {
        self.sessions
            .get(&session)
            .and_then(|key| self.live.get(key))
            .map(|live| live.grant.servers.clone())
            .unwrap_or_default()
    }

    /// Ends every credential past its idle limit, whether or not anyone
    /// presents it again, and returns their sessions.
    pub(crate) fn expire(&mut self) -> Vec<i64> {
        let idle: Vec<_> = self
            .live
            .values()
            .filter(|live| live.used.elapsed() >= self.idle)
            .map(|live| live.grant.session)
            .collect();
        for session in &idle {
            self.revoke(*session, Refusal::Expired);
        }
        idle
    }

    /// Whether a session could use its credential right now.
    pub(crate) fn has(&self, session: i64) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|key| self.usable(key))
    }

    /// Whether this token is the one a session can use right now. A token
    /// replaced by a newer mint isn't.
    pub(crate) fn holds(&self, session: i64, token: &Token) -> bool {
        let key = hash(token.secret());
        self.sessions.get(&session) == Some(&key) && self.usable(&key)
    }

    fn usable(&self, key: &[u8; 32]) -> bool {
        self.live
            .get(key)
            .is_some_and(|live| live.used.elapsed() < self.idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(session: i64) -> Grant {
        Grant {
            session,
            workspace: "/work".into(),
            toolkits: BTreeSet::from(["visual".to_owned()]),
            servers: BTreeSet::new(),
        }
    }

    #[test]
    fn only_the_hash_of_a_token_is_kept() {
        let mut grants = Grants::new(Duration::from_secs(60));
        let token = grants.mint(grant(1));
        assert!(token.secret().len() >= 64, "256 bits");
        assert!(!grants.live.keys().any(|key| hex(key) == token.secret()));
        assert_eq!(format!("{token:?}"), "Token(..)");
        assert_eq!(grants.admit(token.secret()).unwrap().session, 1);
        assert_eq!(grants.admit("forged"), Err(Refusal::Invalid));
    }

    #[test]
    fn a_new_process_replaces_the_token_of_the_one_before() {
        let mut grants = Grants::new(Duration::from_secs(60));
        let old = grants.mint(grant(1));
        let new = grants.mint(grant(1));
        assert_eq!(grants.admit(old.secret()), Err(Refusal::Ended));
        assert!(grants.admit(new.secret()).is_ok());
    }

    #[test]
    fn revoked_and_idle_tokens_say_why() {
        let mut grants = Grants::new(Duration::from_secs(60));
        let token = grants.mint(grant(1));
        assert!(grants.revoke(1, Refusal::Ended));
        assert_eq!(grants.admit(token.secret()), Err(Refusal::Ended));
        assert!(!grants.revoke(1, Refusal::Ended));

        let mut grants = Grants::new(Duration::ZERO);
        let token = grants.mint(grant(2));
        assert_eq!(grants.admit(token.secret()), Err(Refusal::Expired));
        assert!(!grants.has(2));
        assert_eq!(grants.admit(token.secret()), Err(Refusal::Expired));
    }

    #[test]
    fn a_session_past_its_idle_limit_no_longer_counts_as_holding_one() {
        let mut grants = Grants::new(Duration::ZERO);
        let token = grants.mint(grant(1));
        assert!(!grants.has(1), "before any request has seen the expiry");
        assert!(!grants.holds(1, &token));
    }

    #[test]
    fn expiry_ends_idle_credentials_nobody_presents_again() {
        let mut grants = Grants::new(Duration::ZERO);
        let token = grants.mint(grant(1));
        let mut reach = grant(2);
        reach.servers.insert("docs".to_owned());
        grants.mint(reach);
        assert!(grants.holders("docs").is_empty(), "past its limit already");
        assert_eq!(grants.expire().len(), 2);
        assert_eq!(grants.admit(token.secret()), Err(Refusal::Expired));
        assert!(grants.expire().is_empty());
    }

    #[test]
    fn only_the_latest_token_of_a_session_is_held() {
        let mut grants = Grants::new(Duration::from_secs(60));
        let old = grants.mint(grant(1));
        assert!(grants.holds(1, &old));
        let new = grants.mint(grant(1));
        assert!(grants.has(1));
        assert!(
            !grants.holds(1, &old),
            "the session lives on, this token doesn't"
        );
        assert!(grants.holds(1, &new));
        assert!(!grants.holds(2, &new), "a token belongs to one session");
    }

    #[test]
    fn use_and_touch_both_hold_off_expiry() {
        let mut grants = Grants::new(Duration::from_millis(60));
        let token = grants.mint(grant(1));
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(30));
            grants.touch(1);
        }
        assert!(grants.admit(token.secret()).is_ok());
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(grants.admit(token.secret()), Err(Refusal::Expired));
    }

    #[test]
    fn a_grant_is_read_fresh_after_an_update() {
        let mut grants = Grants::new(Duration::from_secs(60));
        let token = grants.mint(grant(1));
        let ticket = BTreeSet::from(["ticket".to_owned()]);
        assert!(grants.update(1, ticket.clone(), BTreeSet::new()));
        let current = grants.admit(token.secret()).unwrap();
        assert!(current.allows("ticket") && !current.allows("visual"));
        assert!(!grants.update(9, BTreeSet::new(), BTreeSet::new()));
        assert!(
            !grants.update(1, ticket.clone(), BTreeSet::new()),
            "no change, nothing to announce"
        );
        assert!(grants.update(1, ticket, BTreeSet::from(["docs".to_owned()])));
        assert_eq!(grants.holders("docs"), [1]);
    }

    #[test]
    fn a_token_minted_bound_is_bound_from_the_start() {
        let mut grants = Grants::new(Duration::from_secs(60));
        let key = "client.example/conversation";
        grants.mint_bound(grant(1), key, serde_json::json!("mine"));
        assert_eq!(
            grants.bound(1),
            Some((key.to_owned(), serde_json::json!("mine")))
        );
        let fresh = grants.mint(grant(1));
        assert_eq!(grants.bound(1), None, "a plain mint starts unbound");
        assert!(grants.holds(1, &fresh));
    }

    #[test]
    fn withdrawing_ends_only_the_token_still_held() {
        let mut grants = Grants::new(Duration::from_secs(60));
        let old = grants.mint(grant(1));
        let new = grants.mint(grant(1));
        assert!(!grants.withdraw(1, &old), "already replaced");
        assert!(grants.holds(1, &new), "the newer token is left alone");
        assert!(!grants.withdraw(2, &new), "a token belongs to one session");
        assert!(grants.withdraw(1, &new));
        assert_eq!(grants.admit(new.secret()), Err(Refusal::Ended));
        assert!(!grants.withdraw(1, &new));
    }

    #[test]
    fn remembered_tokens_stay_bounded() {
        let mut grants = Grants::new(Duration::from_secs(60));
        for session in 0..(REMEMBERED as i64 + 8) {
            grants.mint(grant(session));
            grants.revoke(session, Refusal::Ended);
        }
        assert_eq!(grants.ended.len(), REMEMBERED);
        assert!(grants.live.is_empty() && grants.sessions.is_empty());
    }
}
