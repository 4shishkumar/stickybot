# Specifications Index

> **Specs are pre-implementation artifacts** — what you *planned*. They capture conceptual design
> intent, high-level decisions, and the "why" behind features. Specs are human-curated,
> flat in structure, and deliberately size-controlled for quick reading.
>
> Contrast with [`docs/memory/index.md`](../memory/index.md): memory files are *post-implementation* —
> what actually happened. Memory files are the authoritative source of truth for system behavior,
> maintained by `/fab-continue` (hydrate).
>
> **Ownership**: Specs are written and maintained by humans. No automated tooling creates or
> enforces structure here — organize files however makes sense for your project.

| Spec | Description |
|------|-------------|
| [compatibility](compatibility.md) | Old and new work together: the app and the stick update separately, in any order, and keep working. |
| [safe-updates](safe-updates.md) | Updating a stick is safe to try: nothing personal is rewritten, the old firmware stays until the new one is proven. |
| [faces](faces.md) | Faces anyone can make, that can't hurt a stick: data only, reviewed, signed, no telemetry. |

Plans that live elsewhere in `docs/`:

| Doc | What is planned there |
|-----|-----------------------|
| [ROADMAP](../ROADMAP.md) | The setup website, the web edition and the companion app, in phases. |
| [CONTEXT-ACTIONS](../CONTEXT-ACTIONS.md) | What the buttons do on each screen today, and the proposal for actions that follow context (Part 2). |
| [faces/PLAN](../faces/PLAN.md) | The steps for face packages and the catalog. |
