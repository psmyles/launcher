
### Time to first frame (ms since process creation; 30 rounds after warm-up)

| Variant | Adapter | Median | 95% CI | p90 | Min | Max | Spawn→shown median | First launch |
|---|---|---|---|---|---|---|---|---|
| A1 | Apple M3 Max (IntegratedGpu, Metal) | **196.0** | 191.3–198.1 | 203.4 | 167.5 | 211.3 | 196.5 | 184.0 |
| A1-noocc | Apple M3 Max (IntegratedGpu, Metal) | **133.8** | 131.1–135.8 | 144.9 | 109.5 | 147.4 | 134.1 | 114.5 |
| B | Apple M3 Max (Metal) | **135.7** | 128.9–137.7 | 141.3 | 114.3 | 144.0 | 136.1 | 561.9 |

Per-round difference against B (median of within-round deltas, ms):

| Variant | Δ median |
|---|---|
| A1 | +62.4 |
| A1-noocc | -1.7 |

### Median phase breakdown (ms since process creation)

- **A1**: main 14.1 · gpu instance 14.3 · gpu adapter 27.2 · gpu device 29.0 · renderer init 31.7 · event loop built 67.1 · resumed 94.0 · window created 118.5 · gpu joined 118.5 · surface ready 118.9 · egui state 118.9 · egui pass 127.5 · textures uploaded 128.0 · present skipped 128.0 · present returned 188.3 · window shown 196.0
- **A1-noocc**: main 13.1 · gpu instance 13.2 · gpu adapter 26.2 · gpu device 28.0 · renderer init 30.4 · event loop built 64.9 · resumed 90.9 · window created 116.1 · gpu joined 116.1 · surface ready 116.5 · egui state 116.5 · egui pass 124.5 · textures uploaded 125.0 · present returned 126.4 · window shown 133.8
- **B**: main 13.5 · gpu device 50.9 · sg setup 51.7 · renderer init 52.5 · event loop built 64.8 · resumed 92.5 · window created 116.6 · gpu joined 116.6 · surface ready 117.1 · egui state 117.1 · egui pass 125.5 · textures uploaded 126.1 · present returned 127.8 · window shown 135.7

### Other facts per variant

| Variant | Exe size (MB) | Peak memory (MB) | GPU-related modules loaded |
|---|---|---|---|
| A1 | 5.4 | 102.1 | 30 |
| A1-noocc | 5.4 | 92.2 | 30 |
| B | 3.7 | 92.2 | 31 |
