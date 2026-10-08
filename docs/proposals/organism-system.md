# Organism system (pawn health): evaluation and design proposal

**Status:** proposal, integrated into the specs as Roadmap v4.8, Blueprint v3.2 and Design Document v3.3 (decision D-038). Items marked **[CONFIRM]** are open questions for you; none of them blocks Stage 1.

**Source:** your outline (`organismoutline.txt`, 2026-10-08). This document keeps your intent, says where I think the outline needs adjusting to fit the game's determinism, performance and modding rules, and turns it into a staged build.

## 1. Verdict

The idea fits the project well. A body made of a few simulated essential systems (blood, oxygen, pressure, pain, consciousness) with death only at irreversible brain failure gives the simulation explainable stories ("she passed out from blood loss, her pressure fell, the clinic was too far") instead of hit points, and it is the kind of system a mod community can extend. It matches the existing rules: integer maths, data first, scripts supplement and never write state, everything explainable from reason codes.

The risks are scope, tuning and cost, not feasibility. My recommendations, in order of importance:

1. **Other systems read capacities, never body parts.** The organism publishes a handful of derived integer capacities (consciousness, moving, manipulation, talking, eating, breathing, ...). Movement, the scheduler, conversation and actions read only those. This is what keeps the body model from leaking into every system, and it lets us ship a simple version of the same interface in Stage 1 (§5).
2. **Pack effects are declarative, with Luau only at the edges.** A script per effect per pawn per tick is too expensive (Stage 0 measured about 12 microseconds per call) and too hard to bound. Effects defined in data run natively; scripts hook onset, escalation and outcome (§6).
3. **Acute physiology needs its own time scale.** At the default pace one game minute is one real second, so a realistic bleed-out lasts seconds of real time. We need a clinical time scale decision before tuning anything (§4, question 2).
4. **Build it in its own stage after Stage 2,** not inside Stage 1 or squeezed into Stage 8 (§8).
5. **Parts exist whatever the content rating; the rating and filter only control what is shown** (§7). This keeps saves independent of a presentation setting, as Design Document section 9 already requires for events.

## 2. What I understood, in one page

- **Body:** parts (hands, arms, chest, stomach, head, legs, feet, and the mature-rating parts), organs (brain, heart, throat, left and right lung, liver), skeleton (spine for now). Parts carry **attributes**: innate ones (hair, hair colour) and afflictions (injuries, conditions). Attributes have a type, the parts they may attach to, a name and description, optional sub-attributes and type-specific constants (an injury's `InjuryType`). Static attributes are plain data; dynamic ones are ticked by the organism through a **Status Effect Registry (SER)** at a fixed rate, never independently.
- **Vitals, not hit points:** blood volume, heart rate, blood pressure, breathing, oxygenation (blood and per organ), temperature, pain, adrenaline, nerve, toxicity, consciousness. Death only when the brain has irreversibly stopped.
- **Blood:** volume from body mass and sex; loss lowers oxygen delivery and energy; blood types with transfusion compatibility and an acute hemolytic reaction on mismatch.
- **Organs:** each has oxygenation and efficiency. Lungs move oxygen into blood and can fill with fluid; the heart pumps and is regulated by the brain; the brain is the death marker and its failure has wide effects; the liver clears toxins; the throat carries breath and speech and can be blocked.
- **Pain and adrenaline:** pain sums afflictions; adrenaline dampens it; a per-pawn threshold causes pain shock and unconsciousness; nerve modifies the fight-or-flight outcome (which belongs to psychology, Stage 7, with a placeholder before).
- **UI and API:** a health window from inspection or a button; everything extensible through the API.

## 3. Evaluation against the architecture

| Topic | Assessment | Recommendation |
| --- | --- | --- |
| Determinism | Fine if every quantity is an integer with a fixed unit. Your pressure formula uses ratios; ratios become permille fixed point. | Unit table in section 4. Golden physiology traces under replay on all three systems. |
| Death rule | "Only from cessation of brain activity" is workable and realistic. The outline also says resuscitation can happen "after brain death for a while", which conflicts with Roadmap decision 13 (permanent death, resurrection debug only). | Resolve with explicit states: alive, unconscious, critical, arrest (clinical death: no pulse or breath, brain still recoverable, resuscitation window running), dead (irreversible). The window is the time you described; once it passes, death is final. **[CONFIRM]** (question 4) |
| Pressure formula | As written, `BP = V/Vmax x (1 + 2(BPM - rest)/rest)` rises with heart rate, so a bleeding pawn with a racing heart would have *higher* pressure than a healthy one, and the parenthesis is ambiguous. Real shock is the opposite: the heart races to compensate, then pressure collapses. | Use a small cardiac-output model: output = rate x stroke volume; stroke volume falls with blood volume and heart efficiency; pressure = output x vessel tone (adrenaline raises tone). Compensation first, collapse later, with the same inputs you listed. **[CONFIRM]** (question 3) |
| Blood volume | Your anchors (43 kg: 3.5 L, 68 to 82 kg: 4.5 to 5.5 L, 113 kg: 7.0 L) are not a constant ml per kg (about 81, 66 and 62), which matches real life. | Interpolate through your anchors (integer, piecewise linear) with a data-driven physiology profile factor, instead of one ratio. |
| Cost | About 25 parts and a dozen vitals per pawn, about 200 pawns (stretch 500). Most pawns are healthy most of the time. | Healthy pawns are **dormant** (no per-step work beyond a slow baseline); only pawns with an active effect or out-of-range vital are stepped, and acute ones more often. Budget: under 5 percent of the tick at 200 pawns; per-pawn work is independent, so it parallelises with ordered application like pathfinding. |
| Persistence | Sparse attribute rows plus a vitals row per pawn; both hashed per row (S-023), saved canonically. | New tables in a schema bump with a migration; healthy pawns store almost nothing extra. |
| Strings | "Name" and "Description" that "show values" must be localisable and filter-aware. | Attributes carry string-table keys with arguments, and each description has filter variants and an intensity tier, like event types (Design Document section 9). No literal text in data. |
| Scripts and mods | "Define any code within them" collides with the sandbox cost model and with "scripts never write state". | Section 6: declarative effect programs plus bounded hooks; packs can never set death, write vitals directly or exceed clamped magnitudes. |
| Mental conditions | Panic attacks and similar fall under the roadmap rule: restrained, accurate, calm, never shorthand for danger. | Keep them in Stage 7 with the psychology core; Stage 2B only has the placeholder (adrenaline and a stress input). Outside subject-matter review is already on the tuning list. **[CONFIRM]** (question 12) |
| Fiction | Roadmap decision 13: no real-world instruction. Poisons, drugs and treatments must not become recipes or dosing guidance. | Substances in content are fictional or generic, defined by effects, never real compounds with real doses. |
| Mature parts | Breasts and genitalia "for the mature setting" would tie a saved body to a presentation setting. | Parts exist per the pawn's physiology profile regardless of rating; the rating and the global filter decide whether the health window lists or describes them. They have no art in the abstract visuals. **[CONFIRM]** (question 5) |

## 4. The model, concretely (proposed)

**Units (all integers).** Volume in millilitres; heart and breath rate per minute; pressure as mean arterial pressure in mmHg (systolic and diastolic are derived for display); temperature in tenths of a degree Celsius; every ratio, level and efficiency in permille (0 to 1000) unless stated.

**Vitals per pawn:** blood volume (ml), blood oxygen saturation, heart rate, stroke volume, mean pressure, breath rate, tidal efficiency, core temperature, pain (total and felt), adrenaline, nerve, toxicity load, consciousness (0 to 1000), brain function (0 to 1000), plus an "anoxic exposure" counter that drives irreversibility.

**Organs and parts:** each has `integrity` (structural damage) and organs also `oxygenation` and `efficiency` (a function of both). Lungs add `fluid` (permille full). The throat has `patency`. The spine has `integrity` per segment (see question 13); its damage maps to loss of moving and manipulation capacity.

**One organism step** (fixed order, so results do not depend on iteration order):

1. SER effects due this step add their modifiers (blood loss, pain, toxin, fluid, temperature, and so on).
2. Haemodynamics: blood volume, stroke volume, heart rate (set by the brain, adrenaline, pressure feedback), pressure.
3. Respiration: breath rate and lung function from airway patency, fluid and ambient air; saturation moves toward what the lungs allow.
4. Delivery and organ oxygenation: delivery = output x saturation x oxygen-carrying capacity (proportional to blood volume); each organ moves toward delivery over demand with its own time constant; brain demand is high.
5. Organ efficiency, liver clearance of toxicity, heart autopilot if the brain cannot regulate, temperature drift.
6. Pain and adrenaline: felt pain = total pain x (1 - adrenaline factor) as you specified; pain at or above the pawn's threshold causes pain shock (unconscious). Moving on a broken leg adds temporary pain that decays when the action stops.
7. Consciousness from brain oxygenation, pain shock and cranial trauma.
8. Capacities (section 5), then the state machine (alive, unconscious, critical, arrest, dead).

**Cadence and time scale.** The base step is one game minute (10 ticks); pawns in an acute state step more often (every 2 ticks). Rates are authored per organism minute and divided by a world-level **clinical time scale** so a bleed-out can last game hours rather than real seconds if you want that. At 27x everything runs at the same game-time rates, so scale only changes how fast stories unfold relative to the clock. **[CONFIRM]** (question 2)

**Reasons.** Every vital change records a reason code (which attribute or effect, and the dominant input), so the health window and `pg organism explain <pawn>` can say why a pawn is where she is, in the same way schedules already can.

## 5. Capacities: the integration point

Derived each step, permille, read by everything else:

| Capacity | Used by |
| --- | --- |
| consciousness | scheduler (an unconscious pawn cannot act; replans, asks for help), actions, conversation, possession |
| moving | movement speed (`movement.speed_modifier` already exists), pathing, task feasibility |
| manipulation | actions that handle objects |
| talking | conversation participation; a pawn who cannot talk is still observed, never dropped |
| eating, breathing, blood pumping | needs restore efficiency, the organism itself |
| sight, hearing (later) | perception, overhearing |

**Stage 1 forward step (cheap, proposed):** needs and mood in Stage 1 already need a way to slow an exhausted pawn or stop a collapsed one. Instead of hard-coded penalties, 1.1 defines the `Capacities` interface and fills it from needs (critical energy lowers moving and consciousness; critical hunger lowers manipulation). When the organism lands, it becomes the producer of the same interface and the Stage 1 shortcut is deleted (the same pattern as D-009). The needs hooks and the capacity contract are then stable for mods from API 0.1.

**Needs versus organism.** Hunger, energy and social stay as player-readable needs. Starvation, exhaustion and dehydration are organism conditions that start when a need stays critical: your "starving" and "exhausted" are conditions, not moods. Moods stay emotional only (section 9).

## 6. The Status Effect Registry and the API

**Two kinds of attribute, as you specified.** Static attributes are plain typed data, never ticked. Dynamic attributes name a registered effect.

**Effect programs (the extensibility answer).** A registered effect is data: a cadence (in organism steps), an optional condition on vitals or attributes, a list of bounded vital contributions (additive or multiplicative on named vitals, rates per organism minute), optional transitions (when severity passes X attach attribute Y, or end), and reason text keys. The organism executes these natively, in one place, in a fixed order. This covers bleeding, flu, poison, panic input, dehydration, drowning, and nearly everything in your outline without entering Luau.

**Script hooks, bounded.** Scripts may register attribute types, parts-with-attributes and effect programs at load time (capability `organism.define`), read vitals and capacities (`organism.read`), request interventions as validated commands (`organism.apply`: add or remove an attribute, apply a treatment), and subscribe to health events. Optional hooks run at low cadence in batches (S-034): `organism.onset`, `organism.escalate`, and `organism.vital_modifier` for clamped adjustments. Constraints that keep the existing guarantees: a pack cannot set death or write a vital directly; every contribution is clamped to engine bounds; pack effects must supply description variants for every filter level and an intensity tier or the pack is rejected; costs are metered and shown in the script cost view.

**Type-specific constants.** An attribute type declares required fields through the existing parameter schema (the same machinery as settings and components): an `Injury` type requires `injury_type` from a registered enumeration, which effect programs can branch on ("this injury type gets the bleeding attribute at this rate, scaled by pressure and heart rate"). Packs can add injury types and attribute types.

## 7. Body, parts and content rating

- Parts are a small tree per physiology profile; each carries innate attributes and afflictions. A profile also supplies body mass, height and the sex-physiology factors used for blood volume; it is data, so custom pawns (Stage 5) and mods can define their own. **[CONFIRM]** (question 6)
- Innate appearance attributes (hair, colour) are static attributes that the renderer and pawn creator read; they live with the body so one structure serves health and appearance.
- Mature-rating parts: present in the model per profile; the rating and the filter govern listing and description only, and the abstract visuals never draw them.
- Injury targeting uses weighted hit tables per part, as data.

## 8. Placement in the roadmap

| Stage | Organism work |
| --- | --- |
| 1 (forward step) | `Capacities` interface filled from needs (milestone 1.1); inspector health stub (needs and capacities); needs hooks stable. |
| **2B (new, after Stage 2)** | **Organism core:** vitals and the step, blood and oxygen, the SER and effect programs, parts and attributes, pain and adrenaline, consciousness and capacities, starvation, exhaustion and an illness, minimal hazards (see question 7), the health window, API 0.2b (`pg.organism`), fight-or-flight placeholder (adrenaline and a stress input only), soak scenarios with physiology goldens. |
| 4 (possession) | Health window reachable from the possessed pawn's UI; player sees felt pain and consciousness. |
| 5 | Physiology profile in the pawn creator (mass, height, profile, blood type). |
| 6 | Interiors, jobs and items supply hazards and treatments (food, beds, tools). |
| 7 | Fight-or-flight and mental conditions in the psychology core; brain damage effects on traits and decisions; blood type inheritance for births; aging effects on baselines. |
| 8 | Injury and crime events call the organism; clinic and emergency response; treatment (first aid, transfusion, resuscitation, surgery as scoped); death record and non-graphic event. |
| 11 | API 1.0 for `pg.organism`; tooling (effect lint, plausibility checks, health-window polish). |

Why after Stage 2: it needs the full scheduler (an unconscious pawn replans), API 0.2 (systems and actions, so treatment actions exist), and it has no dependency on town generation or the editor. Why not earlier: Stage 1 is already the largest stage and its value is the observation loop. Why not wait for Stage 8: starvation and exhaustion consequences, the health window and the extension API are valuable for mods and for testing long before crime exists, and the biggest risk (tuning) is better met with a long runway.

## 9. Moods versus conditions (decision for Stage 1)

Moods cover only the emotional range. Proposed list for 1.1, tuned in play: cheerful, content, neutral, bored, uneasy, sad, upset, lonely. "Exhausted", "starving" and similar states are organism conditions; until Stage 2B they are need levels that feed capacities (section 5). Mood rules may read conditions and pain once they exist.

## 10. Testing and tooling for the organism

- Physiology golden traces (vitals over time for a healthy pawn, a bleed with and without care, drowning, starvation, a fever), identical on all three systems and at 1, 2 and N threads.
- Property tests: blood volume and levels stay in range; no negative or overflowing values; a pawn with no active effects does no work; transfusion conserves volume; compatible and incompatible matches behave as defined; death is irreversible once the window passes; scripts cannot exceed clamps.
- Hostile-pack cases for the new API (huge magnitudes, effect loops, missing description variants).
- Developer features: `pg organism sim` (vitals over time with reasons), `pg organism explain`, the overlay's Organism tab (stepped pawns, cost, SER buckets), developer injure and heal tools (already in the Stage 11 tool list).
- Soak: 30 days, 200 pawns with hazards on, shadow verification, bounded attribute counts.

## 11. Suggestions (new, proposed in `docs/SUGGESTIONS.md` as S-040 to S-047)

S-040 capacities layer in Stage 1; S-041 declarative effect programs; S-042 dormancy and acute stepping with a cost budget; S-043 clinical time scale; S-044 physiology goldens and a plausibility lint for pack effects; S-045 parts exist regardless of rating; S-046 blood type inheritance for births; S-047 ambient hazards service (temperature, air, water) for weather, indoors and outdoors.

## 12. Questions for you

Numbered so you can answer by number. My default is in brackets; I proceed with the default if you say "defaults".

1. **Placement.** New Stage 2B after Stage 2 as proposed (section 8)? [yes] Alternatives: fold it into Stage 8, or build it right after Stage 1.
2. **Clinical time scale.** Should acute events (bleeding, suffocation) run at realistic proportions to the game clock (a bleed-out in game minutes, seconds of real time at 1x), slowed so emergencies last game hours, or a per-world setting with a default in between? [per-world setting, default about 10x slower than literal, so a severe bleed takes tens of game minutes]
3. **Pressure model.** Replace the single formula with the cardiac-output model in section 4 (heart rate x stroke volume x vessel tone)? [yes; I keep your inputs]
4. **Death states.** Confirm the reading: arrest is clinical death with a resuscitation window (CPR, defibrillation, adrenaline); the window is shorter when the brain is already damaged and longer in the cold; after it, death is permanent and resurrection stays a developer tool. [yes]
5. **Mature parts.** Parts always exist per physiology profile; rating and filter control only listing and wording; no art. [yes]
6. **Physiology profile.** Blood volume and baselines come from a data-defined profile (mass, height, a sex-physiology factor, fitness) rather than a hard sex switch, so custom pawns and mods can define others. How should the pawn creator describe it to players? [a plain "body type" and "sex" choice mapped to a profile; wording is yours]
7. **Harm before Stage 8.** What may hurt pawns in Stages 2B to 7? Candidates: starvation, exhaustion, illness (flu, food poisoning), kitchen and work accidents, falls, drowning (water exists in town maps), cold and heat (needs weather and seasons, which are not in the roadmap), smoke or fire. [illness, starvation, exhaustion, and a small set of accidents; no weather, fire or drowning until you say so]
8. **Care.** In 2B, can pawns treat themselves and each other (rest, bandage, medicine items) as ordinary actions, or is treatment player-driven only until the clinic in Stage 8? [pawns self-treat and help each other with simple care; clinics and surgery later]
9. **Pain totals.** Sum of all afflictions risks pain shock from many tiny injuries. Use diminishing aggregation (strongest source counts fully, the rest at half, then a quarter) with per-pawn thresholds? [yes]
10. **Speech.** A pawn who cannot talk (throat, brain damage, unconscious) still takes part in the town: conversations skip or use gestures, never an error. [yes]
11. **Extensibility.** Declarative effect programs plus bounded, batched hooks (section 6) instead of per-tick Luau per effect? [yes]
12. **Mental health.** Keep panic and anxiety as Stage 7 content built with outside review, with only an adrenaline and stress placeholder in 2B? [yes]
13. **Anatomy scope.** Start exactly with your list. Do you want eyes and ears (sight and hearing capacities), the neck, and spine segments (cervical, thoracic, lumbar for different paralysis) now, or later as extensions? [spine in segments now; eyes and ears later; neck folded into the throat]
14. **Species.** Is the model for people only, or should the profile idea leave room for animals later (pets, wildlife)? [people only, profile structure left open]
