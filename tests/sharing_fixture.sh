#!/bin/sh
# Isolated route-ownership fixture. No production NM/firewall/mesh changes.
set -eu
if [ "${CX_SHARING_NAMESPACE:-}" != 1 ]; then
    if ! command -v unshare >/dev/null 2>&1 || ! unshare -Urn true 2>/dev/null; then
        echo 'BLOCKED: unprivileged user/network namespaces unavailable'; exit 77
    fi
    exec unshare -Urn env CX_SHARING_NAMESPACE=1 sh "$0"
fi
ip link set lo up
for name in uplink1 uplink2 output1 output2; do
    ip link add "$name" type dummy
    ip link set "$name" up
done
ip address add 198.18.1.2/24 dev uplink1
ip address add 198.18.2.2/24 dev uplink2
ip route add default via 198.18.1.1 dev uplink1
ip route add 203.0.113.0/24 via 198.18.2.1 dev uplink2 metric 123
before=$(ip -4 route show table main)
# These represent only owned downstream addressing. Real NM DHCP/DNS is separate.
ip address add 10.42.2.1/24 dev output1
ip address add 10.42.3.1/24 dev output2
ip address del 10.42.2.1/24 dev output1
ip address del 10.42.3.1/24 dev output2
after=$(ip -4 route show table main)
[ "$before" = "$after" ] || { echo 'FAIL: unrelated route changed'; exit 1; }
echo 'PASS: namespace with two uplinks/two outputs; downstream address lifecycle preserves default and unrelated routes'
echo 'BLOCKED: NM daemon/system bus absent in fixture; DHCP/DNS/pinned egress/hardware rollback not verified'
