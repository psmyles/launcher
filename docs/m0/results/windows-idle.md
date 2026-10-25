
### idle (median of 3 runs)

| Variant | Adapter | CPU ms in 10 s idle | (GetProcessTimes) | Frames drawn |
|---|---|---|---|---|
| A1-warp | Microsoft Basic Render Driver (Cpu, Dx12) | 7971.5 | 7515.6 | 6.0 |
| A1-warp-f | Microsoft Basic Render Driver (Cpu, Dx12) | 6809.1 | 6875.0 | 5.0 |
| A1-hw | NVIDIA GeForce RTX 4080 (DiscreteGpu, Dx12) | 15.6 | 0.0 | 24.0 |
| B-warp | Microsoft Basic Render Driver (D3D11 WARP) | 376.7 | 453.1 | 25.0 |
| B-hw | NVIDIA GeForce RTX 4080 (D3D11 hardware) | 15.7 | 15.6 | 25.0 |
