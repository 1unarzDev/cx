# cx Files benchmark · v0.1.95-baseline

Binary `cx 0.1.95` sha256 `de74eb41ab82`, commit `fd494d1`, runs 5, load 33.6 -> 35.1 on 20 CPUs, remote loopback sshd. Values: median / p95 in ms unless noted. Cold = fresh viewer process (OS-warm directory metadata). Remote viewers settle 2.5 s after startup before each measured action (see the startup table for the unsettled case).

## Straight after startup (Enter as soon as the first listing paints)

| Side | Viewer ready | 2 KB preview first paint | Directory first rows |
|---|---|---|---|
| local | 79.4 / 152.6 | 203.8 / 288.6 | 114.5 / 125.6 |
| remote | 210.9 / 837.8 | 304.3 / 999.5 | 306.4 / 1222 |

## Directory open (Enter -> ...)

| Side | Entries | Case | First rows | Usable (j moves) | Full count | Final count |
|---|---|---|---|---|---|---|
| local | 100 | cold | 103.1 / 106.1 | 130.5 / 195.3 | 151.3 / 228.2 | 100 |
| local | 100 | warm | 2.4 / 7.3 | 35.1 / 47.7 | 63.4 / 78.0 | 100 |
| local | 100 | sibling | 103.2 / 104.5 | 128.8 / 171.8 | 153.2 / 205.9 | 100 |
| local | 1000 | cold | 110.6 / 119.6 | 142.0 / 196.4 | 154.8 / 233.4 | 1000 |
| local | 1000 | warm | 6.8 / 9.3 | 26.1 / 30.0 | 46.7 / 51.8 | 1000 |
| local | 1000 | sibling | 109.8 / 111.9 | 130.3 / 145.6 | 147.4 / 161.1 | 1000 |
| local | 20000 | cold | 151.0 / 204.2 | 227.8 / 249.4 | 557.3 / 1043 | 10612 |
| local | 20000 | warm | 56.6 / 124.3 | 72.5 / 152.9 | 750.5 / 1043 | 10612 |
| local | 20000 | sibling | 125.4 / 310.1 | 193.9 / 372.4 | 733.6 / 1450 | 10578 |
| local | 100000 | cold | 231.5 / 420.5 | 283.1 / 470.5 | 820.4 / 1228 | 10604 |
| local | 100000 | warm | 82.0 / 121.3 | 119.0 / 227.7 | 943.9 / 1305 | 10604 |
| local | 100000 | sibling | 329.2 / 658.1 | 372.6 / 702.5 | 985.1 / 1391 | 10570 |
| remote | 100 | cold | 101.6 / 105.5 | 110.3 / 131.3 | 118.2 / 178.0 | 100 |
| remote | 100 | warm | 2.2 / 3.2 | 10.9 / 60.6 | 18.9 / 82.7 | 100 |
| remote | 100 | sibling | 101.8 / 105.5 | 111.8 / 145.5 | 118.1 / 189.6 | 100 |
| remote | 1000 | cold | 106.1 / 113.7 | 120.6 / 150.2 | 127.1 / 177.1 | 1000 |
| remote | 1000 | warm | 3.0 / 8.1 | 13.4 / 43.9 | 18.4 / 82.2 | 1000 |
| remote | 1000 | sibling | 105.4 / 118.8 | 115.2 / 168.8 | 119.2 / 197.5 | 1000 |
| remote | 20000 | cold | 112.5 / 208.3 | 129.7 / 279.0 | 496.6 / 1116 | 10612 |
| remote | 20000 | warm | 23.5 / 117.6 | 35.5 / 150.5 | 432.6 / 1248 | 10612 |
| remote | 20000 | sibling | 108.7 / 217.6 | 124.1 / 352.0 | 493.3 / 984.4 | 10578 |
| remote | 100000 | cold | 239.1 / 493.7 | 288.6 / 537.4 | 861.5 / 1181 | 10604 |
| remote | 100000 | warm | 33.8 / 87.4 | 61.0 / 234.1 | 552.6 / 1394 | 10604 |
| remote | 100000 | sibling | 219.2 / 501.3 | 245.4 / 540.7 | 609.5 / 1155 | 10570 |

| Side | Entries | Parent (h) | j in loaded list |
|---|---|---|---|
| local | 100 | 3.6 / 15.0 | 6.3 / 17.7 |
| local | 1000 | 3.3 / 7.9 | 7.5 / 18.8 |
| local | 20000 | 5.1 / 5.9 | 59.2 / 219.1 |
| local | 100000 | 2.2 / 13.8 | 53.3 / 127.6 |
| remote | 100 | 2.3 / 3.0 | 1.5 / 15.9 |
| remote | 1000 | 2.0 / 7.5 | 2.9 / 15.4 |
| remote | 20000 | 2.2 / 2.5 | 24.6 / 108.7 |
| remote | 100000 | 2.4 / 23.8 | 27.2 / 101.1 |

Deep tree (24 levels, 10 files each): per hop

| Side | Enter hop | Parent hop | Enter total |
|---|---|---|---|
| local | 104.3 / 119.7 | 2.2 / 5.8 | 2456 / 2519 |
| remote | 104.0 / 114.8 | 2.0 / 6.1 | 2379 / 2418 |

## File preview (Enter -> first content painted; G -> end)

| Side | File | Bytes | First paint | G | End reachable | Helper request (cold) | Response bytes |
|---|---|---|---|---|---|---|---|
| local | src_2k | 2059 | 102.2 / 185.2 | 12.3 / 22.1 | yes | 0.2 / 2.4 | 2342 |
| local | src_200k | 208946 | 211.3 / 554.0 | 19.0 / 43.1 | no | 0.9 / 5.6 | 34115 |
| local | txt_200k | 208946 | 104.7 / 105.7 | 18.9 / 29.0 | no | 1.8 / 4.3 | 34117 |
| local | src_5m | 5505687 | 206.8 / 573.9 | 12.3 / 23.8 | no | 2.8 / 5.0 | 34115 |
| local | src_50m | 56190067 | 209.2 / 617.3 | 15.9 / 34.1 | no | 1.6 / 2.4 | 34117 |
| local | md_2m | 2097194 | 132.6 / 144.0 | 20.0 / 35.3 | no | 0.7 / 5.0 | 33672 |
| local | png | 94384 | 118.2 / 315.9 | - | - | 112.1 / 205.3 | 217192 |
| local | jpeg | 45707 | 225.3 / 322.2 | - | - | 84.0 / 119.8 | 343368 |
| local | pdf | 83373 | 1523 / 2704 | - | - | 3051 / 3093 | 272 |
| local | binary | 1048576 | 102.9 / 104.4 | - | - | 0.3 / 0.3 | 241 |
| local | unicode | 20530 | 307.1 / 406.2 | - | - | 0.4 / 0.4 | 21495 |
| remote | src_2k | 2059 | 101.3 / 183.7 | 6.0 / 13.9 | yes | 0.4 / 2.5 | 2342 |
| remote | src_200k | 208946 | 205.2 / 526.6 | 7.3 / 42.8 | no | 0.7 / 1.3 | 34115 |
| remote | txt_200k | 208946 | 103.2 / 103.8 | 6.0 / 19.7 | no | 0.8 / 1.0 | 34117 |
| remote | src_5m | 5505687 | 207.4 / 537.3 | 14.9 / 44.4 | no | 0.7 / 0.7 | 34115 |
| remote | src_50m | 56190067 | 307.9 / 494.4 | 21.1 / 45.9 | no | 0.6 / 1.4 | 34117 |
| remote | md_2m | 2097194 | 129.1 / 184.5 | 14.5 / 38.2 | no | 0.6 / 1.4 | 33672 |
| remote | png | 94384 | 115.5 / 300.1 | - | - | 39.9 / 112.3 | 217192 |
| remote | jpeg | 45707 | 214.0 / 245.7 | - | - | 52.9 / 175.3 | 343368 |
| remote | pdf | 83373 | 2031 / 2621 | - | - | 1652 / 2871 | 405358 |
| remote | binary | 1048576 | 103.0 / 104.5 | - | - | 0.4 / 0.7 | 241 |
| remote | unicode | 20530 | 306.0 / 404.2 | - | - | 0.6 / 0.7 | 21495 |

## Preview scrolling (key -> first changed paint, p50 / p95 over all presses)

| Side | File | j | PageDown | Wheel | j settled |
|---|---|---|---|---|---|
| local | src_5m | 11.7 / 30.9 | 10.5 / 27.9 | 9.9 / 24.3 | 16.0 / 41.0 |
| local | md_2m | 4.5 / 20.2 | 6.2 / 38.1 | 8.1 / 35.7 | 5.1 / 22.6 |
| remote | src_5m | 17.1 / 29.8 | 16.8 / 38.9 | 20.0 / 38.4 | 18.5 / 35.9 |
| remote | md_2m | 11.1 / 20.5 | 13.0 / 23.1 | 16.9 / 43.2 | 12.5 / 25.2 |

## Helper protocol microbenchmark (framed `cx helper`, no UI)

| Side | Connect -> first response | Minimal request | Info |
|---|---|---|---|
| local | 6.8 / 14.7 | 0.0 / 0.1 | 145.7 / 246.3 |
| remote | 118.3 / 133.5 | 0.1 / 0.1 | 41.5 / 175.1 |

| Side | Entries | First page | Entries/page | Bytes/entry | 1000-page | UI pattern total (requests) | Full listing (requests) | fs floor scandir+sort / lstat page |
|---|---|---|---|---|---|---|---|---|
| local | 100 | 0.8 / 4.2 | 100 | 256.5 | - | 0.8 / 4.2 (1) | 0.8 / 4.2 (1) | 0.1 / 0.2 |
| local | 1000 | 15.8 / 31.6 | 1000 | 254.3 | - | 15.8 / 31.6 (1) | 15.8 / 31.6 (1) | 3.8 / 5.1 |
| local | 20000 | 82.8 / 159.7 | 2612 | 256.1 | 11.5 / 32.1 | 177.4 / 375.1 (9) | 277.5 / 797.8 (8) | 50.9 / 12.7 |
| local | 100000 | 112.9 / 632.4 | 2604 | 257.1 | 8.4 / 37.6 | 180.7 / 858.5 (9) | 1280 / 2690 (39) | 266.9 / 13.0 |
| remote | 100 | 1.1 / 8.5 | 100 | 256.5 | - | 1.1 / 8.5 (1) | 1.1 / 8.5 (1) |  |
| remote | 1000 | 6.0 / 36.9 | 1000 | 254.3 | - | 6.0 / 36.9 (1) | 6.0 / 36.9 (1) |  |
| remote | 20000 | 16.7 / 98.3 | 2612 | 256.1 | 4.1 / 26.8 | 48.9 / 260.0 (9) | 111.2 / 556.9 (8) |  |
| remote | 100000 | 76.3 / 375.9 | 2604 | 257.1 | 4.7 / 28.0 | 108.8 / 541.3 (9) | 781.6 / 2986 (39) |  |

## Notes

- run 2 local preview pdf: timed out waiting for pdf first paint
- run 2 remote preview pdf: timed out waiting for pdf first paint
- run 3 local preview pdf: timed out waiting for pdf first paint
