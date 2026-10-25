
### stress (median of 3 runs)

| Variant | Adapter | CPU % (1 core = 100) | CPU % (GetProcessTimes) | Process CPU ms/frame | Frames/s | Frame ms mean | Frame ms p95 |
|---|---|---|---|---|---|---|---|
| A1-warp | Microsoft Basic Render Driver (Cpu, Dx12) | 1673.9 | 1676.1 | 1819.8 | 9.2 | 108.3 | 116.8 |
| A1-warp-f | Microsoft Basic Render Driver (Cpu, Dx12) | 1675.8 | 1669.5 | 1818.5 | 9.2 | 108.2 | 116.8 |
| A1-hw | NVIDIA GeForce RTX 4080 (DiscreteGpu, Dx12) | 3.2 | 1.4 | 0.5 | 59.9 | 0.5 | 0.7 |
| B-warp | Microsoft Basic Render Driver (D3D11 WARP) | 140.3 | 146.7 | 23.4 | 59.9 | 0.4 | 0.5 |
| B-hw | NVIDIA GeForce RTX 4080 (D3D11 hardware) | 2.5 | 2.5 | 0.4 | 59.9 | 0.3 | 0.3 |
