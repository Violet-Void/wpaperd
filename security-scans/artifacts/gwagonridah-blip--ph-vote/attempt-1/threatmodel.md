# Threat Model

ph-vote is a self-hosted FastAPI service for browsing and rating clips. HTTP authorization is network- and Host-based with no application login; local source, scout, and digger files feed an index and browser rendering; media paths invoke urllib, yt-dlp, ffmpeg, and an optional MLT webhook; file-backed state and optional credentials sit under configured runtime directories. Docker binds loopback by default, while a separate live-box recovery script uses passwordless sudo to preserve Tailscale across resets.

## Assets

- Vote, reason, note, highlight, skip, MLT, index, scoreboard, and download state whose confidentiality and integrity affect the private curation workflow.
- Pornhub cookies and the upstream account authority they grant to favorite-management requests.
- The optional MLT webhook key and destination.
- Host filesystem roots, downloaded media, and configured media-path confinement.
- Server and host availability during source scans, media processing, request-created work, and upstream streams.
- The cached Tailscale package, daemon binaries, root state archive, and recovered node identity used by live-box reset recovery.

## Trust Boundaries

- Browser or API client to FastAPI guards: admitted clients are selected by source network, Host, and Origin controls and share authority because the application has no per-user login.
- HTTP mutation handlers to file-backed state: request values cross validators and atomic-write boundaries before later rendering or processing.
- Source, scout, digger, and export files to index, browser, and server-side media logic: documentation defines external writers, establishing a parsing and integration boundary even when a deployment trusts them.
- Application server to external adult sites and CDNs through proxy/direct urllib, yt-dlp, and ffmpeg execution.
- Application server to credential-bearing Pornhub favorite operations and the optional authenticated MLT webhook.
- Unprivileged box account and persistent home storage to passwordless-sudo recovery of root-owned Tailscale software and state; exact sudoers and scheduler configuration are external.
- Developer or CI host to third-party browser content loaded by the UI smoke test.

## Attacker Capabilities

- An admitted ALLOWED_NETS client can send arbitrary paths, headers, bodies, ranges, and repeated requests, but does not initially control local files, environment configuration, or root.
- A malicious website can attempt cross-site requests but cannot normally forge the victim's same-origin headers or originate from an admitted server-side network.
- A compromised or malicious source, scout, or digger writer can modify matching source and shared metadata but does not initially control the HTTP host, environment, or root.
- A compromised box or application account can write its owned home and persistence files but is not assumed to have root except through a demonstrated privileged workflow.
- Malicious external content can execute inside the browser-test context when the test deliberately embeds it.
- The operator controls environment, volumes, proxies, and root recovery and is trusted; authorized operator configuration is not itself attacker input.

## Security Objectives

- Restrict private data and credential operations to intended network or authenticating-proxy clients, and block cross-site mutations.
- Prevent metadata and request inputs from escaping configured roots, becoming active markup or code, or selecting unintended server-side destinations.
- Keep cookies, webhook secrets, and root node identity on their intended server- or root-side boundary.
- Bound CPU, memory, threads, disk use, upstream reads, and external-process work against admitted clients and malformed sources.
- Preserve atomic file-backed state and tolerate malformed individual inputs without aborting the whole index.
- Authenticate privileged recovery inputs and preserve the boundary between user-writable persistence and root-executed software or restored state.

## Assumptions

- The intended deployment is single-user and self-hosted behind loopback, Tailscale, an allowed network, or an authenticating reverse proxy; the application itself has no login.
- Uvicorn proxy-header trust is disabled in the supplied launch configuration, and Docker publishes to loopback by default; deliberately broadening those boundaries is an operator decision.
- No SECURITY.md, supplied threat model, knowledge base, live .env, or live data tree was available.
- External source-writer trust varies by deployment, but repository documentation establishes those writers as distinct producers at a parsing boundary.
- Exact sudoers, scheduler, Tailscale ACL, and optional-feature enablement are external and were not assumed beyond what repository scripts and documentation establish.
- deploy/box.env documents ENV_FILE launch behavior that scripts/env.sh does not preserve unless START_INHERIT_ENV=1; this is an operational discrepancy, not a validated security finding.
- The PH_PROXY example comment is narrower than current code, which also applies proxy and direct-fallback behavior to XHamster and Eporner; this is a documentation discrepancy, not a separate finding.

---

## Saved Model Context

- Source: scan
- Scan: bdee8bf1-1977-4dd3-af3a-ea2d503003ca
- Target: gwagonridah-blip--ph-vote
- Revision: a07500c4d8b2519817c8736818be1db971121961
- Result status: completed
- Model origin: not recorded
- Model scope: not recorded
- Scan scope: .
- Scan scope description: Complete static Standard security scan of the entire repository at the registered git revision.
