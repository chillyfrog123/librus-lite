# Klaso Desktop
Fully vibecoded - created for the sake of saving time. (don't expect my own code on this acc)
A small Windows desktop starter written in Rust, with native egui rendering.

## Build and run

Install stable Rust and either Visual Studio C++ Build Tools (MSVC target), or MinGW-w64 (GNU target). Then:

```powershell
cargo run --release
```

Executable: `target/release/klaso-desktop.exe`. `cargo test --release` checks login redirects/cookies, required-action screens, Polish text decoding, ISO weeks and timetable normalization. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` check code quality.

You can also run `./run.ps1` in PowerShell; it builds the optimized executable and starts the app.

`target/release/klaso-desktop.exe --demo` opens a clearly labeled offline preview with fictional data. Add `--compact` to start with a 900 × 640 client area. The preview never reads saved credentials, logs in, or writes theme preferences. The ignored `live_account_read_only` integration test requires an existing remembered account in Windows Credential Manager and reads only lists, without printing personal content. It is not run during normal tests.

## Authentication

Starts at `/loguj/portalRodzina`, requests the state-bearing authorization URL without following its next redirect, posts the credentials as a form, then follows the JSON `goTo` response one redirect at a time. Requires a Synergia `oauth_token` cookie applicable to the API path before verifying `/Auth/TokenInfo/`. No credentials are sent to a third-party backend. An active session retains credentials in memory for recovery, including when remembering is disabled; its password buffer is zeroed when that session is dropped. HTTP and GUI dependencies can make temporary copies.

Login failures distinguish a missing session cookie from a verification endpoint rejection. Redirect diagnostics include only HTTP statuses and paths, without query strings or cookie values. Regression tests use a local mock server to check the bootstrap stops before form submission and that intermediate redirect cookies reach subsequent requests.

CAPTCHA and additional verification are reported, not automated. HTTP 200 login pages can contain structured `requiredActions`; password-change and leaked-password notices are recognized and direct you to finish the action on the official website. The app does not change passwords or dismiss these notices. If a new data request encounters an expired session, recovery tries `/refreshToken` and verifies it, otherwise reauthenticates, then repeats the original request once. Recovery is serialized across simultaneous callers, recognizes insufficient scopes separately and reinitializes the messages session. There is no keep-alive timer or background refresh. Failed recovery leaves the last successful data visible.

The login flow is based on observed Librus behavior and independent protocol analysis; it is an unofficial integration and can change. Version 0.3.0 was checked against the remembered account for grades, attendance, message lists, notices, timetable, session renewal and a request after renewal. Private content is not printed by the integration test. Do not commit credentials or student data.
