# Pointcast: the next layer

Status (local/assistant-polish, 2026-11): **S1 and S2 are built.** S5–S7 exist in substance — see/act tools behind a per-turn grant, per-app trust tiers, confirm cards, plans, action journal, pointer ring and a stop key. S3, S4 and S8 remain, as written below.

**End state.** You talk and point, and Pointcast answers or does the task on your Mac or Windows PC. It works through whichever AI you already pay for, with the model and effort you chose. Every Ask and dictation is on Home. It remembers what matters across sessions, and it never acts on your computer beyond what you allowed.

## Where it stands today (0.5.2, from the code)

- **Home.** Two lists from two stores. Dictations live in `history/` (last 100 kept); Ask sessions live in `sessions/` (never pruned). Home shows the last 5 dictations and a short "Recent questions" box inside the Ask card. Stats count dictation only (`stats.rs`).
- **Model and effort.**
  - Codex (ChatGPT): model list and efforts are both offered.
  - API keys and local models: model list only.
  - Every agent route (Claude, Gemini, Copilot, OpenCode): nothing. The code says "an agent picks its own model" (`catalog.rs:738`).
  - Claude Code starts with fixed arguments and no `--model` (`claude.rs:41-70`).
  - The agent protocol (ACP) code reads only `sessionId` from `session/new`, and ignores the agent's model and effort options.
- **Memory.** None across sessions. Inside a session, earlier turns are replayed (pictures for the last 2 only). Summaries run on whatever AI Ask uses (`summary.rs`, `lib.rs:348`); there is no cheap summariser.
- **Actions.** None. Every route is fenced with "No tools exist here": Claude runs with `--tools ""`, ACP permission requests are rejected and Codex approvals are declined. The only faked input is ⌘V/Ctrl+V for paste.
  - Already in place: reading the accessibility tree under the pointer (`element.rs`), screenshots with password-manager windows painted out (`capture.rs`), a local MCP server serving captures and sessions to coding agents (`mcp.rs`), and Microphone, Screen Recording and Accessibility permissions.
- **Earlier plans.** `live-assistant-plan.md` deliberately left out clicking and acting. `ROADMAP.md` has nothing on actions or memory.

## What others do (research, November 2026)

- **Computer use is real and shipping, but tied to one vendor's app.**
  - Claude Code: built-in computer use, macOS CLI only, not available in `-p` (headless) mode.
  - Codex / ChatGPT desktop: computer use on macOS (Apr 2026) and Windows (May 2026), inside their own app.
  - Gemini 3.5 Flash: computer use built into the model.
  - Building on any of these would give a different Pointcast per AI and per OS.
- **What works well:** read the accessibility tree first; fall back to a screenshot and pixel coordinates only when that fails. The pixel loop costs one model round-trip per action plus about 0.5 s for the UI to settle.
  - Anthropic's API tool (`computer_toolset_20260801`) works on Opus, Sonnet and Haiku 5.5 for that pixel fallback on API-key routes.
- **The safety design all vendors converge on:**
  - Approval per app, in tiers (view / click / full). Browsers and terminals are kept low.
  - Confirm before anything irreversible (send, pay, delete, accept terms).
  - A stop key that screen content can't swallow.
  - Hide or exclude the agent's own windows from screenshots.
  - One controller at a time.
  - Screen text is untrusted. Never touch OS password prompts.
- **Model and effort surfaces exist in every app Pointcast already runs:**
  - Claude Code 2.1.295: `--model`, `--fallback-model`, `--effort low…max`.
  - Codex app-server 0.167.2: `model/list` gives each model's `supportedReasoningEfforts` and `defaultReasoningEffort`; `turn/start` takes `model` and `effort` per turn.
  - ACP (stable spec): `session/new` returns `configOptions` with categories `model` and `thought_level`; change them with `session/set_config_option`. After a switch, check which model actually answered: Cursor shipped a bug where the switch changed the label but not the model.
  - Gemini 0.68 `--acp -m`; Copilot 1.0.105 `--acp --model --reasoning-effort`.
- **Memory:**
  - Claude Code keeps `CLAUDE.md` (you write it) plus an auto memory file (it writes, capped at about 200 lines).
  - Codex has an opt-in `~/.codex/memories/`.
  - The lesson: keep Pointcast's own memory local, plain text, capped and editable, and give it the same way to every AI. Don't depend on each vendor's store.

## The plan, in slices

Each slice ships on its own and has its own gate.

**Order:** S1 → S2 → S3 come first and are independent of each other. S4 needs S2. S5 needs S3. S6 needs S5. S7 → S8 need S6.

### S1. One history on Home (smallest; ship first) — **built**

- **What:** Home's "Recent" becomes one timeline of dictations and Ask sessions, newest first.
  - Each entry shows its kind, its first words, the AI that answered, and how long ago.
  - Click an entry to open it (the session or the History entry).
  - Stats count Ask questions too: questions asked and answers read.
  - No storage change: the timeline merges the two stores.
- **Gate:** a test merges both stores, checking order and that no entry is lost. A screenshot of Home with both kinds shown.

### S2. Choose the model and effort on every AI — **built**

- **Claude:** pass `--model` and `--effort`.
  - Models come from Claude Code's aliases (`opus` / `sonnet` / `haiku`) plus any full id; Sonnet is the recommended default.
  - Efforts: low / medium / high / xhigh / max.
- **Codex:** already has both; show each model's own efforts and its default effort.
- **Gemini, Copilot, OpenCode:** read `configOptions` from `session/new`; set them with `session/set_config_option`; follow `config_option_update` messages. After the first answer, check which model actually answered and warn if it differs.
- **UI:** the same Model and Thinking rows for every AI on the AI page and in onboarding, each with "(recommended)".
- **Gate:** for each route, a test with a fake agent proving the chosen model and effort reach the wire. A hand-run probe on this Mac printing each route's models and efforts. A screenshot of the AI page with Claude's picker.

### S3. A cheap summariser — **built** (Automatic; a "Summarise with" picker is the follow-up)

- **Default summariser per AI:**
  - Claude: Haiku with `--effort low`.
  - ChatGPT: the cheapest model in `model/list` that supports `low` or `minimal` effort (today that is gpt-5.6-luna). Picked at runtime, not hardcoded.
  - Keys and local models: the model in use.
- **Setting:** "Summarise with" on the AI page; Automatic is the default. *(Automatic is implemented in `catalog::summary_pick`; the picker UI is not yet.)*
- **Uses:** session summaries now, and memory (S4) next.
- **Gate:** a test that the summary call uses the summariser's model and effort, not Ask's. A hand-run on this Mac.

### S4. Memory you can see — **mostly built**

- **Session memory:** each finished session leaves a summary (S3), and the next Ask in that session starts from it plus the last turns. *(Summaries exist but are started by hand; starting a new Ask from a session's summary is not yet.)*
- **Long-term memory:** one local file, `memory.md` in Pointcast's data folder. **Built** (`memory.rs`):
  - About 200 lines at most: facts you said to remember and facts the AI was asked to remember (marked `remember:`), which wait as proposals. Proposed facts are kept only once you accept them, with one click. *(Summariser-proposed facts are not yet — proposals come only from "remember…".)*
  - Given to every AI the same way, as untrusted reference text inside the system prompt.
  - "Memory" view on the AI page: read, add, forget one line, or forget all.
  - Off switch.
  - Vendor memories (Claude's, Codex's) are left as they are and never read.
- **Gate:** tests for the cap, for forgetting, for delivery to every route, and for "a proposed fact is not kept until accepted". A screenshot of the Memory view.

### S5. Pointcast's own action tools, read-only first

- **What:** Pointcast's existing MCP server gains computer tools, all implemented in Rust:
  - `screen` (a screenshot with password managers painted out);
  - `elements` (the accessibility tree of the front window: AXUIElement on macOS, UI Automation on Windows);
  - `apps` (open windows);
  - `find` (an element by name or role).
- **Still read-only:** nothing clicks yet.
- **Wiring:** routes get these tools instead of the "No tools" fence, only when the person turns on "Let the AI see and act". Claude via `--mcp-config` pointing at Pointcast; Codex via its MCP config; ACP via `mcpServers` in `session/new`.
- **Gate:** tool tests on both OSes in CI (macOS and Windows runners). An injection test proving screen text arrives labelled untrusted.

### S6. Acting, with tiered trust

- **New tools:** `click`, `type`, `key`, `scroll`, `open_app`, `focus`.
  - Accessibility actions first (press, set value).
  - Synthetic input second (CGEvent / SendInput, with enigo as fallback).
  - Pixel coordinates last; on API-key routes, Anthropic's computer-use tool may drive this fallback.
- **Trust, built into the product:**
  - Per-app grants, view / click / full. Browsers, terminals, System Settings and finance apps start at view.
  - A spoken and on-screen confirmation before anything irreversible (send, buy, delete, submit, accept).
  - A visible ring around the pointer and a border while it acts.
  - A global stop key (Esc held, or the Ask key) that screen content can't consume.
  - One task at a time.
  - Never on elevated Windows apps or OS password prompts.
  - Every action logged in the session, for replay.
- **Gate:** a scripted task suite run on CI for both OSes (open TextEdit/Notepad, type, save). Tests for the confirmation, the kill switch and the per-app tier refusal. A screen recording of one real task on this Mac.

### S7. Multi-step tasks by voice ("do it for me")

- **What:** say a task; Pointcast plans it in steps.
  - Before starting, it shows the steps and the apps it will touch.
  - It runs them through S6, narrating in the Ask panel.
  - If anything is unexpected it stops and asks.
- **Model:** the session's own model; the planner may be a stronger model than the summariser.
- **Gate:** a 10-task benchmark on both OSes (open, find, fill, save, attach). Success rate and median time recorded. Every irreversible step confirmed.

### S8. Native integrations

- **What:** before reaching for the screen, use an app's own MCP server or API when Pointcast knows one (mail, calendar, browser over CDP, Finder/Explorer file operations). This is faster and more reliable, and the screen stays the fallback.
- **Gate:** the same tasks as S7, faster with integrations than without.

## Out of scope for this plan (the stated edge)

- Running tasks unattended or on a schedule with nobody watching.
- Acting inside a virtual machine.
- Mobile.
- Paying for anything, or entering passwords or payment details.
- Reading other apps' memory or sign-in files.
- Using the vendors' own computer use (Claude Code's or Codex's) as the base: it differs per AI and per OS.
- Training or hosting models.
- Linux actions (it keeps today's fallbacks).

## First shippable slice

**S1, one history on Home.** It is small, independent, and fixes what the owner hit first. S2, model and effort for Claude, follows: it is the most visible daily need.

## Open questions for the owner

- **Default trust tier for new apps:** view (safest) or click?
- **Stop key:** Esc held for one second, or a dedicated key?
- **Memory:** may the summariser propose facts automatically, or only when you say "remember…"?
