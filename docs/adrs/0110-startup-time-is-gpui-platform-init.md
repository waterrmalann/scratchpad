# 0110. Startup time is GPUI's platform initialisation; load fonts alongside it
Date: 2026-10-09
Status: Accepted

## Context
PLAN §37 asks for < 250 ms cold / < 100 ms warm to the first frame; we measured ~540 ms warm.
Timestamps in the log (and a standalone probe of the same Direct3D and DirectWrite calls) show
where it goes, from process creation, on an RTX 3050 laptop whose only display output is on the
NVIDIA GPU: `main` at ~16 ms; GPUI's `WindowsPlatform::new` creates a DXGI factory (~17 ms), a
Direct3D 11 device on adapter 0 to check it (~180 ms, loading the driver), drops it and creates the
real one (~115 ms), then loads DirectWrite's system font collection with an update check (~160 ms);
then ~45 ms for GPUI to create the window and swap chain. Our own code before the first frame
(config, theme, key bindings, views, `mono_font_family`, logging) takes under 10 ms; notes are read
after the window exists (ADR 0065).

## Decision
- `warm_up::load_fonts` asks DirectWrite for the system font collection on a helper thread before
  `Application::new`. The shared factory and collection are per process, so GPUI's own request
  then costs only the update check (~50 ms). Median first frame 537-541 ms -> 437-439 ms (7 runs,
  alternating builds; a review re-measured GPUI's platform ready at 491 -> 391 ms over 8); memory
  unchanged. Needs `windows` 0.61, which GPUI already builds with the same feature.
- Not done:
  - Keeping a second device alive while GPUI creates its two makes the second ~30 ms instead of
    ~115 ms, but leaves ~7.5 MB more private bytes for the whole run (driver caches), and gains
    little where the iGPU is adapter 0 (second device ~30 ms anyway).
  - Choosing the adapter: GPUI takes `EnumAdapters(0)`. Here that is the NVIDIA GPU because it
    drives the display; the Intel iGPU would create its devices in ~135 ms instead of ~350 ms but
    present through a cross-adapter copy. On laptops whose panel is on the iGPU, adapter 0 is
    already the iGPU. DXGI honours no environment variable; Windows' per-app Graphics setting
    (`HKCU\Software\Microsoft\DirectX\UserGpuPreferences`) is the user's to set, there is no
    manifest entry for it, and `NvOptimusEnablement` can only ask for the discrete GPU.
  - `GPUI_DISABLE_DIRECT_COMPOSITION` changes neither startup nor memory measurably.
  - Forking GPUI to skip the probe device or the font update check (~200 ms together).
- GPUI's default features are off on Windows: only `windows-manifest` applies; the others built
  ~27 unused crates (Vulkan, naga). Build time only.

## Consequences
- The < 250 ms target is not reachable on this machine without changing GPUI: ~400 ms of the
  ~440 ms is its platform initialisation, mostly in the graphics driver.
- `platform initialised` in the log marks where GPUI hands over; `scripts/measure.ps1` reports it.
