# Unverified tests

Checks that a change needs but that could not be run when the change landed — a human must click something, or the environment (a second Windows account, a FAT/exFAT drive, elevation) was not available. Collected here so they can be run together later.

- One row per check. When a check has been run, record the result in the commit that removes the row (or in the topic's TRD item if it belongs to a two-PC check), and delete the row.
- Add a row whenever QA reports something as "not verified" that is not covered by an automated test.

| Check | Why not run yet | How to check | From |
|---|---|---|---|
| Stopping the daemon from the tray menu stops it gracefully (no kill) | QA cannot click the tray; the same stop function was run through the `daemon_shutdown` IPC instead | Start the app, open the tray menu, choose Stop daemon; the daemon log shows `handler returned Close` → `daemon shutdown complete`, no kill line | S37 U2 (other accounts, T47) |
| A data folder on an exFAT drive is refused before the daemon starts, and the band names the fix in English and Korean | No FAT/exFAT drive; creating an exFAT virtual disk needs elevation | Put a portable copy on an exFAT USB stick, start it; the connection band shows the "unprotected volume" cause (switch `ui.language` to check ko); no daemon process starts | S37 U2 (T49, ADR-0399) |
| A non-elevated app against an elevated daemon of the same account shows "could not verify … run both the same way", not "another Windows account" | Needs UAC elevation | Start the daemon elevated (or the app elevated and the daemon not), then the other side normally; read the band text | S37 U2 (T48) |
| Another Windows account's app against this account's running daemon is refused with "this daemon belongs to another Windows account …", and cannot read the token file | Needs a second Windows account | Log in as a second account, start Engram on the same data folder while the first account's daemon runs; check the band text and that `daemon-token.<SID>` cannot be opened | S37 U2 (T47 · T48, PRD AC17) — also in the S37 TRD U5 row |
