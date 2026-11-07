# ADR 16: Application Storage Quota, LRU Eviction Cascade & Visual Lifetime Expiration

## Status
Accepted

## Context
In `dtn-hdy-ham`, internal interactive applications (Chat and Files services) store bundles, messages, attachments, and files in a dedicated SQLite database and disk directory (`payloads/`).
Previously, an active background expiration reaper purged bundles, messages, and files as soon as their DTN `lifetime` elapsed. This behavior created a confusing user experience:
1. **Silent disappearing messages:** Users observed chat messages and received files vanishing unexpectedly from the web interface simply because their DTN transit lifetime expired, even when ample disk storage remained available.
2. **Unbounded disk consumption:** Without an overall application-level storage cap, persistent or sticky data could grow uncontrollably on resource-constrained devices (e.g. Raspberry Pi, embedded field laptops).

Users need messages and files to persist visibly for as long as disk storage allows, with clear visual indications when transit lifetime has elapsed, combined with predictable, deterministic space management when disk quotas are approached.

## Decision

We introduce an application-level storage quota and replace silent expiration purge with visual status indicators and an LRU eviction cascade.

### 1. Visual Lifetime Expiration (No Silent Purge)
- DTN lifetime expiration **no longer causes automatic background deletion**.
- Expired messages and files remain indefinitely stored and visible as long as storage quota is not reached.
- **Visual Presentation in Web UI**:
  - **Chat**: Expired messages are visually attenuated (muted text color) and marked with an hourglass icon (`⏳`) with an informative tooltip (e.g. *"Expired on [date/time] — retained under storage quota"*).
  - **Files**: Expired files are labeled with an hourglass icon (`⏳`) and expiration timestamp in the file inventory table.

### 2. Application Quota Modeling
- Configured in `dtn-hdy-ham` configuration under:
  ```yaml
  application:
    quota:
      max-storage: "300MB" # Supports human-readable units (e.g. "300MB", "300MiB", "1GB"). Defaults to 300MB.
  ```
- **Measurement Scope**:
  - Actual physical disk consumption: sum of all payload files on disk (`payloads/`) + the SQLite database file size.

### 3. Synchronous LRU Eviction Cascade at Insertion
- Space enforcement occurs **synchronously upon inserting any incoming application bundle** (Chat message, attachment, or File).
- If storing the new incoming item would exceed `max-storage`, space is reclaimed according to a strict 4-tier eviction cascade:
  1. **Tier 1 — Expired Chats** (non-sticky, ordered from oldest to newest creation time).
  2. **Tier 2 — Expired Files** (non-sticky, ordered from oldest to newest creation time).
  3. **Tier 3 — Valid/Active Chats** (non-sticky, ordered from oldest to newest creation time).
  4. **Tier 4 — Valid/Active Files** (non-sticky, ordered from oldest to newest creation time).
- **Eviction Mode**: Complete removal (deleted from SQLite and file unlinked from disk).

### 4. Protection of Sticky Items & Saturation Policy
- Items flagged as `sticky` are **strictly protected** and never evicted by the quota reaper.
- If total storage remains saturated after evicting all eligible non-sticky items (due to sticky data occupancy or an oversized item), the incoming bundle is **rejected** with an explicit warning logged.

## Consequences
- Messages and files do not disappear silently; users can read historical chat and access files past DTN transit expiration.
- Disk usage is strictly capped to the configured maximum (default 300MB).
- Deterministic 4-tier cascade ensures expired communication is reclaimed before active items, and ephemeral chat before larger file payloads.
- Sticky retention remains absolute and inviolable.
