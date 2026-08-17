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

/// The admissibility rule, enumerated by hand.
///
/// `allows` is the single predicate every sovereignty guarantee rests on, and
/// it had **no direct test** despite 23 production call sites. What tests it
/// did have exercised it only indirectly, and the headline sovereignty property
/// test in `spectral-graph` used `allows` as its own *oracle* — so inverting
/// the predicate inverted both sides of the assertion and the property stayed
/// green.
///
/// Every case below is therefore written out with a literal expected value.
/// Do not replace this table with a loop over `>=`, `Ord`, or `allows` itself:
/// a table derived from the implementation restates it rather than checking it.
#[cfg(test)]
mod allows_truth_table {
    use super::Visibility::{Org, Private, Public, Team};

    /// All 4x4 (content, context) pairs with the admissible answer spelled out.
    ///
    /// Read as: content at row level, offered into a context requiring column
    /// level. Content is admissible when it is at least as permissive as the
    /// context requires, so the true region is the lower-left triangle
    /// including the diagonal.
    const TABLE: [(super::Visibility, super::Visibility, bool); 16] = [
        // Private content is admissible only in a Private context.
        (Private, Private, true),
        (Private, Team, false),
        (Private, Org, false),
        (Private, Public, false),
        // Team content is admissible in Private and Team contexts.
        (Team, Private, true),
        (Team, Team, true),
        (Team, Org, false),
        (Team, Public, false),
        // Org content is admissible everywhere except a Public context.
        (Org, Private, true),
        (Org, Team, true),
        (Org, Org, true),
        (Org, Public, false),
        // Public content is admissible in every context.
        (Public, Private, true),
        (Public, Team, true),
        (Public, Org, true),
        (Public, Public, true),
    ];

    #[test]
    fn the_full_admissibility_table_is_enumerated_by_hand() {
        for (content, context, expected) in TABLE {
            assert_eq!(
                content.allows(context),
                expected,
                "{content:?} content offered into a {context:?} context: \
                 expected allows() == {expected}"
            );
        }
    }

    /// Guards the table itself. If a future edit collapsed it to all-true or
    /// all-false, the loop above would still pass against a matching bug; this
    /// pins that the rule actually discriminates.
    #[test]
    fn the_table_contains_both_outcomes() {
        assert_eq!(
            TABLE.iter().filter(|(_, _, e)| *e).count(),
            10,
            "the admissible region should be the 10-cell lower triangle"
        );
        assert_eq!(TABLE.iter().filter(|(_, _, e)| !*e).count(), 6);
    }

    /// The asymmetry is the whole point: admissibility is directional, so a
    /// predicate that accidentally became symmetric (`==`, or an unordered
    /// comparison) must fail.
    #[test]
    fn admissibility_is_directional_not_symmetric() {
        assert!(Public.allows(Private), "public content suits any context");
        assert!(
            !Private.allows(Public),
            "private content must never be admissible in a public context"
        );
    }

    /// Every level admits itself. Stated separately so a predicate that became
    /// strictly greater-than fails here rather than only inside the loop.
    #[test]
    fn every_level_admits_its_own_context() {
        for v in [Private, Team, Org, Public] {
            assert!(v.allows(v), "{v:?} content should suit a {v:?} context");
        }
    }
}
