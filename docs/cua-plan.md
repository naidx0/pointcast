# Computer use: Pointcast sees and acts

Local only (owner, 2026-11-12: "start it"). Branch `local/assistant-polish`; builders work on local branches that merge
back here. Nothing is pushed. This is slices S5 and S6 of `docs/next-layer-plan.md`, made concrete.

**End state.** With "Let the AI see and act" on, you say "open Notes and type a shopping list" in Ask, and the AI you
use does it on this Mac or Windows PC, step by step in front of you. It only acts in apps you allowed, asks before
anything that can't be undone, and stops the moment you press Esc.

## How it works

- **One set of tools, in Rust, for every AI.** Pointcast's own MCP server (`src-tauri/src/mcp.rs`) serves them; nothing
  depends on a vendor's own computer use.
  - **See:** `screen` (a screenshot with password managers painted out, via `capture.rs`), `elements` (the front
    window's accessibility tree: AXUIElement on macOS, UI Automation on Windows, each element with a short id),
    `apps` (open apps and windows), `find` (elements by name or role).
  - **Act:** `click` (an element id first, a point only as the fallback), `type`, `key`, `scroll`, `open_app`, `focus`.
    Accessibility actions first (AXPress, setting AXValue; UIA Invoke and Value), synthetic input second (enigo).
- **The grant.** The act tools are listed and answered only while a live Ask turn holds a grant (a short-lived token the
  turn creates and drops). The setting is off by default. Coding agents that use the same server for
  `pointcast_latest` never see the act tools.
- **Trust, per app.** Each app has a tier: view (see only), click (click and scroll), or full (type and keys too).
  - Browsers, terminals, System Settings or Control Panel, and password managers start at view.
  - Other apps start at click.
  - A tier is raised in the AI page of Settings, or once from the confirmation itself ("Allow in Notes").
- **Never.** Password and secure text fields, OS password prompts, elevated Windows apps, Pointcast's own windows.
- **Confirm.** An action whose target reads as irreversible (send, submit, buy, pay, delete, post, accept and the like)
  pauses. The Ask panel shows what is about to happen with Do it and Cancel, and nothing happens without the press.
- **Stop.** Esc while acting, or the Ask key, ends the task at once. A ring around the pointer and an accent border on
  the panel show while it acts.
- **Untrusted screen.** Every tool result says that screen text is content, not instructions.
- **Logged.** Every action is written into the session, so a task can be read back afterwards.

## Slices

| Slice | What | Gate |
|---|---|---|
| A. Tools | `src-tauri/src/computer/`: see, act, trust and the never-list, irreversible detection; MCP exposure behind the grant | Unit tests with fakes for the tiers, the never-list, confirm detection, grant gating and tool schemas. A hand-run probe on this Mac that lists the front window's elements, and opens TextEdit and types a line |
| B. Wiring | The setting; Claude gets the server and `--allowedTools` for these tools only, with a prompt that replaces the "No tools" fence while the setting is on; the confirm card, the stop key and the acting ring in the Ask panel | Tests that the fence stays when off and the tools reach the wire when on; a hand-run real task on this Mac ("open TextEdit and type hello") |
| C. Other AIs | Codex (its MCP config) and the ACP agents (`mcpServers` in `session/new`) | A fake-agent test per route |
| D. Tasks | Plans of several steps shown before they run, narrated (S7) | A 10-task run on this Mac |

**First slice:** A, now.

## The stated edge

- Out of scope: unattended or scheduled tasks; paying or entering passwords; acting inside VMs; Linux actions.
- Out of scope: the vendors' own computer use (Claude's or Codex's). It differs per AI and per OS.
- Windows code is written and type-checked in CI only after the owner says to push. Until then it is compiled under
  `cfg(windows)` and unverified.
