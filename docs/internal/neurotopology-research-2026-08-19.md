# Neurotopology and data structure for Spectral's recognition engines — research memo

Written 2026-08-19, at Jesse's request, after R37–R39 (PRs #295, #296). The
question: what does current research on how biological memory is *structured*
say about how Permagent should structure its data so that Spectral's
recognition engines work — and where do this week's measurements already
confirm or contradict it?

Every principle below is paired with (a) the Spectral structure it bears on,
(b) what we have measured, (c) a recommendation, (d) the experiment that would
test it. Principles without a testable consequence are left out.

## 0. What this week established, in one paragraph

Enrichment (the Librarian's prose description) helps **FTS recall** (+6.4pp
evidence‑turn recall on LoCoMo, n=369) and hurts **every identity engine** it
touches: recognition (all enrolment shapes, via homogenisation of
same‑template memories), spectrogram (separation falls; fingerprint AUC for
evidence ≈ chance), and it does nothing measurable for TACT's hall (regexes
recover 4–11%). Prompt style is irrelevant to FTS (0.7pp, p=0.41). The one
place enrichment carries identity signal is **paraphrase‑shaped
re‑encounters** (top‑1 42% → 67–88%), and there it never clears the
identity gate. Recognition today is at the raw‑content optimum: exact 83.7%
Recognized / 95.3% top‑1, 0/300 false Recognized.

The literature below says the same thing from the other direction: the brain
keeps *index* and *content* apart, keeps similar experiences *decorrelated*,
keeps surprising experiences *raw*, and treats *familiarity* and
*recollection* as different signals with different substrates.

## 1. Pattern separation — similar inputs must get distinct codes

**Research.** The dentate gyrus assigns distinct, sparse codes to similar
inputs, amplifying small differences before CA3 does pattern completion; the
ability to discriminate highly similar memories depends on it. A 2025 human
single‑unit study found that only remembered items with an encoding‑time
firing increase carried a sparse, pattern‑separated code at retrieval, and
only in the hippocampus. Sparseness is what decorrelates overlapping inputs
and minimises interference.

**Our structure.** Recognition's identity trace: ≤32 landmark peaks per
memory (anchors first, then rarity), order‑insensitive pair hashes, winnowed
grams, a shingle set for containment. Rarity is the R9 `TermIdf` seam.

**Measured.** This is exactly where enrichment failed. Descriptions summarise,
so the fifty "Automation '…' completed in N ms" memories converge on shared
vocabulary; any enrolment shape that adds description features raises
containment across the whole cluster and the lead‑margin rule turns
Recognized into Familiar (drop30 59.3% → 31.7% under union enrolment). And
even content‑only, **16.3% of exact re‑encounters are only Familiar** —
same‑template near‑duplicates failing the margin against each other. That is
a pattern‑separation deficit in our own substrate, independent of enrichment.

**Recommendation.** (1) Content is the identity trace; nothing summarised
ever joins it — settled. (2) Build the DG analogue we lack: **template‑aware
separation**. Detect same‑template families at write time (high shingle
containment with an existing memory + shared non‑anchor peaks), and for
members of a family weight the *within‑family distinguishing* features
(numbers, ids, timestamps — the anchors) up and the shared template down when
scoring the margin. Rarity‑weighting already does part of this globally; it
does not do it *within* a family, which is where the 16% lives.

**Experiment R42.** `recognition_e2e` on the real brain: report the
exact‑Familiar rate split by family membership; implement family‑aware margin
behind a config flag; gate = exact Recognized ≥ 90% with false Recognized on
foreign text still 0%.

## 2. Hippocampal indexing — the index is not the content

**Research.** Indexing theory (Teyler & DiScenna; Teyler & Rudy 2007; the
2020 "integrated index" review; the 2018 "engram as index" review) holds that
the hippocampus stores a compact index that binds and reinstates distributed
cortical content; it does not store the content. HippoRAG (NeurIPS 2024) and
HippoRAG 2 (2025) operationalise this for LLM memory: an LLM extracts entities
and relations into a schemaless graph (the index) over raw passages (the
content), and retrieval runs Personalized PageRank from query‑matched nodes.

**Our structure.** Raw `content` is immutable and every engine derives from it:
FTS shadow, recognition index, TACT fingerprints (`constellation_fingerprints`,
440k rows), `co_retrieval_pairs` (296k), episodes (443), the entity graph.
Descriptions live in the same row as content and are indexed into FTS.

**Measured.** Every place a *summary* was allowed to touch an *index* it hurt.
The place it helped (FTS) is the one place it was used as content for
lexical search — a legitimate cortical "gist" role. And our graph — the
HippoRAG‑style index — is inert: 9 triples across 136 entities, zero
production `assert` calls (brain‑substrate audit, PR #291).

**Recommendation.** Make the separation architectural, not conventional:
descriptions and any future enrichment live in a **separate store/channel**
consulted by name, never concatenated into an identity index (R41 below).
And populate the actual index: the Librarian should emit **relations**
(subject–predicate–object) into `assert_typed`, because that is the one
structure HippoRAG shows converts LLM extraction into multi‑hop retrieval,
and it is the largest unrealised capability we have.

**Experiment R43.** Librarian emits ≤3 triples per memory (entities it
already extracts as `term:`/`cat:` mentions become nodes; predicates from a
small closed set); measure `related_memories`/2‑hop recall on the fixture
brain and evidence‑turn recall with a graph‑expansion arm on LoCoMo (the
`graph` retrieval path exists in the bench and currently has nothing to walk).

## 3. Adaptive compression — keep the surprising raw, compress the ordinary

**Research.** Nagy et al. (Nature Reviews Psychology, 2025) frame episodic vs
semantic memory as adaptive compression: semantic memory learns regularities
and compresses; episodic memory keeps *surprising* experiences in a
high‑fidelity, less‑compressed form as a "life‑raft" for later model updates.
GENESIS (2025) models the same interaction generatively and reproduces
gist‑based distortions of episodic recall from semantic processing.

**Our structure.** Consolidation tiers (`Raw → HourlyRollup → DailyRollup →
WeeklyRollup`), `consolidate_as`/`consolidate_extractive`, the Librarian
atoms (LIBRARIAN_ATOMS_BRIEF), and recognition's `novelty` score.

**Measured.** Not directly this week. Adjacent evidence: R31 showed the actor
benefits from *raw* neighbours (adjacency, +6.0pp answers), and the July
read‑time consolidation regressed −9.2pp — a lossy intermediate the actor
over‑trusts. Both are the "gist distorts" effect.

**Recommendation.** Gate compaction on surprise: a memory whose recognition
verdict at write time was Novel (low familiarity, no near‑template) stays
`Raw` longer and is excluded from extractive rollups; ordinary
(high‑familiarity, template‑family) memories are the ones to roll up. This is
cheap: the signal is already computed on every write.

**Experiment R44.** Novelty‑gated compaction vs age‑gated compaction on the
LongMemEval/LoCoMo actor bench at fixed context budget.

## 4. Engram allocation and linking — proximity in time links memories

**Research.** Neurons with higher intrinsic excitability win allocation;
experiences close in time land on overlapping ensembles, and that overlap is
the substrate of memory *linking* (recalling one brings the other). 2024–2026
work confirms competitive, activity‑dependent allocation in CA1.

**Our structure.** `episode_id` (64% coverage), `co_retrieval_pairs`,
`constellation_fingerprints` (time‑delta buckets), turn adjacency (R25/R31).

**Measured.** R31: adjacency — emitting a hit's dialogue neighbours — is the
one enrichment‑free lever that converted to answers (+6.0pp, p=0.0175). That
is memory linking by temporal proximity, working.

**Recommendation.** Treat episodes as the linking substrate and invest there
rather than in prose: raise `episode_id` coverage toward 100% (36% of the
brain has none), and let co‑retrieval strengthen links (the Hebbian half we
record but do not yet use for expansion).

**Experiment R45.** Co‑retrieval‑weighted episode expansion vs plain adjacency
at equal budget on the oracle bench.

## 5. Dual‑process recognition — familiarity and recollection are different signals

**Research.** Yonelinas et al. (2024 review): selective hippocampal damage
impairs recollection and spares familiarity; perirhinal/entorhinal damage
does the reverse; volumes predict the two components separately. The
single‑vs‑dual debate continues, but the dissociation is robust.

**Our structure.** Verdicts `Recognized` (identity), `Familiar` (aggregate
echo without a dominant trace), `Novel`; MINERVA‑style familiarity, REM‑style
odds‑of‑old.

**Measured.** The paraphrase probe is a textbook dissociation: with a
description in the trace, top‑1 identity is right 67–88% of the time, the
engine says Familiar 97–100% and Recognized ≤3%. Familiarity present,
recollection absent — the gate is tuned for content‑shaped stimuli.

**Recommendation (R41).** A **description channel**: enrol description
features in their own index keyed by memory id; at recognize time, if the
identity trace returns Familiar, consult the channel and, when it agrees with
the top identity candidate, return `Recognized` with a provenance flag
(`via: description`) — perirhinal familiarity promoting hippocampal
recollection, never the reverse. `enroll_parts` is the primitive; the verdict
rule is the work.

**Experiment.** `recognition_e2e` para probe: Recognized(correct) from ≤3% to
≥50% with content‑shaped probes unchanged and foreign false Recognized 0%.

## 6. Structural knowledge separate from sensory content (TEM)

**Research.** The Tolman‑Eichenbaum Machine (Whittington et al., Cell 2020;
successors through 2025) separates a *structural* basis (entorhinal: where
things sit relative to each other) from *sensory* content, with hippocampal
cells binding the two; generalisation comes from the structural half.

**Our structure.** TACT's structural code is `hash(hall, target_hall, wing,
time_bucket)`. It is the closest thing we have to an entorhinal basis.

**Measured.** It is degenerate: `hall = event` (rule fallback) on 77.7% of
the real brain, `wing = general` on 49%. Prose recovers 4–11%.

**Recommendation (R40).** The structural basis must be **supplied, not
regex‑guessed**: the Librarian emits an explicit `HALL:` from the closed hall
vocabulary (and Permagent supplies domain `hall_rules`), the same way it
already emits categories. Then TACT tier‑1 has something to route on.

**Experiment.** Oracle `tact` path with a `--hall-map` (memory_key → hall)
applied after ingest, vs the same without; evidence‑turn recall.

## 7. Topological neuroscience — co‑activity has shape; not every space does

**Research.** Persistent homology recovers environment topology from
place‑cell co‑firing (Annual Review of Neuroscience 2024, "Topological
Neuroscience"; the 2025 TDA/TDL review). The lesson that transfers is
methodological: *test whether a representation has task‑relevant structure
before building on it.*

**Measured.** We did exactly that for the 7‑dimensional spectrogram: AUC for
gold‑vs‑non‑gold evidence 0.42–0.55 (chance 0.50), with a gold turn as seed
0.52–0.57. That space has no task‑relevant topology on this data. The
co‑retrieval graph (296k pairs) has never been analysed this way and is the
natural candidate.

**Recommendation.** Retire the spectrogram from the roadmap for retrieval and
recognition (it stays behind `spectrogram-legacy`). If a "cognitive shape"
space is ever wanted again it must clear the AUC probe first. Analyse the
co‑retrieval graph's community structure before spending on it.

## 8. Fingerprinting lineage — the peak‑pair design is still the right one for C3

**Research.** Landmark peak‑pair fingerprints (Shazam/Wang; Ellis's robust
implementation) remain the noise‑robust baseline; 2024–2025 work adds learned
contrastive embeddings and peak‑based neural variants (PeakNetFP, 2025) mainly
for extreme time‑stretch and variable‑length queries. Recall/precision is
governed by peak density and fan‑out; the identity emerges as a vote cluster.

**Our structure.** Peak pairs + winnowed grams + MinHash containment, zero
inference (C3). Density/fan‑out are `max_peaks` (32), `fan_out` (8),
`pair_window` (16).

**Recommendation.** No change of family. The two levers we have not swept
against the e2e probe are `max_peaks` and family‑aware weighting (§1). Do that
before considering any learned component, which C3 forbids at the default
build anyway.

## What to structure differently in Permagent — ranked

1. **Description is a channel, not content** — separate index, consulted by
   name; never concatenated into any identity index. (R41)
2. **Librarian emits structure, not just prose:** `HALL:` from the closed
   vocabulary (R40) and ≤3 relations per memory into `assert_typed` (R43).
   These feed TACT and the graph — the two indices that are currently
   degenerate — and cost the same call.
3. **Template‑aware separation** at write time for same‑template families
   (R42) — fixes the 16% exact‑Familiar independent of enrichment.
4. **Novelty‑gated compaction** — surprising memories stay raw longer (R44).
5. **Episodes as the linking substrate** — coverage to 100%, co‑retrieval
   used for expansion (R45).
6. **Nothing built on the 7‑dim spectrogram** unless it clears the AUC probe.

## Caveats

- Sources are cited for the principle, not for our numbers; the mapping to
  Spectral is mine and each mapping carries the experiment that would refute
  it.
- HippoRAG's gains are on multi‑hop QA benchmarks over web text; the
  transfer to per‑user memory is a hypothesis (R43 tests it).
- The dual‑process literature is contested at the level of "two processes vs
  one continuum"; the engineering consequence (separate channel, familiarity
  never overrides recollection) holds either way.

## Sources

- Neuronal allocation and sparse coding of episodic memories in the human hippocampus (Sci Rep 2025): https://www.nature.com/articles/s41598-025-21967-7
- Encoding‑scheme‑dependent pattern separation in a DG network (2025): http://www.aimspress.com/article/doi/10.3934/era.2025285
- Adult neurogenesis, feedback inhibition and DG sparseness: https://www.ncbi.nlm.nih.gov/pmc/articles/PMC4542503/
- The hippocampal indexing theory and episodic memory: updating the index (Teyler & Rudy): https://www.semanticscholar.org/paper/43517286be948a72d8f8fe2357450cb25f3a2345
- An Integrated Index: Engrams, Place Cells, and Hippocampal Memory (Neuron 2020): https://www.cell.com/neuron/fulltext/S0896-6273(20)30528-6
- The Hippocampal Engram as a Memory Index (2018): https://www.ncbi.nlm.nih.gov/pmc/articles/PMC6287299/
- HippoRAG (NeurIPS 2024): https://proceedings.neurips.cc/paper_files/paper/2024/file/6ddc001d07ca4f319af96a3024f6dbd1-Paper-Conference.pdf
- From RAG to Memory: Non‑Parametric Continual Learning for LLMs (HippoRAG 2, 2025): https://arxiv.org/pdf/2502.14802
- Adaptive compression as a unifying framework for episodic and semantic memory (Nat Rev Psychol 2025): https://www.nature.com/articles/s44159-025-00458-6
- GENESIS: A Generative Model of Episodic‑Semantic Interaction (2025): https://arxiv.org/pdf/2510.15828
- Intrinsic neural excitability biases allocation and overlap of memory engrams (J Neurosci 2024): https://www.jneurosci.org/content/44/21/e0846232024
- Neuronal competition shapes encoding, consolidation and retrieval of precise spatial memories (Curr Biol 2026): https://www.cell.com/current-biology/abstract/S0960-9822(26)00378-7
- The role of recollection, familiarity, and the hippocampus in episodic and working memory (Yonelinas et al., 2024): https://pmc.ncbi.nlm.nih.gov/articles/PMC10872349/
- The Tolman‑Eichenbaum Machine (Cell 2020): https://www.cell.com/cell/fulltext/S0092-8674(20)31388-X
- Structure abstraction and generalization in a hippocampal‑entorhinal inspired world model (2025/26): https://arxiv.org/pdf/2605.15733
- Topological Neuroscience: Linking Circuits to Function (Annu Rev Neurosci): https://www.annualreviews.org/content/journals/10.1146/annurev-neuro-112723-034315
- TDA and topological deep learning beyond persistent homology — a review (2025): https://arxiv.org/abs/2507.19504
- Robust landmark‑based audio fingerprinting (Ellis): https://www.ee.columbia.edu/~dpwe/LabROSA/matlab/fingerprint/
- Variable‑length audio fingerprinting (2026): https://arxiv.org/pdf/2603.23947
