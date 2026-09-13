//! Read-only observations, never authorization to perform a cleanup action.
//! Native adapters supply a fresh, bounded, visible accessibility snapshot.
use serde::{Deserialize, Serialize};

pub const PACKAGE: &str = "com.google.android.apps.photos";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub package: String,
    pub locked: bool,
    pub complete: bool,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub account: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub text: String,
    pub actionable: bool,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Observation {
    Locked,
    OtherApp,
    Incomplete,
    Unknown,
    AccountEntry,
    CleanupEntry,
    Confirmation,
    NothingToFree,
    Releasing,
    Completed,
}

pub fn classify(s: &Snapshot) -> Observation {
    use Observation::*;
    if s.locked {
        return Locked;
    }
    if s.package != PACKAGE {
        return OtherApp;
    }
    if !s.complete
        || s.nodes.len() > 64
        || s.nodes
            .iter()
            .any(|n| n.id.len() > 160 || n.text.len() > 2048)
    {
        return Incomplete;
    }
    // Duplicated controls can mean overlapping pages during a transition.
    let one = |id: &str| {
        let qualified = format!("{PACKAGE}:id/{id}");
        let mut found = s.nodes.iter().filter(|n| n.id == qualified);
        let first = found.next();
        if found.next().is_some() {
            None
        } else {
            first
        }
    };
    let exact = |id, labels: &[&str]| one(id).is_some_and(|n| labels.contains(&n.text.trim()));
    let button = |id| one(id).is_some_and(|n| n.actionable);
    let prefix = |id, labels: &[&str]| {
        one(id).is_some_and(|n| labels.iter().any(|p| n.text.trim().starts_with(p)))
    };
    let confirmation = prefix(
        "safetyTip",
        &[
            "These items have been safely backed up",
            "这些内容已按照您选择的画质安全备份。",
        ],
    ) && prefix("free_up_button", &["Free up ", "释放 "])
        && one("free_up_button")
            .is_some_and(|n| n.actionable && n.text.chars().any(|c| c.is_ascii_digit()));
    let completed = prefix(
        "free_up_space_completed_title",
        &["You freed up ", "您已释放 "],
    ) && button("done_button");
    let empty =
        exact("title", &["Nothing to free up", "没有可释放的空间"]) && button("close_button");
    let releasing = one("free_up_space_progress_text").is_some();
    let outcomes = [
        (confirmation, Confirmation),
        (completed, Completed),
        (empty, NothingToFree),
        (releasing, Releasing),
    ];
    let mut recognized = outcomes.into_iter().filter(|(found, _)| *found);
    if let Some((_, observation)) = recognized.next() {
        return if recognized.next().is_none() {
            observation
        } else {
            Unknown
        };
    }
    // A partial cleanup dialog must not fall through to a home/menu behind it.
    if [
        "safetyTip",
        "free_up_button",
        "free_up_space_completed_title",
        "free_up_space_progress_text",
    ]
    .iter()
    .any(|id| s.nodes.iter().any(|n| n.id == format!("{PACKAGE}:id/{id}")))
    {
        return Unknown;
    }
    if s.nodes
        .iter()
        .filter(|n| {
            n.id == format!("{PACKAGE}:id/og_bento_card_title")
                && ["Free up space on this device", "释放此设备的空间"].contains(&n.text.trim())
        })
        .count()
        == 1
    {
        return CleanupEntry;
    }
    if button("selected_account_disc") {
        return AccountEntry;
    }
    Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(entries: &[(&str, &str, bool)]) -> Snapshot {
        Snapshot {
            package: PACKAGE.into(),
            locked: false,
            complete: true,
            account: None,
            nodes: entries
                .iter()
                .map(|(id, text, action)| Node {
                    id: format!("{PACKAGE}:id/{id}"),
                    text: (*text).into(),
                    actionable: *action,
                })
                .collect(),
        }
    }
    fn confirmation() -> Snapshot {
        snapshot(&[
            (
                "safetyTip",
                "These items have been safely backed up at your chosen quality.",
                false,
            ),
            ("free_up_button", "Free up 1.2 GB", true),
        ])
    }
    #[test]
    fn menu_siblings_and_bounded_input() {
        let mut s = snapshot(&[
            ("og_bento_card_title", "", false),
            ("og_bento_card_title", "释放此设备的空间", false),
        ]);
        assert_eq!(classify(&s), Observation::CleanupEntry);
        s.nodes.push(Node {
            id: format!("{PACKAGE}:id/og_bento_card_title"),
            text: "释放此设备的空间".into(),
            actionable: false,
        });
        assert_eq!(classify(&s), Observation::Unknown);
        let mut s = confirmation();
        s.nodes[0].text = "x".repeat(2049);
        assert_eq!(classify(&s), Observation::Incomplete);
        let s = snapshot(&vec![("unknown", "", false); 65]);
        assert_eq!(classify(&s), Observation::Incomplete);
    }
    #[test]
    fn recognized_pages_are_observations_only() {
        assert_eq!(classify(&confirmation()), Observation::Confirmation);
        for (entries, expected) in [
            (
                vec![
                    ("safetyTip", "这些内容已按照您选择的画质安全备份。", false),
                    ("free_up_button", "释放 1 GB", true),
                ],
                Observation::Confirmation,
            ),
            (
                vec![("selected_account_disc", "", true)],
                Observation::AccountEntry,
            ),
            (
                vec![("og_bento_card_title", "释放此设备的空间", false)],
                Observation::CleanupEntry,
            ),
            (
                vec![
                    ("title", "Nothing to free up", false),
                    ("close_button", "Close", true),
                ],
                Observation::NothingToFree,
            ),
            (
                vec![("free_up_space_progress_text", "", false)],
                Observation::Releasing,
            ),
            (
                vec![
                    ("free_up_space_completed_title", "You freed up 2 GB", false),
                    ("done_button", "Done", true),
                ],
                Observation::Completed,
            ),
        ] {
            assert_eq!(classify(&snapshot(&entries)), expected);
        }
    }
    #[test]
    fn incomplete_locked_other_apps_and_disabled_controls_are_rejected() {
        let mut s = confirmation();
        s.locked = true;
        assert_eq!(classify(&s), Observation::Locked);
        s.locked = false;
        s.package = "com.example.fake".into();
        assert_eq!(classify(&s), Observation::OtherApp);
        s.package = PACKAGE.into();
        s.complete = false;
        assert_eq!(classify(&s), Observation::Incomplete);
        s.complete = true;
        s.nodes[1].actionable = false;
        assert_eq!(classify(&s), Observation::Unknown);
        s.nodes[1].actionable = true;
        s.nodes[0].text = "Manage storage".into();
        assert_eq!(classify(&s), Observation::Unknown);
    }
    #[test]
    fn duplicate_and_mixed_transition_frames_do_not_confirm() {
        let mut s = confirmation();
        s.nodes.push(Node {
            id: s.nodes[1].id.clone(),
            text: "Free up 1 GB".into(),
            actionable: true,
        });
        assert_eq!(classify(&s), Observation::Unknown);
        let mut s = confirmation();
        s.nodes.push(Node {
            id: format!("{PACKAGE}:id/free_up_space_progress_text"),
            text: String::new(),
            actionable: false,
        });
        assert_eq!(classify(&s), Observation::Unknown);
    }
    #[test]
    fn text_without_ids_and_cloud_trash_are_not_cleanup_evidence() {
        let s = snapshot(&[
            (
                "photo_caption",
                "These items have been safely backed up",
                false,
            ),
            ("delete_button", "Free up 2 GB", true),
        ]);
        assert_eq!(classify(&s), Observation::Unknown);
        let s = snapshot(&[
            (
                "safetyTip",
                "Delete these items from your Google Account",
                false,
            ),
            ("free_up_button", "Free up 2 GB", true),
            ("selected_account_disc", "", true),
        ]);
        assert_eq!(classify(&s), Observation::Unknown);
    }
}
