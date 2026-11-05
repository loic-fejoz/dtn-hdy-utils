# 0010. Active DTN Services Tracking and Real-Time SSE Dashboard for `dtn-hdy-ham`

Date: 2026-11-04

## Status

Accepted

## Context

Amateur radio operators monitoring their station via the `dtn-hdy-ham` web dashboard require real-time visibility into the active services registered on the BPv7 Bundle Processing Agent (BPA).
Crucially:
1. The standard DTN Echo service (`hardy-echo-service`) must always run permanently upon station startup so remote and local nodes can ping the station (conforming to RFC 9171 and draft-taylor-dtn-echo-service).
2. Operators need to see external services dynamically as they connect and disconnect (e.g., `dtnprint --service incoming`, sensor telemetry forwarders, or chat listeners).
3. The core `hardy` BPA implementation (`../hardy`) must not be modified directly.
4. Updates must propagate to the web interface in real time without continuous polling or excessive connection overhead.

## Decision

1. **Service Registration Interception via `BpaRegistration`**:
   - Rather than modifying `hardy-bpa`, `dtn-hdy-ham` introduces a `ServiceTracker` proxy implementing `hardy_bpa::bpa::BpaRegistration`.
   - The standalone daemon passes this tracker to `hardy_proto::server::GrpcServer::new(...)`.
   - Whenever an external client (such as `dtnprint`) registers a service or application via gRPC (`register_service` or `register_application`), `ServiceTracker` wraps the inner service to monitor its lifecycle:
     - On registration (`on_register`): records the resolved EID (e.g. `dtn://f4jxq/incoming`), service type (`gRPC Client`), and an ISO 8601 registration timestamp with timezone.
     - On disconnection (`on_unregister`): automatically removes the service from the active map when the client disconnects or terminates.
     - Transparently proxies bundle deliveries (`on_deliver`) and status reports (`on_status_notify`).

2. **Permanent Built-in Echo Service**:
   - `dtn-hdy-ham` integrates `hardy-echo-service` as an in-process dependency.
   - Upon station startup (in `HamServer::run`), the daemon unconditionally registers `EchoService::new()` under the DTN service named `"echo"` (resolving to `dtn://<callsign>/echo`).
   - The Echo service is recorded with type `"Built-in Echo"` and is always visible in the dashboard.

3. **Web API & Server-Sent Events (SSE)**:
   - `GET /api/v1/services`: REST endpoint returning the current snapshot of active services as a JSON array (`eid`, `type`, `registered_at`).
   - `GET /api/v1/events`: Real-time SSE stream (`text/event-stream`) powered by `axum::response::sse` and `tokio::sync::broadcast`.
   - Upon connection, the SSE stream sends an initial `services_update` snapshot, followed by push notifications whenever a service registers or unregisters.

4. **Web Dashboard Integration**:
   - The Station Overview view in `dtn-hdy-ham/web` (`ConfigView.tsx`) includes an **Active DTN Services** card showing all registered services, their type badges, and registration timestamps.
   - The frontend connects to `EventSource('/api/v1/events')` to update the active services dynamically with zero latency.

## Consequences

- Real-time reactivity: Adding a service with `dtnprint` or an external tool instantly reflects in the web UI without manual page reloads or polling loops.
- Hardy integrity: Full introspection and tracking achieved strictly through trait decoration without patching or branching `hardy`.
- Echo availability: The station always answers DTN pings on `dtn://<callsign>/echo`.
- Robust network transport: SSE utilizes standard HTTP streaming, providing automatic browser reconnection and seamless traversal of reverse proxies and SSH tunnels.
