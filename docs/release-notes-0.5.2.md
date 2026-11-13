**macOS** (Terminal): `curl -fsSL https://raw.githubusercontent.com/naidx0/pointcast/main/install.sh | bash`
**Windows** (PowerShell): `irm https://raw.githubusercontent.com/naidx0/pointcast/main/install.ps1 | iex`
Or download the file for your system below. Already on 0.5.x? Pointcast offers this update itself.

## Set up your AI in one click

- **Install, Update and Sign in do the work for you.** No commands to copy.
  - Install and Update use the app's own installer: Homebrew if that's where it came from, `claude update` for Claude Code, npm for the rest. If npm can't write to its own folder, the app goes into your home folder, with no password needed.
  - Sign in opens the app's own sign-in, and the page turns ready by itself when you're done.
- **Onboarding sets up your AI on the spot.** Choose an AI, install it or sign in to it, and pick its model (the recommended one is marked), all without leaving onboarding. You can also skip it.
- **See who you're signed in as.** Claude shows its plan and account ("Claude Pro"). ChatGPT names your plan, and the model hint says which models that plan offers.
- **Refresh shows that it's working.** Checking…, Installing…, Updating… and Signing in… each show a spinner until they finish.

## Works when opened from the Dock

- On macOS, an app opened from the Dock or Finder couldn't find Node or Homebrew, so Gemini, Copilot and other AI apps installed with npm failed to start. Pointcast now uses the same PATH as your Terminal.
- An AI app that's installed but can't start is no longer shown as ready.

## Clearer when something is missing

- **No screen permission:** the Ask panel says so ("No picture: Pointcast isn't allowed to see your screen") with an **Allow** button. Your words still go.
- **Paste didn't happen:** dictation says the text is on the clipboard (press ⌘V) and offers to allow auto-paste.
- **Permission rows update right away** after you change them in System Settings. If Screen Recording needs a restart, the row offers **Reopen**.

## Polish

- Sidebar icons are centred in their tiles on macOS.
- The Ask panel has rounded corners on macOS.
- Ollama and LM Studio show **Download**, which opens their download page.
