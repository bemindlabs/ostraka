# Ostraka — Design Philosophy

**Ostraka 3.0** continues **BWOC 2.x**. The engineering philosophy did not change; the vocabulary did.

BWOC used Pali terms as section names for engineering ideas. That worked as a thinking aid for people who already knew the terms, and worked poorly as a first contact for everyone else — an English-speaking developer reading `saṃvara` has to look it up before they learn anything. Ostraka states the same ideas in words that need no lookup, and names its agents after **civic offices** rather than religious concepts.

The choice is deliberate and narrow: **plain words on the outside, identical engineering on the inside.** Nothing was softened, and nothing was removed.

---

## Why civic vocabulary carries the same idea

The Greek and Roman civic offices Ostraka is named after exist for one reason: **power that checks itself**. An archon held authority for a fixed term. An ephor could put a king on trial. A euthynos examined every magistrate on the way out, whether or not anything had gone wrong.

That is the same structure BWOC expressed as restraint and heedfulness — arrived at independently, by a civilization solving the problem of trusted actors who must not go unchecked. It is exactly the problem an agent orchestrator has.

So the philosophy is not diluted by the translation. It is stated in the register the audience already reads.

---

## The seven principles

| # | Ostraka (plain) | What it means in the code | Was, in BWOC |
|---|---|---|---|
| 1 | **Declare before you act** | Policy, permissions and sandbox mode are stated up front and enforced at the gate — never introduced at runtime | *sīla* — precepts taken before the situation arises |
| 2 | **Nothing merges unreviewed** | Format, lint, test, build, then a diff review — no exception for the most capable agent in the fleet | *appamāda* — diligence, not assuming it is fine |
| 3 | **Stay inside the bounds** | Targeted changes only. No unrequested refactors, no widening scope mid-task | *saṃvara* — restraint, guarding the gates |
| 4 | **Everything is visible** | An action that leaves no trace did not happen. Streams, decisions and approvals are recorded as they occur | *sati* — continuous awareness |
| 5 | **The runtime favors no vendor** | Route on capability, cost and availability — never on habit. The control plane is not itself an agent | *upekkhā* and *anattā* — non-partiality; no self at the center |
| 6 | **Strong defaults, real escape hatches** | Constrain the common path hard, document the exit clearly. Opinionated is not the same as trapped | *majjhima* — the middle way |
| 7 | **Every action traces to who authorized it** | The audit trail is a product surface, not a debug artifact. Action and consequence stay connected | *kamma* — action and result are one chain |

**Read the right-hand column as etymology, not as doctrine.** It records where the ideas came from, for anyone who wants the lineage. Nothing in Ostraka requires knowing it, and no interface, command name, or error message uses those terms.

---

## The assembly

`ostraka` are the potsherds a citizen assembly inscribed its judgments on — the cheap, durable medium that carried a decision and outlived it. That is the shape of the product twice over: independent agents must convene and pass review before anything reaches main, and what they decided survives as a record anyone can re-read.

The word carries the product twice over:

- **What it is:** the medium a fleet's decisions are recorded on, and can be re-read from
- **Where it comes from:** the assembly where a decision is examined before it becomes an act

**It is not an acronym.** An earlier name for this project expanded to "Agent Governance &
Orchestration Runtime"; it was abandoned before release over a trademark conflict, and the
expansion went with it. The identity is the word and the line under it: *Run agent fleets you
can actually review.*

---

## The seven offices

Each agent is a civic office, and the office defines the boundary of what it may do.

| Agent | Office | Function in Ostraka |
|---|---|---|
| 🏛️ **Archon** | Chief magistrate, one-year term, audited on exit | Orchestrator & fleet lead |
| ⚖️ **Ephor** | One of five overseers who supervised the kings | Verification gate & change reviewer |
| ⛵ **Navarch** | Admiral holding a fleet in formation | Parallel fleet execution manager |
| 📯 **Keryx** | Inviolable herald between distrustful parties | Vendor adapter & task routing |
| 📜 **Grammateus** | Public scribe of the city's records | Shared memory & records keeper |
| 📐 **Nomothetes** | Lawmaker who must state a rule publicly | Policy & guardrail author |
| 🔍 **Euthynos** | Auditor who examined every magistrate | Post-run audit & accountability |

Two properties of this roster carry the philosophy structurally, not just in prose:

1. **The orchestrator cannot approve itself.** Archon plans; Ephor gates. Separate offices, separate agents.
2. **The auditor audits the auditors.** Euthynos examines Archon and Ephor too. No office is exempt.

---

## What changed from BWOC 2.x, precisely

**Changed**
- Pali section names → plain English
- Agent theme (Chinese deities) → civic offices
- Public identity: "Buddhist Way of Coding" → Ostraka, *Run agent fleets you can actually review*
- Positioning: a philosophy of coding → a runtime for governed agent fleets

**Unchanged**
- All seven principles, in force and enforced
- The verification gate as a hard requirement
- Worktree isolation for parallel work
- Memory-first, verify-before-acting operating loop
- Backend neutrality and the symlink design

**Kept but relocated**
- The Pali lineage lives here in this document. It is linked from the README, not printed on it.

---

## On dropping the Buddhist framing

Recorded honestly, because the reasoning matters more than the outcome:

The rebrand was originally motivated by a belief that Buddhist framing limits mainstream adoption. **That belief was researched and found unsupported** — no documented case exists of a developer tool losing adoption, failing procurement, or facing rename pressure over religious naming or framing. The clearest counterexample is Anubis, which ships an explicit death-god metaphor in its own repository description and is deployed at GNOME, the Linux kernel infrastructure, FFmpeg and UNESCO with zero name-based objections on record.

The changes that *are* justified by evidence are narrower:

- **Namespace and trademark.** crates.io is first-come-first-serve with name transfers no longer mediated, and common-law trademark rights attach to freely distributed open source with no registration required. A coined, unowned name is defensible; a common word is not.
- **Functional clarity.** The one identity-adjacent requirement in any widely used OSS health checklist is that the project must describe what it does in minimal jargon. A doctrinal tagline standing alone arguably fails that; a functional one beside it passes at no cost.

So: the top line is functional because a checklist asks for it, and the name is coined because the registry demands it. **Neither reason required hiding where the ideas came from** — which is why this document exists and is linked rather than quietly retired.

---

*Lineage: BWOC 2.x → Ostraka 3.0 · [`bwoc-framwork`](https://github.com/bemindlabs/BWOC-Framework)*
