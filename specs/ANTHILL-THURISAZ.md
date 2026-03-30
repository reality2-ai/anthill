# ANTHILL-THURISAZ: Bayesian Epistemology and Confidence Dynamics

| Field       | Value                                                        |
|-------------|--------------------------------------------------------------|
| Version     | 0.1 Draft                                                    |
| Date        | 2026-03-30                                                   |
| Status      | Draft                                                        |
| Depends on  | ANTHILL-KNOWLEDGE                                            |
| Related     | TH-WEAVE, ANTHILL-RUMINATION                                 |

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in [RFC 2119][rfc2119].

[rfc2119]: https://www.ietf.org/rfc/rfc2119.txt

---

## 1. Introduction

All knowledge in Anthill is conjectural. There are no facts -- only
conjectures with varying degrees of confidence. The Thurisaz epistemic
engine implements Karl Popper's critical-rationalist epistemology
structurally: confidence is earned through surviving genuine refutation,
not through accumulation of confirming instances. This specification
defines the mathematical foundations and algorithms that enforce that
principle.

### 1.1 Philosophical Foundation

**Popper's Conjecture and Refutation.** Science does not proceed by
induction. A hypothesis gains corroboration not by counting confirmations
but by withstanding sincere attempts to disprove it. A single decisive
refutation outweighs any number of confirming observations.

**Agrippa's Trilemma.** Every chain of justification must end in one of
three unsatisfying ways: infinite regress, circular reasoning, or
dogmatic assertion. Classical foundationalism picks dogmatic assertion
(axioms). Coherentism picks circularity. Both are inadequate.

**Fading Foundations.** Anthill resolves the trilemma by allowing
foundations to fade. A belief does not require an unshakeable ground
truth; it requires only that its supporting evidence has not yet decayed
to insignificance. Given enough well-supported links in a justificatory
chain (each with conditional probability > 0.5), the influence of the
ground link fades exponentially, and total justification converges to a
stable limit determined by the chain structure itself -- not by the
ground. This follows the model of Peijnenburg & Atkinson (2017).

---

## 2. Log-Odds Representation

A conforming implementation MUST represent belief states internally in
log-odds space.

### 2.1 Definitions

Given a confidence probability p in (0, 1):

    L = ln(p / (1 - p))

And the inverse (sigmoid):

    p = 1 / (1 + e^(-L))

### 2.2 Clamping

Log-odds MUST be clamped to the interval [-6.9, 6.9], corresponding to
probabilities [0.001, 0.999]. An implementation MUST NOT represent
certainty (p = 0 or p = 1). The input to `to_log_odds` is clamped to
[0.001, 0.999] before computation.

### 2.3 Sequential Bayesian Update

Given a current log-odds L and a Bayes factor BF > 0:

    L_new = clamp(L + ln(BF), -6.9, 6.9)

If BF <= 0, the update MUST be a no-op (invalid Bayes factor).

### 2.4 Rationale

Log-odds are preferred over raw probabilities because:

1. Sequential updates are additive: each piece of evidence contributes
   ln(BF), regardless of the current state.
2. Numerical stability: raw probabilities suffer from floating-point
   precision loss near 0 and 1; log-odds remain well-behaved.
3. Symmetry: positive and negative evidence are treated uniformly.

---

## 3. Evidence Types and Bayes Factors

A conforming implementation MUST support the following twelve evidence
types, each with its predefined base Bayes factor. All Bayes factors are
subsequently reputation-adjusted per Section 4.

| # | Evidence Type          | Base BF | Direction | Meaning                                                      |
|---|------------------------|---------|-----------|--------------------------------------------------------------|
| 1 | RefutationSurvived     | 2.5     | Positive  | Actively tried to disprove; claim held                       |
| 2 | RefutationFailed       | 0.1     | Negative  | Actively tried to disprove; claim failed                     |
| 3 | CompetitionWon         | 2.0     | Positive  | Won head-to-head evaluation against rival hypothesis         |
| 4 | Corroboration          | 2.0     | Positive  | Supporting evidence found in another source                  |
| 5 | PatternTransfer        | 1.8     | Positive  | Cross-domain insight strengthens this idea                   |
| 6 | HumanAttestation       | 1.5     | Positive  | User confirmed or corrected                                  |
| 7 | Consistency            | 1.5     | Positive  | Consistent with existing knowledge graph                     |
| 8 | Synthesis              | 1.2     | Positive  | Transitive inference from two strong edges                   |
| 9 | InconsequentialSearch   | 1.0     | Neutral   | Searched for counter-evidence but found nothing (NO change)  |
|10 | Inconsistency          | 0.4     | Negative  | Inconsistent with existing knowledge graph                   |
|11 | CompetitionLost        | 0.3     | Negative  | Lost head-to-head evaluation against rival hypothesis        |
|12 | Contradiction          | 0.3     | Negative  | Contradicting evidence found                                 |

An implementation MAY also define an `Unknown` type with BF = 1.0 as a
catch-all for unrecognised evidence type strings during deserialization.

### 3.1 Ordering Rationale

RefutationSurvived carries the highest positive weight (2.5) because
surviving active disproof is the gold standard of Popperian
corroboration. RefutationFailed carries the most devastating negative
weight (0.1) -- a failed disproof attempt is near-decisive. The neutral
InconsequentialSearch (BF = 1.0) is critical: absence of evidence is NOT
evidence of absence (see Section 5.4).

---

## 4. Reputation-Weighted Evidence

Every Bayes factor MUST be adjusted by the reputation of the evidence
source before application.

### 4.1 Adjustment Formula

    BF_adjusted = BF_base ^ (0.5 + 0.5 * r)

Where r is the source reputation score, clamped to [0.0, 1.0].

The exponent w(r) = 0.5 + 0.5 * r ranges from 0.5 to 1.0:

| Reputation r | Exponent w(r) | Effect on BF_base = 2.0 |
|--------------|---------------|-------------------------|
| 0.0          | 0.50          | 2.0^0.5 = 1.414        |
| 0.5          | 0.75          | 2.0^0.75 = 1.682       |
| 0.7          | 0.85          | 2.0^0.85 = 1.807       |
| 1.0          | 1.00          | 2.0^1.0 = 2.000        |

Reputation can only attenuate evidence strength, never amplify it beyond
the base BF.

### 4.2 Source Reputation Registry

Source reputations are persisted in `memory/reputation.json`. Each source
entry tracks:

- `score`: current reputation [0.0, 1.0]
- `category`: source classification (see below)
- `first_seen`: Unix timestamp of first encounter
- `last_updated`: Unix timestamp of last reputation change
- `corroborations`: count of confirmed claims
- `contradictions`: count of disproved claims

### 4.3 Source Categories and Initial Reputations

| Category          | Initial Reputation | Description                           |
|-------------------|--------------------|---------------------------------------|
| Document          | 0.5                | A file or document being analysed     |
| AiInference       | 0.5                | AI-generated inference                |
| User              | 0.7                | Human user (benefit of the doubt)     |
| ThematicAnalysis  | 0.5                | Thematic analysis output              |
| Mcp               | 0.6                | MCP tool call                         |
| Ant               | 0.6                | Peer ANT in the colony                |
| Unknown           | 0.3                | Unrecognised source                   |

### 4.4 Reputation Updates

Source reputation itself is updated via Bayesian updating:

- **Corroboration** (a claim from this source is confirmed by another):
  likelihood ratio = 1.5. From p = 0.5, one corroboration shifts
  reputation to approximately 0.6.

- **Contradiction** (a claim from this source is disproved):
  likelihood ratio = 0.5. From p = 0.5, one contradiction shifts
  reputation to approximately 0.33.

### 4.5 Reputation Decay

Source reputation decays toward 0.5 (neutral) with a half-life of 7
days. The decay formula operates on the offset from neutral:

    offset(t) = (score - 0.5) * 2^(-elapsed / half_life)
    score(t) = 0.5 + offset(t)

Decay is applied lazily on access, not continuously. The result is
clamped to [0.01, 0.99].

---

## 5. Anti-Confirmation Bias Mechanisms

AI systems exhibit a strong tendency toward confirmation bias. A
conforming implementation MUST enforce all four of the following
structural countermeasures. These operate at the mathematical level, not
merely through prompt engineering.

### 5.1 Evidence Diversity Ceiling

An edge's confidence MUST be capped by the number of distinct evidence
types present in its evidence log:

| Distinct Evidence Types | Maximum Confidence |
|-------------------------|--------------------|
| 0 -- 1                  | 70%                |
| 2                       | 85%                |
| 3                       | 92%                |
| 4 or more               | 99%                |

If a Bayesian update would push confidence above the ceiling, the
implementation MUST clamp confidence to the ceiling and recompute
log-odds from the clamped value.

**Rationale.** Real strength comes from diversity of evidence, not
quantity. An edge with ten corroborations but no refutation attempts is
weaker than an edge with three corroborations and two survived
refutations. This ceiling prevents monotonic confirmation from reaching
high confidence.

### 5.2 Consecutive-Confirmation Dampening

If the last 5 or more entries in the evidence log are all positive
(BF > 1.0), and the current update is also positive, the confidence
increase MUST be dampened.

The dampening formula:

    confidence_dampened = confidence_before + 0.3 * (confidence_after - confidence_before)

That is, only 30% of the computed increase is retained. The log-odds
MUST be recomputed from the dampened confidence.

**Rationale.** Real knowledge encounters friction. A long unbroken
streak of confirmations suggests the system is not genuinely testing --
it is merely accumulating agreement.

### 5.3 Confirmation Bias Detection

An implementation SHOULD warn when an edge's evidence trail is
suspiciously one-sided: all positive entries with zero negative entries,
or a high positive rate combined with low evidence type diversity.

### 5.4 Inconsequential Search (BF = 1.0)

Searching for counter-evidence and finding nothing MUST NOT strengthen a
belief. The InconsequentialSearch evidence type has BF = 1.0 (no change
to log-odds). This is a deliberate and critical design choice.

Absence of evidence is NOT evidence of absence. Only active, failed
refutation (RefutationSurvived, BF = 2.5) strengthens a claim. The
distinction is:

- "I looked for counter-evidence and found none" --
  InconsequentialSearch, BF = 1.0. No update.
- "I found specific counter-evidence and tested the claim against it;
  the claim withstood the test" -- RefutationSurvived, BF = 2.5.
  Significant positive update.

---

## 6. Belief Decay (Fading Foundations)

All beliefs decay toward p = 0.5 (maximum uncertainty, log-odds = 0)
over time unless reinforced by fresh evidence. Decay is toward
uncertainty, not toward disbelief.

### 6.1 Decay Formula

    log_odds(t) = log_odds(t_0) * 2^(-elapsed / half_life)

Where:
- `t_0` is the time of the last evidence update
- `elapsed` is the time since `t_0` (in seconds)
- `half_life` is determined by the decay category (in seconds)

As elapsed approaches infinity, log_odds approaches 0 and confidence
approaches 0.5.

### 6.2 Decay Categories

Each edge is assigned a decay category that determines how quickly it
fades without reinforcement:

| Category    | Half-Life | Half-Life (seconds) | Example                                     |
|-------------|-----------|---------------------|---------------------------------------------|
| Fact        | 30 days   | 2,592,000           | "Anthill is written in Rust"                |
| Decision    | 14 days   | 1,209,600           | "We chose petgraph over SurrealDB"          |
| Observation | 7 days    | 604,800             | "Alfred is running v0.4.0"                  |
| Inference   | 3 days    | 259,200             | "This architecture seems scalable"          |
| Assumed     | 1 day     | 86,400              | "The user probably wants X"                 |

An `Other` or unrecognised category MUST default to Fact (30 days).

The decay category MAY be inferred from the edge's basis:
- `observed` -> Observation
- `told` -> Fact
- `inferred` -> Inference
- `assumed` -> Assumed

### 6.3 Resolution of Agrippa's Trilemma

Fading foundations resolves the trilemma by rejecting the premise that
justification requires a fixed endpoint. In an infinite chain of
probabilistic links (each with conditional probability > 0.5), the
remainder term -- the product of all conditional probabilities beyond
link n -- fades toward zero as n grows. Total justification converges
to:

    limit = p / (2p - 1)    for uniform link confidence p > 0.5

Clamped to a maximum of 0.99 (certainty is never reached).

For p = 0.7: limit = 0.7 / 0.4 = 1.75, clamped to 0.99.
For p = 0.5: no convergence (limit undefined; chain provides no net
justification).

### 6.4 Chain Confidence Computation

For a justificatory chain of link confidences [c_1, c_2, ..., c_n]:

1. **Classify each link:** Deductive (c >= 0.95) or Probabilistic
   (c < 0.95).

2. **All-deductive chain (Exceptional Class):** return the simple
   product c_1 * c_2 * ... * c_n. Ground dominates.

3. **Chains with probabilistic links:**
   a. Compute the naive product of all links.
   b. Compute the average probabilistic link confidence (excluding
      deductive links).
   c. Compute the FF convergence limit from the average.
   d. Compute the blend weight:

          convergence_weight = clamp(1 - e^(-0.3 * (n_prob - 1)), 0, 0.95)

      Where n_prob is the number of probabilistic links. This produces:
      - n = 1: 0% convergence, 100% product
      - n = 5: approximately 50/50
      - n = 10: approximately 90% convergence

   e. Blend:

          result = product * (1 - convergence_weight) + convergence_limit * convergence_weight

   f. For mixed chains, multiply by the deductive product.

4. Clamp result to [0.01, 0.99].

---

## 7. Darwinian Competition

Competing hypotheses are organised into competition groups.

### 7.1 Competition Groups

An edge MAY carry a `competition_group` identifier (string). All edges
sharing the same non-empty competition group are rival hypotheses
attempting to explain the same phenomenon.

### 7.2 Head-to-Head Evaluation

When competition is triggered (during rumination or consolidation), the
system evaluates rival hypotheses in the same group:

- The **winner** receives CompetitionWon evidence (BF = 2.0).
- The **loser** receives CompetitionLost evidence (BF = 0.3).

Multiple rounds are permitted. Over time, the stronger hypothesis
accumulates fitness while the weaker one fades.

### 7.3 Interaction with Decay

Competition results are subject to the same decay rules as all other
evidence. A hypothesis that won a competition 30 days ago gains less
from that victory than one that won yesterday. Sustained fitness
requires sustained testing.

---

## 8. Beneficial Impact

Each edge carries a `beneficial_impact` score in [-1.0, 1.0]:

- **1.0**: Strongly beneficial for people and planet.
- **0.0**: Neutral (default).
- **-1.0**: Harmful.

### 8.1 Fitness Modifier

The beneficial impact produces a fitness modifier applied during
relevance scoring:

    fitness = 1.0 + 0.2 * beneficial_impact

This yields a range of [0.8, 1.2]:

| beneficial_impact | fitness |
|-------------------|---------|
| -1.0              | 0.80    |
| -0.5              | 0.90    |
|  0.0              | 1.00    |
|  0.5              | 1.10    |
|  1.0              | 1.20    |

### 8.2 Design Constraint

This is NOT censorship. Harmful ideas are not suppressed; they are
subject to a fitness disadvantage in relevance ranking. An idea with
negative impact must be proportionally stronger (higher confidence and
importance) to achieve the same relevance as a neutral or beneficial
idea. All ideas remain in the graph and are retrievable.

---

## 9. Relevance Scoring

The relevance score determines which edges surface in prompts and query
results.

### 9.1 Formula

    relevance = confidence * importance * fitness * network_bonus * citation_bonus

Where:

- **confidence**: current probability from the Bayesian engine [0.001, 0.999]
- **importance**: how central this edge is to the project/user [0.0, 1.0],
  growing with reference count via: importance = 0.5 + 0.5 * (1 - 1/(1 + references/10))
- **fitness**: 1.0 + 0.2 * beneficial_impact (see Section 8.1)
- **network_bonus**: 1.0 + 0.1 * corroboration_strength (see Section 10)
- **citation_bonus**: 1.0 + 0.15 * avg_citation_quality if citations
  exist; 1.0 if the edge has no citations. No penalty for uncited edges
  -- an edge that has survived refutation is valid regardless of whether
  it carries formal citations.

### 9.2 Minimum Prompt Threshold

Edges with confidence below 0.15 (15%) MUST NOT appear in the prompt
context. They remain in the graph for audit and historical purposes.

---

## 10. Corroboration Strength

Corroboration strength measures how well an edge is supported by its
network neighbourhood.

### 10.1 Computation

For each node in the graph, compute the average confidence of all edges
connected to that node. For a given edge (src -> tgt):

    corroboration_strength = (avg_confidence(src) + avg_confidence(tgt)) / 2

Where avg_confidence(node) is the mean confidence of all edges incident
to that node.

Corroboration strength is clamped to [0.0, 1.0] and is recomputed
during graph consolidation.

### 10.2 Effect

The network bonus provides a mild boost (up to 10%) for well-connected,
high-confidence edges:

    network_bonus = 1.0 + 0.1 * corroboration_strength

An isolated edge with no network support has corroboration_strength near
the default (0.5 from a single connection), while an edge embedded in a
cluster of high-confidence relationships receives a stronger bonus.

---

## 11. Reference Quality

Each edge MAY carry citations (references to external sources). Each
reference has a quality score [0.0, 1.0] that starts based on the
reference type and evolves over time.

### 11.1 Reference Types and Initial Quality

| Reference Type  | Initial Quality | Description                     |
|-----------------|-----------------|---------------------------------|
| PeerReviewed    | 0.8             | Peer-reviewed scientific paper  |
| OfficialReport  | 0.7             | Government or official report   |
| Book            | 0.7             | Textbook or monograph           |
| Personal        | 0.5             | Personal communication          |
| News            | 0.5             | Reputable news source           |
| Website         | 0.4             | General web source              |
| AntKnowledge    | 0.6             | Knowledge from a peer ANT       |
| Blog            | 0.3             | Blog post or opinion piece      |
| AiInference     | 0.3             | AI-generated without source     |

---

## 12. Evidence Record Structure

Each evidence entry in the log MUST record:

| Field              | Type          | Description                                        |
|--------------------|---------------|----------------------------------------------------|
| date               | String (ISO)  | When this evidence was observed                    |
| evidence_type      | EvidenceType  | One of the 12 types from Section 3                 |
| test               | String        | What was tested or observed                        |
| detail             | String        | The evidence itself                                |
| source_id          | String        | Links to the reputation registry                   |
| source_reputation  | f64           | Source reputation at time of evidence (for audit)  |
| bayes_factor       | f64           | The effective BF applied (after reputation adj.)   |
| log_odds_before    | f64           | Log-odds before this update                        |
| log_odds_after     | f64           | Log-odds after this update                         |

This provides a complete audit trail: any observer can replay the
evidence log from a neutral prior (log_odds = 0, p = 0.5) and arrive at
the current belief state.

---

## 13. Justificatory Chain

Each edge MAY carry a justificatory chain answering "why do I believe
this?" Each step records:

| Field      | Type   | Description                                      |
|------------|--------|--------------------------------------------------|
| step       | u32    | Step number in the chain                         |
| process    | String | What process produced this evidence              |
| confidence | f64    | Confidence at this point in the chain            |
| source     | String | Source identifier (e.g. "document:README.md")    |

---

## 14. Conformance

A conforming implementation of ANTHILL-THURISAZ:

1. MUST represent belief states in log-odds space with clamping to
   [-6.9, 6.9].

2. MUST implement sequential Bayesian updating via
   L_new = clamp(L + ln(BF), -6.9, 6.9).

3. MUST support all 12 evidence types with the base Bayes factors
   defined in Section 3.

4. MUST apply reputation-weighted adjustment to all Bayes factors via
   BF_adjusted = BF_base ^ (0.5 + 0.5 * r).

5. MUST implement all four anti-confirmation bias mechanisms:
   a. Evidence diversity ceiling (Section 5.1)
   b. Consecutive-confirmation dampening (Section 5.2)
   c. Confirmation bias detection/warning (Section 5.3)
   d. Inconsequential search neutrality (Section 5.4)

6. MUST implement belief decay toward p = 0.5 using the formula in
   Section 6.1, with category-specific half-lives per Section 6.2.

7. MUST support Darwinian competition groups with CompetitionWon and
   CompetitionLost evidence types.

8. MUST support beneficial impact as a fitness modifier in relevance
   scoring.

9. MUST NOT represent certainty: probabilities are always in
   (0.001, 0.999).

10. SHOULD implement chain confidence using the Fading Foundations
    blend model (Section 6.4).

11. SHOULD persist the full evidence log for audit and replay.

---

## 15. Test Vectors

The following test vectors allow implementors to verify correctness.
All values are rounded to three decimal places.

### 15.1 Log-Odds Conversion

| Probability p | Log-odds L      |
|---------------|-----------------|
| 0.500         | 0.000           |
| 0.100         | -2.197          |
| 0.250         | -1.099          |
| 0.750         | 1.099           |
| 0.900         | 2.197           |

### 15.2 Single Bayesian Update from Neutral Prior

Starting from p = 0.5 (L = 0.0):

| Evidence Type        | BF (r=1.0) | L_after       | p_after |
|----------------------|------------|---------------|---------|
| RefutationSurvived   | 2.500      | 0.916         | 0.714   |
| RefutationFailed     | 0.100      | -2.303        | 0.091   |
| Corroboration        | 2.000      | 0.693         | 0.667   |
| CompetitionWon       | 2.000      | 0.693         | 0.667   |
| PatternTransfer      | 1.800      | 0.588         | 0.643   |
| HumanAttestation     | 1.500      | 0.405         | 0.600   |
| Consistency          | 1.500      | 0.405         | 0.600   |
| Synthesis            | 1.200      | 0.182         | 0.545   |
| InconsequentialSearch| 1.000      | 0.000         | 0.500   |
| Inconsistency        | 0.400      | -0.916        | 0.286   |
| CompetitionLost      | 0.300      | -1.204        | 0.231   |
| Contradiction        | 0.300      | -1.204        | 0.231   |

### 15.3 Reputation-Adjusted Update

Starting from p = 0.5 (L = 0.0), applying Corroboration (BF_base = 2.0):

| Reputation r | w(r) | BF_adjusted | L_after | p_after |
|--------------|------|-------------|---------|---------|
| 0.0          | 0.50 | 1.414       | 0.347   | 0.586   |
| 0.5          | 0.75 | 1.682       | 0.520   | 0.627   |
| 0.8          | 0.90 | 1.866       | 0.625   | 0.651   |
| 1.0          | 1.00 | 2.000       | 0.693   | 0.667   |

### 15.4 Belief Decay

Starting from p = 0.9 (L = 2.197), decay category = Observation
(half-life = 7 days):

| Elapsed     | L_decayed | p_decayed |
|-------------|-----------|-----------|
| 0 days      | 2.197     | 0.900     |
| 7 days      | 1.099     | 0.750     |
| 14 days     | 0.549     | 0.634     |
| 30 days     | 0.138     | 0.534     |
| 100 days    | 0.000     | 0.500     |

### 15.5 TH-WEAVE Section 10.1 Relay Example

This worked example from the TH-WEAVE specification serves as an
integration test:

1. **Prior:** p = 0.5, L = 0.0
2. **Breadcrumb from X** (BF_base = 3.0, r = 0.5):
   w(0.5) = 0.75, BF_adj = 3.0^0.75 = 2.28, L = 0.824, p = 0.69
3. **Delivery confirm from B** (BF_base = 3.0, r = 0.7):
   w(0.7) = 0.85, BF_adj = 3.0^0.85 = 2.63, L = 1.791, p = 0.86
4. **Refutation survived** (BF = 2.5):
   L = 2.707, p = 0.94
5. **After 2 hours decay** (half-life = 1 hour, 2 half-lives):
   L = 2.707 * 2^(-2) = 0.677, p = 0.66

### 15.6 Evidence Diversity Ceiling

An edge with only Corroboration evidence (1 type) is capped at 70%.
Even with 20 corroborations pushing raw Bayesian confidence to 99.9%,
the effective confidence is 70%.

After adding one RefutationSurvived (2 types), the ceiling rises to 85%.
After adding one Consistency (3 types), the ceiling rises to 92%.
After adding one HumanAttestation (4 types), the ceiling rises to 99%.

### 15.7 Fading Foundations Convergence

For a uniform probabilistic chain with link confidence 0.7:

| Chain Length | Naive Product | FF Blended Confidence |
|--------------|---------------|-----------------------|
| 1            | 0.700         | 0.700                 |
| 2            | 0.490         | 0.490                 |
| 5            | 0.168         | > 0.50                |
| 8            | 0.058         | > 0.50                |

The FF convergence limit for p = 0.7 is 0.7 / (2*0.7 - 1) = 1.75,
clamped to 0.99. Long chains converge toward this limit rather than
collapsing to zero.

---

## 16. Security Considerations

The reputation system is a trust model, not a security boundary.
Reputation scores are locally computed and locally stored. In a
federated deployment, reputation MUST NOT be blindly imported from
remote colonies without independent verification.

---

## 17. References

- Popper, K. R. (1963). *Conjectures and Refutations: The Growth of
  Scientific Knowledge*. Routledge.

- Peijnenburg, J. & Atkinson, D. (2017). *Fading Foundations:
  Probability and the Regress Problem*. Springer.

- TH-WEAVE: Thurisaz Weave Protocol Specification, Sections 3.2, 4.1,
  7.1, 8.3, 10.1.
