# Recording off the developer's screens

## Goal

A recorded app should not take over the developer's screen while a check films it. Opt in, and absent (not failing) wherever it cannot work. The spike (`spike/virtual-display`) left three problems: the app opens on the main screen first, AeroSpace rearranges windows, and input does not reach the app.

## What was built

- `FEARLEZZ_OFF_SCREEN=1` in the developer's environment parks every recorded window off screen as soon as it appears. Window capture still records the full, live window.
- AeroSpace parks it when it runs (floating, on a workspace it does not show). Accessibility parks it in the screen corner where the least of it shows otherwise. Where neither can, the log says why and the window is recorded where it opened.
- Focus goes back to the developer's app whenever the recorded app takes it while it settles, including while AeroSpace still has to pick the window up.
- Window positions the app saved while parked are put back after the run.
- New windows are found through the window server's list (about 2 ms a listing, every 10 ms), so the window shows where it opened for about 0.12 s by Accessibility and 0.2 to 0.45 s by AeroSpace.

## Decisions

- **Park off screen** (public API, no virtual display). A strip of about 40 by 32 points stays visible on plain macOS, 1 px under AeroSpace.
- **No input.** "We only care about actually moving the application to its own screen, because most harnesses like Codex and Claude already have tools to interact with the UI." The check's own test drives the app.
- **AeroSpace decoupled.** "If people have it, it can automatically help them. Not having AeroSpace shouldn't break anything."
- **Your machine.** One environment variable for every project, set by the developer, never by an agent.
- **Fast watch for all.** CGWindowList finds new windows for every recording; ScreenCaptureKit only captures.
- **Reliable under load.** "Assume that very often my system is very loaded." AeroSpace gets up to 10 s to pick up a window, and tools get a 5 s timeout.
- **Wait up to 3 s, then record anyway.** At load 100 to 150 a short demo app closed before AeroSpace picked its window up when recording waited 10 s for parking, failing the run. Recording at once instead let AeroSpace tile and float the window during the film, changing its size under a capture of fixed size. So recording waits for parking up to 3 s, then records anyway while parking goes on, and the log says when the size changed during the film. Passed 3 of 3 at load 400 to 560.
- **Restore after run, only parked positions.** macOS saves a window frame on every move, so a parked app reopened in the corner. Only a saved frame that is mostly off its screen is repaired: to its value from before when that was not parked, removed otherwise. The developer's own moves and a parallel recording stay; a killed run's leftover is cleared by the next run.
- **No AGENTS.md lines** for loaded machines, env var switches or recorder unit tests.

## Dropped tries

- **Virtual display** (the spike). Recording there works, but the window opens on the main screen first and needs moving anyway; window capture records a parked window just as well without one.
- **Move on the Accessibility window created notification.** It never fired; the app's window is drawn before Accessibility answers.
- **Plain Accessibility move under AeroSpace.** AeroSpace moves the window back within about 50 ms.
- **Posting input to the app's pid.** A background Flutter app ignored the events; out of scope after the no-input decision.
- **A macOS VM.** Full isolation and real input, but minutes to boot, toolchains inside the guest, and at most two per Mac.
- **ScreenCaptureKit for watching.** About 70 ms a listing, so the window showed far longer.
- **Giving focus back as the cause of slow pick up.** A window that never got focus was picked up after 20 to 52 s, but one that got focus and lost it at once took 1.5 to 1.8 s, the same as one that kept it.
- **CFPreferences for saved frames.** It cannot reach a sandboxed app's container and reads a stale cache by path; the `defaults` tool can.

## Left open

- Films fail machine wide after many recordings until `replayd` restarts; reproduced on `main`, listed in ROADMAP Known problems. Every film now goes into the machine's film log (`films.jsonl` in the cache folder) to count it.
- With AeroSpace on and Accessibility off, giving focus back is unproven on a real machine.
- When a command exits while the app it started keeps running, the app may save its parked position after the put back.
