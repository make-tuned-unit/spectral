//! Four-level visibility model for Spectral content.
//!
//! Visibility controls who can access an entity. The levels form a total
//! order from most restrictive ([`Private`](Visibility::Private)) to least
//! restrictive ([`Public`](Visibility::Public)).

use serde::{Deserialize, Serialize};

/// Content visibility level.
///
/// Ordered from most restrictive to least restrictive:
/// `Private < Team < Org < Public`.
///
/// # Ordering
///
/// ```
/// use spectral_core::visibility::Visibility;
///
/// assert!(Visibility::Private < Visibility::Team);
/// assert!(Visibility::Team < Visibility::Org);
/// assert!(Visibility::Org < Visibility::Public);
/// ```
///
/// # The `allows` method
///
/// ```
/// use spectral_core::visibility::Visibility;
///
/// // Org-level content is visible to Org and Public consumers
/// assert!(Visibility::Org.allows(Visibility::Org));
/// assert!(Visibility::Public.allows(Visibility::Org));
/// assert!(!Visibility::Team.allows(Visibility::Org));
/// ```
///
/// # Serde as lowercase strings
///
/// ```
/// use spectral_core::visibility::Visibility;
///
/// let json = serde_json::to_string(&Visibility::Team).unwrap();
/// assert_eq!(json, "\"team\"");
/// let v: Visibility = serde_json::from_str("\"public\"").unwrap();
/// assert_eq!(v, Visibility::Public);
/// ```
///
/// # Default is Private
///
/// ```
/// use spectral_core::visibility::Visibility;
///
/// assert_eq!(Visibility::default(), Visibility::Private);
/// ```
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    #[default]
    Private,
    Team,
    Org,
    Public,
}

impl Visibility {
    /// Returns `true` if a consumer at this visibility level can access
    /// content at the `target` visibility level.
    ///
    /// A consumer can access content when their level is at least as
    /// permissive as the content's level (`self >= target`).
    pub fn allows(&self, target: Visibility) -> bool {
        *self >= target
    }
}
