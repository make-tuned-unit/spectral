# Cognitive Spectrogram Design

## Overview

The cognitive spectrogram classifies each memory along seven dimensions and uses these fingerprints to find "resonant" memories across wings (topic areas). Regular fingerprints pair memories within a single wing. Cross-wing fingerprints pair memories across wings when their cognitive dimensions align.

## Cognitive Dimensions

Each dimension is computed via lightweight deterministic heuristics (regex, keyword matching, simple counts). No LLM required.

### entity_density (0.0 - 1.0)

Count of capitalized multi-word phrases, ALL_CAPS abbreviations, and name-like patterns, divided by sqrt(content_length). Higher values indicate content rich in named entities.

### action_type (enum)

One of: decision, discovery, task, observation, advice, reflection. Classified by keyword patterns:
- decision: "decided", "chose", "going with", "locked in"
- discovery: "found that", "noticed", "realized"
- task: "build", "implement", "fix", "deploy"
- observation: default fallback
- advice: "should", "recommend", "suggest"
- reflection: "thinking about", "considering"

### decision_polarity (-1.0 to 1.0)

Only meaningful for action_type == decision. Detects "yes/proceed/approved" vs "no/cancel/rejected" patterns. Neutral (0.0) for non-decision memories.

### causal_depth (0.0 - 1.0)

Count of causal connectives ("because", "therefore", "so that", "as a result", "leads to") divided by sentence count, capped at 1.0. Measures how much the memory contains explicit causal reasoning.

### emotional_valence (-1.0 to 1.0)

Count of positive sentiment words minus negative sentiment words, normalized by total matches. Neutral (0.0) when balanced or absent.

### temporal_specificity (0.0 - 1.0)

Count of explicit time anchors (dates, "yesterday", "last week", day names, date patterns) divided by sentence count. Higher values indicate time-anchored content.

### novelty (0.0 - 1.0)

Proportion of terms in the content that do not appear in the existing corpus for the same wing. Higher values indicate new information.

## Cross-Wing Matching

Two memories "resonate" when:
1. They share the same action_type (required)
2. At least 3 of 6 numeric dimensions are within configurable tolerances (default 0.3)

The resonance_score is the fraction of matching dimensions (out of 6), so 3/6 = 0.5, 6/6 = 1.0.

This lets a query about "build sessions" surface relevant decision memories from other projects, even though the specific vocabulary differs. The cognitive structure (same action_type, similar emotional_valence and causal_depth) is what matches, not keywords.

## Integration

Spectrogram computation is opt-in via `BrainConfig::enable_spectrogram`. When enabled, `remember_with()` computes and stores the spectrogram alongside the memory. Existing memories without spectrograms can be backfilled via `Brain::backfill_spectrograms()`.

Cross-wing recall is available via `Brain::recall_cross_wing(seed_query, visibility, max_results)`.
