# spike #1 result — measured Windows Job Object breakaway

Date: 2026-06-14. Code: `src-tauri/examples/spike_breakaway.rs` (preserved). Decision: **GO (but spawn goes through WMI Win32_Process.Create).**

## Measurement environment
The parent Job of the shell this spike runs in (Claude Code CLI/wezterm):
- `parent_in_job = true`
- `parent_job_limit_flags = 0x2000` = **KILL_ON_JOB_CLOSE only, no BREAKAWAY_OK** = worst case.
- That is, "the parent Job binds children with KILL_ON_JOB_CLOSE and also forbids breakaway" — exactly the environment design §4 worried about.

## Results (worst-case Job)
| spawn method | result | evidence |
|---|---|---|
| `CREATE_BREAKAWAY_FROM_JOB \| DETACHED_PROCESS` directly | ❌ failed | `os error 5` (access denied) — a Job without breakaway_ok rejects it |
| `cmd /c start /b` fallback | ❌ failed | after selfkill only `ALIVE` in the marker, no `SURVIVED` (died together) |
| **WMI `Win32_Process.Create`** | ✅ **succeeded** | `RV=0`, `checkjob in_job=false`, `ALIVE` recorded — WmiPrvSE is the parent so the caller's Job is not inherited |

`in_job=false` = outside the reach of KILL_ON_JOB_CLOSE → the daemon survives even when the parent (IDE/shell) exits (logically certain + ALIVE measured).

## Conclusion / design impact
1. **Standard breakaway (CREATE_BREAKAWAY_FROM_JOB) and the start/b fallback are ineffective in a worst-case Job (IDE integrated terminals, some CLI wrappers).** The fallback assumption of design §4 is discarded.
2. **Daemon spawn = adopt WMI `Win32_Process.Create`** (via WmiPrvSE = the caller's Job is not inherited, most robust). Depends on the winmgmt service (always running, unlikely to be disabled).
3. **Adaptive spawn strategy (recommended):**
   - Query the parent Job flags (`QueryInformationJobObject(None)`) →
     - not KILL_ON_JOB_CLOSE → normal spawn (safe even if inherited).
     - BREAKAWAY_OK present → `CREATE_BREAKAWAY_FROM_JOB` (light).
     - worst case (KILL_ON_JOB_CLOSE + breakaway forbidden) → **WMI Create**.
   - Simplified alternative: always WMI Create (no branching, consistent regardless of environment). First-phase recommendation.
4. **Derived constraint (important): WMI Create cannot inject environment variables** (only a CommandLine string). → Token passing is **forced to the ACL port.json method** of design §5 (the env path is discarded). daemon-design §1-4/§5 updated.
5. WMI Create does not return stdout/stdin handles — irrelevant since the daemon communicates over WS (the PTY child is openpty'd directly by the daemon).

## Unverified (follow-up, absorbed into the phase 2 harness)
- The parent Job flags in the real deployment environment (explorer double-click) — usually no Job / breakaway allowed, so easier. Since the worst case passed, the best case is expected to pass automatically.
- Calling WMI Create directly from Rust (COM IWbemServices, the `wmi` crate) vs via PowerShell — to be decided at phase 2 implementation. First choice is direct COM (removes the PowerShell dependency).
- The daemon putting PTY children into its own KILL_ON_JOB_CLOSE Job so they are cleaned up together on a daemon crash — reuse the existing `platform/windows.rs` as is (already verified).
