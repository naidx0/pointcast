# Design reference study

Written 2026-11-12, for Pointcast's onboarding window, Settings, the Ask panel and the dictation capsule. It reads
public pages only: no sign-ins and no installers. Every claim names its source. **Unverified** means the source is a
third party (a clone, a reverse-engineered token sheet, a review) or the claim is my own design judgment rather than
something a product shipped.

Where Pointcast stands today (from `docs/design/assistant-polish.md`, `docs/live-assistant-plan.md`,
`src/styles/live.css`):

- The Ask panel is docked at the top right, 500 px by default and resizable from 380 to 900 px from a bottom-left grip.
- It uses native vibrancy (macOS popover, Windows 11 acrylic) under a tint of the panel colour: 0.80 light, 0.86 dark.
- The macOS corner is 14 px with a 0.5 px inset hairline, the padding is 18 px (20 px on Windows), and the type is
  Geist at the body size with a 20 px line.
- Colour roles: red means the microphone is live, the accent means going to or coming from the AI, and orange means a
  wait ran long or something failed.
- One animation per state, and every change of state is a 180 ms fade. No spinner and no state words; text appears
  only once a wait runs long.
- The voice is the breathing wave: three layers with masked ends, 6 % height at silence and still under reduced motion.
- The panel is never content-protected.

The rules below build on that. They are meant to sharpen it, not replace it.

---

## 1. Cluely: the overlay bar

**What it does well**

- **One compact panel that holds everything.** The site shows a single chat-style panel with these parts: a Hide
  control at the top, a prompt field ("Ask about your screen or conversation"), a row of quick-action chips (Assist,
  Follow-up questions, Recap), a mode label ("Smart"), a recording timer and a "Viewed screen" tag that says a picture
  was read. ⌘↵ triggers Assist. ([cluely.com](https://cluely.com))
- **The hero is the real context.** The panel sits on a full-bleed macOS desktop mock (wallpaper plus Dock), not on an
  abstract backdrop, so the visitor sees the panel at true scale over real work. ([cluely.com](https://cluely.com))
- **Material.** Cluely's "liquid glass" direction is a sparse, semi-transparent overlay you keep working through.
  ([Sourcery](https://www.sourcery.vc/p/deep-dive-cluely-raises-15m-from)) A review describes the window as
  translucent. ([Constella](https://desktop.constella.app/blog/cluely-review)) Neither source gives blur or tint
  numbers: **unverified**.
- **Staying out of the way.** It has a Hide control, the window can be moved, and it is invisible to screen sharing.
  ([cluely.com](https://cluely.com), [TechCrunch](https://techcrunch.com/2025/04/29/startups-launch-products-to-catch-people-using-ai-cheating-app-cluely/))
  Pointcast deliberately rejects the invisibility (see Avoid).
- **Keys.** ⌘↵ is the only key confirmed by a second source. ([Dupple](https://dupple.com/reviews/cluely)) Hide with
  ⌘B or ⌘\ and move with ⌘ plus the arrow keys come only from clone docs:
  [free-cluely](https://www.mintlify.com/saivighnesh2190/free-cluely/features/keyboard-shortcuts). **Unverified for
  Cluely.**

**What to take.** Chips for the next action under the answer, a small tag when a picture was read, and one key to
hide. Leave the stealth.

## 2. Clicky: the companion beside the cursor

Clicky's source is public, so these are real numbers from the code, not estimates.
([README](https://github.com/farzaa/clicky),
[OverlayWindow.swift](https://raw.githubusercontent.com/farzaa/clicky/main/leanring-buddy/OverlayWindow.swift),
[CompanionPanelView.swift](https://raw.githubusercontent.com/farzaa/clicky/main/leanring-buddy/CompanionPanelView.swift))

**The buddy beside the cursor**

- **The mark.** A 16 pt blue triangle, resting at −35°, held 35 pt right and 25 pt below the cursor.
- **Following.** The cursor is read every 16 ms and followed with a spring (0.2 s response, 0.6 damping). The blue glow
  has an 8 pt radius.
- **Pointing at something.** The buddy flies on a quadratic arc. The control point rises by 20 % of the distance, up
  to 80 pt. The flight takes distance ÷ 800 seconds, clamped to 0.6–1.4 s, with a smoothstep ease. The buddy swells to
  1.3× at mid-flight, stays at least 20 pt from the screen edges, holds at the target for 3 s, then fades for 0.5 s and
  flies back. On the way back, a cursor move over 100 pt cancels the flight.
- **Labels.** 11 pt medium white text on a blue tag: 8 × 4 pt padding, 6 pt continuous radius. A tag pops in from 0.5×
  to 1× on a spring (0.4 s response, 0.6 damping). Text types out at 30–60 ms per character.
- **Voice states.** Listening is five 2 pt bars with 2 pt gaps, 3–10 pt tall, drawn at 36 fps. Thinking is a 14 pt
  ring with a 2.5 pt stroke and a 70 % arc that turns once every 0.8 s. The states cross-fade in 0.15–0.25 s.

**The menu-bar panel (it is also the onboarding)**

- **Shape.** 320 pt wide on a solid fill with no blur. Corner radius 12, continuous. Two shadows: black 50 % at radius
  20, y 10, and black 30 % at radius 4, y 2.
- **Spacing.** Edge padding 16 throughout. The header is padded 16 × 14 and the footer 16 × 12. Gaps are 8, 12 and 16.
- **Type.** Title 14 semibold, status 12 medium, body 11–12, permission name 13 medium, section label 10 semibold in
  capitals.
- **Fills and hairlines.** Fills are white at 6, 8 and 10 % opacity. Hairlines are 0.5 pt.
- **Permissions.** One row per permission, in this order: Microphone → Accessibility → Screen Recording → Screen
  Content. The last row appears only after Screen Recording is granted.
  - Each row has an icon in the warning colour, the permission name, a one-line reason and a pill-shaped "Grant"
    button.
  - Once granted, the row shows a 6 pt green dot and "Granted".
  - The Screen Recording reason changes once granted, from "Quit and reopen after granting" to "Only takes a screenshot
    when you use the hotkey".
- **After setup.** The panel ends on one line that teaches the gesture: "Hold Control+Option to talk".
- **First launch.** The cursor fades in over 2 s, then "hey! i'm clicky" types out, then a 330 × 186 onboarding video
  with a 10 pt radius plays. The panel also offers "Watch Onboarding Again".

**What to take.** The permission row pattern: a reason that changes once granted, and a request that waits until its
prerequisite is met. Also the pointing arc and its timing, if Pointcast ever points back, and ending setup on the one
gesture to learn. Clicky's panel is functional rather than beautiful: no material, a hard double shadow.

## 3. Wispr Flow: the dictation pill

**What it does well**

- **A small bar that is always there.** Flow sits as a small bar so voice is always visible as available without
  adding visual noise. The team chose responsiveness over showing feedback, and does not stream words while you speak
  because that distracts.
  ([Wispr research, 2025-07-18](https://wisprflow.ai/research/designing-a-natural-and-useful-voice-interface))
- **States and controls.** The bar shows resting, recording, processing and errors, and has a language picker plus
  cancel and confirm.
  - When hidden at rest, clicks pass through it.
  - It docks to the bottom, left or right edge (never the top) by drag, remembers the edge, follows the cursor's
    display, and keeps clear of the menu bar, the Dock and the taskbar.
  - Esc cancels a drag.
  - It appears in screen shares by default.
  - ([Move and dock the Flow Bar](https://docs.wisprflow.ai/articles/1790396454-move-and-dock-the-flow-bar-on-desktop))
- **Changes in 2026.** It warns when the mic picks up no sound (Oct 9). Desktop sign-in became animated slides (Aug
  21). Dark mode arrived with Light still the default (Oct 19). On Android the bubble shrinks after 5 s idle, with
  opacity from 20 to 100 %. ([What's new](https://wisprflow.ai/whats-new/march-2026))
- **Waveform.** White bars on a dark capsule that rise with the voice and stay low in silence. The onboarding mic test
  reuses the same meter. This comes from a third-party UX reference:
  [wispr-ux.md](https://cdn.jsdelivr.net/npm/@agent-native/core@0.124.6/corpus/templates/clips/desktop/design-refs/wispr-ux.md).
  It gives no sizes or colours. **Unverified.**
- **Onboarding on Mac.** Sign in through the browser (a 5 min timeout with a retry), then Microphone and Accessibility,
  then setup questions and the privacy notice. Next a mic test with "Change microphone", then the shortcut (fn by
  default, else Ctrl+Opt; push to talk or hands-free), then languages. Then practice demos you can skip, where text
  appears in a demo window, and a data preference before "Explore Wispr Flow".
  - Permissions already granted are skipped. "Not now" lets setup continue.
  - On Windows only the microphone is asked.
  - ([Setup guide](https://docs.wisprflow.ai/articles/3152211871-setup-guide))
- **Colour and backdrop.** The rebrand (2025-09-15) uses soft neutrals with restrained contrast, green for calls to
  action, Figtree for text and an unnamed editorial serif, lifestyle photography and loose sketch-like illustration.
  - The team names glossy 3D and gradient overlays among the things it moved away from.
  - The in-app UI stays grayscale and utilitarian, while the brand carries the warmth.
  - ([Wispr rebrand](https://wisprflow.ai/rebrand))
  - Third-party palette sheets list #1A1A1A / #4D65FF / #034F46 (**unverified**).

**What to take.** A try-it step inside onboarding where your own words land in a demo field. A mic-silence warning.
Clicks that pass through the capsule at rest. A grayscale product with any expressiveness kept out of the working
surfaces. A capsule that always appears in screen shares.

## 4. Superwhisper

**What it does well**

- **The recording window.** Five controls, left to right: resize (main or mini), waveform, mode (click it, or ⌥⇧K),
  Stop, and Cancel (Esc).
  - A cancel under 30 s happens at once; a longer one asks first.
  - A context light shows when clipboard or selected text was picked up.
  - A realtime model shows the words as you speak.
  - ([Recording window](https://superwhisper.com/docs/get-started/interface-rec-window))
- **The mini window.** It can stay on screen as a small indicator. Hover it to record, switch modes or expand; while
  recording, hovering shows Stop. Right-click opens Expand / Settings / History.
  ([Recording window](https://superwhisper.com/docs/get-started/interface-rec-window),
  [Advanced settings](https://superwhisper.com/docs/get-started/settings-advanced))
- **Quiet mode.** The window can be turned off entirely, leaving sound cues only. By default it closes once a paste
  is detected. ([Advanced settings](https://superwhisper.com/docs/get-started/settings-advanced))
- **Craft in the changelog.** ([Changelog](https://superwhisper.com/changelog))
  - A placeholder waveform in the Ready state avoids the UI popping in (1.19.17, Dec 2023).
  - The wave was made larger (1.15.3), with dynamic normalisation and gating of the live wave (2.14.0, May 2026).
  - The window stays inside the screen edges (2.16.1).
  - The mini pill settles into place when dragged (3.2.0, Nov 2026).
  - Mode switches no longer steal focus (2.16.4).
  - The settings window got an animated picker and disclosure (2.17.2).
  - Onboarding has been rebuilt four times, most recently simplified (2.8.0, Jan 2026), with guiding toasts added
    (2.13.0).
- **Colour.** Not documented. The site's icon file names suggest yellow accents (**unverified**).

**What to take.** Draw the wave's resting shape before audio arrives, so nothing pops. Normalise and gate the level.
Clamp to the screen edges. Ask before a long dictation is cancelled. Hover reveals the controls. A setting for sound
only.

## 5. Raycast: onboarding and Settings

**What it does well**

- **The 2.0 rebuild** (beta 2026-05-14, release 2026-08-21). "Every pixel is new." It aims to sit naturally in macOS
  while staying professional, and uses Liquid Glass sparingly, for function rather than decoration.
  ([The New Raycast](https://www.raycast.com/blog/the-new-raycast))
- **Windows that never flash.** ([Technical deep dive](https://www.raycast.com/blog/a-technical-deep-dive-into-the-new-raycast))
  - Web views are transparent over the window material.
  - A window is drawn before it is shown; on macOS it is kept at alpha 0 so it is ready.
  - On Windows, start-up is controlled so there is no white flash.
  - Popovers and tooltips are native windows.
  - Controls do not use the pointer cursor, and most do not highlight on hover.
  - Floating panels do not take focus.
  - Settings is its own native window.
  - The test is whether the app feels like a regular Mac app.
- **Settings.** One window with named tabs: Account, General, Launcher, Shortcuts, Keyboard, Advanced, Organizations,
  About, AI, Applications, Extensions. ([Manual: Settings](https://manual.raycast.com/settings))
  - ⌘F searches every setting.
  - Each row has a bold label, a one-line description and a control (toggle, key recorder, segmented, menu, button).
  - A pane-wide switch sits at the top right (AI).
- **Settings changes in the changelog.** ([Changelog](https://raycast.com/changelog/1.104.0))
  - Opening a setting from elsewhere scrolls to it and highlights it (Oct 9).
  - Usage meters turn orange, then red, near the limit (Oct 28).
  - Contrast adjusts automatically across the UI (Oct 5).
  - A wait for the keychain explains itself after 10 s (Oct 9).
  - When a permission was denied, Raycast opens System Settings.
- **Onboarding.** First-time users and migrating users get separate paths. The first use of a command offers a small
  guide to set its hotkey. ([Changelog](https://raycast.com/changelog/1.104.0),
  [macOS beta changelog](https://www.raycast.com/changelog/macos-beta)) No public screen-by-screen record of the
  current macOS onboarding exists: **unverified**.
- **Backdrops.** Raycast ships 35 wallpapers at 5K–6K. Their names suggest distortion and glaze series, light streaks
  ("Ray of Lights", "Bright Rain") and chromatic variants. ([Wallpapers](https://www.raycast.com/wallpapers)) The
  styles are inferred from the names only: **unverified**.
- **The 2022 refresh.** A bigger search field and icons, one bottom action bar for title, toast and actions, and an
  outline icon set with shared stroke and radius rules.
  ([A fresh look and feel](https://www.raycast.com/blog/a-fresh-look-and-feel))

**What to take.** Settings as a native window with a row grammar of label, description and control. Search across
settings, and deep links that scroll to a setting and highlight it. Draw before you show. No hover chrome, no pointer
cursor. Meters that change colour near a limit. Waits that explain themselves after a fixed time.

## 6. Grok and xAI: "xAI clean"

The xAI site and grok.com refuse automated reads (403) or return no styling. Everything below comes from
reverse-engineered or third-party sheets, so treat the numbers as a well-sourced guess: **unverified**.

**What it does well**

- **Tokens.** From the gallery sheet at [design-gallery](https://design-gallery.pages.dev/design/x.ai), which
  [open-design](https://open-design.ai/systems/x-ai/) agrees with:
  - **Colour.** Background `#1f2228` with white text at 100 / 70 / 50 / 30 % for the text levels. Surfaces are white at
    3 / 5 / 8 %.
  - **Hairlines and depth.** All borders are 1 px: white 10 % by default, 20 % when hovered or strong. No shadows;
    depth comes only from borders and fill opacity.
  - **Type.** Weights 300 and 400 only. A huge light mono display face, body 16/1.5, labels 14, meta 12, and mono tags
    at 12 px in capitals with 1 px tracking.
  - **Spacing.** An 8 px base: 4, 8, 24, 48. Card padding 24, button padding 12 × 24.
  - **Radius.** 0 on the marketing site.
  - **Hero art.** No hero imagery: the type scale and the empty space do the work.
- **The Grok app.** A calm landing where the composer pill is the only object. ([UX teardown, 2026-06-13](https://aiuxplayground.com/teardowns/grok/composer))
  - "+" on the left; the model pill ("Fast") and the mic on the right.
  - An attached image shows as a small thumbnail with an × inside the pill.
  - A filled send arrow appears only once there is content.
  - The criticisms: one loud orange upsell steals focus, and the icon on the right changes meaning as you type.
- **The composer, rebuilt.** A clone at [assistant-ui](https://www.assistant-ui.com/examples/grok.md) measures:
  - a thin 1 px inset ring (`#e5e5e5` light / `#2a2a2a` dark);
  - a fully round composer up to 768 px wide;
  - a 36 px round action button in inverted ink;
  - a page at `#fdfdfd` / `#141414` and a surface at `#f8f8f8` / `#212121`;
  - a model-name pill that collapses while you type.
- **xAI's own write-up of the Grok Bot.** ([Designing Grok Bot](https://x.ai/news/designing-grok-bot), title and
  summary seen through search; the page itself returned 403)
  - The avatar carries the state, with no separate status layer, and detail shows on hover.
  - The team rejected three animated dots because they cannot tell working from stuck.
  - Panel controls and metadata were removed: every feature had to help you delegate, not add something to manage.

**What to take.** Hairlines instead of shadows. Two type weights. Text in 3–4 opacity levels of one ink, not new
greys. Pills that shrink or vanish while you type. The single object owns the space. No loud colour apart from the
one that carries meaning.

---

## Adopt for Pointcast

Numbers are starting values to try on screen. Each rule names the reference it comes from.

### Shared foundation (all four surfaces)

1. **Ink and text levels.** Use one ink at four opacities for text: 100 / 70 / 50 / 30 % (xAI). Use white or black at
   3 / 5 / 8 % for fills and selection (xAI, Clicky's 6 / 8 / 10 %). No new grey hex values.
2. **Hairlines.** 0.5 px on macOS at 10 % ink, 1 px on Windows (Pointcast already uses `0.5px var(--box-line)`). Use a
   hairline wherever a shadow would go. The one shadow allowed is the system window shadow (xAI, Raycast).
3. **Two weights.** 400 and 500 only, with 600 kept for the one title per window. Sizes 11 / 12 / 13 / 15 / 20 / 28
   (Clicky's 10–14 working range plus two display steps; xAI's 300/400 restraint).
4. **Spacing.** A 4 px base on the scale 4, 8, 12, 16, 24, 32, 48. Panel edges at 16–18 (Clicky 16, Pointcast 18).
5. **Pills.** Height 24 (dense) or 28. Fully round, a 5 % fill, a 10 % hairline, 12 px text at weight 500, padding
   10 × 4 (Clicky's Grant capsule, Grok's model pill). A pill that is not needed while you type collapses to its icon
   (Grok clone).
6. **Motion.** Keep 180 ms fades for changes of state. Moving things use springs: 0.2 s response with 0.6 damping to
   follow, 0.4 s with 0.6 to pop in (Clicky). Settle on drag-release (Superwhisper 3.2.0). Every moving part is still
   under reduced motion, and the material turns opaque under Reduce Transparency.
7. **Draw before show.** No window appears blank, white or stale. Render it hidden, then show it (Raycast's deep dive).
   This matters most for Tauri web views on Windows.
8. **One meaning per colour.** Red is the live mic, the accent is the AI, orange is a long wait or failure (already
   Pointcast's rule). Add Raycast's step: a usage or limit meter goes to orange near the limit and red at it. No
   promotional colour anywhere (the Grok teardown's criticism).

### Onboarding window (painted backdrop behind a frosted card)

1. **Layers.**
   - Bottom: a full-bleed painted backdrop.
   - Middle: one frosted card centred.
   - Top: nothing else; no logo wall and no step rail outside the card.
   - Window about 720 × 520. Card 440 px wide, height set by its content, radius 20 on macOS and 8 on Windows 11.
   - (My judgment, **unverified**.) Wispr and Raycast both keep onboarding to one focus per screen.
2. **The backdrop.** Horizontal streaks, as if a painted canvas were dragged sideways: a directional motion blur of
   about 40–80 px along one axis and almost none across it. Use 3–4 hues drawn from the accent and one warm
   counter-tone, and add 2–3 % monochrome grain so it reads as paint, not as a CSS gradient.
   - Ship it as a pre-rendered 2× image (WebP or AVIF under 400 KB), not a live shader. The window must open at once
     (Raycast's draw-before-show).
   - Reference points: Raycast's streak and distortion wallpapers ("Ray of Lights", "Distortion", "Glaze"), seen by name
     only (**unverified**).
   - Caution: Wispr deliberately moved away from gradient overlays, so this has to look painted, never like a glossy 3D
     or mesh gradient.
3. **Motion in the backdrop.** At most a slow drift: a 1–2 % translate along the streak axis over 20–30 s, ease in and
   out, looping by reversing. It stops under reduced motion and whenever the window is in the background. It never
   moves when a step changes; only the card content changes, with a 180 ms fade plus 8 px of travel.
4. **The frosted card.** Real window vibrancy is not available inside a web view over our own image, so use CSS
   `backdrop-filter: blur(24px) saturate(140%)` over the painting.
   - The tint is the panel colour at 72 % light and 78 % dark, so text meets AA on the brightest part of the art. Check
     this against the real image, not a flat colour.
   - Add a 0.5 px top-lit hairline at 12 % white plus a 10 % ink outer hairline, and no drop shadow.
   - Under Reduce Transparency the card is fully opaque.
5. **Inside the card.** A 24 px edge pad. Title 20/28 at 600, one line of reason at 13/18 in 70 % ink, then the
   control, then one primary button at the bottom right with a quiet text button for "Not now" or "Skip".
   - One idea per step. A step counter as 4 small dots at 6 px with 30 % ink, the current one at 100 % (Clicky's 6 pt
     dot).
6. **Permissions.** Use Clicky's row grammar inside the card: icon, name at 13/500, a one-line reason in 50 % ink, and
   a pill "Allow" button at the right.
   - Once granted, the pill becomes a 6 px green dot and "Allowed", and the reason line switches to a promise (Clicky:
     "Only takes a screenshot when you use the hotkey").
   - Ask in the order the first try needs: Microphone, then Accessibility (for the chord), then Screen Recording when
     Ask is first introduced (Clicky's order, Wispr's Microphone-then-Accessibility).
   - Skip any permission already granted, and allow "Not now" (Wispr).
   - If a permission was denied, the button opens the exact System Settings pane (Raycast).
   - Where macOS needs a relaunch after granting, say so on the row before the person clicks (Clicky).
7. **A mic test with the real wave.** The mic step shows the breathing wave reacting live, with a "Change microphone"
   menu. After 3 s of silence while the step is open, one quiet line reads "Not hearing anything" (Wispr's mic test and
   its silence warning).
8. **A try-it step.** Before the end there is a demo field: hold Ctrl+Shift+Space, say a sentence, and see it land in
   the field. Then hold Ctrl+Shift+/ to ask about a sample picture and see a short answer stream into a model of the
   panel. The step can be skipped (Wispr's practice demos).
9. **The last screen teaches one gesture.** "Hold Ctrl+Shift+Space to talk" in a key-cap pill, with Settings reachable
   later (Clicky's ready line). No feature tour.

### Settings (a subtle backdrop wash)

1. **A native window with a sidebar.** About 760 × 560. A 200 px sidebar on the window material (macOS sidebar
   vibrancy, Windows Mica) with an opaque content pane (Raycast 2.0: Settings is its own native window, with sidebars
   updated for the current macOS).
2. **The wash.** Behind the content pane, a very faint version of the onboarding painting.
   - Blurred to 120 px or more and at 4–6 % opacity in light, 6–8 % in dark, so it reads as a temperature, not an
     image. It sits only at the top of the content: a fade to the plain fill by about 180 px down.
   - Rows sit on the plain fill. No texture is ever under text. (My judgment, **unverified**: it ties Settings to
     onboarding without decorating a working surface.)
3. **The row grammar.** A label at 13/500, a description at 12/16 in 50 % ink, and the control on the right. Rows at
   least 44 px high with 12 px vertical padding. Groups separated by a hairline and a section label at 11/500 in 50 %
   ink, sentence case (Raycast's manual; Clicky's spacing).
4. **Search and deep links.** ⌘F or Ctrl+F filters every setting across panes. "Open the setting" links from the panel
   or onboarding scroll there and pulse a 5 % fill on the row for 1.2 s (Raycast, Oct 9 2026).
5. **Waits and limits explain themselves.** Provider sign-in or keychain waits show a reason after 2 s (Pointcast's
   own threshold), not a spinner (Raycast: after 10 s). Plan-limit meters turn orange near the limit and red at it.
6. **No hover chrome.** Controls do not use the pointer cursor, and rows do not highlight on hover. Only focus rings,
   at 2 px of the accent at 50 % (Raycast; xAI's focus ring).

### Ask panel (500 px, resizable, the breathing wave)

1. **Keep the material and tighten the edge.** Keep the 14 px corner, the 0.80 / 0.86 tint and the 0.5 px hairline.
   Drop any shadow beyond the system one. The panel is the single object; nothing floats around it (xAI, Grok bot).
2. **The header is one line.** Session title at 13/500 on the left, the provider and model as one pill on the right,
   24 px high, 12 px text at 500, in the accent text colour (Grok's model pill). While listening, the pill fades to its
   icon. No other header buttons; close is Esc.
3. **The listening row.** The breathing wave fills the full inner width, 500 − 2 × 18 = 464 px, at 28–32 px tall, with
   the ends masked over 24 px.
   - Draw the resting wave from the first frame so nothing pops (Superwhisper's placeholder wave), and normalise the
     level (Superwhisper 2.14.0). This matches the current 6 % rest height and tanh gain.
   - At 900 px the wave keeps its height and gains length, not amplitude.
4. **The answer.** Body 14/22 at 400 and 100 % ink, paragraphs 10 px apart, code at 12.5/18 mono on a 5 % fill with a
   radius of 8.
   - The measure stays readable when widened: cap the text column at about 680 px and centre it at widths over about
     720 px. That keeps the space full without making lines too long.
5. **Picture and next steps as pills.** The picture that was sent shows as a 32 px thumbnail pill with its caption at
   the top of the answer (Grok's thumbnail inside the composer; Cluely's "Viewed screen" tag).
   - Follow-up actions (Summarize, Continue in the app, a follow-up question) appear only after Done, as 24 px pills at
     the bottom edge (Cluely's chips). At most 3 visible; the rest go behind "…".
6. **The thinking state.** No dots and no ring. The wave lowers to its rest and slows to 1.8 rad/s. Text appears only
   at the 8 s and 20 s thresholds (xAI's rejection of the three dots; Pointcast's thresholds).
7. **The grip and resize.** Keep the hidden-until-hover grip. On release the panel settles with a 0.2 s spring and no
   overshoot over 4 px (Superwhisper's "settles into place"), and it is clamped inside the screen margins
   (Superwhisper 2.16.1). It already is.
8. **Visible and quiet.** The panel always shows in screen shares (Wispr's default, Pointcast's rule). When closed it
   is fully click-through.

### Dictation capsule

1. **Size and shape.** 44 px high, fully round, about 168 px wide while listening and 28 × 28 px as a resting dot if
   "always visible" is on. Bottom centre, at least 24 px above the Dock or taskbar (Wispr keeps clear of the Dock and
   taskbar). It docks to the bottom, left or right by drag and remembers the edge. No top edge: that is the Ask panel's
   territory (Wispr).
2. **Material.** The same tint recipe as the panel (0.86 dark) with a 0.5 px hairline. The capsule is dark in both
   themes so the wave reads against any app (Wispr's white-on-dark capsule, **unverified**). Red appears only as a
   6 px live dot at the leading end.
3. **The wave.** Switch the capsule to the breathing wave so Pointcast has one voice language (this answers the open
   question in `assistant-polish.md`). Use two layers instead of three at this size, 120 px long and 20 px tall, with
   the same gain and rest height.
4. **Controls on hover only.** At rest the capsule is click-through. Hover reveals × to cancel at the left and ✓ to
   finish at the right (Wispr's cancel and confirm; Superwhisper's hover Stop). Esc cancels.
   - A cancel after more than 30 s of speech asks first (Superwhisper).
5. **Silence warning.** After 3 s with no input level, the wave flattens and one 11 px line reads "No sound from
   [device]" (Wispr, Oct 2026).
6. **No live words in the capsule.** It shows the voice, not the transcript (Wispr's reason: streaming words
   distracts). Text goes straight to the field.
7. **Timing.** It appears within 1 frame of the key going down, with a 120 ms fade and a 0.92 → 1 scale. It leaves
   with a 180 ms fade once the paste lands (Superwhisper closes on a detected paste).

---

## Avoid

- **Hiding from screen capture or proctoring** (Cluely's "invisible to others"). Pointcast is test-locked to be visible.
- **Spinners, three dots and state words** while waiting (xAI rejected the dots; Pointcast's own rule).
- **Glossy 3D, mesh gradients or a "Siri orb"** on the onboarding backdrop. Wispr names gradient overlays among what
  it moved away from. Painted streaks with grain are the line not to cross.
- **Texture or art behind body text.** Art stays in onboarding and the top edge of Settings, never under rows or
  answers.
- **More than one loud colour on a screen.** The Grok teardown's main criticism is an orange upsell pill. Red, the
  accent and orange each keep their single meaning.
- **Controls that change meaning in place.** The teardown also flags a send icon that swaps from waveform to arrow.
  Keep a separate icon for each meaning.
- **Hard double drop shadows** on panels (Clicky's 50 % / 30 % black). Use hairlines.
- **Hover highlights and pointer cursors** on native-feeling controls (Raycast).
- **A white flash or blank frame** when any Tauri window opens (Raycast's deep dive).
- **Long onboarding tours.** One idea per step, and end on one gesture (Clicky, Wispr).
- **Weights above 600 and new grey hex values.** Use opacity steps of one ink (xAI).

## Sources not reachable or not verified

- x.ai, x.ai/grok and openai.com/brand returned 403. grok.com returned text with no styling. Apple's Materials page
  returned no body text. The xAI tokens come only from third-party sheets.
- No official Cluely docs or changelog were found. The Cluely keys beyond ⌘↵ come from clones.
- The Wispr waveform look and onboarding screen count come from a third-party reference. No official sizes or colours
  were found.
- Raycast's wallpaper styles are inferred from their names. No screen-by-screen record of the current macOS
  onboarding was found.
- The backdrop, wash and card numbers in "Adopt" are my own design judgment, calibrated to these references. They need
  checking on screen against the real art (`docs/design/backdrop-draft.html` is the natural place).
