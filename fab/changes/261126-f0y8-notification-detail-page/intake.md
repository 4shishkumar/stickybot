# Notification detail page

Open an arriving notification with A. Show its title, actor, subtitle (including the repository when present), and body. A opens a supplied HTTP(S) link on the Mac that sent it, holding A reads the event aloud, and B closes. Preserve notification queues, approvals, pairing, and older firmware or hosts.

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | Stack on agent-notify-who | The owner specified PR #25's wire fields. | S:99 R:95 A:99 D:99 |
| 2 | Certain | Validate against ephemeral successful deliveries on the same connection | The Mac must recognize the event and exact link before opening. | S:99 R:95 A:99 D:99 |
| 3 | Confident | Read aloud using existing reply speech with event text from the host | Keeps voice routing and volume consistent. | S:90 R:92 A:90 D:90 |
