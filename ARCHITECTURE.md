# WEFT — a flow-native ERP for mid-size industry

> Working name. Weft = the thread that crosses the warp. Goods, money and promises are threads;
> the business is the cloth they make together.

## 0. Target and non-goals

| | |
|---|---|
| Customer | Discrete/process manufacturer or distributor, 150–1,500 staff, 1–5 sites, 5k–50k SKUs |
| Volume | ~1–5 M ledger lines/month, ~500 concurrent users. **Correctness and explainability beat raw scale.** |
| Non-goals | Hyperscale multi-tenant SaaS, ERP-for-everything (no retail POS, no full HCM suite), clone of SAP/Odoo menus |

Design bet: at this size one well-built Postgres and one deployable unit outperform a microservice estate on cost, correctness and speed of change.

---

## 1. Five theses (what makes this not a generic ERP)

Generic ERP = a pile of modules (Sales, Purchase, Inventory, Accounting), each owning tables, synced by batch jobs, shown as menus and grids. WEFT inverts that.

1. **One ledger, many views.** Every business fact is an immutable *Event* in a single operational ledger. Inventory, GL, AR/AP, WIP, cost, KPIs are *projections* of it. Finance is not a silo you post into; it is a lens over the same facts. Books can never disagree with the warehouse because they are derived from the same events.
2. **Promises are first-class.** Orders, POs, work orders, payment promises, delivery slots are *Commitments* with a lifecycle, and Events *fulfil* them. The ERP always knows what the company has promised, to whom, and whether each promise is at risk. Most ERPs only know what already happened.
3. **Bitemporal everything.** Every fact carries *valid time* (when true in the world) and *recorded time* (when we learned it). Backdated corrections, audit, period close and "what did we believe on 31 March?" become queries, not forensic projects.
4. **The company can be forked.** Copy-on-write *Scenarios* branch the ledger for planning, pricing and what-if (MRP, price change, supplier loss, machine down) without touching production, and show the diff in cash, service level and margin.
5. **Rules are data, simulated before shipped.** Approvals, pricing, costing method, credit policy, routing are versioned policy objects. Before publishing, replay the new policy against the last 90 days and see what would have changed.

The UI (section 9) is built on the same ideas: a *Pulse* of at-risk promises instead of a menu, a *time scrubber* instead of reports, a *Why* trace on every number.

---

## 2. Domain model: Resource–Event–Commitment

Primitives (small on purpose; every module is expressed in these):

| Primitive | Meaning | Examples |
|---|---|---|
| **Party** | Someone who can promise or receive | customer, supplier, employee, bank, tax authority |
| **Thing** | Resource: stock item, service, money, machine hour, labour hour | SKU, a CNC-hour, EUR |
| **Place** | Where a Thing is | site, bin, line, in-transit, a bank account |
| **Event** | Immutable fact: Thing × quantity × Place/Party moved at valid time | goods issued, hours logged, invoice posted, cash received |
| **Commitment** | Promise to cause future Events | sales order line, PO line, work order, payment term, maintenance plan |
| **Fulfilment** | Link Event ↔ Commitment (many-to-many, partial allowed) | a shipment fulfils 60% of an SO line |
| **Claim** | Derived obligation/receivable (a Commitment once its reciprocal Event happened) | AR, AP, accrual |
| **Strand** | A causal chain through Events/Commitments | quote → SO → pick → ship → invoice → cash, the thread you can pull and follow |

### Double entry generalised: *triple* balance
Every Event is balanced on three axes, enforced by the database, not by app code:
- **Quantity** per Thing (units in = units out across Places),
- **Value** per currency (debits = credits),
- **Commitment** (a Fulfilment cannot exceed what was promised without an explicit *Variance* event).

GL accounts are a *projection*: a mapping policy turns Events into postings. Change the mapping policy and you can re-derive the books for any period (under Scenario, never silently in production).

### Costing as a projection
Standard, FIFO, weighted-average are interchangeable *cost policies* applied to the Event stream. Landed cost, variance and WIP revalue by re-projection, not by overwriting rows.

---

## 3. System architecture

```
┌──────────────────────────────── EXPERIENCE ────────────────────────────────┐
│  Atelier (web, desktop-class)   Floor (PWA, handheld/kiosk, offline)         │
│  Pocket (mobile approvals)      Public API / EDI / Webhooks                  │
└───────────────┬──────────────────────────────────────────────┬─────────────┘
                │ typed RPC (ConnectRPC) + live queries         │
┌───────────────▼──────────────────────────────────────────────▼─────────────┐
│ EDGE: AuthN (OIDC/SAML/passkeys) · AuthZ (RBAC+ABAC+SoD) · rate limit · tenant│
└───────────────┬────────────────────────────────────────────────────────────┘
┌───────────────▼────────────────────────────────────────────────────────────┐
│                MODULAR MONOLITH  (one Go binary, enforced boundaries)        │
│                                                                              │
│  Commands ──► Domain modules ──► LEDGER KERNEL ──► Outbox                    │
│               Catalog  Parties   (append-only      │                         │
│               Order    Flow       events,          ▼                         │
│               Make     Money      commitments,   Projectors ► Read models    │
│               People   Assets     invariants,     (inventory, GL, AR/AP,      │
│               Quality  Insight    hash chain)     WIP, cost, KPIs, search)   │
│                                                                              │
│  Cross-cutting: Policy Engine · Durable Workflow Engine · Scenario Forker    │
│                 Document Service · Notification · Audit · Extension Host     │
└───────────────┬────────────────────────────────────────────────────────────┘
┌───────────────▼────────────────────────────────────────────────────────────┐
│ PostgreSQL 16 (primary + sync standby, PITR)   Object store (docs, drawings) │
│ NATS JetStream (fan-out, optional)             OpenSearch/pg_trgm (search)   │
│ Redis-free by default. Analytics: DuckDB reading Parquet exports of ledger.  │
└────────────────────────────────────────────────────────────────────────────┘
```

### Key decisions

| Decision | Choice | Why |
|---|---|---|
| Deployment unit | **Modular monolith**, 1 binary, N worker roles via flags | Mid-size ops team (2–4 people) cannot run 25 services. Module boundaries are enforced at compile time (Go `internal/` + arch tests), so splitting later is a mechanical step. |
| Language | **Go** core, **TypeScript** UI | Boring, fast, static, easy to hire/audit. Contracts in Protobuf → generated Go and TS types, so UI and API cannot drift. |
| DB | **PostgreSQL only** for system of record | ACID, RLS, partitioning, JSONB, range types (bitemporal), LISTEN/NOTIFY, logical decoding. One thing to back up. |
| Consistency | Strong inside a module/ledger transaction; **eventual for projections**, with *read-your-writes tokens* | A command returns a ledger position; a query can say "wait until position ≥ N". UI never shows stale after the user's own write. |
| Events vs CRUD | **Event-sourced ledger kernel; plain relational for master data** | Don't event-source a customer address. Do event-source stock moves, postings, commitments. |
| Messaging | Transactional **outbox** → projectors; NATS only for fan-out to external consumers | No dual-write bugs. No broker required for correctness. |
| Workflows | **Durable executor on Postgres** (workflows as checkpointed state machines) | Long-running processes (approval → PO → receive → 3-way match) survive deploys. Temporal is an acceptable swap if the team prefers it. |

---

## 4. Ledger kernel (the foundation)

The only code allowed to write facts. Small (~5k LOC), heavily tested, formally specified.

### 4.1 Schema sketch

```sql
-- append-only; no UPDATE/DELETE grants for the app role
CREATE TABLE ledger_event (
  seq            bigint GENERATED ALWAYS AS IDENTITY,   -- global order
  event_id       uuid PRIMARY KEY,                       -- idempotency key from caller
  tenant_id      uuid NOT NULL,
  kind           text NOT NULL,                          -- 'goods.issued', 'invoice.posted'...
  valid_at       timestamptz NOT NULL,                   -- world time
  recorded_at    timestamptz NOT NULL DEFAULT now(),     -- system time
  strand_id      uuid NOT NULL,                          -- causal chain
  actor          uuid NOT NULL,                          -- who/what caused it
  correction_of  uuid REFERENCES ledger_event(event_id), -- corrections are new events
  payload        jsonb NOT NULL,                         -- schema-versioned, validated
  prev_hash      bytea NOT NULL,
  hash           bytea NOT NULL                          -- sha256(prev_hash || canonical(event))
) PARTITION BY RANGE (recorded_at);

CREATE TABLE ledger_leg (                                -- balanced legs of an event
  event_id   uuid REFERENCES ledger_event,
  axis       text CHECK (axis IN ('qty','value')),
  thing_id   uuid,  place_id uuid,  party_id uuid,
  amount     numeric(28,8) NOT NULL,                     -- signed
  unit       text NOT NULL                               -- UoM or ISO-4217
);
-- deferred constraint trigger: SUM(amount) per (event_id, axis, unit) = 0

CREATE TABLE commitment (
  commitment_id uuid, version int, tenant_id uuid,
  kind text, party_id uuid, thing_id uuid,
  qty numeric, due tstzrange, valid tstzrange, recorded tstzrange,   -- bitemporal
  state text, terms jsonb,
  EXCLUDE USING gist (commitment_id WITH =, valid WITH &&, recorded WITH &&)
);

CREATE TABLE fulfilment (event_id uuid, commitment_id uuid, qty numeric,
  PRIMARY KEY (event_id, commitment_id));
-- deferred trigger: SUM(qty) <= commitment.qty unless a variance event exists
```

### 4.2 Invariants enforced in-DB
Balanced legs; no negative stock unless the Place is flagged backflush-tolerant; fulfilment ≤ commitment; closed period immutable (corrections post to open period with `valid_at` in the past, flagged *restatement*); hash chain unbroken.

### 4.3 Rules of the road
- **Corrections are new events** (`correction_of`), never edits. Reversal and rebooking are one atomic command.
- **Idempotent commands**: caller supplies `event_id`; replays are safe (essential for handhelds and EDI).
- **Schema evolution**: payloads versioned; *upcasters* convert old to new on read. Old events never rewritten.
- **Tamper evidence**: hash chain anchored daily (signed checkpoint written to the object store with object-lock). Auditors get a verifier CLI.
- **Money**: `numeric` + ISO currency, never float. Rounding policy explicit per currency/tax. FX rate is an Event.
- **Units**: UoM conversion table per Thing, with dimensional checks (kg ≠ m).

---

## 5. Modules (bounded contexts)

Each module owns its master data and its command handlers; it may read others only through published query interfaces or events. No cross-module table access (enforced by schema-per-module and CI check).

| Module | Owns | Distinctive behaviour |
|---|---|---|
| **Catalog** | Things, BOMs, routings, UoM, variants | Effectivity-dated BOMs (bitemporal), engineering change as a Scenario you *merge* |
| **Parties** | Customers, suppliers, contacts, credit, price lists | Relationship graph; trust/behaviour score derived from Strands (pays late, ships short) |
| **Order** | Quotes, SO, PO, contracts, ATP/CTP | Promise engine: quotes a *date with confidence* ("Tue, 82%") from live capacity + supplier history, not a static lead time |
| **Flow** | Warehouses, bins, lots, serials, transfers, counts | Continuous cycle counting driven by risk; lot genealogy is just Strand traversal (recall in seconds) |
| **Make** | MRP, work orders, shop floor, scrap, subcontract | Finite-capacity scheduling solver (OR-Tools CP-SAT) over a *Floor Graph* of machines, tools, crews, calendars |
| **Money** | GL, AR, AP, tax, FX, bank, assets, period close | Pure projections + posting policies; continuous close (always-near-closed books); 3-way match as a Strand check |
| **People** | Roster, skills, time, shifts, leave, light payroll export | Skill/qualification as constraints the scheduler respects |
| **Assets** | Machines, maintenance plans, downtime | Maintenance = Commitments against machine-hours; PM competes fairly with production in the scheduler |
| **Quality** | Inspections, NCRs, CAPA, certificates | NCR attaches to Strand; auto-quarantine Events; supplier scorecards feed Promise engine |
| **Insight** | KPI definitions, anomaly detection, forecasts | KPIs are named, versioned queries; every number has a lineage back to Events |

### Cross-cutting services
- **Policy Engine**: policies written in **CEL** (or Rego) over typed facts; compiled, versioned, signed. Used for approvals, pricing, credit, posting maps, costing, reorder rules. *Simulate* mode replays a candidate policy over a ledger window and returns a structured diff. Publishing needs a simulated diff reviewed (four-eyes for finance policies).
- **Workflow Engine**: durable state machines with human tasks, timers, compensation. Definitions are data (YAML/visual), versioned; in-flight instances pin the version they started on.
- **Scenario Forker**: a Scenario is `(base_seq, overlay)`. Overlay is an in-memory/ephemeral Postgres schema of hypothetical events, with projectors run over base+overlay. Merge = replay overlay commands against production with conflict report. Used by planning, pricing, ECO, capex.
- **Document Service**: every document (invoice PDF, label, certificate) is *rendered from* ledger state + template version, hashed, stored. Re-renderable, never "the" source of truth.
- **Extension Host**: tenant extensions run as sandboxed **WASM** modules with a capability manifest (read X, emit Y). They register new fields, policies, UI Lenses, and workflow steps; they cannot bypass the kernel. This is how the customer's quirks live without forks.

---

## 6. Read side

- **Projectors** consume the outbox in `seq` order, are idempotent, and checkpoint their position. Every read model is rebuildable from scratch (`weft projector rebuild inventory --from 0`), which is also the disaster-recovery and migration story.
- **Read models** (Postgres tables, optionally materialised views): stock-on-hand/available/promised by Place × Thing × lot, GL balances, open AR/AP, WIP, margin by Strand, capacity load, KPI facts.
- **Live queries**: the API supports subscribed queries; server pushes row-level diffs over WebSocket/SSE (driven by projector change feed). The UI is reactive without polling.
- **Analytics**: nightly (or streaming) Parquet export of ledger → DuckDB for ad-hoc and heavy analysis; keeps OLTP clean. Insight KPIs can target either.
- **Search**: Postgres FTS + trigram for default; OpenSearch optional above ~10M documents.

---

## 7. Integration

| Need | Approach |
|---|---|
| Public API | ConnectRPC/gRPC + generated REST/OpenAPI, idempotency keys, cursor pagination, ETag |
| EDI (X12/EDIFACT), e-invoicing (Peppol/ZATCA/etc.) | **Anti-corruption gateway** service: translate to canonical Commands; never leak partner formats into domain |
| Banks | Statement import (CAMT.053/MT940) + open-banking connectors → `cash.received` candidate events; reconciliation = proposing Fulfilments (ML-assisted, human-confirmed above threshold) |
| Machines/IoT | MQTT/OPC-UA bridge → telemetry (time-series table, not ledger); only *counts/states that matter* promote into Events |
| Webhooks | Outbox-driven, signed, retried with backoff, replayable by seq |
| Data migration | Staged import with dry-run against a Scenario; opening balances loaded as `opening.*` events with a reconciliation report |

---

## 8. Security, compliance, resilience

- **Identity**: OIDC/SAML SSO, passkeys, SCIM. Service accounts with short-lived tokens.
- **AuthZ**: RBAC for roles, **ABAC** for context (site, cost-centre, amount, customer tier), **Segregation of Duties** matrix checked at command time and at policy publish time. Decision log is audited.
- **Tenancy**: Postgres **RLS** on `tenant_id` always on (even single-tenant installs), so a future shared-cloud offering is a config change, not a rewrite.
- **Data protection**: TLS everywhere, field-level encryption (envelope keys via KMS) for bank, tax IDs, salary; PII tagging drives masking in UI/exports and GDPR erasure (crypto-shredding the PII sidecar; ledger keeps pseudonymous IDs).
- **Audit**: ledger *is* the audit trail; plus an access log (who viewed salary) and a decision log (why was PO approved).
- **Availability**: sync standby + automated failover (Patroni or managed), RPO≈0, RTO<15 min. PITR backups, quarterly restore drills automated. Stateless app tier behind LB, ≥2 replicas.
- **Migrations**: expand → migrate → contract; online; every migration tested against a production-shaped snapshot in CI.
- **Observability**: OpenTelemetry traces with `strand_id` as an attribute (click a slow invoice, see its whole causal trace), RED metrics, projector lag SLO (<1 s p99), structured logs.
- **Quality gates**: property-based tests on the kernel (random Event sequences must preserve invariants), deterministic simulation of projector replays, contract tests for each module, golden-ledger regression (replay a year of synthetic data, hash must match).

---

## 9. Experience layer — *Atelier*

### 9.1 Philosophy
An ERP is where people spend 8 hours. Treat it as a **professional instrument** (like a DAW, Figma, Bloomberg terminal), not a form-filling site. Principles:

1. **Answer before menu.** Home shows what needs attention now, not a grid of module icons.
2. **Every number is explainable.** Hover/press `?` on any figure → *Why trace*: the Strand, Events and policy that produced it.
3. **Time is a control.** A global time scrubber moves the whole UI to any past moment (bitemporal), or forward into a Scenario.
4. **Keyboard-first, mouse-lovely.** Everything reachable from one command bar; everything also direct-manipulable.
5. **Density is a user choice.** Calm / Standard / Dense modes; planners want 60 rows, executives want 6 cards.
6. **Zero dead ends.** From any entity, the relationships are one click away (graph navigation).

### 9.2 Surfaces (the signature screens)

| Surface | What it is | Why it's not a generic screen |
|---|---|---|
| **Pulse** (home) | Ranked feed of *promises at risk*, anomalies, approvals waiting, cash-timing alerts. Each item has an inline resolve action (reschedule, expedite, substitute, approve). | Driven by the Commitment graph and Insight; the ERP tells you where to look |
| **Command Bar** (⌘K) | Natural-language + structured: `ship SO-2231 partial`, `what if supplier ACME slips 2 weeks`, `margin by customer last quarter`. Shows a preview/plan before executing. | Same typed commands as the API; the LLM only *proposes* commands, the user confirms; never writes directly |
| **Strand View** | A horizontal, zoomable timeline of one business thread (quote → ship → cash) with documents, people, deviations pinned. | Replaces ten linked document screens. Lot recall = view the Strand backwards |
| **Flow Map** | Live Sankey/graph of material and cash: suppliers → inbound → stock → WIP → shipped → receivable → bank. Bottlenecks glow; click any band to drill to the Events. | Shows the *shape* of the business; finance and ops see the same picture |
| **Floor Twin** | 2D plant plan (SVG/Canvas, optional 3D) with machines, queues, crews, live status, WIP blocks you can drag between machines; scheduler re-solves in <1 s and shows ghost-plan diff before commit. | Planning as direct manipulation with solver guardrails |
| **Promise Board** | Kanban-meets-Gantt of Commitments grouped by customer/date/risk, with confidence bands and drag-to-renegotiate. | A visible, honest view of the order book |
| **Ledger Studio** | Power-user explorer: filter, pivot, diff, time-travel on raw Events; saved as shareable Lenses. | Finance and auditors get a real instrument, not exports |
| **Policy Lab** | Edit a policy, see simulated diff on last 90 days (affected docs, $ impact), then request publish. | Safe change of business rules without a developer |
| **Scenario Room** | Side-by-side base vs scenario: cash curve, service level, margin, capacity. Merge or discard. | What-if is a core workflow, not a spreadsheet |

Record screens (an invoice, a PO) still exist, but as **Cards**: layered side panels that stack and peek (like a navigable stack), editable in place, with a pinned causal rail on the side.

### 9.3 Visual and motion language
- **Look**: dark-first and light themes from one token set (OKLCH palette, WCAG AA+ contrast verified in CI). Neutral, warm greys; one confident accent; semantic colours reserved for status only. Glass/blur used sparingly for layered Cards, never for text surfaces.
- **Type**: variable font pair (UI sans + tabular mono for numbers); tabular numerals and right-aligned decimals *everywhere*; units and currency as quiet suffixes.
- **Motion**: 120–240 ms spring/ease curves, motion conveys causality (an Event "flies" from Strand to the balance it changed), respects `prefers-reduced-motion`. Never decorative.
- **Charts**: direct labelling, small multiples, uncertainty bands (the Promise engine's confidence), consistent colour roles; no pie charts.
- **Empty/loading/error**: skeletons with real layout, optimistic updates with visible undo toast, errors in plain language with the offending Event/policy linked.
- **Sound/haptics** on Floor (handheld): confirm/err cues, large touch targets (≥56 px), glove mode.
- **Accessibility**: full keyboard map, screen-reader semantics on grids, colour-blind-safe status shapes, i18n/RTL, locale-correct number/date/tax.

### 9.4 Front-end architecture
- **Stack**: TypeScript, React 19, Vite, TanStack Router/Table/Virtual, Radix primitives + own design system (**Loom**, tokens → Tailwind/CSS vars), Canvas/WebGL (PixiJS or deck.gl) for Flow Map and Floor Twin, d3 for scales.
- **Data**: generated typed client from Protobuf; **live queries** over WebSocket; normalised client cache; **local-first** layer (SQLite-WASM/IndexedDB + operation log) for Floor PWA: scans and confirmations queue offline and replay as idempotent commands.
- **Command-driven UI**: every user action is a typed Command with preview → confirm → result; this gives undo, audit and the command bar for free.
- **Extension points**: Lens/panel slots rendered from extension manifests in sandboxed iframes/Shadow DOM with a postMessage capability bridge.
- **Perf budget**: first interactive <1.5 s on mid laptop; 100k-row grid at 60 fps (virtualised); command round trip p95 <150 ms.

### 9.5 Role-shaped workspaces
Same data, different default Workbench: *Planner* (Floor Twin + Promise Board), *Buyer* (Flow Map inbound + supplier trust), *Controller* (Ledger Studio + Policy Lab + close cockpit), *Warehouse lead* (Floor + counts), *Executive* (Pulse + Scenario Room, six numbers, each explainable).

---

## 10. Repository layout (proposed)

```
/kernel            ledger kernel, invariants, hash chain, property tests
/modules/<name>    catalog parties order flow make money people assets quality insight
/engine            policy, workflow, scenario, extension host
/projectors        read-model builders
/gateway           API, auth, EDI/bank/IoT adapters
/proto             Protobuf contracts (source of truth)
/web/atelier       desktop-class web app
/web/floor         PWA for handhelds
/web/loom          design system + tokens
/ops               Terraform/Helm, runbooks, restore drills
/docs/adr          architecture decision records
```

---

## 11. Delivery roadmap

| Phase | Scope | Exit criterion |
|---|---|---|
| **0 — Spine** (6–8 wks) | Kernel, invariants, hash chain, Catalog, Parties, auth, Loom, Command Bar skeleton | Property tests green; replay of 1M synthetic events rebuilds identical read models |
| **1 — Trade** | Order, Flow, Money (AR/AP/GL as projections), Pulse, Strand View | Quote→cash end-to-end for a pilot product line; books reconcile to Flow by construction |
| **2 — Make** | BOM/routing, MRP, shop floor, Floor PWA, Floor Twin, Quality | Pilot plant runs a week on WEFT with offline scanning |
| **3 — Foresight** | Promise engine, Scenario Forker, Policy Lab, Flow Map, Scenario Room | Demo: "supplier slips 2 weeks" shows $ and date impact in <5 s |
| **4 — Ecosystem** | Extension host, EDI/e-invoice, bank AI recon, People/Assets, analytics export | Customer ships a self-built extension without vendor code |

---

## 12. Risks and answers

| Risk | Mitigation |
|---|---|
| Event-sourcing complexity | Confine it to the kernel; master data stays relational; strong tooling (`weft replay`, `weft why`) and property tests |
| Users expect classic ERP forms | Cards preserve familiar record editing; Pulse/Command Bar are additive; role Workbenches ship with sensible defaults |
| Projection lag confuses users | Read-your-writes tokens; UI shows "settling" micro-state only when lag >300 ms |
| Scenario merge conflicts | Merge is replay with explicit conflict report; Scenarios are short-lived by policy |
| LLM safety in Command Bar | LLM output is a *proposed typed Command*; server validates, previews, requires confirmation; never has direct DB access; PII-minimised context |
| Single-binary perceived as legacy | Boundaries enforced by CI; each module has its own contract tests and could be extracted; this is a feature for the 2–4-person ops team |
| Solver unpredictability | Deterministic seeds, time-boxed solves, always show ghost-plan diff before commit |

---

## 13. Open decisions for you

1. Primary vertical (discrete vs process manufacturing changes Make: lot-genealogy/yield vs routings/OEE).
2. Home market(s): drives tax, e-invoicing and localisation scope in Phase 1.
3. Hosting: customer-hosted (k8s/VMs) vs managed cloud single-tenant.
4. Team size/skills: if no Go skills, the same architecture works in Kotlin/Java or Rust; the kernel design is language-independent.
