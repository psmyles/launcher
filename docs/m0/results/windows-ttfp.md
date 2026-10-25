
### Time to first frame (ms since process creation; 30 rounds after warm-up)

| Variant | Adapter | Median | 95% CI | p90 | Min | Max | Spawn→shown median | Shown, out of process: median | First launch |
|---|---|---|---|---|---|---|---|---|---|
| R |  | **50.8** | 49.5–51.9 | 61.5 | 48.8 | 76.7 | – | 40.6 | 56.4 |
| A0 | eframe default | **434.5** | 428.5–437.6 | 446.9 | 419.0 | 454.8 | 434.9 | 406.2 | 437.3 |
| A1-warp | Microsoft Basic Render Driver (Cpu, Dx12) | **275.6** | 270.6–284.4 | 286.4 | 262.1 | 298.2 | 276.1 | 265.5 | 273.1 |
| A1-warp-f | Microsoft Basic Render Driver (Cpu, Dx12) | **66.6** | 65.9–67.5 | 70.4 | 63.7 | 71.5 | 67.1 | 56.0 | 65.7 |
| A1-hw | NVIDIA GeForce RTX 4080 (DiscreteGpu, Dx12) | **295.5** | 291.8–300.5 | 307.7 | 284.6 | 311.4 | 295.9 | 284.9 | 290.2 |
| B-warp | Microsoft Basic Render Driver (D3D11 WARP) | **43.3** | 42.2–44.2 | 46.9 | 40.8 | 54.0 | 43.6 | 32.7 | 42.1 |
| B-hw | NVIDIA GeForce RTX 4080 (D3D11 hardware) | **165.5** | 163.5–168.5 | 171.9 | 161.0 | 176.4 | 165.9 | 155.1 | 166.8 |

Per-round difference against B-warp (median of within-round deltas, ms):

| Variant | Δ median | Δ median, out-of-process shown |
|---|---|---|
| R | +8.1 | +8.4 |
| A0 | +389.9 | +372.9 |
| A1-warp | +231.3 | +231.7 |
| A1-warp-f | +23.2 | +23.4 |
| A1-hw | +252.7 | +252.5 |
| B-hw | +122.7 | +123.3 |

### Median phase breakdown (ms since process creation)

- **A0**: main 9.0 · first update 396.3 · window shown 434.5
- **A1-warp**: main 9.0 · event loop built 12.1 · resumed 12.1 · window created 19.3 · gpu instance 20.4 · gpu adapter 242.4 · gpu device 243.2 · renderer init 253.4 · gpu joined 253.6 · surface ready 255.1 · egui state 255.1 · egui pass 262.8 · textures uploaded 263.0 · present returned 264.1 · window shown 275.6
- **A1-warp-f**: main 9.0 · event loop built 12.2 · resumed 12.3 · gpu instance 20.7 · window created 19.3 · gpu adapter 33.0 · gpu device 33.7 · renderer init 44.0 · gpu joined 44.3 · surface ready 45.6 · egui state 45.6 · egui pass 53.2 · textures uploaded 53.4 · present returned 54.5 · window shown 66.6
- **A1-hw**: main 8.9 · event loop built 12.0 · resumed 12.1 · window created 19.3 · gpu instance 20.3 · gpu adapter 230.3 · gpu device 259.0 · renderer init 269.6 · gpu joined 269.9 · surface ready 273.0 · egui state 273.0 · egui pass 280.7 · textures uploaded 280.9 · present returned 283.4 · window shown 295.5
- **B-warp**: main 8.8 · event loop built 12.0 · resumed 12.1 · window created 19.0 · gpu device 20.2 · sg setup 20.4 · renderer init 20.6 · gpu joined 20.7 · surface ready 22.3 · egui state 22.3 · egui pass 30.2 · textures uploaded 30.5 · present returned 31.2 · window shown 43.3
- **B-hw**: main 8.8 · event loop built 11.8 · resumed 11.9 · window created 19.1 · gpu device 139.4 · sg setup 139.6 · renderer init 140.5 · gpu joined 140.9 · surface ready 143.2 · egui state 143.2 · egui pass 151.0 · textures uploaded 151.8 · present returned 153.6 · window shown 165.5

### Other facts per variant

| Variant | Exe size (MB) | Peak memory (MB) | Scale | DPI awareness / code page | GPU-related modules loaded at the stamp |
|---|---|---|---|---|---|
| R | – | – | – | – / – |  |
| A0 | 10.4 | 210.0 | – | 2 / 65001 | opengl32.dll, dxgi.dll, vulkan-1.dll, nvoglv64.dll, d3d12.dll, d3dcompiler_47.dll, D3D12Core.dll, nvldumdx.dll, nvwgf2umx.dll, D3DSCache.dll, d3d10warp.dll, Microsoft.Internal.WarpPal.dll |
| A1-warp | 10.0 | 80.3 | 1.5 | 2 / 65001 | dxgi.dll, opengl32.dll, d3d12.dll, d3dcompiler_47.dll, D3D12Core.dll, D3DSCache.dll, d3d10warp.dll, Microsoft.Internal.WarpPal.dll |
| A1-warp-f | 10.0 | 38.7 | 1.5 | 2 / 65001 | dxgi.dll, opengl32.dll, d3d12.dll, d3dcompiler_47.dll, D3D12Core.dll, d3d10warp.dll, D3DSCache.dll, Microsoft.Internal.WarpPal.dll |
| A1-hw | 10.0 | 165.1 | 1.5 | 2 / 65001 | dxgi.dll, opengl32.dll, d3d12.dll, d3dcompiler_47.dll, D3D12Core.dll, nvldumdx.dll, nvwgf2umx.dll, D3DSCache.dll, Microsoft.Internal.WarpPal.dll |
| B-warp | 6.2 | 33.3 | 1.5 | 2 / 65001 | d3d11.dll, dxgi.dll, D3D10Warp.dll, Microsoft.Internal.WarpPal.dll |
| B-hw | 6.2 | 52.4 | 1.5 | 2 / 65001 | d3d11.dll, dxgi.dll, nvldumdx.dll, nvwgf2umx.dll, Microsoft.Internal.WarpPal.dll |
