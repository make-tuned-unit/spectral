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
/// `content.allows(context)` returns `true` if content at this visibility
/// level can be shared into a federation or query context with the given
/// clearance. Content is shareable when its visibility is at least as
/// permissive as the context requires.
///
/// ```
/// use spectral_core::visibility::Visibility;
///
/// // Public content can be shared into any context
/// assert!(Visibility::Public.allows(Visibility::Org));
/// assert!(Visibility::Public.allows(Visibility::Private));
///
/// // Org content can be shared into Org or stricter-but-broader contexts
/// assert!(Visibility::Org.allows(Visibility::Org));
///
/// // Team content cannot be shared into an Org-clearance context
/// assert!(!Visibility::Team.allows(Visibility::Org));
///
/// // Private content stays private
/// assert!(Visibility::Private.allows(Visibility::Private));
/// assert!(!Visibility::Private.allows(Visibility::Public));
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
    /// Returns `true` if content at this visibility level can be shared
    /// into a context with the given clearance.
    ///
    /// Content is shareable when its visibility is at least as permissive
    /// as the context requires (`self >= context`). Public content is
    /// shareable everywhere; Private content is shareable only into
    /// Private contexts.
    pub fn allows(&self, target: Visibility) -> bool {
        *self >= target
    }
}
