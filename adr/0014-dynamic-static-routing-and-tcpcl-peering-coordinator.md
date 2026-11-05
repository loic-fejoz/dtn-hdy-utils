# ADR 0014: Dynamic Static Routing, Out-of-the-Box Gateway Peering & TCPCL Contact Coordinator

## Status
Accepted

## Context

Amateur radio operators deploying `dtn-hdy-ham` often connect to an established DTN BPv7 gateway (such as `dtn://f4jxq/` over AMPRNet/Internet via TCPCLv4) and exchange messages with companion mobile nodes (such as `dtn://f4jxq-9/` running `dtn-android-messenger`).

Previously:
1. **Static Route Immutability**: All static routes were defined strictly at startup in the YAML configuration. Adding or removing a route required stopping the daemon, editing YAML, and restarting, disrupting running services.
2. **Missing Out-of-the-Box Routing for Secondary SSIDs & Gateways**: When peering with F4JXQ, bundles addressed to companion devices (e.g. `dtn://f4jxq-9/chat` or `dtn://f4jxq-9/files`) or third-party radioamateurs were dropped or could not be forwarded because no static route existed.
3. **TCPCLv4 Connection Lifecycle & Contact Window Management**: While amateur radio CLAs (LTP, Mercury, HQFBP) use the `ContactCoordinator` for scheduled contact windows, TCPCLv4 links also require structured connection management suitable for both continuous IP links and intermittent amateur radio networks (Hamnet / AREDN).

## Decisions

### 1. Dynamic Routing Agent (`HamRoutingAgent`) & REST Endpoints
- Implement a dedicated `HamRoutingAgent` implementing Hardy BPA's `RoutingAgent` trait.
- The agent retains the BPA's `RoutingSink` in thread-safe shared state (`Arc<RwLock<Option<Box<dyn RoutingSink>>>>`).
- Dynamic REST API endpoints:
  - `GET /api/v1/routes`: Returns all active static routes (pattern, next-hop, CLA, priority).
  - `POST /api/v1/routes`: Pushes a new route into Hardy's Routing Information Base (RIB) via `sink.add_route(...)` with **immediate hot-reload** effect, without restarting the daemon.
  - `DELETE /api/v1/routes`: Withdraws the route from Hardy's RIB via `sink.remove_route(...)` immediately.
  - Changes are persisted to the node's YAML configuration file (`routes` section) so they survive restarts.

### 2. Turnkey Initialization with F4JXQ Gateway & Default Route
- In `dtn-hdy-ham --wizard`, when the operator elects to peer with gateway `dtn://f4jxq/`:
  - Automatically install the specific static route for companion devices:
    - `pattern: "dtn://f4jxq-9/*"` $\rightarrow$ `next-hop: "dtn://f4jxq/"` (CLA: `tcpcl-f4jxq`).
  - Automatically configure the DTN Default Gateway route:
    - `pattern: "dtn://**"` $\rightarrow$ `next-hop: "dtn://f4jxq/"` (CLA: `tcpcl-f4jxq`).
  - Include permissive RFC 9171 validity defaults:
    - `primary-block-integrity: false` (to accept bundles without CRC/BIB from Android and third-party nodes).
    - `bundle-age-required: false` (to prevent bundle rejection on nodes lacking bundle-age extension blocks).

### 3. TCPCLv4 Session Lifecycle & Contact Coordinator
- **Startup Connection**: The TCPCLv4 session is initiated automatically upon daemon startup.
- **Continuous Healthy Session**: The TCPCL session is kept alive and maintained open continuously as long as the underlying link is healthy.
- **Scheduled Contact Window (43 minutes)**: TCPCLv4 peering is bound to the station's `ContactCoordinator` with a default recurring cron period of 43 minutes (`*/43 * * * *`). The coordinator acts as a periodic contact probe and recovery mechanism, re-dialing the peer if the connection had dropped.
- **No On-Demand Dial on Outbox Injection**: Posting a message or file bundle while the TCPCL connection is closed does **not** trigger an automatic immediate dial. In accordance with DTN store-and-forward principles, the bundle is safely queued in the BPA Outbox. Routing may involve alternate hamradio CLAs, or the underlying hamnet link may not yet be established. Forwarding occurs upon the next scheduled contact window or via manual trigger.
- **Visual Outbox & Connection Status**: The web console provides explicit real-time visual badges:
  - Connection status for TCPCL peers: 🟢 *Connected*, 🔴 *Disconnected*, 🟡 *Connecting*.
  - Bundle status in Chat and Bundles views: ⏳ *Queued in Outbox* indicating safe retention in BPA storage awaiting the next contact opportunity.
- **Manual Force-Connect**: Operators can trigger an immediate connection at any time via the web UI.

## Consequences

- Operators can manage static routes in real-time from the web console without interrupting active DTN services.
- Out-of-the-box interoperability with gateway `dtn://f4jxq/` and mobile companion `dtn://f4jxq-9/` without manual configuration editing.
- Predictable, radio-friendly TCPCLv4 scheduling respecting DTN store-and-forward queuing without uncontrolled dial-out loops.
