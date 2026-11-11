# Feedback ("Report an issue")

StickyBot.app has a **Feedback** page (in the dashboard's sidebar, and **Report an issue…** in the
menu bar menu and the app's Help menu). People pick *Bug*, *Idea* or *Question*, say what happened
(and for a bug what they expected and the steps), optionally leave a name or email, and can tick
**Attach diagnostics**.

Where it goes: a **Google Sheet** the team shares. In front of it sits a small **Google Apps Script
web app** bound to the Sheet ([`docs/feedback/apps-script.gs`](feedback/apps-script.gs)). The app
POSTs one JSON report to the web app's `/exec` address; the script checks it and adds a row. There
is no key or token in the app (the app is public): the web app takes anonymous posts and can only
add rows.

Until the address is set (see 3.), the page says *"Feedback isn't set up yet"* and offers
**Copy report** and **Email it instead**.

## 1. Create the Sheet and the web app (the owner, once)

1. In Google Drive: **New → Google Sheets**. Name it e.g. *StickyBot feedback*.
2. In the Sheet: **Extensions → Apps Script**. Delete the sample code, paste the whole of
   [`docs/feedback/apps-script.gs`](feedback/apps-script.gs), and save (name the project e.g.
   *StickyBot feedback*).
3. Optional check: pick `doGet` in the function list and **Run** once. Google asks you to authorize
   the script (it only needs this spreadsheet and its own cache); allow it.
4. **Deploy → New deployment**. Gear icon → **Web app**.
   - Description: `v1`
   - **Execute as: Me** (the script writes to *your* Sheet; reporters need no Google account)
   - **Who has access: Anyone** (not "Anyone with a Google account": the app can't sign in)
   - **Deploy**, then copy the **Web app URL**, which ends in `/exec`:
     `https://script.google.com/macros/s/AKfy…/exec`
5. Open that URL in a browser: it should say `{"ok":true,"service":"stickybot-feedback"}`. The
   *Feedback* sheet and its header row appear with the first report:

   | Received | ID | Type | What happened | Expected | Steps | Contact | App | Firmware | macOS | Stick | Diagnostics | Status | Area | Owner | Fix/Release | Notes |
   |---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|

6. **Share the Sheet** (Share button) with the teammates who triage, as *Editor*. Don't share the
   Apps Script project with people who shouldn't change it. Never make the Sheet public: reports can
   hold a contact address.

## 2. What the script does

- `doGet`: the health check above; it shows no data.
- `doPost`: parses the JSON `{v, id, install, type, what, expected, steps, contact, app, fw, macos,
  stick, diagnostics, created, hp}`, and refuses anything that isn't a report: `v` must be 1, `id`
  `SB-XXXXXX`, `install` 16 hex digits, `type` Bug / Idea / Question, `what` not empty, text fields
  at most 4 000 characters (contact, versions 200), diagnostics at most 30 000.
- `hp` is a honeypot the app always sends empty; a post with it filled is answered "ok" and dropped.
- At most 10 reports an hour per install (a random id the app makes; nothing about the Mac) and 300
  an hour overall (CacheService). The same report ID twice (the app retrying after a timeout that
  had gone through) is stored once.
- Appends the row under a script lock; text that starts with `= + - @` is stored as text, never as a
  formula. Answers `{ok: true, id}` or `{ok: false, error}`.

Apps Script answers a POST with a redirect to `script.googleusercontent.com`, where the answer is
read with a GET; the app follows it that way.

## 3. Give the address to the app

Either:

- **Built in** (for a release): put the `/exec` URL in `DEFAULT_URL` in
  `desktop/crates/stickybot-server/src/feedback.rs` and release. Every copy of the app then reports
  there.
- **Per Mac** (to try it first): set `"feedback_url": "https://script.google.com/macros/s/…/exec"` in
  `~/.stickybot-rs/config.json` (quit the app first), or
  `curl -X PUT -H "X-Stick-Token: $(cat ~/.stickybot-rs/token)" -H 'Content-Type: application/json' -d '{"feedback_url": "…/exec"}' http://localhost:8670/api/config`.
  Only `https://script.google.com/macros/…/exec` addresses (or `http://localhost`, for a stand-in)
  are accepted.

## 4. Updating the script

Edit the code in the Apps Script editor, then **Deploy → Manage deployments → (your deployment) →
Edit (pencil) → Version: New version → Deploy**. That keeps the same `/exec` URL, so the apps out
there keep working. (**Deploy → New deployment** would make a *new* URL that no app knows.)

Keep `docs/feedback/apps-script.gs` in this repo the same as what's deployed.

## 5. Triage (the team, weekly or so)

The script fills *Status* with `New`; the last columns are for the team:

| Column | Use |
|---|---|
| **Status** | `New` → `Triaged` → `In progress` → `Fixed` / `Won't fix` / `Duplicate` / `Answered` |
| **Area** | `Stick firmware`, `Mac app`, `Voice`, `Claude`, `GitHub watcher`, `Setup`, `Docs`, … |
| **Owner** | who takes it |
| **Fix/Release** | the PR or issue link, and the version it ships in (e.g. `v2.5.0`) |
| **Notes** | anything else (the reply sent, a duplicate's ID) |

A routine that works:

1. Filter *Status = New*. For each: set *Area*, *Owner* and *Status = Triaged*; mark duplicates
   (`Duplicate` + the original's ID in *Notes*).
2. Bugs worth fixing get a GitHub issue (link it in *Fix/Release*). Copy only what's needed from
   *Diagnostics*: it's redacted, but keep it in the Sheet.
3. If there's a *Contact* and it's a question or the fix shipped, answer and note it.
4. Data validation on *Status* and *Area* (Data → Data validation → dropdown) keeps the values tidy;
   a filter view per owner helps.

People can quote their report ID (`SB-XXXXXX`, shown after sending and under *My reports*) when
they write to you; search the *ID* column for it.

## 6. Privacy

- Nothing is sent unless the user presses **Send report**. Diagnostics are **opt-in** (unticked by
  default) and the page shows exactly the text that will be sent; the user can edit it or untick it.
- Before it's shown, the diagnostics are redacted on the Mac (`feedback.rs` `redact`): the app's
  token and API keys (`sk-…`, `ghp_…`, `github_pat_…`, `Bearer …`, `token=` / `password=` …), email
  addresses, `/Users/<name>` paths (→ `~`), the user and Mac name, GitHub repos, orgs and PR titles
  (the GitHub watcher's), voice transcripts / heard text / Claude's replies / event texts / tool-call
  arguments, Wi-Fi names, the stick's name, and Bluetooth addresses, device UUIDs and serial numbers
  (replaced by a short hash, so two reports about one stick can still be matched). What the user
  writes is sent as written, except that keys and tokens pasted by mistake are removed.
- The diagnostics hold: app and firmware version, macOS version and CPU, the stick's model and link
  state, which features are on (on/off words only), and the last 200 lines of `app.log`, redacted.
- The app keeps a copy of each report (redacted, as sent) in `~/.stickybot-rs/feedback/` for *My
  reports*; unsent ones can be retried or copied. Delete the folder to forget them.
- The Sheet is private to the team; reports may contain a contact address the user chose to give.
  Delete rows on request.
