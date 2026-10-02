# Changelog

## Unreleased

- Windows: Settings → Transparency — island background, cards and a fade while the mouse is elsewhere, each as a slider, plus Solid / Glass / Ghost presets; changes show live on every display
- Windows: one Mochi on every display. Each island remembers its own position and dock side on its display; hooks, integrations, Spotify and reminders reach all of them, an approval can be answered from any display, and event sounds play once. Settings → General → "Island lives on" → Every display (default) / Main display / Display under the cursor
- Windows: Spotify in the island — while a song plays Mochi puts on headphones, sways to the beat and lets music notes out; a Spotify pill shows the track, cover and progress with previous / play-pause / next buttons. Read locally through Windows media sessions (no account, no network); can be switched off in Settings → Mochi
- Compact island on screens without a notch (#22) — thanks @Kamasoutra
- Only web links (http/https) open from the notch; other kinds of links from Claude or integrations are ignored (#16) — thanks @Cris1670
- Hook socket limited to your own user account, with size and time limits; logs no longer keep commands, n8n data or full URLs, and stay under 1 MB (#16) — thanks @Cris1670 and @Vignesh-Thangamariappan
- The island always reopens after folding, and Settings opens below it, resizable — thanks @rouderz
- Choose the Claude model for the chat in Settings; the list comes from your Anthropic account, and Claude Sonnet 4.6 stays the default — thanks @rouderz
- Windows build artifacts are now downloadable from a manual CI run — thanks @MysJofR
- Any agent can talk to Mochi: tag a hook payload with `coucou_agent` (e.g. `nb-hook --agent my-agent`) and it gets its own pill in the island (#7, #9) — thanks @lacatu5
