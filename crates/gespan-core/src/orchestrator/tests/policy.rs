use crate::agent::AccessMode;
use crate::orchestrator::policy::{
    child_access, rank, Code, Refusal, MAX_DEPTH, MAX_LIVE_CHILDREN,
};

const ALL: [AccessMode; 5] = [
    AccessMode::ReadOnly,
    AccessMode::Supervised,
    AccessMode::AutoAcceptEdits,
    AccessMode::Auto,
    AccessMode::FullAccess,
];

#[test]
fn modes_rank_narrowest_first() {
    let ranks: Vec<u8> = ALL.into_iter().map(rank).collect();
    assert!(ranks.windows(2).all(|pair| pair[0] < pair[1]), "{ranks:?}");
}

#[test]
fn the_limits_are_the_agreed_ones() {
    assert_eq!(MAX_DEPTH, 2);
    assert_eq!(MAX_LIVE_CHILDREN, 5);
}

#[test]
fn a_child_inherits_its_parents_mode() {
    for parent in ALL {
        assert_eq!(child_access(parent, None), Ok(parent));
    }
}

#[test]
fn a_narrower_or_equal_mode_is_honoured() {
    for (p, parent) in ALL.into_iter().enumerate() {
        for requested in ALL.into_iter().take(p + 1) {
            assert_eq!(child_access(parent, Some(requested)), Ok(requested));
        }
    }
}

#[test]
fn every_broader_mode_is_an_escalation() {
    for (p, parent) in ALL.into_iter().enumerate() {
        for requested in ALL.into_iter().skip(p + 1) {
            let refusal = child_access(parent, Some(requested)).unwrap_err();
            assert_eq!(refusal.code, Code::PermissionEscalation);
            assert!(refusal.message.contains(parent.id()), "{refusal}");
            assert!(refusal.message.contains(requested.id()), "{refusal}");
        }
    }
}

#[test]
fn a_refusal_reads_code_first() {
    let refusal = Refusal::new(Code::DepthLimit, "too deep");
    assert_eq!(refusal.to_string(), "depth_limit: too deep");
    let codes = [
        (Code::DepthLimit, "depth_limit"),
        (Code::FanoutLimit, "fanout_limit"),
        (Code::PermissionEscalation, "permission_escalation"),
        (Code::NotFound, "not_found"),
        (Code::OutOfScope, "out_of_scope"),
        (Code::SelfTarget, "self_target"),
        (Code::HopLimit, "hop_limit"),
        (Code::InvalidInput, "invalid_input"),
    ];
    for (code, text) in codes {
        assert_eq!(code.as_str(), text);
    }
}
