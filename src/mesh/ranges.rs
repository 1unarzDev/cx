//! Address ranges that overlays claim, and the collision check between them and the user's LANs.
//! A small local `IpNet` (no new dependency): parse, contain, overlap.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IpNet {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl IpNet {
    pub fn new(addr: IpAddr, prefix: u8) -> Result<Self> {
        let max = if addr.is_ipv4() { 32 } else { 128 };
        anyhow::ensure!(prefix <= max, "prefix /{prefix} too long");
        let net = Self { addr, prefix };
        Ok(Self {
            addr: net.network(),
            prefix,
        })
    }
    fn bits(addr: IpAddr) -> u128 {
        match addr {
            IpAddr::V4(a) => u32::from(a) as u128,
            IpAddr::V6(a) => u128::from(a),
        }
    }
    fn width(&self) -> u8 {
        if self.addr.is_ipv4() {
            32
        } else {
            128
        }
    }
    fn mask(&self) -> u128 {
        if self.prefix == 0 {
            return 0;
        }
        (u128::MAX << (128 - self.prefix as u32)) >> (128 - self.width() as u32)
    }
    fn network(&self) -> IpAddr {
        let v = Self::bits(self.addr) & self.mask();
        match self.addr {
            IpAddr::V4(_) => IpAddr::V4((v as u32).into()),
            IpAddr::V6(_) => IpAddr::V6(v.into()),
        }
    }
    pub fn contains(&self, ip: &IpAddr) -> bool {
        ip.is_ipv4() == self.addr.is_ipv4()
            && Self::bits(*ip) & self.mask() == Self::bits(self.addr)
    }
    pub fn overlaps(&self, other: &IpNet) -> bool {
        if self.addr.is_ipv4() != other.addr.is_ipv4() {
            return false;
        }
        let (wide, narrow) = if self.prefix <= other.prefix {
            (self, other)
        } else {
            (other, self)
        };
        wide.contains(&narrow.addr)
    }
}

impl std::fmt::Display for IpNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl std::str::FromStr for IpNet {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (s, None),
        };
        let addr: IpAddr = addr.trim().parse().context("invalid address")?;
        let prefix = match prefix {
            Some(p) => p.trim().parse().context("invalid prefix")?,
            None if addr.is_ipv4() => 32,
            None => 128,
        };
        IpNet::new(addr, prefix)
    }
}

impl Serialize for IpNet {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for IpNet {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

pub fn net(s: &str) -> IpNet {
    s.parse().expect("static range")
}

/// Cloudflare WARP's virtual-IP mesh (docs/mesh.md, the default `profile.mesh`).
pub const WARP_V4: &str = "100.96.0.0/12";
/// Tailscale: CGNAT IPv4 and its ULA IPv6 (tailscale.com/kb/1015/100.x-addresses).
pub const TAILSCALE_V4: &str = "100.64.0.0/10";
pub const TAILSCALE_V6: &str = "fd7a:115c:a1e0::/48";

/// A range with who claims it ("warp", "tailscale", "lan:enp5s0", ...).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub owner: String,
    pub range: IpNet,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collision {
    pub a: Claim,
    pub b: Claim,
    /// "nested" when one range contains the other (e.g. WARP's /12 inside Tailscale's /10),
    /// "identical" when both claim the same block. Either way an address there is ambiguous.
    pub kind: String,
    pub detail: String,
}

/// Every pair of claims from different owners whose ranges overlap. Identical owners never collide
/// with themselves. Deterministic: sorted input order, a before b.
pub fn collisions(claims: &[Claim]) -> Vec<Collision> {
    let mut out = Vec::new();
    for (i, a) in claims.iter().enumerate() {
        for b in &claims[i + 1..] {
            if a.owner == b.owner || !a.range.overlaps(&b.range) {
                continue;
            }
            // CIDR ranges that overlap always nest; equal ones are the same block claimed twice.
            let kind = if a.range == b.range {
                "identical"
            } else {
                "nested"
            };
            out.push(Collision {
                a: a.clone(),
                b: b.clone(),
                kind: kind.into(),
                detail: format!(
                    "{} ({}) overlaps {} ({}): addresses there are ambiguous",
                    a.range, a.owner, b.range, b.owner
                ),
            });
        }
    }
    out
}

/// Which claim (most specific first) an address falls in.
pub fn classify<'a>(claims: &'a [Claim], ip: &IpAddr) -> Option<&'a Claim> {
    claims
        .iter()
        .filter(|c| c.range.contains(ip))
        .max_by_key(|c| c.range.prefix)
}

/// The user's LANs as observed by `ip -j address show` (network.rs): each IPv4/IPv6 global or
/// private interface address with its prefix, owned by "lan:<ifname>". Interfaces with an address
/// in a known overlay claim are skipped whole, and so are host (/32, /128) addresses.
pub fn lans_from_ip_json(data: &serde_json::Value, overlays: &[Claim]) -> Vec<Claim> {
    let mut out = Vec::new();
    for item in data.as_array().into_iter().flatten() {
        let ifname = item["ifname"].as_str().unwrap_or("?");
        let addrs = item["addr_info"].as_array().into_iter().flatten();
        // An overlay's interface is the overlay's (its other addresses included), not a LAN.
        if addrs.clone().any(|a| {
            a["local"]
                .as_str()
                .and_then(|s| s.parse::<IpAddr>().ok())
                .is_some_and(|ip| classify(overlays, &ip).is_some())
        }) {
            continue;
        }
        for a in addrs {
            let (Some(local), Some(prefix)) = (a["local"].as_str(), a["prefixlen"].as_u64()) else {
                continue;
            };
            let Ok(ip) = local.parse::<IpAddr>() else {
                continue;
            };
            if ip.is_loopback()
                || matches!(ip, IpAddr::V6(v) if v.is_unicast_link_local())
                || matches!(ip, IpAddr::V4(v) if v.is_link_local())
                || prefix >= if ip.is_ipv4() { 32 } else { 128 }
            // a host address is no LAN
            {
                continue;
            }
            if let Ok(range) = IpNet::new(ip, prefix as u8) {
                out.push(Claim {
                    owner: format!("lan:{ifname}"),
                    range,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_contains_and_overlap() {
        let warp = net(WARP_V4);
        let ts = net(TAILSCALE_V4);
        assert_eq!(warp.to_string(), "100.96.0.0/12");
        assert!(ts.overlaps(&warp) && warp.overlaps(&ts));
        assert!(ts.contains(&"100.100.1.2".parse().unwrap()));
        assert!(warp.contains(&"100.100.1.2".parse().unwrap()));
        assert!(!warp.contains(&"100.64.0.1".parse().unwrap()));
        assert_eq!(
            "10.1.2.3/8".parse::<IpNet>().unwrap().to_string(),
            "10.0.0.0/8"
        );
        assert_eq!("192.168.1.7".parse::<IpNet>().unwrap().prefix, 32);
        assert!("0.0.0.0/0"
            .parse::<IpNet>()
            .unwrap()
            .contains(&"8.8.8.8".parse().unwrap()));
        assert!(net(TAILSCALE_V6).contains(&"fd7a:115c:a1e0::1".parse().unwrap()));
        assert!(!net(TAILSCALE_V6).overlaps(&ts), "families never overlap");
        assert!("1.2.3.4/33".parse::<IpNet>().is_err());
        let json = serde_json::to_string(&warp).unwrap();
        assert_eq!(serde_json::from_str::<IpNet>(&json).unwrap(), warp);
    }

    #[test]
    fn warp_inside_tailscale_and_lan_collisions() {
        let claims = vec![
            Claim {
                owner: "warp".into(),
                range: net(WARP_V4),
            },
            Claim {
                owner: "tailscale".into(),
                range: net(TAILSCALE_V4),
            },
            Claim {
                owner: "lan:eth0".into(),
                range: net("192.168.1.0/24"),
            },
            Claim {
                owner: "lan:wlan0".into(),
                range: net("192.168.0.0/16"),
            },
            Claim {
                owner: "lan:eth0".into(),
                range: net("192.168.1.0/25"),
            },
        ];
        let found = collisions(&claims);
        let pairs: Vec<_> = found
            .iter()
            .map(|c| (c.a.owner.as_str(), c.b.owner.as_str(), c.kind.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("warp", "tailscale", "nested"),
                ("lan:eth0", "lan:wlan0", "nested"),
                ("lan:wlan0", "lan:eth0", "nested")
            ]
        );
        assert_eq!(
            classify(&claims, &"100.100.0.1".parse().unwrap())
                .unwrap()
                .owner,
            "warp"
        );
        assert_eq!(
            classify(&claims, &"100.64.0.1".parse().unwrap())
                .unwrap()
                .owner,
            "tailscale"
        );
        assert!(classify(&claims, &"8.8.8.8".parse().unwrap()).is_none());
    }

    #[test]
    fn lans_skip_overlay_and_link_local_addresses() {
        let data = serde_json::json!([
            {"ifname":"enp5s0","addr_info":[{"local":"192.168.1.20","prefixlen":24},{"local":"fe80::1","prefixlen":64}]},
            {"ifname":"tailscale0","addr_info":[{"local":"100.70.1.2","prefixlen":32},{"local":"fd7a:115c:a1e0::9","prefixlen":48}]},
            {"ifname":"wg0","addr_info":[{"local":"10.9.0.2","prefixlen":32}]},
            {"ifname":"lo","addr_info":[{"local":"127.0.0.1","prefixlen":8}]}
        ]);
        let overlays = vec![Claim {
            owner: "tailscale".into(),
            range: net(TAILSCALE_V4),
        }];
        let lans = lans_from_ip_json(&data, &overlays);
        assert_eq!(
            lans,
            vec![Claim {
                owner: "lan:enp5s0".into(),
                range: net("192.168.1.0/24")
            }]
        );
    }
}
