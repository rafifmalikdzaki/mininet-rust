# Topology, routing, tenancy, and link-design catalogue

This document is the durable design catalogue for complex topologies that go
beyond the six default research presets. It defines **what to build**, **how to
test it**, and **what the runner can and cannot do today**.

The designs are intentionally layered. A design may reuse an underlay from an
earlier design and add routing, tenancy, or a link-failure model on top.

## 1. Runner constraints that shape every design

| Constraint | Consequence |
|---|---|
| Node kinds are `host` or `switch` only | Routers, firewalls, and servers are `host` nodes running FRR/iptables/etc. A `switch` is a real Open vSwitch bridge by default; `lxbr` is a legacy alternative. |
| Links are veth pairs | Every link is point-to-point; L2 broadcast domains require a `switch` node or a bridge created inside a host namespace. |
| Scenario switches use OVS (`ovs,failMode=standalone`) by default | Use `ovs-vsctl` for VLAN access/trunk configuration. OVS has no controller; standalone learning is used. LACP/EVPN/OpenFlow control planes remain unimplemented. `lxbr` scenarios must declare `"switch":"lxbr"`. |
| Node names 1–9 ASCII letters/digits/underscore | Use short prefixes: `sp1`, `lf2`, `cr1`, `pe1`, `h10`, `t1a`. |
| Interface names `<node>-eth<port>` ≤ 15 chars | A 9-character node name cannot have a 3-digit port. Prefer 3–5-character node names for dense fabrics. |
| Host ports start at 0; switch ports start at 1 | Always write explicit ports for designs with more than one link per node. |
| MCP graph bounds | 256 nodes / 1024 links. The research runner itself is bounded by kernel resources, not this limit. |
| Scenario commands run as root in a privileged container | Treat every scenario as trusted code. Do not run untrusted designs. |
| Steps are fail-fast | A failed step stops later steps. Test independent failures in separate scenarios or add `continue_on_failure` support first. |
| No built-in link parameters in the scenario schema | Apply `tc`/netem, bonding, or VXLAN inside step commands. The core `TCLink` exists, but the research runner’s custom graph uses default links. |

## 2. Layer model

| Layer | What it means in this workbench | Typical primitives |
|---|---|---|
| L0 physical | veth pairs, namespaces, ports | `Topo`, link list, explicit ports |
| L1 link | single/parallel/redundant links, loss/delay | veth, bonding, `tc netem` |
| L2 switching | bridges, VLAN filtering, trunks, access ports | OVS (`ovs-vsctl` VLAN tags/trunks), legacy LinuxBridge (`bridge vlan`) |
| L3 routing | OSPF, IS-IS, eBGP, ECMP, route policy | FRR in host namespaces |
| L3.5 tenancy | VRFs, overlapping prefixes, route leaking | Linux VRF, FRR VRF instances |
| L4–7 service | NAT/firewall/service chain/anycast | iptables/nftables, server processes |
| Control | protocol timers, route reflectors, policies | FRR configuration |
| Evidence | events, telemetry, provenance, monitor | runner artifacts, monitor, Loki |

## 2.1 Design index

| ID | Design | Focus | Nodes / links | Complexity |
|---|---|---|---|---|
| D01 | Clos spine-leaf | eBGP L3 fabric, ECMP | 14 / 16 | large |
| D02 | Three-tier hierarchy | core/agg/access, OSPF areas, VLANs | 18 / 20 | large |
| D03 | VRF-lite tenancy | overlapping tenants, shared core | 12 / 13 | large |
| D04 | Multi-site VLAN trunks | L2 isolation + redundant trunks | 15 / 13 | medium |
| D05 | VXLAN overlay | tenant L2 over routed underlay | 8 / 8 | large |
| D06 | Dual-homed access | active-backup / LACP boundary | 3 / 2 | small–medium |
| D07 | Ring + chords | IS-IS/OSPF, ECMP, partition | 6 / 9 | medium |
| D08 | Hub-and-spoke | eBGP policy, transit prevention | 7 / 6 | small–medium |
| D09 | Torus fabric | scale, path diversity | 32 / 48 | large |
| D10 | Service chain | firewall, NAT, tenant policy | 6 / 5 | medium |
| D11 | IPv6 dual-stack | OSPFv3/BGP, anycast failover | 6 / 5 | medium |
| D12 | Chaos overlay | fault, interruption, cleanup | any | cross-cutting |

### 2.1.1 Implemented scenario files

| Design | Scenario file | Status |
|---|---|---|
| D04 | `vlan-ovs.json`, `multi-tenant-ovs.json` | implemented, rootless OVS |
| D04/C4 | `multi-tenant-fault-ovs.json` | implemented, trunk fault/recovery |
| D01 (L2 variant) | `clos-l2-ovs.json` | implemented, redundant OVS + STP |
| D02/D03 routing partial | `ospf-ovs.json` | implemented, OSPF over OVS |
| D06a | `dual-home-ovs.json` | implemented, active-backup failover |
| D12 | failure steps in the OVS scenarios | partial |

The remaining D01 L3, D02, D03, D05, D06b, D07–D11 designs are specified below
and tracked in `ROADMAP.md`.

---

## 3. Naming, addressing, and isolation conventions

Use one plan across designs so results are comparable:

| Object | Rule | Example |
|---|---|---|
| Spine / core | `spN` / `crN` | `sp1`, `cr2` |
| Leaf / aggregation | `lfN` / `agN` | `lf1`, `ag2` |
| Access switch | `acN` or `sN` | `ac1` |
| Tenant host | `<tenant><site>` | `a1`, `b2`, `t1a` |
| Underlay link | `/31` from `10.0.0.0/16` | `10.0.12.0/31` |
| Router loopback | `/32` from `10.255.0.0/16` | `10.255.0.11/32` |
| Tenant block | deliberately overlapping per tenant | `10.20.0.0/24` for all tenants |
| Tenant VLAN | `100 + tenant_id` | tenant 1 → VLAN 101 |
| Tenant VRF | `t<tenant_id>` | `t1`, `t2` |
| VNI | `10000 + tenant_id` | `10001` |
| IPv6 ULA | `fd00:<tenant>:<site>::/64` | `fd00:1:2::/64` |

**Isolation rule:** every positive test needs a matching negative test. A
tenant design is not accepted until the full tenant × tenant matrix has been
checked, not just one sampled pair.

## 3.1 Routing choices

| Scheme | Best for | Strengths | Costs / limits |
|---|---|---|---|
| Static routes | small hub-spoke, service chains | deterministic, easy to debug | no reconvergence, manual failure handling |
| OSPF | campus/enterprise hierarchy, areas | multi-area summarization, familiar | area design and ABR placement needed |
| IS-IS | service-provider-style underlay, rings/torus | level hierarchy, fast convergence, address-family neutral | less familiar; NET planning |
| eBGP | spine-leaf fabric, WAN, policy-heavy designs | explicit policy, AS-path control, ECMP | more configuration; policy mistakes can leak routes |
| OSPFv3 / BGP for IPv6 | dual-stack designs | native IPv6 | telemetry probes currently IPv4-oriented |

## 3.2 Tenancy choices

| Mechanism | Isolation | Overlapping addresses | Scale | Runner feasibility |
|---|---|---|---|---|
| VLAN per tenant | L2 broadcast isolation | Only with separate L3 gateways | 4094 VLANs, bridge limits | **Supported today** via OVS `tag`/`trunks` (and legacy `bridge vlan`) |
| VRF-lite | L3 routing-table isolation | Yes | one VRF per tenant per router | **Prototype**; Linux VRF + FRR |
| VRF + VLAN | L2 and L3 isolation | Yes | good for campus/PE | Prototype |
| VXLAN VNI | L2 overlay isolation | Yes | high, but MTU/encap overhead | **Prototype**; kernel VXLAN + static FDB |
| EVPN/VXLAN | overlay with control plane | Yes | high | Not implemented; large project |

**Recommendation:** implement D04 first (VLAN isolation, low risk), then D03
(VRF-lite, high research value), then D05 (VXLAN, highest complexity).

## 3.3 Link and layer choices

| Link design | Topology | Failure behaviour | Feasibility |
|---|---|---|---|
| Single link | any | partition | supported |
| Parallel links | leaf pair / dual-homed host | one link survives | supported; host bond needed |
| Active-backup bond | dual-homed host to two OVS bridges/ports | automatic failover | **verified** in rootless Podman (`dual-home-ovs`) |
| LACP bond | dual-homed host to one logical OVS switch | graceful member loss | OVS supports LACP; host-bond combination not yet tested |
| Ring | routers/switches | alternate path if one cut | supported |
| Full mesh | core/leaf | rich ECMP, expensive cabling | supported at small scale |
| Torus | regular high-radix fabric | path diversity | supported at small scale |
| Overlay | VXLAN over routed underlay | depends on underlay | prototype |

---

# Design catalogue

## D01 — Clos / spine-leaf, L3 underlay with eBGP

**Purpose.** Modern data-centre fabric: two spines, several leaves, servers
attached to leaves, eBGP between leaves and spines, ECMP to server prefixes.

**Implementation status.** The redundant L2 fabric variant is implemented as
`clos-l2-ovs.json` with four OVS switches and STP. The L3 eBGP/ECMP variant
remains to be built.

```text
        sp1 =============== sp2
       /   \               /   \
     lf1    lf2         lf3    lf4
     / \    / \          / \    / \
   h1  h2  h3  h4      h5  h6  h7  h8
```

| Item | Plan |
|---|---|
| Nodes | 2 spine routers, 4 leaf routers (all `host`), 8 servers |
| Links | 8 leaf–spine + 8 leaf–server = 16 |
| Addressing | `/31` underlay; loopback `10.255.0.<n>/32`; server subnet `10.10.<leaf>.0/24` |
| Routing | eBGP: spines AS 65000, leaves AS 65001–65004; advertise loopbacks and server prefixes |
| ECMP | Each leaf has two equal paths to every remote server prefix |
| Ports | Explicit; leaf uplinks 0/1, server downlinks 2/3 |

**Test plan.**

1. **T0 plan:** node, link, and port counts; every port unique; no interface >15
   characters.
2. **T1 reachability:** all 8 servers ping all 8 servers, 0% loss.
3. **T2 routing:** every leaf has two BGP sessions `Established`; every leaf
   has two next hops for each remote server prefix.
4. **T4 redundancy:** down one leaf–spine link; remote reachability survives;
   ECMP set shrinks to one; restore and confirm two paths return.
5. **T7 failure:** down one leaf; only its two servers become unreachable;
   other leaves keep full reachability.
6. **Evidence:** `events.jsonl` contains one `finish` with `success: true`;
   copied FRR logs show both sessions.

**Acceptance.** 56 ordered server pairs pass; no path blackhole after one
uplink failure; FRR route count matches expected prefix count.

**Feasibility.** High. All routers are `host` nodes; direct host–host links are
supported. Main effort is generated FRR configuration and per-interface
addressing.

**Effort:** large (configuration generator recommended).

---

## D02 — Three-tier hierarchy: core / aggregation / access with OSPF areas

**Purpose.** Classic campus or enterprise hierarchy, not a flat fabric. Tests
area boundaries, summarization, VLANs, and north–south paths.

```text
        cr1 ========== cr2            OSPF area 0
        /  \           /  \
      ag1  ag2       ag3  ag4          areas 1 / 2
      |    |          |    |
     ac1  ac2        ac3  ac4          L2 VLAN access
     / \   / \       / \   / \
    h1 h2 h3 h4    h5 h6 h7 h8
```

| Item | Plan |
|---|---|
| Nodes | 2 core routers, 4 aggregation routers, 4 access bridges, 8 hosts |
| Links | core–agg full mesh (8), agg–access (4), access–host (8) = 20 |
| Routing | OSPF: core area 0; agg pairs in areas 1 and 2; loopbacks and access subnets advertised |
| L2 | Access bridge with two VLANs (110 users, 120 services); trunk to aggregation |
| Addressing | Core/agg `/31`; access VLANs `10.30.<area>.<vlan>.0/24` |
| Policy | Area summaries at aggregation; no transit through access |

**Test plan.**

1. **T0:** plan and port allocation.
2. **T1:** intra-VLAN reachability within each access switch.
3. **T2:** OSPF neighbors `Full`; inter-area route exists; core sees summarized
   routes, not every host route.
4. **T3:** VLAN 110 cannot reach VLAN 120 without the aggregation router.
5. **T4:** failure of one agg–core link reroutes through the other core.
6. **T6:** ping loss during reconvergence; record attempts to converge.

**Acceptance.** All intra-VLAN pairs pass; inter-VLAN policy matches intent;
one core failure causes no loss after convergence; summaries present.

**Feasibility.** High, but aggregation nodes need multiple bridge/VLAN
interfaces. This is the best design for testing “layers” rather than a flat
fabric.

**Effort:** large.

---

## D03 — VRF-lite multi-tenant shared core with overlapping addresses

**Purpose.** Tenant separation at L3 without overlay encapsulation. Two sites,
two PEs, four tenants; every tenant reuses `10.20.0.0/24`.

```text
 site A:  t1a t2a t3a t4a        site B:  t1b t2b t3b t4b
             \ | / |                        \ | / |
              pe1 (VRF t1..t4) ==== pe2 (VRF t1..t4)
                    \                 /
                     cr1 ========= cr2
```

| Item | Plan |
|---|---|
| Nodes | 2 core routers, 2 PE routers, 8 tenant hosts |
| Links | PE–hosts (8), PE–core (4), core–core (1) = 13 |
| Tenancy | One Linux VRF per tenant on each PE; one VLAN/VRF attachment per host |
| Addressing | Every tenant uses `10.20.0.0/24`; hosts get `10.20.0.1–.8` |
| Routing | Per-VRF OSPF or static routes over the core; shared core uses its own routing instance |
| Shared service | Optional `10.99.0.0/24` route leaked into all VRFs; test deliberate leak |
| Isolation | No route leaking between tenant VRFs by default |

**Test plan.**

1. **T1:** the eight same-tenant host pairs across sites ping successfully.
2. **T3:** all 48 ordered cross-tenant pairs fail with ping exit 1.
3. **T3b:** inspect `ip route show vrf tN`; assert tenant prefixes exist only in
   the matching VRF.
4. **T5:** enable a shared service in one VRF; assert only that tenant reaches
   it, then leak it intentionally and assert all tenants reach it.
5. **T4:** core link failure; per-VRF routes reconverge; isolation unchanged.
6. **Evidence:** each probe records the VRF name in the test output.

**Acceptance.** Positive matrix 8/8; negative matrix 48/48; no default route
leak; shared-service behaviour matches policy.

**Feasibility.** Medium–large. Requires Linux VRF support plus FRR VRF
instances; this is the highest-value tenancy design and a prerequisite for
meaningful “multi-tenant routing.”

**Effort:** large; validate VRF support in the container before investing.

---

## D04 — Multi-site VLAN trunks with redundant trunk paths

**Purpose.** Extend the existing `multi-tenant` preset into a resilient L2
design: three sites, three tenants, two trunk paths between sites, and a
failure/recovery sequence.

```text
 site A          site B          site C
 s1   s2         s3   s4         s5   s6
 (hosts omitted; three tenant hosts per site)

 primary trunks:   s1 -- s3 -- s5
 alternate trunks: s2 -- s4 -- s6
```

| Item | Plan |
|---|---|
| Nodes | 6 bridges, 9 tenant hosts |
| Links | 9 access + 2 primary trunks + 2 redundant trunks = 13 |
| VLANs | 10, 20, 30 on all inter-site trunks; access ports untagged |
| Redundancy | Primary path `s1–s3–s5`; alternate `s2–s4–s6`; one trunk down at a time |
| Protection | Linux bridge STP, or an explicit loop-free topology; avoid accidental loops |
| Addressing | Overlapping tenant blocks as in the existing preset |

**Test plan.**

1. **T1:** full intra-tenant matrix across all three sites.
2. **T3:** full cross-tenant matrix; all must fail.
3. **T4:** down the primary trunk; intra-tenant reachability recovers through
   the alternate path; cross-tenant isolation remains.
4. **T4b:** restore the primary; no duplicate delivery or persistent loop.
5. **T7:** down an access port; only that tenant host is isolated.
6. **Cleanup:** bridge FDB/VLAN entries and links gone after stop.

**Acceptance.** All tenant pairs recover within the retry budget; zero
cross-tenant success at every stage (before/during/after fault).

**Feasibility.** High; closest to existing presets. Best first step for
multi-site tenant research.

**Effort:** medium.

---

## D05 — VXLAN overlay with a routed underlay

**Purpose.** Decouple tenant L2 domains from the physical leaf/spine underlay.
Two sites, two VTEP leaves, two tenants, overlapping tenant subnets.

```text
 tenant A at site 1 --- lf1 (VTEP) ==== routed underlay ==== lf2 (VTEP) --- tenant A at site 2
 tenant B at site 1 --- /                                      \ --- tenant B at site 2
```

| Item | Plan |
|---|---|
| Nodes | 2 spine routers, 2 leaf/VTEP routers, 4 tenant hosts |
| Links | leaf–spine (4), leaf–host (4) = 8 |
| Underlay | OSPF; loopbacks reachable; VXLAN UDP 4789 |
| Overlay | VNI 10001/10002; static remote VTEP FDB (no EVPN control plane) |
| Tenancy | One VNI and one bridge VLAN per tenant; same tenant subnet at both sites |
| Isolation | No VNI/VLAN cross-connect between tenants |

**Test plan.**

1. **T1:** underlay loopbacks ping.
2. **T2:** OSPF routes for both VTEP loopbacks.
3. **T1-overlay:** tenant A site 1 ↔ tenant A site 2 passes; same for B.
4. **T3:** all cross-tenant pings fail.
5. **T4:** break the underlay path; overlay fails; restore and verify recovery.
6. **T5:** change one VNI/VLAN mapping and assert deliberate misconfiguration is
   observable, not silently isolating both tenants.

**Acceptance.** Same-tenant cross-site reachability; cross-tenant failure;
correct VTEP/FDB state; no underlay route leaks into tenant namespaces.

**Feasibility.** Medium. Needs kernel VXLAN support and careful namespace
placement. Treat as a prototype until a minimal two-VTEP smoke test passes in
the research image.

**Effort:** large.

---

## D06 — Dual-homed access links: active-backup and LACP variants

**Purpose.** Test link redundancy and aggregation semantics. Two variants make
the capability boundary explicit.

```text
                 lf1
                /   \
             h1      (two links)
                \   /
                 lf2
```

| Variant | Node/link plan | What it tests |
|---|---|---|
| **D06a active-backup** | 1 host with 2 links to 2 bridges; `bond0` mode 1 | failover when one link/bridge dies; no switch LACP required |
| **D06b LACP** | same links; `bond0` mode 4 | requires LACP on switch side; LinuxBridge cannot peer, so this needs OVS or an external switch |

**Test plan.**

1. **T0:** both member interfaces exist; bond reports expected mode.
2. **T1:** host reaches a peer through the bond.
3. **T4a:** down the active member; traffic continues through the backup;
   `active_slave` changes.
4. **T4b:** restore the first member; no loop or duplicate traffic.
5. **T7:** down one bridge entirely; failover still works.
6. **D06b:** only run if the switch side actually supports LACP; otherwise
   record the test as unsupported rather than a pass.

**Acceptance.** Active-backup failover under 2 s with no permanent loss; LACP
either passes with a LACP-capable datapath or is explicitly marked unsupported.

**Feasibility.** D06a is implemented and verified rootless. D06b is not yet
verified: OVS supports LACP, but the host bond and OVS port configuration need
a dedicated experiment before claiming support.

**Effort:** small for D06a; medium for D06b with OVS.

---

## D07 — Ring plus chords with IS-IS and ECMP

**Purpose.** Non-hierarchical topology with path diversity, equal-cost paths,
and loop-free reconvergence.

```text
      r1 ---- r2 ---- r3
      |               |
      r6 ---- r5 ---- r4

      chords: r1-r4, r2-r5, r3-r6
```

Concrete plan: six routers in a ring (`r1–r2–r3–r4–r5–r6–r1`) plus three
chords (`r1–r4`, `r2–r5`, `r3–r6`).

| Item | Plan |
|---|---|
| Nodes | 6 routers (hosts), optionally 6 attached servers |
| Links | 6 ring + 3 chords = 9, plus access links |
| Routing | IS-IS level 2 or OSPF area 0; loopbacks advertised |
| ECMP | Two or three equal paths between opposite routers |
| Failure | Any single link; then any single node |

**Test plan.**

1. **T2:** all adjacencies `Up`/`Full`.
2. **T1:** all loopback pairs reachable.
3. **T2b:** `ip route` shows multiple next hops for at least one remote prefix.
4. **T4:** cut one ring link; no loss after convergence; path via chord.
5. **T7:** cut two links that partition the ring; only the disconnected subset
   loses reachability; no false recovery.
6. **T8:** repeat with 12 routers and more chords.

**Acceptance.** Single-link failure preserves all reachability; two-link
partition is diagnosed correctly; no forwarding loop after restore.

**Feasibility.** High. Good test of route churn and ECMP without complex
tenancy.

**Effort:** medium.

---

## D08 — Hub-and-spoke eBGP with route policy

**Purpose.** WAN-style routing: one hub, three spokes, default routes, prefix
filters, and prevention of transit leaks.

```text
        s1        s2        s3
         \        |        /
          \       |       /
                hub
```

| Item | Plan |
|---|---|
| Nodes | 1 hub, 3 spokes, 3 server hosts |
| Links | 3 router–router + 3 router–server = 6 |
| Routing | eBGP; hub AS 65000, spokes AS 65001–65003 |
| Policy | Hub advertises default only; spokes advertise their server prefix; no spoke-to-spoke transit |
| Failure | Hub link down; only that spoke loses remote reachability |

**Test plan.**

1. **T2:** all three sessions `Established`.
2. **T1:** spoke servers reach each other via hub.
3. **T5:** remove the prefix filter; assert the hub does **not** re-advertise a
   spoke prefix back to another spoke (no transit).
4. **T4:** down one spoke–hub link; other spokes remain reachable.
5. **T5b:** inject a bogus prefix; assert it is rejected by policy.

**Acceptance.** Positive reachability 3/3; no transit route in any spoke;
failure blast radius exactly one spoke.

**Feasibility.** High; small and fast.

**Effort:** small–medium.

---

## D09 — Torus / high-radix fabric at scale

**Purpose.** Scale, path diversity, and ECMP under a regular high-radix graph.

Start with a 4×4 torus of router-hosts (or bridge switches) and one server per
router; then 6×6. The built-in `torus,4,4,1` model can be used for a plan-only
or switch-based variant; a routing variant uses router `host` nodes.

| Item | Plan |
|---|---|
| Small | 16 routers + 16 servers; 32 torus links + 16 access links |
| Medium | 36 routers + 36 servers; 72 + 36 links |
| Routing | OSPF/IS-IS; loopbacks; ECMP |
| Tests | all server pairs, path count, single/double link failure, start/stop time |
| Metrics | wall time, RSS, artifact size, convergence attempts, leak count |

**Acceptance.** 16-node fabric starts and passes all pairs; one-link failure
does not partition; 36-node run stays within a documented time/disk budget.

**Feasibility.** Medium–large. Name/interface limits and kernel resource limits
must be checked; use generated configuration.

**Effort:** large; do after D01/D07.

---

## D10 — Layered service chain with tenant policy

**Purpose.** Add L4–7 layers to a routed fabric: edge → firewall → services,
with tenant-specific allow/deny policy.

```text
 tenant A/B hosts --- edge router --- firewall --- app1/app2
                         |
                    shared service
```

| Item | Plan |
|---|---|
| Nodes | 2 tenant hosts, 1 edge router, 1 firewall host, 2 app hosts |
| Links | tenant–edge (2), edge–firewall (1), firewall–apps (2) |
| L3 | Static routes or OSPF; firewall forwarding enabled |
| Policy | iptables/nftables: allow tenant A → app1, deny tenant A → app2; tenant B inverse |
| NAT | Optional source NAT for one tenant; verify return path |
| Tests | positive allow, negative deny, stateful return traffic, policy after link flap |

**Acceptance.** All four policy cells (A/B × app1/app2) match intent; denied
traffic produces no successful connection; logs preserve the policy decision.

**Feasibility.** Medium. Requires forwarding sysctls and iptables modules;
container has iptables. This is the best design for “layers” beyond L3.

**Effort:** medium.

---

## D11 — IPv6 dual-stack with anycast service

**Purpose.** Test IPv6 routing, dual-stack coexistence, and anycast failover.

```text
        r1 ========= r2
        |             |
       srv1          srv2
     (anycast)     (anycast)
```

| Item | Plan |
|---|---|
| Nodes | 2 routers, 2 servers, 2 clients |
| Addressing | IPv4 underlay + IPv6 ULA `fd00:1::/64`; both servers share an anycast address |
| Routing | OSPFv3 or BGP; advertise the anycast prefix from both sites |
| Anycast | Same address on both servers; routing selects nearest |
| Tests | ping4, ping6, traceroute/route selection, kill one server, failover |
| Telemetry gap | Existing probes are IPv4-oriented; add IPv6 probes or document the gap |

**Acceptance.** Both address families work; anycast failover completes within
the retry budget; no IPv4-only assumption leaks into assertions.

**Feasibility.** Medium. FRR supports OSPFv3; the runner command layer can use
`ping -6`/`ip -6`. The telemetry schema needs an IPv6 probe extension.

**Effort:** medium.

---

## D12 — Chaos and reliability matrix over any design

This is not a topology; it is a test overlay that every design above should
inherit.

| Fault | Injection | Required assertions |
|---|---|---|
| Link down | `ip link set <intf> down` | reachability after convergence; isolation preserved |
| Trunk down | down an inter-site trunk | alternate path or expected partition |
| Node/daemon kill | kill FRR daemon or namespace process | reconvergence; no zombie daemon |
| Packet loss | `tc qdisc ... netem loss` | protocol behaviour and recovery |
| Delay/jitter | `tc qdisc ... netem delay` | convergence and timeout sensitivity |
| Partial startup | invalid link/port after some nodes start | cleanup removes all created state |
| Interrupt | SIGINT during start/step/hold | `finish` event, cleanup, nonzero exit |
| Repeat failure | rerun into an existing directory | refusal, no evidence overwrite |
| Control-plane churn | flap a link repeatedly | bounded attempts, no permanent blackhole |

**Acceptance.** For every fault: expected reachable set, expected unreachable
set, a measurable recovery bound, and a clean teardown. A test that only checks
“eventually something works” is not sufficient.

---

# 4. Test matrix

`P` = primary, `S` = supporting, blank = not applicable.

| Design | T0 plan | T1 connectivity | T2 routing | T3 isolation | T4 redundancy | T5 policy | T6 perf | T7 chaos | T8 scale |
|---|---|---|---|---|---|---|---|---|---|
| D01 Clos eBGP | P | P | P | | P | S | S | P | S |
| D02 three-tier | P | P | P | P | P | S | S | P | S |
| D03 VRF tenancy | P | P | P | P | S | P | | S | S |
| D04 VLAN sites | P | P | S | P | P | | | P | S |
| D05 VXLAN | P | P | P | P | P | S | S | P | S |
| D06 dual-home | P | P | | | P | | S | P | |
| D07 ring/chords | P | P | P | | P | | | P | S |
| D08 hub-spoke | P | P | P | | P | P | | S | |
| D09 torus | P | P | P | | P | | P | P | P |
| D10 service chain | P | P | P | P | S | P | S | P | |
| D11 IPv6 anycast | P | P | P | | P | S | | P | |
| D12 chaos overlay | | S | S | S | P | S | S | P | S |

## 4.1 Standard evidence per scenario

Every accepted design run must retain:

- `scenario.json` (exact input);
- `events.jsonl` (start, every attempt, finish);
- `steps/NNNN-attempt-MMMM.{json,stdout,stderr}`;
- `telemetry.jsonl`;
- `provenance.json`;
- copied FRR configs/logs for each router pathspace;
- a machine-readable summary (future: `summary.json`) with positive/negative
  matrix results and convergence timings.

## 4.2 Test harness conventions

| Convention | Value |
|---|---|
| Positive ping | `ping -c 1 -W 2 <addr>` with retry |
| Negative ping | require exit status `1`; reject `0` and ambiguous errors |
| Routing readiness | `vtysh -N <pathspace> -c 'show ...'` with retry |
| BGP readiness | `show bgp summary` → `Established` |
| OSPF readiness | `show ip ospf neighbor` → `Full` |
| IS-IS readiness | `show isis neighbor` → `Up` |
| Convergence | timestamp of fault vs first successful probe; record attempt count |
| Isolation | full matrix, not one pair |
| Cleanup | start/stop cycle and leak counter |

## 4.3 Reference mini-Clos skeleton

Shape only; add FRR configuration and addressing steps. Names and ports satisfy
the runner limits.

```json
{
  "name": "mini-clos",
  "nodes": [
    {"name":"sp1","kind":"host"},{"name":"sp2","kind":"host"},
    {"name":"lf1","kind":"host"},{"name":"lf2","kind":"host"},
    {"name":"h1","kind":"host"},{"name":"h2","kind":"host"}
  ],
  "links": [
    {"a":"sp1","b":"lf1","port_a":0,"port_b":0},
    {"a":"sp1","b":"lf2","port_a":1,"port_b":0},
    {"a":"sp2","b":"lf1","port_a":0,"port_b":1},
    {"a":"sp2","b":"lf2","port_a":1,"port_b":1},
    {"a":"lf1","b":"h1","port_a":2,"port_b":0},
    {"a":"lf2","b":"h2","port_a":2,"port_b":0}
  ],
  "steps": []
}
```

Use `cargo run -p experiment-runner -- <scenario> <new-run-dir>` in a privileged
research image. The empty `steps` array is for shape validation only; a real
design must configure interfaces, forwarding, FRR, and tests.

This exact skeleton was smoke-tested in `mininet-research:test`: six hosts and
six links started, produced artifacts, finished with `success: true`, and
cleaned up. That validates the shape and naming, not routing (the steps are
empty).

---

# 5. Reusable building blocks

## 5.1 Bridge VLAN access and trunk

```sh
ip link set dev s1 type bridge vlan_filtering 1
bridge vlan del dev s1-eth1 vid 1
bridge vlan add dev s1-eth1 vid 110 pvid untagged   # access
bridge vlan add dev s1-eth4 vid 110                 # trunk
bridge vlan add dev s1-eth4 vid 120
```

## 5.2 VRF-lite

```sh
ip link add t1 type vrf table 101
ip link set t1 up
ip link set pe1-eth0 master t1
ip vrf exec t1 ping -c 1 10.20.0.1
```

FRR must be started per VRF (`-N <pathspace>` and a VRF-aware config) or use
the same pathspace with VRF configuration, depending on the FRR version. Test
this primitive in isolation before building D03.

## 5.3 Active-backup bond

```sh
ip link add bond0 type bond mode active-backup miimon 100
ip link set h1-eth0 down
ip link set h1-eth0 master bond0
ip link set h1-eth1 down
ip link set h1-eth1 master bond0
ip link set bond0 up
cat /sys/class/net/bond0/bonding/active_slave
```

LACP (`mode 802.3ad`) needs a LACP-capable switch side; LinuxBridge does not
provide it. Use D06a unless OVS support is added.

## 5.4 VXLAN (prototype)

```sh
ip link add vx100 type vxlan id 100 remote 10.255.0.2 dstport 4789 dev lf1-eth0
ip link set vx100 up
ip link set vx100 master br100
```

Validate kernel VXLAN support and underlay reachability first. Static FDB is
the initial scope; EVPN is not implemented.

## 5.5 Link impairment

```sh
tc qdisc add dev h1-eth0 root netem delay 20ms 5ms loss 1%
tc qdisc show dev h1-eth0
tc qdisc del dev h1-eth0 root
```

Use this in D02/D05/D07/D09 to measure convergence under impaired links. The
scenario runner does not expose link parameters in its schema, so these are
explicit steps.

---

# 6. Implementation sequence

| Phase | Designs | Why first | Deliverable |
|---|---|---|---|
| 1 | D01, D02 | establishes routed underlay and hierarchy | config generator + reachability/routing tests |
| 2 | D03, D04 | tenancy at L3 and L2; highest research value | isolation matrices and fault recovery |
| 3 | D06a, D07, D08 | link redundancy, ECMP, policy | failover and route-policy tests |
| 4 | D05, D10, D11 | overlays, service layers, IPv6 | prototype gates + acceptance suites |
| 5 | D09, D12 | scale and chaos across all designs | benchmark and leak matrix |

Proposed files, one per design family:

```text
example/scenarios/designs/
  d01-clos-ebgp.json
  d02-three-tier-ospf.json
  d03-vrf-tenancy.json
  d04-site-vlan-trunks.json
  d05-vxlan-overlay.json
  d06a-active-backup.json
  d06b-lacp.json
  d07-ring-isis.json
  d08-hub-spoke-bgp.json
  d09-torus-scale.json
  d10-service-chain.json
  d11-ipv6-anycast.json
```

D12 is a fault-injection suite over the other scenarios, not a single JSON file.

## 6.1 Definition of done for every design

- Scenario JSON committed under `example/scenarios/designs/` (new directory).
- Plan-only validation passes.
- Privileged run passes with `finish: success`.
- Full positive and negative matrices are recorded.
- Fault injection is tested; blast radius matches the design.
- Artifacts and FRR logs are complete.
- Teardown leaves no namespaces, veths, bridges, or FRR processes.
- Docs update the scenario map and report.

## 6.2 Known gaps to resolve before some designs

| Gap | Affects | Resolution |
|---|---|---|
| No `continue_on_failure` | multi-fault designs | runner option, or split scenarios |
| No link params in scenario schema | D02/D05/D07/D09 impairment | add `tc` steps or schema support |
| No LACP on LinuxBridge | D06b | OVS or external switch |
| VRF/FRR integration unproven | D03 | isolated prototype first |
| VXLAN kernel support unproven | D05 | minimal two-VTEP smoke test |
| IPv4-oriented telemetry | D11 | add IPv6 probes |
| No summary matrix artifact | all | generate machine-readable results |
| No per-step continue/parallelism | scale | runner evolution |

---

# 7. Relationship to the research plans

| Design | Primary research plan |
|---|---|
| D01, D07, D09 | P2 datapath, P3 control plane, P8 scale |
| D02, D04 | P4 isolation, P5 resilience |
| D03, D10 | P4 isolation, P10 security |
| D05, D11 | P2 datapath, P3 control plane, P9 portability |
| D06 | P2 datapath, P5 resilience |
| D08 | P3 control plane, P10 policy |
| D12 | P5 chaos, P7 observability |

See `RESEARCH_PLANS.md` for hypotheses, metrics, and programme sequencing; see
`SCENARIO_MAP.md` for the C0–C8 complexity ladder.
