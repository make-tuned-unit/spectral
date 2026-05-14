# Topology — Conceptual Lineage

Spectral's "recognition not retrieval" stance is the endpoint of
a chain of influences. Each maps to a live component. This table
is the rationale behind the Track 2 — Topology backlog section.

| Influence | Spectral component | What it does |
|---|---|---|
| Shazam / spectrography | Spectrogram, TACT fingerprint | Recognise a memory by signal signature, not by reading contents |
| ACR / smart TVs | RecognitionContext, ambient boost | Continuous background matching; memory always running, not query-gated |
| Recommendation algorithms | Signal re-ranking, signal_scorer | Surface relevance by score and position, not by search-string match |
| Brain topography / topology | Constellation, wings/halls, co-retrieval pairs, Kuzu graph neighborhood | Memory as structured space; adjacency encodes relatedness |
| The intoponet | The Constellation graph as a whole | Information organised by position and adjacency |

Adjacency exists today in two forms: co-retrieval pairs (session
co-occurrence, shipped PR #90, live in cascade ranking at weight
0.10) and the Kuzu graph neighborhood (entity-graph BFS, built
PRs #2/#3, currently inert — see backlog item T1).
