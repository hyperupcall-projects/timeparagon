//! Inbox task ordering, shared by settings and the desktop view.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxSort {
    #[default]
    Manual,
    CreatedAscending,
    CreatedDescending,
    ModifiedAscending,
    ModifiedDescending,
}

impl InboxSort {
    pub const ALL: [Self; 5] = [
        Self::Manual,
        Self::CreatedAscending,
        Self::CreatedDescending,
        Self::ModifiedAscending,
        Self::ModifiedDescending,
    ];

    /// The name it is saved under in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::CreatedAscending => "created_ascending",
            Self::CreatedDescending => "created_descending",
            Self::ModifiedAscending => "modified_ascending",
            Self::ModifiedDescending => "modified_descending",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sort| sort.name() == name)
    }
}

impl std::fmt::Display for InboxSort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Manual => "Manual order",
            Self::CreatedAscending => "Created: oldest first",
            Self::CreatedDescending => "Created: newest first",
            Self::ModifiedAscending => "Modified: oldest first",
            Self::ModifiedDescending => "Modified: newest first",
        })
    }
}
