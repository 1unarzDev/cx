# cx Files benchmark · v0.1.95-rtt20

Binary `cx 0.1.95` sha256 `de74eb41ab82`, commit `fd494d1`, runs 3, load 38.0 -> 19.5 on 20 CPUs, remote loopback sshd + 20.0 ms relay. Values: median / p95 in ms unless noted. Cold = fresh viewer process (OS-warm directory metadata). Remote viewers settle 2.5 s after startup before each measured action (see the startup table for the unsettled case).

## Straight after startup (Enter as soon as the first listing paints)

| Side | Viewer ready | 2 KB preview first paint | Directory first rows |
|---|---|---|---|
| remote | 411.6 / 561.5 | 302.2 / 391.9 | 309.0 / 598.5 |

## Directory open (Enter -> ...)

| Side | Entries | Case | First rows | Usable (j moves) | Full count | Final count |
|---|---|---|---|---|---|---|
| remote | 100 | cold | 101.7 / 101.8 | 115.0 / 116.0 | 122.9 / 123.2 | 100 |
| remote | 100 | warm | 3.0 / 3.1 | 10.9 / 45.3 | 105.5 / 110.3 | 100 |
| remote | 100 | sibling | 101.6 / 103.8 | 110.0 / 113.3 | 118.8 / 121.6 | 100 |
| remote | 1000 | cold | 106.6 / 114.7 | 133.5 / 136.6 | 144.4 / 153.5 | 1000 |
| remote | 1000 | warm | 3.1 / 5.2 | 12.6 / 17.0 | 120.5 / 123.0 | 1000 |
| remote | 1000 | sibling | 105.6 / 117.0 | 118.3 / 145.5 | 124.8 / 168.6 | 1000 |
| remote | 20000 | cold | 108.2 / 111.1 | 115.9 / 125.4 | 519.3 / 593.7 | 10612 |
| remote | 20000 | warm | 60.7 / 61.0 | 77.1 / 87.3 | 591.9 / 841.4 | 10612 |
| remote | 20000 | sibling | 106.1 / 106.8 | 118.7 / 119.5 | 492.1 / 508.8 | 10578 |
| remote | 100000 | cold | 112.8 / 113.1 | 126.6 / 131.0 | 580.3 / 597.2 | 10604 |
| remote | 100000 | warm | 18.1 / 19.9 | 43.0 / 47.8 | 524.8 / 572.8 | 10604 |
| remote | 100000 | sibling | 105.9 / 109.9 | 114.3 / 123.5 | 497.1 / 549.9 | 10570 |

| Side | Entries | Parent (h) | j in loaded list |
|---|---|---|---|
| remote | 100 | 1.9 / 2.2 | 1.8 / 5.4 |
| remote | 1000 | 2.1 / 2.3 | 4.6 / 9.9 |
| remote | 20000 | 2.0 / 2.1 | 20.5 / 34.6 |
| remote | 100000 | 1.7 / 3.3 | 19.8 / 53.0 |

Deep tree (24 levels, 10 files each): per hop

| Side | Enter hop | Parent hop | Enter total |
|---|---|---|---|

## File preview (Enter -> first content painted; G -> end)

| Side | File | Bytes | First paint | G | End reachable | Helper request (cold) | Response bytes |
|---|---|---|---|---|---|---|---|
| remote | src_2k | 2059 | 101.2 / 104.3 | 3.1 / 4.0 | yes | 21.8 / 24.6 | 2342 |
| remote | src_200k | 208946 | 205.7 / 210.0 | 8.8 / 14.8 | no | 28.7 / 33.6 | 34115 |
| remote | txt_200k | 208946 | 102.7 / 102.8 | 4.8 / 5.2 | no | 23.8 / 31.8 | 34117 |
| remote | src_5m | 5505687 | - | - | - | 25.4 / 444.6 | 34115 |
| remote | md_2m | 2097194 | - | - | - | 24.5 / 367.4 | 33672 |

## Preview scrolling (key -> first changed paint, p50 / p95 over all presses)

| Side | File | j | PageDown | Wheel | j settled |
|---|---|---|---|---|---|

## Helper protocol microbenchmark (framed `cx helper`, no UI)

| Side | Connect -> first response | Minimal request | Info |
|---|---|---|---|
| remote | 223.9 / 257.2 | 20.5 / 21.6 | 56.5 / 82.8 |

| Side | Entries | First page | Entries/page | Bytes/entry | 1000-page | UI pattern total (requests) | Full listing (requests) | fs floor scandir+sort / lstat page |
|---|---|---|---|---|---|---|---|---|
| remote | 100 | 21.2 / 28.6 | 100 | 256.5 | - | 21.2 / 28.6 (1) | 21.2 / 28.6 (1) |  |
| remote | 1000 | 26.3 / 28.3 | 1000 | 254.3 | - | 26.3 / 28.3 (1) | 26.3 / 28.3 (1) |  |
| remote | 20000 | 44.9 / 56.7 | 2612 | 256.1 | 24.5 / 32.0 | 237.5 / 297.1 (9) | 261.8 / 377.0 (8) |  |
| remote | 100000 | 91.0 / 124.4 | 2604 | 257.1 | 24.7 / 28.9 | 285.1 / 332.9 (9) | 1248 / 1339 (39) |  |
