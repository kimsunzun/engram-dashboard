//! Pseudo-terminal facts that differ by OS. No code — constants only.

// ADR-0266

/// Whether closing a pseudo-terminal waits until the output it still holds has been read.
///
/// `true` on Windows: `ClosePseudoConsole` does not return while conhost holds output that nobody reads
/// (Microsoft: keep draining the output pipe while closing). Measured on Windows 26200 with portable-pty
/// 0.8.1: blocked > 20 s when the reader had stopped mid-burst, 12–17 ms while it kept reading (S34).
/// So a caller that closes the pseudo-terminal while output may still be flowing keeps reading until the
/// close has returned. `false` elsewhere — closing a pty master does not wait on its reader (not measured
/// here; development and CI run Windows only).
pub const CLOSE_NEEDS_DRAIN: bool = cfg!(windows);
