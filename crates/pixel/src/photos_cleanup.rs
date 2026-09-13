//! Bounded decisions for the opt-in native Google Photos adapter. The adapter
//! must durably save the returned state BEFORE executing an action, and obtain
//! a fresh, equivalent snapshot before any click. Pending never confirms again.
use crate::photos_probe::{classify, Observation, Snapshot};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Home,
    Menu,
    Offer,
    Pending,
    Returning,
    Finished,
    Stopped,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct State {
    pub phase: Phase,
    pub backs: u8,
    pub actions: u8,
    #[serde(default)]
    pub nothing_to_free: bool,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Wait,
    Back,
    Account,
    Entry,
    Confirm,
    Done,
    Close,
    Finish,
    Stop,
}
#[derive(Serialize)]
pub struct Decision {
    pub state: State,
    pub action: Action,
    pub reason: &'static str,
}
pub fn step(mut state: State, snapshot: &Snapshot, expected: &str) -> Decision {
    use Action::*;
    use Observation::*;
    let observed = classify(snapshot);
    let pending = state.phase == Phase::Pending;
    let (action, reason) = if matches!(state.phase, Phase::Finished | Phase::Stopped) {
        (Stop, "stopped")
    } else if observed == Locked {
        (Stop, "locked")
    } else if snapshot
        .account
        .as_deref()
        .is_some_and(|value| value != expected)
    {
        (Stop, "account_changed")
    } else if pending {
        match observed {
            Completed => {
                state.phase = Phase::Returning;
                (Done, "completed")
            }
            NothingToFree => {
                state.phase = Phase::Returning;
                state.nothing_to_free = true;
                (Close, "nothing_to_free")
            }
            _ => (Wait, "pending"),
        }
    } else if state.phase == Phase::Returning {
        match observed {
            AccountEntry => {
                state.phase = Phase::Finished;
                (
                    Finish,
                    if state.nothing_to_free {
                        "nothing_to_free"
                    } else {
                        "completed"
                    },
                )
            }
            Completed => (Done, "returning"),
            NothingToFree => (Close, "returning"),
            _ if state.backs < 3
                && snapshot.package == crate::photos_probe::PACKAGE
                && snapshot.complete =>
            {
                state.backs += 1;
                (Back, "returning")
            }
            _ => {
                state.phase = Phase::Finished;
                (Finish, "return_home_failed")
            }
        }
    } else if state.actions >= 12 {
        (Stop, "unrecognized")
    } else {
        match (&state.phase, observed) {
            (Phase::Home, AccountEntry)
                if !expected.is_empty() && snapshot.account.as_deref() == Some(expected) =>
            {
                state.phase = Phase::Menu;
                (Account, "opening_menu")
            }
            (Phase::Menu, CleanupEntry) => {
                state.phase = Phase::Offer;
                (Entry, "opening_cleanup")
            }
            (Phase::Offer, Confirmation) => {
                state.phase = Phase::Pending;
                (Confirm, "confirming")
            }
            (Phase::Offer, NothingToFree) => {
                state.phase = Phase::Returning;
                state.nothing_to_free = true;
                (Close, "nothing_to_free")
            }
            (_, Releasing) => {
                state.phase = Phase::Pending;
                (Wait, "pending")
            }
            (_, Completed) => {
                state.phase = Phase::Returning;
                (Done, "completed")
            }
            (_, Incomplete | OtherApp) => (Stop, "unrecognized"),
            (_, AccountEntry) if snapshot.account.is_none() => (Stop, "account_unavailable"),
            _ if state.backs < 3 => {
                state.phase = Phase::Home;
                state.backs += 1;
                (Back, "finding_home")
            }
            _ => (Stop, "unrecognized"),
        }
    };
    if !matches!(action, Wait | Stop | Finish) {
        state.actions = state.actions.saturating_add(1);
    }
    // Preserve a pending phase even on lock/interruption so restart cannot click again.
    if action == Stop && !pending {
        state.phase = Phase::Stopped;
    }
    Decision {
        state,
        action,
        reason,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::photos_probe::{Node, PACKAGE};
    fn page(nodes: &[(&str, &str, bool)]) -> Snapshot {
        Snapshot {
            package: PACKAGE.into(),
            locked: false,
            complete: true,
            account: Some("bound".into()),
            nodes: nodes
                .iter()
                .map(|(id, text, actionable)| Node {
                    id: format!("{PACKAGE}:id/{id}"),
                    text: (*text).into(),
                    actionable: *actionable,
                })
                .collect(),
        }
    }
    #[test]
    fn account_safety_and_durable_single_confirmation() {
        let home = page(&[("selected_account_disc", "", true)]);
        let menu = page(&[("og_bento_card_title", "释放此设备的空间", true)]);
        let offer = page(&[
            ("safetyTip", "这些内容已按照您选择的画质安全备份。", false),
            ("free_up_button", "释放 1 GB", true),
        ]);
        let d = step(State::default(), &home, "bound");
        assert_eq!(d.action, Action::Account);
        let d = step(d.state, &menu, "bound");
        assert_eq!(d.action, Action::Entry);
        let d = step(d.state, &offer, "bound");
        assert_eq!(d.action, Action::Confirm);
        let saved = serde_json::to_string(&d.state).unwrap();
        let d = step(serde_json::from_str(&saved).unwrap(), &offer, "bound");
        assert_eq!(d.action, Action::Wait);
        let mut locked = offer;
        locked.locked = true;
        let d = step(d.state, &locked, "bound");
        assert_eq!(d.state.phase, Phase::Pending);
        assert_eq!(d.action, Action::Stop);
        assert_eq!(
            step(State::default(), &home, "different").action,
            Action::Stop
        );
        let unknown = page(&[]);
        let mut state = State::default();
        for _ in 0..3 {
            let d = step(state, &unknown, "bound");
            assert_eq!(d.action, Action::Back);
            state = d.state;
        }
        assert_eq!(step(state, &unknown, "bound").action, Action::Stop);
    }
}
