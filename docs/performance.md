# Performance

## Budgets (PLAN §37)

| Metric | Target |
| --- | --- |
| Cold startup to first frame | < 250 ms where realistically achievable |
| Warm startup to first frame | < 100 ms |
| Input-to-render | < 16 ms |
| Idle memory | < 50 MB (stretch < 30 MB) |
| Large documents | 1 / 5 / 10 MB Markdown stay usable |

## Measurements

Release build, Windows 11, RTX 3050 Laptop GPU, 125 % scaling. Startup is from the log
(`first frame shown`, measured from the start of `main`); memory from PowerShell
`Get-Process` 3 s after launch.

| Date | Commit | Build | First frame (warm) | Working set | Private bytes | Binary |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-10-08 | Milestone 0 shell | release | ~550 ms | ~50 MB | ~76 MB | 5.5 MB |
| 2026-10-08 | Milestone 0 shell | dev | ~810 ms | ~54 MB | ~76 MB | 14.7 MB |

Where the startup time goes (release log): ~20 ms to GPUI platform init, ~315 ms creating the
Direct3D 11 device, ~160 ms DirectWrite setup, ~60 ms creating and drawing the window. Almost
all of it is inside GPUI/driver initialisation before any Scratchpad code runs, so the
< 250 ms target needs work at that level (GPUI picks the first DXGI adapter, here the
discrete GPU).

To reproduce: run `target/release/scratchpad.exe` and read `%LOCALAPPDATA%\Scratchpad\scratchpad.log`
(`SCRATCHPAD_LOG=debug` for more detail). Dev builds log to stderr.
