# M0 renderer spike: results

**Status (2026-09-29): both halves measured; decided.** The decision rule points to the **sokol
painter** (see [Decision](#decision)), confirmed by the owner as N16 in
[`docs/DECISIONS.md`](../DECISIONS.md).
Still open: cold-cache runs and a second machine with a discrete GPU (see [Still open](#still-open)).

| | Windows (WARP) | Windows (hardware) | macOS |
|---|---|---|---|
| **B**, sokol painter | **43.3 ms** | 165.5 ms | **136.1 ms** |
| **A1**, egui-wgpu tuned, unpatched | 275.6 ms | 295.5 ms | 200.7 ms |
| A1 with a wgpu-hal patch (`-warp-f` / `-noocc`) | 66.6 ms stamped, but the frame is only drawn at ~158 ms | – | 133.8 ms |
| Budget | 150 ms | 150 ms | 150 ms |

The spike itself (`spike/`: the variants, `bench` and `visual-check.ps1`) was deleted at the end of
M1; it is in git history at commit `3801339`, and its bench lives on as `cargo xtask ttfp`.

Raw data: [`results/`](results/). Every table was produced by `bench`
(`spike/README.md` in that commit): 30 measured rounds per variant in seeded shuffled order, a warm-up round
reported separately as "first launch", and a 1 s gap between launches.

**TTFP definition (master's):** ms from kernel process creation until the first frame with real
content has been presented and `set_visible(true)` has returned. The "spawn→shown" column (from the
bench's own spawn stamp) agrees with it to within 0.5 ms on macOS and Windows.

## Machine (macOS)

- MacBook Pro, Apple M3 Max (30-core GPU, Metal 4), 36 GB, built-in Retina display 3456×2234, on AC power.
- macOS 27.0 (26A428), rustc 1.98.1.
- eframe/egui/egui-wgpu/egui-winit 0.36.2, wgpu 30.0.1, winit 0.30.13, sokol @ b22a545.
- All binaries are release builds: opt-level "z", fat LTO, stripped. They are ad hoc signed, launched as bare binaries unless marked `-app`.

## Headline (macOS)

| Variant | What | Median TTFP | 95% CI | p90 |
|---|---|---|---|---|
| **B** | sokol painter, Metal | **136.1** | 132.3–139.1 | 143.5 |
| **A1** | egui-wgpu, tuned (Metal only, device + renderer on a thread) | **200.7** | 195.6–204.5 | 208.5 |
| A1-noocc | A1 with wgpu-hal's occlusion check switched off (patched) | 133.8 | 131.1–135.8 | 144.9 |
| A0 | stock eframe | 157.9* | 153.0–161.7 | 164.0 |

\* A0 isn't comparable. eframe can only be stamped at its second `update()`, which doesn't prove its
content was on screen, and eframe's wgpu has the same occlusion skip as A1.

A second, independent run (seed 2) put A1 at 196.0, A1-noocc at 133.8 and B at 135.7 ms. Within
each round, A1 minus B was +62.4 ms and A1-noocc minus B was −1.7 ms.

## Findings (macOS)

1. **B meets the 150 ms budget on macOS: 136 ms median, p90 144 ms.**

2. **A1 misses it by about 50 ms, and GPU setup isn't the reason.**
   - All of wgpu's work (instance, adapter, device, egui renderer) takes about 18 ms and is fully hidden on the GPU thread.
   - The cause is in wgpu-hal's Metal backend. `acquire_texture` returns `Occluded` while the hosting `NSWindow` isn't visible, a workaround for gfx-rs/wgpu#8309 (the 1 s `nextDrawable` hang).
   - A hidden window counts as occluded, and so does one shown moments ago, because the occlusion state updates asynchronously. So the first frame can't be presented until about 60 ms after `set_visible`.
   - Showing first doesn't help (A1-first: 197.9 ms). Neither does using wgpu's own layer instead of raw-window-metal's (A1-ownlayer: 198.4 ms).

3. **Without that check, wgpu is exactly as fast as sokol on macOS** (A1-noocc: 133.8 ms, −1.7 ms against B per round).
   - The change is a three-line env-var switch in `patches/wgpu-hal/src/metal/surface.rs`.
   - On macOS, then, the renderers differ only in this check. **egui-wgpu can't meet the budget on macOS without patching wgpu-hal**, or without an upstream option to present while hidden.

4. **The critical path is AppKit/winit, not the renderer** (median phases, B):

   | Phase | ms since process creation |
   |---|---|
   | `main` | 15 |
   | event loop built | 66 |
   | `resumed` | 93 |
   | window created | 118 |
   | egui pass | 127 |
   | present | 129 |
   | shown | 136 |

   - The GPU thread finishes long before the window exists: the Metal device plus `sg_setup` plus the pipeline are ready at 53 ms.
   - The ~14 ms of headroom under 150 ms is thin; the ~100 ms of NSApplication and window creation is where any further macOS gain has to come from (M1).

5. **The GPU thread is worth about 6 ms** on macOS (B-nothread +6.6 ms, A1-nothread +6–8 ms). Metal device creation is cheap here; the thread matters more on Windows.

6. **Launching from an `.app` bundle adds about 7 ms** (B-app 143.7 ms, +6.8 ms; A1-app 195.6 ms). It still fits, with less headroom.

7. **The first launch of a newly written binary is 100–250 ms slower.** macOS checks each new binary once; it isn't in our code.
   - Three fresh copies of `sokol-app` took 240–382 ms on first launch, with `main` reached at 120–242 ms. Their second launches took 123–137 ms, with `main` at 9–16 ms.
   - Every update will pay this once. Re-measure with a Developer ID signed and notarized build in M10.

8. **Steady state:**

   | | A1 (wgpu) | B (sokol) |
   |---|---|---|
   | Stress: 60 Hz, 200 coloured lines/s into a 5,000-line console, 900×700 pt at 2× | 19.8% of one core, frame CPU 1.5 ms (p95 2.0) | 12.0% of one core, frame CPU 0.9 ms (p95 1.2) |
   | Idle 10 s after show | 17 frames (egui's scroll-bar fade-in), then none; 36 ms CPU | 19 frames, then none; 64 ms CPU |

   A1 uses **1.65×** B's CPU while streaming. That's over the plan's 1.5× threshold, but small in absolute terms on this machine. Windows (WARP rasterizes on the CPU) will show whether it matters.

9. **Size and memory:** A1 5.4 MB and 102 MB peak; B 3.7 MB and 92 MB peak; A0 5.9 MB.

10. **Rendering:** A1 and B draw the test scene identically by eye (screenshots checked). A pixel-exact diff is left for Windows.
    - The painter's key choices work: B reuses sokol_imgui's precompiled Metal shader with `gamma = 1.0` and premultiplied blending.
    - The Metal path of the vendored sokol, never built before, compiled and ran unchanged.

## What macOS alone meant for the decision

- "egui-wgpu with no custom code" isn't on offer for macOS. The macOS choices are:
  - **wgpu plus the wgpu-hal patch:** small, but a fork to carry through wgpu upgrades;
  - **wgpu unpatched:** about 200 ms, over budget;
  - **the sokol painter:** about 350 lines in `sokol-app/src/painter.rs` plus the per-OS device glue.
- Proposed addition to the decision rule: treat **A1-noocc** as the macOS A1 candidate and count its patch as custom code. On Windows, A1-warp-f is a second patch to the same crate.

## Machine (Windows)

- AMD Ryzen 9 7900 (12 cores, 24 threads), 64 GB.
- Windows 11 Enterprise 25H2, build 26200.9457.
- GPUs:
  - NVIDIA GeForce RTX 4080, driver 32.0.16.1692 (2026-09-04). It is DXGI adapter 0 and drives the display.
  - Two virtual display adapters: Meta Virtual Monitor and Parsec Virtual Display.
- One 3840×2160 display at 150% scaling. Every variant ran per-monitor-v2 aware with the UTF-8 code page (both checked at runtime), scale factor 1.5.
- rustc 1.98.1; the same crate versions as macOS (egui 0.36.2, wgpu 30.0.1, winit 0.30.13).
- The code is b537bc4 plus this session's harness changes.
- Timing hygiene:
  - Binaries ran from `%LOCALAPPDATA%\Programs\spike` (NTFS). `D:`, where the repo lives, is a Dev Drive.
  - Power mode was set to Best performance for the runs, then restored to Balanced.
  - Desktop on AC, Defender real-time protection on.
  - VS Code stayed open, because the session ran inside it.
  - The user PATH holds a `dxcompiler.dll` (an unrelated tool). bench removes that folder from the children's PATH.
- R is master's `launcher.exe`, built from this tree. `crates/` and `vendor/` are identical to master's. It ran with its game-launcher config beside it.

## Headline (Windows)

30 measured rounds, seed 1 ([table](results/windows-ttfp.md), [raw](results/windows-ttfp.jsonl)).
"Shown, out of process" is the `EVENT_OBJECT_SHOW` time of the child's real window, measured by
bench. It is the one end point every variant shares, eframe and R included.

| Variant | What | Median TTFP | 95% CI | p90 | Shown, out of process | Δ vs B-warp per round |
|---|---|---|---|---|---|---|
| **B-warp** | sokol painter, D3D11 WARP | **43.3** | 42.2–44.2 | 46.9 | 32.7 | – |
| R | master (ImGui + sokol D3D11, WARP) | 50.8 | 49.5–51.9 | 61.5 | 40.6 | +8.1 |
| A1-warp-f | A1-warp with wgpu-hal enumerating only WARP (patched) | 66.6* | 65.9–67.5 | 70.4 | 56.0* | +23.2 |
| B-hw | sokol painter, D3D11 on the RTX 4080 | 165.5 | 163.5–168.5 | 171.9 | 155.1 | +122.7 |
| A1-warp | egui-wgpu tuned, DX12, WARP | 275.6 | 270.6–284.4 | 286.4 | 265.5 | +231.3 |
| A1-hw | egui-wgpu tuned, DX12, RTX 4080 (same adapter as B-hw, by LUID) | 295.5 | 291.8–300.5 | 307.7 | 284.9 | +252.7 |
| A0 | stock eframe | 434.5 | 428.5–437.6 | 446.9 | 406.2 | +389.9 |

\* The frame is submitted then, but not drawn: WARP needs another ~106 ms to rasterize it (finding 3).

## Findings (Windows)

1. **B meets the budget with a wide margin: 43.3 ms median, p90 46.9 ms.** That is faster than
   master's ImGui launcher (R, 50.8 ms here; master recorded 57 ms on its own machine).

2. **Unpatched egui-wgpu misses by more than 100 ms, on WARP and on hardware.** The time goes into adapter selection:
   - The `gpu adapter` phase runs from 20 to 242 ms in A1-warp. wgpu's DX12 enumeration creates a D3D12 device on every adapter to query its capabilities, which loads the NVIDIA driver even when WARP is requested.
   - Peak memory shows it: A1-warp 80 MB, A1-warp-f 39 MB.
   - A1-hw pays the same enumeration, plus 29 ms for its own device.

3. **Patching the enumeration (A1-warp-f) gets the submit down to 66.6 ms, but on WARP egui-wgpu's
   frames are extremely slow to draw.**
   - With a diagnostic wait for the GPU after the first frame (`SPIKE_WAIT_GPU`, 5 rounds, [raw](results/windows-gpu-idle.jsonl)):

     | Variant | Textures uploaded | GPU finished frame one | Drawing |
     |---|---|---|---|
     | A1-warp-f | 52.5 | 158.4 | **105.9 ms** |
     | B-warp | 30.8 | 39.0 | 8.1 ms |
     | A1-hw | 294.9 | 297.2 | 2.4 ms |
     | B-hw | 165.1 | 166.7 | 1.6 ms |

   - So A1-warp-f's window shows before its content is drawn. Its real first-content time is about 160 ms, over budget.
   - The cost is egui-wgpu's draws on D3D12 WARP, not wgpu's frame overhead:
     - With egui's draws skipped (the pass only clears, `SPIKE_WGPU_CLEAR_ONLY`), A1-warp-f streams at 60 fps and about 1 core.
     - DXC instead of FXC changes nothing (`SPIKE_WGPU_DXC_PATH`, stress CPU 1,686% vs 1,676%).
     - The atlas size changes nothing (`SPIKE_MAX_TEXTURE_SIDE=2048`).
   - Likely cause, not proven: egui-wgpu's shader. It samples an sRGB texture and does per-pixel sRGB conversions; the sokol shader only multiplies texture × vertex colour. Confirming it would mean patching egui-wgpu's shader.

4. **Hardware first costs about 120 ms on this machine.** Creating the NVIDIA D3D11 device takes 19 → 139 ms (B-hw), against 1 ms for WARP. That puts B-hw at 165.5 ms, over budget. So the first frame has to come from WARP, as it does on master.

5. **Where B-warp's 43 ms goes** (median phases):

   | Phase | ms since process creation |
   |---|---|
   | `main` | 8.8 |
   | event loop built | 12.0 |
   | window created (hidden) | 19.0 |
   | GPU thread joined (WARP device, `sg_setup`, pipeline were ready at 20.6) | 20.7 |
   | swapchain ready | 22.3 |
   | egui pass (layout, tessellation, font atlas) | 30.2 |
   | textures uploaded | 30.5 |
   | present returned | 31.2 |
   | window visible, out of process | 32.7 |
   | `set_visible(true)` returned (the TTFP stamp) | 43.3 |

   - **`set_visible(true)` takes about 12 ms to return in every variant**, although the window is shown 1.4 ms after the present. So the in-process TTFP overstates on-screen time by about 11 ms, equally for all variants (R included: 50.8 vs 40.6).
   - The first egui pass (~8 ms) is the largest block of our own work.

6. **A new binary's first launch costs about 5 ms** (three fresh copies of `sokol-app`: 51–53 ms first, 46–51 ms second). That is Defender's scan. The macOS equivalent was 100–250 ms.

7. **Steady state** ([stress](results/windows-stress.md), [idle](results/windows-idle.md); 3 runs each, median):
   - Stress is 60 Hz, 200 coloured lines/s into a 5,000-line console, 900×700 pt at 1.5×.
   - Idle is the 10 s after the window is shown.
   - CPU is process-wide, measured with `QueryProcessCycleTime` scaled by the TSC. `GetProcessTimes` agrees within a few percent over 10 s.

   | | CPU (1 core = 100%) | Frames/s | Process CPU per frame | Idle 10 s: CPU / frames drawn |
   |---|---|---|---|---|
   | B-warp | 140% | 59.9 | 23.4 ms | 377 ms / 25 |
   | B-hw | 2.5% | 59.9 | 0.4 ms | 16 ms / 25 |
   | A1-hw | 3.2% | 59.9 | 0.5 ms | 16 ms / 24 |
   | A1-warp-f | **1,676%** | **9.2** | **1,819 ms** | 6,809 ms / 5 |
   | A1-warp | 1,674% | 9.2 | 1,820 ms | 7,972 ms / 6 |

   - Every variant stops drawing once egui's scroll-bar fade ends. On WARP, A1 hadn't finished the fade after 10 s.
   - **On WARP, A1 uses 12× B's CPU and can't hold 10 fps.** That is far over the plan's 1.5× threshold. On hardware the two are within 1.3× and both negligible.
   - **B on WARP costs about 1.4 cores while the console streams at 60 Hz.** That's acceptable for a first frame, but not for a long build log. This sets the device policy (see [Decision](#decision)).

8. **Rendering** (`spike/visual-check.ps1`, captures in [`results/windows-shots/`](results/windows-shots/)):
   - With dithering off, **A1 and B are pixel-identical at 150% scaling** (840×960 px, 0 differing pixels), on WARP and on hardware.
   - WARP and the RTX 4080 differ by at most 3/255 on 5.8% of pixels (rasterizer rounding).
   - The captures are of the frame presented while the window was hidden (`--hold`), so the hidden-present policy works on Windows.

9. **Size and memory:**
   - Exe size: B 6.2 MB, A1 10.0 MB, A0 10.4 MB; R 3.9 MB.
   - Peak working set: B-warp 33 MB, B-hw 52 MB, A1-warp-f 39 MB, A1-warp 80 MB, A1-hw 165 MB, A0 210 MB.

## Harness bugs found along the way (lessons for M1)

macOS:

- **egui-winit returns `repaint: true` for `RedrawRequested` itself.** A shell that answers with `request_redraw()` redraws forever at the display's refresh rate: 120 Hz here, 1,190 frames in 10 s of "idle".
- **egui's `show_rows` needs truly fixed-height rows.** Wrapped rows make the content height oscillate, which retriggers the scroll-bar animation every frame. The M5 console view must lay out one grid row per line.
- **egui 0.36 asserts that a `TexturesDelta` is cleared once the painter has applied it.**
- **On macOS, a panic inside a winit handler aborts the process,** because the callback comes from Objective-C and can't unwind. M1's `catch_unwind` firewall is needed there too.
- **Don't reconfigure the wgpu surface on a same-size `Resized`.** winit sends one right after the window is shown.
- **A backend must report whether it actually presented.** Otherwise a skipped frame, such as wgpu's occluded acquire, is silently stamped as "shown".

Windows:
- **"Present returned" doesn't mean "drawn" on WARP.** WARP rasterizes asynchronously on its own threads. `xtask ttfp` in M1 should keep a GPU-completion check (an event query) as a diagnostic.
- **eframe runs `update()` more than once before it shows its window on Windows.** Stamping at the second `update()` exits before the window ever appears.
- **An out-of-process show hook must accept a window that's already gone.** A variant that terminates right after showing destroys its window before the hook thread handles the event, so a PID lookup on the window fails. The hook is now scoped to the child's PID with `SetWinEventHook(.., pid, ..)`.
- **`GetProcessTimes` is coarse but fine over 10 s.** It matched the cycle-based CPU time within a few percent. Only per-frame CPU needs the cycle counter.

## Decision

Applying the decision rule (docs/PLAN.md, M0):

1. **A1 (egui-wgpu) ≤ 150 ms on both OSes?** No.
   - Unpatched, it measured 275.6 ms (Windows WARP), 295.5 ms (Windows hardware) and 200.7 ms (macOS).
   - Two wgpu-hal patches (`-warp-f`, `-noocc`) get the stamps under budget. But the Windows WARP frame is only drawn at ~158 ms, and every later frame on WARP costs ~1.8 s of CPU.
   - Hardware avoids that and costs 295 ms instead.
2. **A2 (passthrough shaders)?** Its precondition doesn't hold. The join wait isn't dominated by renderer init (egui-wgpu's renderer is ~10 ms and hidden on the GPU thread), and DXC made no difference to the draw cost. Precompiled shaders don't address what's slow, so A2 wasn't run.
3. **B (sokol painter) ≤ 150 ms?** Yes, on both: **43.3 ms on Windows** (WARP) and **136.1 ms on macOS**. It needs no patches to third-party crates.
4. **Stress:** on WARP, A1 uses 12× B's CPU (the rule's threshold is 1.5×), which reinforces B. On hardware they are equal.

**N16 (confirmed): egui + egui-winit with our own sokol_gfx painter** (D3D11 on Windows, Metal on
macOS). The painter moves from `sokol-app/` into `crates/launcher/src/gfx/` in M1. The cost is about
350 lines of painter plus the per-OS device glue, all of it already written and measured here.

**Device policy for M1 (Windows, part of N16): WARP first, then move to hardware after the first frame.**
- WARP gives the first frame at ~43 ms. Hardware first costs ~120 ms more on this machine (over budget).
- WARP costs ~1.4 cores while the console streams, while the RTX 4080 costs 2.5% of one.
- So once the window is shown, a background thread creates the hardware device. The painter re-creates its textures from the CPU copies it already keeps, and the swapchain moves over.
- If there is no hardware adapter, it stays on WARP.
- An alternative, if the switch proves fiddly: switch only when the console starts streaming.
- A run on a second machine with a discrete GPU should confirm the hardware device cost there.

macOS has no WARP question: Metal on the system GPU is both fast to create and cheap to draw.

## Still open

- **Cold-cache runs** (5 per variant after emptying the standby list).
  - These need `RAMMap64 -Et` from an elevated shell. RAMMap isn't installed here and the session wasn't elevated.
  - The warm-up "first launch" column and the fresh-binary test (finding 6) are the closest stand-ins.
- **A second machine with a discrete GPU**: the same bench, to check B-warp's margin and the hardware device cost on another machine.
- **A1-dxc** (optional) wasn't run as a timing variant. A DXC build of A1 was only used as a diagnostic, for the draw cost.
- **Why egui-wgpu is slow on D3D12 WARP** is narrowed to its draws but not proven. It doesn't matter for the decision unless egui-wgpu comes back.
