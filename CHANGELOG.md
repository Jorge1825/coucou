# Changelog

## Unreleased

- Windows: OK on a notification card also removes it from the list, on every display
- Windows: notifications leave the bell list 5 minutes after they arrive (never while you are reading the card); the sweep timer only runs while there are any. App logos and the seen-apps list are capped
- Windows: notifications are discreet by default — the compact island widens into a one-line banner for 4 s (pointing at it opens the full card), Mochi just glances at it, bursts from one app become a single banner with a count, and a dot stays afterwards. No chime of our own unless you turn it on. Nothing pops up while the island is open, in quiet hours, or while Windows is in Do not disturb or an app is full screen. Settings → Notifications → When one arrives: Discreet / Open the island / Only the dot on the bell
- Windows: Windows notifications in the island. A new toast opens the island (or only peeks, if you prefer) with the app, title and text; page through the last 20, silence everything or one app from the card, and find them later under the bell in the header. Settings → Notifications: Windows access, silence, pop-up mode, sound, muted apps. Needs Windows' own "Notification access" switch; contents stay on the PC and are never logged
- Windows: lighter and smoother. The cursor poll no longer asks the UI thread for the window geometry 180 times a second (drags no longer stutter), goes quiet while the cursor is on another display, and checks the display layout once every 2 s for all islands; ambient motion (breathing, dancing) runs at 30 fps — 20 in the compact island — while anything you drive stays at 60; hidden mini bots and CSS animations in hidden views stop drawing; Spotify is looked for every 5 s while closed. With music playing, total CPU went from ~18 % to ~3 %
- Windows: Mochi no longer vanishes on its own. The compact island used to hide completely after 60 s, leaving only an invisible 6 px strip to wake it; it now stays until you choose otherwise in Settings → General → Hide completely (Never / 1 / 5 / 15 min), and when it does hide a small bar marks where to point
- Windows: interface in English, Spanish, Russian and Chinese — Settings → General → Language (Auto follows Windows). The island and Settings switch at once, and Mochi's chat answers in the chosen language
- Windows: fix the build after the last merge (the chat shortcut setting had landed outside the Settings struct)
- Windows: a minimize button (—) in the island header folds it right away, no countdown; auto-close can now be as short as 1 s (3 s quick choice in the island)
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
