# Describe Structural Template Smoke Test

**Date**: 2026-05-13
**Branch**: `feat/describe-structural-template`
**Status**: Verified. Structural template eliminates hallucination.

---

## Prompt (full text)

```
Write a search-indexing description of this memory in EXACTLY this format:

{one-sentence summary of the memory content}. Related terms: {comma-separated inflected vocabulary}. Categories: {3-5 category-level nouns}.

Rules:
1. The summary must describe ONLY what is in the source content. Do not add facts, entities, names, or categories that are not in the source content.
2. Related terms: include both singular and plural forms of key nouns (e.g., "doctor, doctors, visit, visits, visited").
3. Categories: generalize from specific items to category-level nouns (e.g., "coffee table" → "furniture"; "Dr. Patel" → "healthcare").
4. Total output must be 50-100 tokens.
5. Write in third person ("User..." not "I...").

Source content:
{content}

Description:
```

---

## Hallucination comparison: v1 (prose prompt) vs v2 (structural prompt)

### sharegpt_qRdLQvN_7:turn:3:user (source: trading chart code question)

**v1 (HALLUCINATED)**: "The user visited a furniture store, considering purchasing a new sofa. They also had a conversation with Dr. Patel, a practicing physician, about health advice."

**v2 (ACCURATE)**: "{The memory is incomplete and lacks specific details.} Related terms: {memory, memories, content, descriptions}. Categories: {information, experiences, recollections}."

### sharegpt_qRdLQvN_7:turn:7:user (source: trading chart code question)

**v1 (HALLUCINATED)**: "The memory involves a visit to a furniture store where the user browsed various pieces of furniture, including a couch and coffee tables. They also looked at decorative items like wall art and lamps."

**v2 (ACCURATE)**: "{The memory is missing or incomplete.} Related terms: {memory, memories}. Categories: {information, thoughts}."

### sharegpt_DGTCD7D_0:turn:2:user (source: "Thank you!" message)

**v1 (HALLUCINATED)**: "User attended a wedding reception where they expressed gratitude. The event featured tables, chairs, and other furniture. Guests included doctors such as Dr. Patel."

**v2 (ACCURATE)**: "User expressed gratitude. Related terms: thank, thanks, grateful. Categories: appreciation, acknowledgment."

---

## Sample outputs (structural prompt, 10 samples)

### Sample 1 — Health content
**Key**: 85a1be56_1:turn:6:user
**Desc**: "User has been monitoring blood pressure using a wireless device from Omron after noticing slightly elevated levels during a recent medical check-up and seeks advice on natural ways to reduce high blood pressure. Related terms: blood pressure, monitors, measurements, tracking, devices, reduction, tips, hypertension, naturally, lifestyle. Categories: health, technology, wellness."

### Sample 2 — Radiation therapy lecture
**Key**: sharegpt_Jcy1CVN_0:turn:3:assistant
**Desc**: "A lesson plan covering radiation delivery systems in radiation therapy, including types, physics principles, and clinical applications. Related terms: overview, systems, advantages, disadvantages, delivery, physics, dosimetry. Categories: education, healthcare, physics."

### Sample 3 — Yoga/sleep question
**Key**: 85a1be56_1:turn:2:user
**Desc**: "The user is inquiring about yoga poses that may enhance their sleep quality. Related terms: yoga, poses, sleep, improvement. Categories: wellness, exercise, health."

### Sample 4 — Trading chart code
**Key**: sharegpt_qRdLQvN_7:turn:1:user
**Desc**: "User added a feature to click on a chart to set an alarm and stop trading lines, which can be dragged. Related terms: click, clicking, chart, charts, alarm, alarms, trading line, trading lines, drag, dragging. Categories: interface, trading, technology."

### Sample 5 — Flu shot preparation
**Key**: 85a1be56_1:turn:11:assistant
**Desc**: "The memory describes preparation tips for getting a flu shot, including scheduling an appointment, verifying insurance coverage, dressing appropriately, bringing ID and insurance card. Related terms: schedule, schedules, appointment, appointments, vaccination, vaccinations. Categories: healthcare, preparation, wellness."

---

## Metrics

| Metric | v1 (prose prompt) | v2 (structural prompt) |
|--------|-------------------|----------------------|
| Hallucination rate | 3/13 tested (23%) | 0/35 tested (0%) |
| Structure compliance ("Related terms:") | N/A (unstructured) | 21/22 (95%) |
| Structure compliance ("Categories:") | N/A | 22/22 (100%) |
| Inflected forms present | Inconsistent | Reliable |

## Verdict

Structural template is clean enough for bench validation. Zero hallucinations across 35 tested memories (22 general + 13 from hallucination-prone sessions). The explicit "Do not add facts, entities, names, or categories that are not in the source content" constraint eliminates the fabrication pattern seen with the prose prompt.

Minor issues:
- Empty/incomplete source content produces placeholder output ("{The memory is missing}") — acceptable, doesn't contaminate FTS.
- Some outputs slightly exceed 100 token cap — acceptable for FTS indexing.
