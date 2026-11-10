# Context actions: what the buttons do on each screen

The stick has two buttons (A on the front, B on the side) and a motion sensor. That is not
much, so a screen can give them its own meaning, and says so in a hint bar at its bottom.
This page lists every screen's controls today and proposes more screen-specific ones.

- **Part 1** is what firmware 2.6.0 does (implemented).
- **Part 2** is a proposal: nothing in it is built yet.

## How it works (2.6.0)

Each screen declares a table of bindings in `m5-stick-s3/src/logic/actions.cpp`: an input (A
click, hold A, B click, hold B, tilt, shake, ...), the action, and the hint text. A binding can
depend on the screen's state (flags): the reply's "A: next" only shows when the reply is longer
than the screen, "hold A: listen" only when the Mac app can read aloud, "stop" while it reads.
The hint bar is made from the same table, so the hints can't drift from what the buttons do.
`lookup(screen, input, flags)` returns `NONE` for inputs a screen doesn't claim, and the caller
then does what that input always does. That is how the global gestures stay global.

**Global gestures** (the same on every screen that doesn't own the buttons outright: games,
the remotes, the Launcher, the settings menu, an approval):

| Input | Action |
|---|---|
| hold B | talk to the Mac agent (from Home, the agent screens and the reply) |
| double B | type the Mac password (unlock) |
| triple B | wake the screen; awake: the Launcher |
| shake | Escape on the Mac (off in settings); on an agent screen it closes / cancels instead |
| lift | wake the screen |
| face down 3 s | standby |

The screens that differ from these are marked **differs** in the tables.

## Part 1: today (firmware 2.6.0)

Abbreviations: **A** = a click of A, **hold A** = held past 0.5 s, **B** = one click of B
(decided after the double-click window), **2×B / 3×B** = double / triple click, **hold B** =
held past 0.6 s, **A+B** = both pressed together.

### Home (the face)

| Input | Action | Hint |
|---|---|---|
| A (press, held = held) | the key combo (dictation in Wispr, or your shortcut) | A dictate |
| B | Enter | B enter |
| 2×B | unlock the Mac | |
| 3×B | the Launcher | 3×B apps |
| hold B | talk to the agent | hold B talk |
| A+B | nothing | |
| tilt | the eyes follow it (faces) | |
| shake | Escape + a face toy | |
| flick L/R | pages: Home / Agent / Status | |

The hint bar shows for 4 s after the screen wakes, over the bottom of the face.

### Talking (B held)

| Input | Action | Hint |
|---|---|---|
| let go of B | send it to the agent | let go send |

### Agent: thinking / working

| Input | Action | Hint |
|---|---|---|
| B or shake | cancel the request, Home | B cancel |
| A | the key combo (as on Home) | |
| hold B | talk (the agent answers "still working") | |
| flick | nothing (**differs**: no page change) | |

### Agent: error

| Input | Action | Hint |
|---|---|---|
| B or shake | close, Home | B close |
| hold B | talk again | hold B again |

### Reply view (the answer, new in 2.6.0) **differs: A is the reader's**

| Input | Action | Hint | When |
|---|---|---|---|
| A | next page; after the last page, the top | A next | longer than the screen |
| hold A | the Mac reads the answer aloud | hold A listen | the Mac app can (`"speak":true`) |
| hold A | stop reading | hold A stop | while it reads |
| tilt forward / back | scroll down / up, faster the further | tilt scroll | longer than the screen |
| B or shake | close (also stops the reading), Home | B close | |
| hold B | talk (a follow-up question) | hold B ask | the reply fits one screen |
| 2×B, 3×B | unlock, Launcher (global) | | |
| flick | nothing | | |

Tilt: the angle when the reply opens (and after each A) is "level"; past a 12° dead zone it
scrolls at 0.3 lines/s per degree, at most 10 lines/s. Small changes in how it's held (inside
the dead zone) are followed slowly, so it doesn't creep. The reply stays 60 s after the last
touch, and while it is being read aloud.

### Other screens today (not on the action tables yet)

These own the buttons outright (their own code, their own hints):

| Screen | A | hold A | B | 2×B / 3×B | hold B | A+B | tilt / twist | shake | flick |
|---|---|---|---|---|---|---|---|---|---|
| Approval | approve | | deny | | | | | deny (if on) | |
| Question (ask) | option A | | option B | | | | | dismiss | |
| Launcher grid | next tile | | open | | back to Home | | | | prev / next page |
| Quick settings | next | | change | | back to grid | | | | |
| Faces picker | next | | use it | | back to grid | | | | |
| Settings menu | next item | | change / select | | fast scroll on "Key" | | | | |
| Games: picker | next game | | play | | Home | | | | |
| Games: playing | the game's button | | pause | | pause | | the tilt games | | |
| Air mouse | left button (hold = drag) | | right click | | + tilt: scroll | tap: pause; hold: Home | move the pointer | | |
| Presenter | next slide | laser pointer | previous slide | 2×B: black screen | restart the timer | tap: pause timer; hold: Home | | | |
| Media remote | play / pause | | mute | | | hold: Home | twist: volume | | next / prev track |
| IR remote: remotes | open | | next | 2×B: previous | Home | | sideways: scroll | | |
| IR remote: buttons | send (hold: repeat) | | next | 2×B prev, 3×B favourite | remotes | | | | |
| Event (merged PR...) | dismiss | | dismiss | | | | | | |
| Pairing code | accept | | reject | | | | | | |

## Part 2: proposal (not implemented)

Principles:

1. **Hold B is always talk** wherever the screen doesn't own the buttons outright (games,
   remotes, menus, approvals keep theirs). A talk from any context should get that context
   (see *talking in context* below).
2. **A+B is the free chord.** Outside the remotes it does nothing today. It is the natural
   "screen action" key: one per screen, always shown in the hint bar.
3. **Every screen shows its hints.** The other screens move onto the action tables, so their
   hints come from the same place as their buttons.
4. **A stays the key combo on Home.** Dictation is the stick's first job; only screens that
   are "reading" (reply, approvals, menus) take A.

| Screen | Today | Proposed |
|---|---|---|
| **Home** | A combo, B Enter, 2×B unlock, 3×B Launcher, hold B talk, shake Esc, flick pages | **A+B: reopen the last reply** (the reply view, from the stored answer). With media playing (below): the card shows the track; **A+B: play / pause** instead, and the hint says so. Nothing else changes. |
| **Reply** | A next, hold A listen / stop, tilt scroll, B close, hold B ask | **A+B: copy the answer** to the Mac's clipboard (`{"evt":"copy"}`, the Mac app sets the pasteboard) or "send to the open app" (types it); **2×A: previous page** (costs a click delay on A; maybe only with long replies). Hold B: the follow-up carries "about your last answer" so Claude keeps the thread (it already does with the session; make it explicit in the hint: "hold B: follow up"). |
| **Agent working** | B / shake cancel | **hold A: listen** to the answer so far (the same speak path, `what:"progress"`); **A+B: open on the Mac** (the Claude session or terminal in front). |
| **Approval** | A yes, B no, shake deny | **hold A: always allow this tool** (the hook's `allow` with "remember" if Claude Code offers it; else "yes for this session"); **A+B: show the whole command** (the reply view with the hint text, scrollable: approvals cut long commands today); **hold B: answer by voice** ("yes but only in src/"), sent as the decision's message. |
| **Launcher** | A next, B open, hold B back, flick page | **A+B: the last app** (reopen what was used last); hints from the table. |
| **Games** | own buttons | unchanged (games need the buttons); show "hold B: pause" in the picker's hints. |
| **Air mouse** | A left, B right, hold B + tilt scroll, A+B pause / Home | unchanged; the hint bar replaces the static help lines, from the table. |
| **Presenter** | A next, B prev, hold A laser, 2×B black, hold B timer, A+B | **shake: black screen** (2×B too slow mid-talk); the timer's target settable from the stick (hold A+B on the timer). |
| **Media remote** | twist volume, flick tracks, A play / pause, B mute | **with Now Playing from the Mac**: the title / artist / app on the screen, the play state honest (today A toggles blind); **hold A: "what's playing?"** read aloud; **tilt: seek** (slow) when paused. |
| **IR remote** | own buttons | **A+B: power** of the open remote (the most used button); hints from the table. |
| **Settings menu** | A next, B change | unchanged; hints from the table (today a static line). |
| **Faces picker** | A next, B use | **tilt: preview** the selected face live (the demo frame) before choosing. |
| **Event screens** | any press dismisses | **A: open it on the Mac** (the PR / the CI run: the agent opens the URL), B: dismiss. |

### Talking in context

Hold B works from Home, the agent screens and the reply. The talk message could say where it
was held: `{"evt":"talk","state":"start","ctx":"reply","rid":7}` or `"ctx":"media"`. The agent
then prefixes the request ("About your last answer: ...", "While Spotify plays Dua Lipa: ...")
so "skip this one", "louder", "read that again", "shorter please" just work. Cheap: one field in
the talk message, a line in the prompt; older agents ignore it.

### "Media playing": how the stick would know

The stick can't see what the Mac plays. The Mac app has to tell it, e.g. in the hello and on
change: `{"src":"agent","media":{"state":"playing"|"paused"|"none","app":"Spotify","title":"…",
"artist":"…"}}` (≈150 bytes, only when it changes: no cost on the link). The question is how
the Mac app learns it:

| Way | What it covers | Cost / risk |
|---|---|---|
| **MediaRemote** (private framework: `MRMediaRemoteGetNowPlayingInfo` + the change notification) | everything that shows in Control Center's Now Playing: Music, Spotify, Safari / Chrome tabs, Podcasts, VLC... | Private API. Reported broken for third-party apps since macOS 15.4 (Apple limited it to its own processes): `nowplaying-cli` and similar return nothing there. Workaround in use by others: `mediaremote-adapter`, which loads a helper through `/usr/bin/perl` (Apple-signed, still allowed) and streams JSON on change; works today, could break with any macOS update. Event-driven, ~0 CPU idle. |
| **Distributed notifications** (`com.spotify.client.PlaybackStateChanged`, `com.apple.Music.playerInfo`) | Spotify and Music only | Public, no permission prompt, event-driven (PyObjC `NSDistributedNotificationCenter` / Rust `objc2`): ~0 CPU. Misses browsers and other players. |
| **AppleScript polling** (`tell application "Spotify" to player state`) | Spotify, Music (and a few others with a dictionary) | An Automation permission prompt per app; ~30-60 ms CPU per `osascript` run, every 2-5 s; must not launch the app (check it's running first). |
| `nowplaying-cli` | as MediaRemote | as MediaRemote (broken on 15.4+ for the same reason); an extra install. |

**Recommendation**: distributed notifications for Spotify and Music first (public, free, no
prompt; covers the common case), then optionally `mediaremote-adapter` behind a setting for
"everything else" with a clear "may stop working after a macOS update" note. The stick only ever
sees the `media` message, so the source can change without a firmware change. Control works
without any of this (the media keys over Bluetooth HID already go to whatever plays); Now
Playing only adds the state and the title.

What "media playing" would change on the stick (proposal): Home's card shows the track (the
agent's card already rotates; Now Playing pins it like an approval), **A+B: play / pause** on
Home while it plays, and the media remote screen shows the title and the real state.

### Costs and risks

- Every context action is one more thing to remember; the hint bar is the answer, so every
  screen needs one. On portrait Home the bar covers the face's bottom card for 4 s.
- A double-click on A (2×A) delays A's single click by the click window (~300 ms): only on
  screens where A isn't time-critical (the reply, never Home).
- The chord A+B needs both pressed within ~80 ms; the remotes already use it, so it's learnt.
- Each new Mac → stick message is ignored by older firmware (no `"state"` → a hello), each new
  stick → Mac event by older Mac apps: mixing versions loses the new action, nothing else.
