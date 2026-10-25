
### Time to first frame (ms since process creation; 30 rounds after warm-up)

| Variant | Adapter | Median | 95% CI | p90 | Min | Max | Spawn→shown median | First launch |
|---|---|---|---|---|---|---|---|---|
| A0 | eframe default | **157.9** | 153.0–161.7 | 164.0 | 133.9 | 184.1 | 158.3 | 625.7 |
| A1 | Apple M3 Max (IntegratedGpu, Metal) | **200.7** | 195.6–204.5 | 208.5 | 174.1 | 246.2 | 201.2 | 174.3 |
| A1-first | Apple M3 Max (IntegratedGpu, Metal) | **197.9** | 194.4–204.6 | 208.1 | 162.1 | 247.3 | 198.3 | 199.8 |
| A1-ownlayer | Apple M3 Max (IntegratedGpu, Metal) | **198.4** | 195.1–200.6 | 209.1 | 171.3 | 246.1 | 198.8 | 198.7 |
| A1-nothread | Apple M3 Max (IntegratedGpu, Metal) | **206.4** | 201.5–212.1 | 215.2 | 177.8 | 260.6 | 206.8 | 181.0 |
| A1-app | Apple M3 Max (IntegratedGpu, Metal) | **195.6** | 192.7–197.3 | 200.2 | 161.1 | 244.9 | 196.0 | 659.1 |
| B | Apple M3 Max (Metal) | **136.1** | 132.3–139.1 | 143.5 | 123.7 | 169.4 | 136.5 | 520.0 |
| B-first | Apple M3 Max (Metal) | **136.7** | 132.7–138.2 | 144.6 | 116.4 | 157.7 | 137.0 | 112.6 |
| B-nothread | Apple M3 Max (Metal) | **142.0** | 138.3–145.3 | 148.7 | 129.3 | 163.8 | 142.4 | 118.8 |
| B-app | Apple M3 Max (Metal) | **143.7** | 137.1–147.6 | 150.3 | 121.1 | 186.1 | 144.2 | 573.3 |

Per-round difference against B (median of within-round deltas, ms):

| Variant | Δ median |
|---|---|
| A0 | +21.6 |
| A1 | +64.7 |
| A1-first | +61.5 |
| A1-ownlayer | +61.5 |
| A1-nothread | +70.7 |
| A1-app | +61.0 |
| B-first | +1.4 |
| B-nothread | +6.6 |
| B-app | +6.8 |

### Median phase breakdown (ms since process creation)

- **A0**: main 15.0 · first update 150.3 · window shown 157.9
- **A1**: main 15.1 · gpu instance 15.2 · gpu adapter 28.1 · gpu device 30.0 · renderer init 32.7 · event loop built 68.2 · resumed 96.6 · window created 121.9 · gpu joined 121.9 · surface ready 122.3 · egui state 122.3 · egui pass 130.4 · textures uploaded 130.7 · present skipped 130.8 · present returned 194.2 · window shown 200.7
- **A1-first**: main 14.9 · gpu instance 15.1 · gpu adapter 28.0 · gpu device 30.0 · renderer init 32.6 · event loop built 67.3 · resumed 98.1 · window created 122.7 · gpu joined 122.7 · surface ready 123.2 · egui state 123.2 · set visible 131.0 · egui pass 139.6 · textures uploaded 140.1 · present skipped 140.2 · present returned 190.3 · window shown 197.9
- **A1-ownlayer**: main 15.0 · gpu instance 15.1 · gpu adapter 28.2 · gpu device 30.1 · renderer init 32.8 · event loop built 67.7 · resumed 96.7 · window created 120.7 · gpu joined 120.7 · surface ready 121.1 · egui state 121.1 · egui pass 129.5 · textures uploaded 129.9 · present skipped 129.9 · present returned 190.7 · window shown 198.4
- **A1-nothread**: main 14.5 · event loop built 68.3 · resumed 96.9 · window created 126.7 · gpu instance 126.7 · gpu adapter 126.8 · gpu device 128.1 · renderer init 130.0 · surface ready 130.4 · egui state 130.4 · egui pass 138.2 · textures uploaded 138.5 · present skipped 138.5 · present returned 199.3 · window shown 206.4
- **A1-app**: main 14.9 · gpu instance 15.1 · gpu adapter 31.8 · gpu device 33.4 · renderer init 35.7 · event loop built 67.0 · resumed 106.1 · window created 127.3 · gpu joined 127.3 · surface ready 127.8 · egui state 127.8 · egui pass 136.3 · textures uploaded 136.8 · present skipped 136.8 · present returned 188.1 · window shown 195.6
- **B**: main 14.7 · gpu device 51.7 · sg setup 52.5 · renderer init 53.3 · event loop built 65.9 · resumed 93.4 · window created 118.2 · gpu joined 118.2 · surface ready 118.7 · egui state 118.7 · egui pass 126.7 · textures uploaded 127.2 · present returned 128.7 · window shown 136.1
- **B-first**: main 14.3 · gpu device 51.6 · sg setup 52.4 · renderer init 53.2 · event loop built 66.0 · resumed 95.8 · window created 118.2 · gpu joined 118.2 · surface ready 118.7 · egui state 118.7 · set visible 125.7 · egui pass 134.5 · textures uploaded 135.0 · present returned 136.7 · window shown 136.7
- **B-nothread**: main 14.9 · event loop built 67.2 · resumed 96.9 · window created 123.5 · gpu device 123.6 · sg setup 124.2 · renderer init 125.0 · surface ready 125.4 · egui state 125.4 · egui pass 133.4 · textures uploaded 133.8 · present returned 135.1 · window shown 142.0
- **B-app**: main 14.9 · gpu device 52.0 · sg setup 52.8 · renderer init 53.6 · event loop built 68.9 · resumed 105.2 · window created 125.3 · gpu joined 125.3 · surface ready 125.8 · egui state 125.8 · egui pass 134.1 · textures uploaded 134.6 · present returned 135.9 · window shown 143.7

### Other facts per variant

| Variant | Exe size (MB) | Peak memory (MB) | GPU-related modules loaded |
|---|---|---|---|
| A0 | 5.9 | 102.4 | 30 |
| A1 | 5.4 | 102.1 | 30 |
| A1-first | 5.4 | 102.4 | 30 |
| A1-ownlayer | 5.4 | 102.4 | 30 |
| A1-nothread | 5.4 | 102.2 | 31 |
| A1-app | 5.3 | 103.8 | 31 |
| B | 3.7 | 92.3 | 31 |
| B-first | 3.7 | 92.1 | 30 |
| B-nothread | 3.7 | 91.8 | 31 |
| B-app | 3.7 | 93.9 | 31 |
