# Security Review: gwagonridah-blip--ph-vote

## Scope

Complete static Standard security scan of the entire repository at the registered git revision.

- Scan mode: repository
- Target kind: git_revision
- Target ID: target_sha256_576ad0539ea283bd965510663c263021ccbfb9160dc8c2de4aa689413dfed123
- Revision: a07500c4d8b2519817c8736818be1db971121961
- Inventory strategy: repository
- Included paths: .
- Excluded paths: none
- Artifacts reviewed: All 63 git-tracked repository files, Application Python modules under app/, Browser assets under web/, Deployment, recovery, and utility scripts under deploy/ and scripts/, Tests, documentation, container files, and project metadata
- Scan context: All 63 git-tracked files were fully reviewed through an independent baseline audit, architecture mapping, focused HTTP/browser, filesystem/egress, deployment, data-ingestion, and remaining-file reviews, followed by parent validation and attack-path reconciliation.

Limitations and exclusions:
- Static offline source review only; no live service, third-party site, webhook, ffmpeg, yt-dlp, or privileged reset workflow was executed.
- External sudoers, scheduler, Tailscale ACLs, environment values, live cookies, and production source-directory permissions were unavailable; dependent findings state those preconditions and use calibrated confidence.
- Optional feature enablement and reverse-proxy behavior were inferred only from repository code and documentation.

### Scan Summary

| Field | Value |
| --- | --- |
| Scan outcome | completed |
| Reportable findings | 10 |
| Severity mix | high: 1, medium: 7, low: 2 |
| Confidence mix | high: 8, medium: 2 |
| Coverage | complete |
| Validation mode | Source-backed static validation with independent baseline review, focused investigators, duplicate reconciliation, and explicit source-to-sink attack paths. |

Canonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.

## Threat Model

ph-vote is a self-hosted FastAPI service for browsing and rating clips. HTTP authorization is network- and Host-based with no application login; local source, scout, and digger files feed an index and browser rendering; media paths invoke urllib, yt-dlp, ffmpeg, and an optional MLT webhook; file-backed state and optional credentials sit under configured runtime directories. Docker binds loopback by default, while a separate live-box recovery script uses passwordless sudo to preserve Tailscale across resets.

### Assets

- Vote, reason, note, highlight, skip, MLT, index, scoreboard, and download state whose confidentiality and integrity affect the private curation workflow.
- Pornhub cookies and the upstream account authority they grant to favorite-management requests.
- The optional MLT webhook key and destination.
- Host filesystem roots, downloaded media, and configured media-path confinement.
- Server and host availability during source scans, media processing, request-created work, and upstream streams.
- The cached Tailscale package, daemon binaries, root state archive, and recovered node identity used by live-box reset recovery.

### Trust Boundaries

- Browser or API client to FastAPI guards: admitted clients are selected by source network, Host, and Origin controls and share authority because the application has no per-user login.
- HTTP mutation handlers to file-backed state: request values cross validators and atomic-write boundaries before later rendering or processing.
- Source, scout, digger, and export files to index, browser, and server-side media logic: documentation defines external writers, establishing a parsing and integration boundary even when a deployment trusts them.
- Application server to external adult sites and CDNs through proxy/direct urllib, yt-dlp, and ffmpeg execution.
- Application server to credential-bearing Pornhub favorite operations and the optional authenticated MLT webhook.
- Unprivileged box account and persistent home storage to passwordless-sudo recovery of root-owned Tailscale software and state; exact sudoers and scheduler configuration are external.
- Developer or CI host to third-party browser content loaded by the UI smoke test.

### Attacker Capabilities

- An admitted ALLOWED_NETS client can send arbitrary paths, headers, bodies, ranges, and repeated requests, but does not initially control local files, environment configuration, or root.
- A malicious website can attempt cross-site requests but cannot normally forge the victim's same-origin headers or originate from an admitted server-side network.
- A compromised or malicious source, scout, or digger writer can modify matching source and shared metadata but does not initially control the HTTP host, environment, or root.
- A compromised box or application account can write its owned home and persistence files but is not assumed to have root except through a demonstrated privileged workflow.
- Malicious external content can execute inside the browser-test context when the test deliberately embeds it.
- The operator controls environment, volumes, proxies, and root recovery and is trusted; authorized operator configuration is not itself attacker input.

### Security Objectives

- Restrict private data and credential operations to intended network or authenticating-proxy clients, and block cross-site mutations.
- Prevent metadata and request inputs from escaping configured roots, becoming active markup or code, or selecting unintended server-side destinations.
- Keep cookies, webhook secrets, and root node identity on their intended server- or root-side boundary.
- Bound CPU, memory, threads, disk use, upstream reads, and external-process work against admitted clients and malformed sources.
- Preserve atomic file-backed state and tolerate malformed individual inputs without aborting the whole index.
- Authenticate privileged recovery inputs and preserve the boundary between user-writable persistence and root-executed software or restored state.

### Assumptions

- The intended deployment is single-user and self-hosted behind loopback, Tailscale, an allowed network, or an authenticating reverse proxy; the application itself has no login.
- Uvicorn proxy-header trust is disabled in the supplied launch configuration, and Docker publishes to loopback by default; deliberately broadening those boundaries is an operator decision.
- No SECURITY.md, supplied threat model, knowledge base, live .env, or live data tree was available.
- External source-writer trust varies by deployment, but repository documentation establishes those writers as distinct producers at a parsing boundary.
- Exact sudoers, scheduler, Tailscale ACL, and optional-feature enablement are external and were not assumed beyond what repository scripts and documentation establish.
- deploy/box.env documents ENV_FILE launch behavior that scripts/env.sh does not preserve unless START_INHERIT_ENV=1; this is an operational discrepancy, not a validated security finding.
- The PH_PROXY example comment is narrower than current code, which also applies proxy and direct-fallback behavior to XHamster and Eporner; this is a documentation discrepancy, not a separate finding.

## Findings

| Finding | Severity | Confidence | Detailed write-up |
| --- | --- | --- | --- |
| [A box-user-writable cached package is installed and executed as root during reset recovery](#finding-1) | high | medium | inline below |
| [Malformed or non-regular source files can stall or abort the entire index rebuild](#finding-2) | medium | high | inline below |
| [Vote restore persists an unvalidated note timestamp into an HTML attribute](#finding-3) | medium | high | inline below |
| [Root Tailscale node state is downgraded to box-user-writable persistent storage](#finding-4) | medium | medium | inline below |
| [Unbounded fuzzy duplicate matching performs quadratic work on every queue and home request](#finding-5) | medium | high | inline below |
| [The UI smoke test renders live third-party content with Chromium's sandbox disabled](#finding-6) | medium | high | inline below |
| [Admitted clients can create unbounded background threads on preview, queue, and webhook paths](#finding-7) | medium | high | inline below |
| [Namespaced clip metadata controls server-side media destinations without egress validation](#finding-8) | medium | high | inline below |
| [Synthetic range fallback amplifies a one-byte request into up to 32 MiB of upstream reads](#finding-9) | low | high | inline below |
| [An unvalidated clip identifier pollutes Object.prototype in browser watch state](#finding-10) | low | high | inline below |

### Confidence Scale

| Label | Meaning |
| --- | --- |
| high | Direct evidence supports the finding with no material unresolved blocker. |
| medium | Evidence supports a plausible issue, but material runtime or reachability proof remains. |
| low | Evidence is incomplete and the item is retained only for explicit follow-up. |

<a id="finding-1"></a>

### [1] A box-user-writable cached package is installed and executed as root during reset recovery

| Field | Value |
| --- | --- |
| Severity | high |
| Confidence | medium |
| Confidence rationale | The user-writable source, privileged install fallback, root daemon launch, persistence path, and recurring reset workflow are explicit. The exact scheduler identity and sudoers policy are external to the repository, but the script itself requires passwordless sudo and the documented workflow states it runs every 15–30 minutes. |
| Category | Untrusted user-writable code crosses a privileged recovery boundary |
| CWE | CWE-494 |
| Affected lines | deploy/box.env:18, scripts/ensure-up.sh:71-81, scripts/ensure-up.sh:93-114, scripts/ensure-up.sh:118-121, scripts/ensure-up.sh:150-155, docs/live-switch.md:72-79 |

#### Summary

scripts/ensure-up.sh trusts the newest filename-matching Tailscale .deb under a directory owned by the unprivileged box account. When a reset removes the system binaries, the recurring recovery workflow installs that local package with sudo apt-get or extracts its binaries and sudo-installs them, then starts tailscaled under sudo. A process that compromises the box account can plant a crafted package and gain root execution at the next recovery.

#### Root Cause

Privileged recovery treats an unprivileged account's persistence directory as a trusted software supply source and authenticates artifacts only by filename.

**Recovery artifacts live in the box user's persistent home** — `scripts/ensure-up.sh:71-81`

Mode 0700 excludes other OS users but makes the box account authoritative for files later consumed by sudo operations. deploy/box.env sets /home/box/.persist/tailscale.

```bash
PERSIST=$(_abs "${TS_PERSIST_DIR:-$HOME/.persist/tailscale}")
...
SUDO="sudo -n"
$SUDO true 2>/dev/null || { ...; SUDO=""; }
mkdir -p "$PERSIST" && chmod 700 "$PERSIST" ...
```

**The newest matching local package is installed with sudo** — `scripts/ensure-up.sh:93-104`

Selection trusts filename and mtime only. No root ownership, pinned version, digest, or signature check protects the local artifact.

```bash
install_tailscale(){
  deb=$(ls -t "$PERSIST"/tailscale_*_amd64.deb 2>/dev/null | head -1)
  if [ -n "$deb" ] && [ "$SYS" = 1 ]; then
    if apt_q install -y --no-install-recommends "$deb" || { apt_q update && apt_q install -y --no-install-recommends "$deb"; }; then
      ...; return 0
    fi
    ...
  fi
```

**Fallback copies package-controlled executables into privileged paths** — `scripts/ensure-up.sh:104-110`

Even without package maintainer scripts, attacker-chosen executable bytes are installed as root-owned system binaries.

```bash
if [ -n "$deb" ]; then
  t=$(mktemp -d)
  if dpkg-deb -x "$deb" "$t" && $SUDO install -D -m 755 "$t/usr/sbin/tailscaled" "$TAILSCALED_BIN" \
     && $SUDO install -D -m 755 "$t/usr/bin/tailscale" "$TAILSCALE_BIN"; then
    rm -rf "$t"; ...; return 0
  fi
fi
```

**Recovery starts the installed daemon under sudo** — `scripts/ensure-up.sh:150-155`

The package-controlled tailscaled binary is executed with root authority after installation.

```bash
if [ -x "$TAILSCALED_BIN" ] && ! tsd_running; then
  $SUDO mkdir -p "$(dirname "$TS_SOCKET")"
  ...
  $SUDO setsid nohup "$TAILSCALED_BIN" --state="$STATE" --socket="$TS_SOCKET" ${TS_DAEMON_ARGS:-} ... &
fi
```

#### Validation

Confirmed user-writable package selection leads to privileged binary installation and execution in the documented reset workflow.

Validation method: Independent static privilege-boundary validation

- **Status:** validated
- **Disposition:** report

**Recovery artifacts live in the box user's persistent home** — `scripts/ensure-up.sh:71-81`

Mode 0700 excludes other OS users but makes the box account authoritative for files later consumed by sudo operations. deploy/box.env sets /home/box/.persist/tailscale.

```bash
PERSIST=$(_abs "${TS_PERSIST_DIR:-$HOME/.persist/tailscale}")
...
SUDO="sudo -n"
$SUDO true 2>/dev/null || { ...; SUDO=""; }
mkdir -p "$PERSIST" && chmod 700 "$PERSIST" ...
```

**The newest matching local package is installed with sudo** — `scripts/ensure-up.sh:93-104`

Selection trusts filename and mtime only. No root ownership, pinned version, digest, or signature check protects the local artifact.

```bash
install_tailscale(){
  deb=$(ls -t "$PERSIST"/tailscale_*_amd64.deb 2>/dev/null | head -1)
  if [ -n "$deb" ] && [ "$SYS" = 1 ]; then
    if apt_q install -y --no-install-recommends "$deb" || { apt_q update && apt_q install -y --no-install-recommends "$deb"; }; then
      ...; return 0
    fi
    ...
  fi
```

**Fallback copies package-controlled executables into privileged paths** — `scripts/ensure-up.sh:104-110`

Even without package maintainer scripts, attacker-chosen executable bytes are installed as root-owned system binaries.

```bash
if [ -n "$deb" ]; then
  t=$(mktemp -d)
  if dpkg-deb -x "$deb" "$t" && $SUDO install -D -m 755 "$t/usr/sbin/tailscaled" "$TAILSCALED_BIN" \
     && $SUDO install -D -m 755 "$t/usr/bin/tailscale" "$TAILSCALE_BIN"; then
    rm -rf "$t"; ...; return 0
  fi
fi
```

**Recovery starts the installed daemon under sudo** — `scripts/ensure-up.sh:150-155`

The package-controlled tailscaled binary is executed with root authority after installation.

```bash
if [ -x "$TAILSCALED_BIN" ] && ! tsd_running; then
  $SUDO mkdir -p "$(dirname "$TS_SOCKET")"
  ...
  $SUDO setsid nohup "$TAILSCALED_BIN" --state="$STATE" --socket="$TS_SOCKET" ${TS_DAEMON_ARGS:-} ... &
fi
```

**Reset recovery is expected and scheduled** — `docs/live-switch.md:72-79`

The missing-binary trigger and repeated execution are part of the documented live deployment rather than a hypothetical manual path.

```markdown
- **Keep-alive / reset recovery**: `/workspace/ph-vote/scripts/ensure-up.sh` ... reinstalls Tailscale from the saved
  `.deb`, restores the `picks` identity ...
  ... a server-side routine runs it every 15-30 min.
```

Assertions:
- The persistence directory is under /home/box and created by the unprivileged script account.
- The newest matching .deb is selected without integrity checks.
- sudo apt-get consumes that path, or fallback extracts and sudo-installs package binaries.
- The resulting tailscaled path is launched through sudo.
- A reset removes system binaries and the recurring workflow reinstalls them.

Counterevidence and remaining uncertainty:
- Directory mode 0700 blocks unrelated Unix users.
- A trigger requires binaries to be missing, normally after a documented reset.
- Actual sudoers scoping and scheduler identity are not committed.
- If the box account already has unrestricted root authority, there is no incremental gain.

Limitations:
- The external automation and sudoers configuration were unavailable; confidence is therefore medium despite a complete in-script source-to-sink path.

#### Dataflow

The canonical finding records the affected path at deploy/box.env:18, scripts/ensure-up.sh:71-81, scripts/ensure-up.sh:93-114, scripts/ensure-up.sh:118-121, scripts/ensure-up.sh:150-155, docs/live-switch.md:72-79, but no expanded source-to-sink narrative was recorded.

Attack steps:
- Create a valid tailscale_\*_amd64.deb in /home/box/.persist/tailscale with attacker-controlled usr/sbin/tailscaled and make it newest.
- Wait for or induce the documented reset condition in which system Tailscale binaries are absent.
- The recurring ensure-up.sh invocation selects the crafted package.
- sudo apt-get installs it, or dpkg-deb plus sudo install copies its executables into system paths.
- ensure-up starts the attacker-controlled tailscaled binary under sudo, yielding root code execution.

#### Reachability

The documented live reset process repeatedly reaches the privileged package consumption path.

- **Attacker:** Compromised unprivileged box account

- **Entry point:** TS_PERSIST_DIR/tailscale_\*_amd64.deb

- **Source:** Attacker-controlled package executables

- **Sink:** sudo apt-get or sudo install followed by sudo execution of tailscaled

- **Outcome:** Root code execution

Preconditions:
- Attacker has code execution as the box account or equivalent write access to TS_PERSIST_DIR.
- Passwordless sudo permits the operations used by ensure-up.sh.
- Recovery encounters missing Tailscale binaries.
- Reset/missing binaries and applicable passwordless sudo policy.

Existing controls:
- 0700 persistence permissions prevent unrelated local users.
- Missing-binary check prevents reinstall while healthy binaries remain.
- Neither control authenticates content from the box account against the root boundary.

**The newest matching local package is installed with sudo** — `scripts/ensure-up.sh:93-104`

Selection trusts filename and mtime only. No root ownership, pinned version, digest, or signature check protects the local artifact.

```bash
install_tailscale(){
  deb=$(ls -t "$PERSIST"/tailscale_*_amd64.deb 2>/dev/null | head -1)
  if [ -n "$deb" ] && [ "$SYS" = 1 ]; then
    if apt_q install -y --no-install-recommends "$deb" || { apt_q update && apt_q install -y --no-install-recommends "$deb"; }; then
      ...; return 0
    fi
    ...
  fi
```

**Fallback copies package-controlled executables into privileged paths** — `scripts/ensure-up.sh:104-110`

Even without package maintainer scripts, attacker-chosen executable bytes are installed as root-owned system binaries.

```bash
if [ -n "$deb" ]; then
  t=$(mktemp -d)
  if dpkg-deb -x "$deb" "$t" && $SUDO install -D -m 755 "$t/usr/sbin/tailscaled" "$TAILSCALED_BIN" \
     && $SUDO install -D -m 755 "$t/usr/bin/tailscale" "$TAILSCALE_BIN"; then
    rm -rf "$t"; ...; return 0
  fi
fi
```

**Recovery starts the installed daemon under sudo** — `scripts/ensure-up.sh:150-155`

The package-controlled tailscaled binary is executed with root authority after installation.

```bash
if [ -x "$TAILSCALED_BIN" ] && ! tsd_running; then
  $SUDO mkdir -p "$(dirname "$TS_SOCKET")"
  ...
  $SUDO setsid nohup "$TAILSCALED_BIN" --state="$STATE" --socket="$TS_SOCKET" ${TS_DAEMON_ARGS:-} ... &
fi
```

**Reset recovery is expected and scheduled** — `docs/live-switch.md:72-79`

The missing-binary trigger and repeated execution are part of the documented live deployment rather than a hypothetical manual path.

```markdown
- **Keep-alive / reset recovery**: `/workspace/ph-vote/scripts/ensure-up.sh` ... reinstalls Tailscale from the saved
  `.deb`, restores the `picks` identity ...
  ... a server-side routine runs it every 15-30 min.
```

#### Severity

**High** — Successful exploitation converts box-account code execution into root code execution with complete host impact. It requires control of the box account's persistent directory and the expected reset/missing-binary recovery condition, so attack complexity is elevated.

Raise likelihood if resets are frequent or an attacker can remove the binaries; lower if the external scheduler or sudo policy cannot execute the privileged package/install/start operations shown.

Impact assessment:
- **Level:** high
- **Rationale:** Arbitrary native code executes as root with complete host confidentiality, integrity, and availability impact.

Likelihood assessment:
- **Level:** medium
- **Rationale:** The workflow is recurring and resets are expected, but exploitation requires prior box-account compromise and an external sudo policy consistent with the script.

#### Remediation

Move cached packages and recovery state to a root-owned directory that the service/box account cannot modify. Pin an expected Tailscale version and verify the package's repository signature or an operator-pinned cryptographic digest before any privileged consumption. Remove the unauthenticated extraction/install fallback. Make the privileged component fetch or copy verified artifacts itself, and narrowly scope sudo to a root-owned wrapper that accepts no user-controlled paths.

Tests:
- As the box account, place a newer filename-matching .deb in the persistence directory and assert recovery refuses it before any sudo operation.
- Modify one byte of the approved cached package and assert digest/signature verification fails closed.
- Verify the privileged package directory and every parent are root-owned and not group/world/user writable by the app account.
- Exercise offline reset recovery with the pinned authentic package and confirm it still succeeds.

Preventive controls:
- Separate unprivileged application persistence from root recovery artifacts.
- Verify provenance and integrity immediately before privileged installation or execution.
- Audit sudoers and external scheduler configuration so a writable script/account cannot broaden privileged commands.

<a id="finding-2"></a>

### [2] Malformed or non-regular source files can stall or abort the entire index rebuild

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | The open/read, unchecked regex dereferences, whole-build catch, and missing per-file bounds are explicit in source. Exact memory exhaustion depends on host limits but the tiny filename aborts are deterministic. |
| Category | Unbounded and non-isolated file-backed source ingestion |
| CWE | CWE-400, CWE-20 |
| Affected lines | app/store.py:66-71, app/index.py:64-77, app/index.py:95-130, app/index.py:157-176, app/server.py:72-79 |

#### Summary

The recursive source scanner opens every matching JSON or saved HTML path without verifying a regular no-follow file, enforcing byte or aggregate limits, or isolating all per-file parsing failures. A source writer can block the rebuild with a FIFO/device or exhaust memory with a large file. Separately, broad glob patterns feed narrower unchecked filename regexes, so a tiny malformed JSON or HTML filename raises before the local error handling and aborts the whole build.

#### Root Cause

SOURCES_DIR is treated as a trusted collection instead of a parsing boundary: filename grammar, file type/confinement, resource budgets, and per-file fault isolation are not enforced before blocking reads and regex captures.

**JSON helper reads an unrestricted path to completion** — `app/store.py:66-71`

The catch handles parse exceptions only after reads return; it does not limit bytes, reject symlinks/special files, or prevent a FIFO/device from blocking.

```python
def load_json(p, default=None):
    try:
        with open(p) as f:
            return json.load(f)
    except Exception:
        return default
```

**Recursive scanning trusts every matching file path** — `app/index.py:95-115`

Directory filtering does not lstat or confine each file target, and there is no regular-file, symlink, byte, record, depth, or aggregate budget.

```python
for root, dirs, files in os.walk(WS):
    dirs[:] = [x for x in dirs if ...]
    for fn in files:
        if not fn.endswith('.json') or fn.startswith(('_exclude', '_prior')):
            continue
        p = os.path.join(root, fn)
        ...
        try:
            for vk, o in items_of(load_json(p)):
                ...
        except RecursionError:
            pass
```

**Saved HTML is read wholly without a file budget** — `app/index.py:116-124`

A matching path is read completely, and the narrower regex dereference occurs before the exception boundary.

```python
for p in glob.glob(os.path.join(WS, 'ph-vid-*.html')) + glob.glob(...):
    vk = re.search(r'(?:ph-vid-|_page_)([0-9a-z]+)\.html$', p).group(1)
    ...
    try:
        s = open(p, errors='ignore').read()
    except Exception:
        continue
```

**Broad JSON globs feed unchecked date captures** — `app/index.py:64-77`

A tiny filename matching the glob but lacking a date raises AttributeError and stops build_index.

```python
for p in sorted(glob.glob(os.path.join(WS, 'ph-midnight-final-*.json'))):
    d = re.search(r'(\d{4}-\d\d-\d\d)', p).group(1)
...
for p in sorted(glob.glob(os.path.join(WS, 'ph-hourly-lace-dig-*.json'))):
    m = re.search(r'(\d{4}-\d\d-\d\d)(?:-(\d\d)(\d\d))?', p)
    d, hh, mm = m.group(1), m.group(2) or '00', m.group(3) or '00'
```

**One source failure aborts the complete refresh** — `app/index.py:157-176`

The top-level catch keeps the process alive but cannot commit valid partial input; a fresh deployment remains empty and an existing deployment stays stale.

```python
def refresh_index(force=False):
    ...
    try:
        ...
        picks = build_index()
        INDEX.update(...)
        ...
    except Exception as e:
        log('index build failed:', repr(e))
    finally:
        building[0] = False
```

#### Validation

Confirmed deterministic whole-build aborts from glob/regex mismatches and unbounded blocking reads from source-controlled paths.

Validation method: Independent static file-ingestion validation

- **Status:** validated
- **Disposition:** report

**Recursive scanning trusts every matching file path** — `app/index.py:95-115`

Directory filtering does not lstat or confine each file target, and there is no regular-file, symlink, byte, record, depth, or aggregate budget.

```python
for root, dirs, files in os.walk(WS):
    dirs[:] = [x for x in dirs if ...]
    for fn in files:
        if not fn.endswith('.json') or fn.startswith(('_exclude', '_prior')):
            continue
        p = os.path.join(root, fn)
        ...
        try:
            for vk, o in items_of(load_json(p)):
                ...
        except RecursionError:
            pass
```

**Saved HTML is read wholly without a file budget** — `app/index.py:116-124`

A matching path is read completely, and the narrower regex dereference occurs before the exception boundary.

```python
for p in glob.glob(os.path.join(WS, 'ph-vid-*.html')) + glob.glob(...):
    vk = re.search(r'(?:ph-vid-|_page_)([0-9a-z]+)\.html$', p).group(1)
    ...
    try:
        s = open(p, errors='ignore').read()
    except Exception:
        continue
```

**Broad JSON globs feed unchecked date captures** — `app/index.py:64-77`

A tiny filename matching the glob but lacking a date raises AttributeError and stops build_index.

```python
for p in sorted(glob.glob(os.path.join(WS, 'ph-midnight-final-*.json'))):
    d = re.search(r'(\d{4}-\d\d-\d\d)', p).group(1)
...
for p in sorted(glob.glob(os.path.join(WS, 'ph-hourly-lace-dig-*.json'))):
    m = re.search(r'(\d{4}-\d\d-\d\d)(?:-(\d\d)(\d\d))?', p)
    d, hh, mm = m.group(1), m.group(2) or '00', m.group(3) or '00'
```

**One source failure aborts the complete refresh** — `app/index.py:157-176`

The top-level catch keeps the process alive but cannot commit valid partial input; a fresh deployment remains empty and an existing deployment stays stale.

```python
def refresh_index(force=False):
    ...
    try:
        ...
        picks = build_index()
        INDEX.update(...)
        ...
    except Exception as e:
        log('index build failed:', repr(e))
    finally:
        building[0] = False
```

Assertions:
- A malformed midnight/hourly filename can match the glob while the subsequent date regex returns None.
- A malformed saved-page name can match its glob while the viewkey regex returns None outside the try block.
- Recursive JSON file entries are opened without lstat, no-follow, confinement, or byte limits.
- HTML paths are read completely.
- refresh_index catches only around the entire build.

Counterevidence and remaining uncertainty:
- os.walk does not follow directory symlinks by default.
- Own code/media/cache directories are excluded at the directory level.
- Normal JSON exceptions return defaults and RecursionError is caught in one traversal.
- An existing index cache remains available after a failed refresh, though stale.

Limitations:
- The exact permissions granted to each external scout/source producer are deployment-dependent.

#### Dataflow

The canonical finding records the affected path at app/store.py:66-71, app/index.py:64-77, app/index.py:95-130, app/index.py:157-176, app/server.py:72-79, but no expanded source-to-sink narrative was recorded.

Attack steps:
- Create ph-midnight-final-invalid.json, ph-hourly-lace-dig-invalid.json, or ph-vid-invalid!.html to trigger an unchecked capture; alternatively place a matching file symlink/FIFO/oversized regular file.
- Startup or an asynchronous refresh calls build_index.
- Filename parsing raises before its per-file try block, or open/json.load/read blocks or consumes unbounded memory.
- The top-level refresh handler logs the exception without committing new picks.
- A fresh service has no usable index or an existing service remains stale; resource-heavy inputs can degrade the process itself.

#### Reachability

SOURCES_DIR is recursively scanned by design, including bot/scout-produced files.

- **Attacker:** Source-directory writer or compromised automation

- **Entry point:** Matching JSON/HTML path beneath SOURCES_DIR

- **Source:** Filename, file type, and file contents

- **Sink:** Unchecked regex group, json.load, or whole-file read in build_index

- **Outcome:** Index rebuild abort, indefinite block, or memory exhaustion

Preconditions:
- Attacker or compromised automation can create entries in SOURCES_DIR.
- An index build or refresh is triggered.
- Index refresh occurs.

Existing controls:
- Directory exclusions avoid scanning known code/media/cache directories.
- A whole-build try/finally prevents a permanent building flag, but it does not preserve partial progress or bound parsing.

**Recursive scanning trusts every matching file path** — `app/index.py:95-115`

Directory filtering does not lstat or confine each file target, and there is no regular-file, symlink, byte, record, depth, or aggregate budget.

```python
for root, dirs, files in os.walk(WS):
    dirs[:] = [x for x in dirs if ...]
    for fn in files:
        if not fn.endswith('.json') or fn.startswith(('_exclude', '_prior')):
            continue
        p = os.path.join(root, fn)
        ...
        try:
            for vk, o in items_of(load_json(p)):
                ...
        except RecursionError:
            pass
```

**Saved HTML is read wholly without a file budget** — `app/index.py:116-124`

A matching path is read completely, and the narrower regex dereference occurs before the exception boundary.

```python
for p in glob.glob(os.path.join(WS, 'ph-vid-*.html')) + glob.glob(...):
    vk = re.search(r'(?:ph-vid-|_page_)([0-9a-z]+)\.html$', p).group(1)
    ...
    try:
        s = open(p, errors='ignore').read()
    except Exception:
        continue
```

**Broad JSON globs feed unchecked date captures** — `app/index.py:64-77`

A tiny filename matching the glob but lacking a date raises AttributeError and stops build_index.

```python
for p in sorted(glob.glob(os.path.join(WS, 'ph-midnight-final-*.json'))):
    d = re.search(r'(\d{4}-\d\d-\d\d)', p).group(1)
...
for p in sorted(glob.glob(os.path.join(WS, 'ph-hourly-lace-dig-*.json'))):
    m = re.search(r'(\d{4}-\d\d-\d\d)(?:-(\d\d)(\d\d))?', p)
    d, hh, mm = m.group(1), m.group(2) or '00', m.group(3) or '00'
```

**One source failure aborts the complete refresh** — `app/index.py:157-176`

The top-level catch keeps the process alive but cannot commit valid partial input; a fresh deployment remains empty and an existing deployment stays stale.

```python
def refresh_index(force=False):
    ...
    try:
        ...
        picks = build_index()
        INDEX.update(...)
        ...
    except Exception as e:
        log('index build failed:', repr(e))
    finally:
        building[0] = False
```

#### Severity

**Medium** — A writer in the source-integration boundary can deterministically prevent fresh deployments from building an index or leave existing deployments stale using tiny inputs, and can create process-level memory/blocking pressure with special or oversized files. The attacker needs local write influence over SOURCES_DIR.

Raise if a remote automation service can write directly into SOURCES_DIR; lower if the directory is strictly operator-only and all producers enforce regular-file and size constraints externally.

Impact assessment:
- **Level:** medium
- **Rationale:** Can make a fresh application empty, keep an existing index indefinitely stale, or exhaust shared memory/block a build.

Likelihood assessment:
- **Level:** medium
- **Rationale:** Tiny deterministic filename triggers exist, but the attacker needs local source-write capability.

#### Remediation

For each candidate path, use lstat/open-no-follow semantics and accept only regular files whose resolved/opened identity remains beneath SOURCES_DIR. Enforce per-file and aggregate byte, record, nesting, string-length, and processing-time budgets before parsing. Validate basenames with fullmatch and check every match before accessing groups. Put all filename parsing and file processing inside a per-file exception boundary so one bad source is skipped and logged. Consider a resource-limited worker for source parsing and commit only a complete validated snapshot.

Tests:
- Add glob-matching JSON filenames without ISO dates and assert they are skipped while valid sources still build.
- Add saved-page filenames with uppercase/punctuation outside the viewkey grammar and assert the rebuild succeeds.
- Add a file symlink escaping SOURCES_DIR and a FIFO/special-file fixture; assert the scanner rejects them without opening.
- Add oversized JSON and HTML fixtures and assert byte/aggregate limits fail fast without replacing the last good index.
- Verify a malformed file cannot keep building true or block later valid refreshes.

Preventive controls:
- Centralize safe source-file opening and budget enforcement.
- Use strict fullmatch filename grammars before opening files.
- Run integration parsers with filesystem and resource isolation appropriate to their trust level.

<a id="finding-3"></a>

### [3] Vote restore persists an unvalidated note timestamp into an HTML attribute

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | The client-controlled nested notes value is persisted without clean_notes and reaches a quoted attribute in markup assigned through innerHTML; the ordinary write path demonstrates the missing intended validator. |
| Category | Stored browser injection caused by inconsistent nested restore validation |
| CWE | CWE-79 |
| Affected lines | app/api.py:701-704, app/store.py:180-202, app/store.py:128-131, app/index.py:828-835, web/index.html:2850-2862 |

#### Summary

An admitted HTTP client can submit a crafted historical vote entry through the undo/restore branch. Unlike normal vote writes, restore_vote does not apply clean_notes, so an attacker-controlled note timestamp is stored and later interpolated without escaping into a data-t attribute in the My votes list. Rendering that list can execute injected markup in another admitted browser.

#### Root Cause

The undo API treats a client-supplied historical object as trusted server state. Its allowlist validates only top-level keys and a few scalar fields, omitting the nested notes schema enforced by normal writes.

**Restore forwards the supplied entry unchanged** — `app/api.py:701-704`

The restore operation accepts the nested entry object directly and does not call clean_notes.

```python
if level == 'restore':
    at = b.get('at')
    n = core.restore_vote(vk, b.get('entry'), at if isinstance(at, int) and not isinstance(at, bool) else None)
```

**Nested notes are not normalized before persistence** — `app/store.py:180-202`

notes is an allowed key, but its member types and timestamp are never checked before entry is written to votes.json.

```python
PAGE_KEYS = {'viewkey', 'vote', 'strength', 'reason', 'notes', 'date', 'source', 'title', 'uploader', 'lane'}

def restore_vote(vk, entry, at=None):
    if entry is not None:
        if not (isinstance(entry, dict) and entry.get('source') == 'vote-page' and entry.get('viewkey') == vk
                and entry.get('vote') in ('up', 'down') and set(entry) <= PAGE_KEYS):
            raise ValueError('bad restore entry')
    ...
    votes[pos[0]] = entry
    ...
    write_votes(votes)
```

**The intended validator requires a bounded numeric timestamp** — `app/store.py:140-151`

Normal vote and reason writes call this function; the restore branch is the inconsistent bypass.

```python
def clean_notes(notes):
    ...
    t = n.get('t'); text = str(n.get('text') or '').strip()[:200]; mode = n.get('mode')
    if isinstance(t, bool) or not isinstance(t, (int, float)) or not (0 <= t <= 86400): raise ValueError('bad note time')
    ...
    out.append({'t': int(round(t)), 'text': text, 'mode': mode})
```

#### Validation

Confirmed inconsistent restore validation and a persistent unescaped attribute sink.

Validation method: Independent static source-to-sink validation

- **Status:** validated
- **Disposition:** report

**Restore forwards the supplied entry unchanged** — `app/api.py:701-704`

The restore operation accepts the nested entry object directly and does not call clean_notes.

```python
if level == 'restore':
    at = b.get('at')
    n = core.restore_vote(vk, b.get('entry'), at if isinstance(at, int) and not isinstance(at, bool) else None)
```

**Nested notes are not normalized before persistence** — `app/store.py:180-202`

notes is an allowed key, but its member types and timestamp are never checked before entry is written to votes.json.

```python
PAGE_KEYS = {'viewkey', 'vote', 'strength', 'reason', 'notes', 'date', 'source', 'title', 'uploader', 'lane'}

def restore_vote(vk, entry, at=None):
    if entry is not None:
        if not (isinstance(entry, dict) and entry.get('source') == 'vote-page' and entry.get('viewkey') == vk
                and entry.get('vote') in ('up', 'down') and set(entry) <= PAGE_KEYS):
            raise ValueError('bad restore entry')
    ...
    votes[pos[0]] = entry
    ...
    write_votes(votes)
```

**Stored timestamp is interpolated unescaped into innerHTML** — `web/index.html:2850-2860`

A quote in n.t terminates data-t and can insert an event-bearing element when rowHTML is assigned to #rows.innerHTML.

```html
function rowHTML(it, i){
  const v = it.vote, lv = v ? v.level : 'none', r = splitReason(v && v.reason, v && v.notes);
  ...
  <div class="rnotes">${(v && v.notes || []).map((n, k) => `<span class="nchip" data-act="seek" data-t="${n.t}" data-k="${k}" role="button" tabindex="0" title="open the full video at ${fmtT(n.t)}">...</span>`).join('')}</div>
```

Assertions:
- An admitted client controls entry.notes\[\].t in the restore request.
- restore_vote persists that value without clean_notes.
- The My votes response returns stored notes.
- rowHTML inserts n.t into a quoted attribute and the resulting string is assigned with innerHTML.

Counterevidence and remaining uncertainty:
- Host, allowed-network, same-origin-style JSON, and known-viewkey guards restrict the source request.
- Exploitation requires another browser to render My votes.
- Ordinary vote and reason writes do validate notes.

Limitations:
- The application is documented as single-user by default; multi-client deployment determines how often the cross-browser impact is meaningful.

#### Dataflow

The canonical finding records the affected path at app/api.py:701-704, app/store.py:180-202, app/store.py:128-131, app/index.py:828-835, web/index.html:2850-2862, but no expanded source-to-sink narrative was recorded.

Attack steps:
- Send POST /api/vote for a known viewkey with level=restore and entry.notes containing a string timestamp that closes data-t and adds active markup.
- restore_vote accepts and writes the entry to votes.json.
- A victim opens the My votes view, causing queue('myvotes') to return the stored notes.
- rowHTML builds a string containing the attacker timestamp, and #rows.innerHTML parses the injected element.

#### Reachability

Source-to-sink path is fully reachable within the documented multi-client network model.

- **Attacker:** Admitted network client

- **Entry point:** POST /api/vote with level=restore

- **Source:** entry.notes\[\].t

- **Sink:** web/index.html rowHTML data-t interpolation parsed by innerHTML

- **Outcome:** Persistent script execution in a victim's application origin

Preconditions:
- Attacker can directly reach the application from ALLOWED_NETS and satisfy its JSON/Origin or Fetch Metadata guard.
- The target viewkey is known to the application.
- A victim later renders My votes.
- Victim opens My votes.

Existing controls:
- Network, Host, request-content, Origin/Fetch-Metadata, and known-viewkey checks remain effective but do not validate the restore object's nested notes.
- The response CSP only sets frame-ancestors and does not block inline event handlers.

**Restore forwards the supplied entry unchanged** — `app/api.py:701-704`

The restore operation accepts the nested entry object directly and does not call clean_notes.

```python
if level == 'restore':
    at = b.get('at')
    n = core.restore_vote(vk, b.get('entry'), at if isinstance(at, int) and not isinstance(at, bool) else None)
```

**Nested notes are not normalized before persistence** — `app/store.py:180-202`

notes is an allowed key, but its member types and timestamp are never checked before entry is written to votes.json.

```python
PAGE_KEYS = {'viewkey', 'vote', 'strength', 'reason', 'notes', 'date', 'source', 'title', 'uploader', 'lane'}

def restore_vote(vk, entry, at=None):
    if entry is not None:
        if not (isinstance(entry, dict) and entry.get('source') == 'vote-page' and entry.get('viewkey') == vk
                and entry.get('vote') in ('up', 'down') and set(entry) <= PAGE_KEYS):
            raise ValueError('bad restore entry')
    ...
    votes[pos[0]] = entry
    ...
    write_votes(votes)
```

**Stored timestamp is interpolated unescaped into innerHTML** — `web/index.html:2850-2860`

A quote in n.t terminates data-t and can insert an event-bearing element when rowHTML is assigned to #rows.innerHTML.

```html
function rowHTML(it, i){
  const v = it.vote, lv = v ? v.level : 'none', r = splitReason(v && v.reason, v && v.notes);
  ...
  <div class="rnotes">${(v && v.notes || []).map((n, k) => `<span class="nchip" data-act="seek" data-t="${n.t}" data-k="${k}" role="button" tabindex="0" title="open the full video at ${fmtT(n.t)}">...</span>`).join('')}</div>
```

#### Severity

**Medium** — The attacker must already be inside an allowed network and a victim must open My votes, but the primitive crosses from ordinary shared-state mutation into persistent script execution in another browser's private application origin.

Raise if the service is shared with mutually untrusted network clients or the origin gains additional browser-held credentials; lower if the deployment is provably single-user.

Impact assessment:
- **Level:** medium
- **Rationale:** Application-origin script can read private UI/API data, alter shared state, and access device-local origin storage.

Likelihood assessment:
- **Level:** medium
- **Rationale:** The request is simple for an admitted peer, but deployment is primarily single-user and a victim interaction is required.

#### Remediation

Do not accept a client-supplied vote entry as an undo token. Retain previous state server-side or issue an opaque, integrity-protected reference. If restore remains, reconstruct the entry from explicitly validated fields and run notes through clean_notes before persistence. In the browser, require finite numeric timestamps and create note elements with DOM APIs/textContent/dataset instead of HTML strings. A restrictive script-src CSP adds defense in depth after inline handlers are removed or nonced.

Tests:
- POST a restore entry whose notes timestamp is a string containing a quote and active markup; expect HTTP 400 and confirm votes.json is unchanged.
- Restore a valid historical entry and confirm notes are normalized exactly as in ordinary vote writes.
- Render a stored malformed legacy note and assert no element or event-handler injection occurs.
- Add an end-to-end two-client test: attacker submits the restore payload, victim opens My votes, and no script-visible sentinel changes.

Preventive controls:
- Use one server-side schema validator for new votes, reason edits, and restores.
- Represent undo state by server-generated opaque identifiers rather than round-tripping trusted objects through the browser.
- Build dynamic DOM with node APIs and enforce a script-src CSP.

<a id="finding-4"></a>

### [4] Root Tailscale node state is downgraded to box-user-writable persistent storage

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | medium |
| Confidence rationale | The root-to-user archive and unauthenticated restore are explicit, and documentation states the archive restores identity without login. Exact clone behavior and tailnet privileges depend on Tailscale state semantics and external ACLs not present in the repository. |
| Category | Privileged credential material stored across a weaker local trust boundary |
| CWE | CWE-732 |
| Affected lines | scripts/ensure-up.sh:71-81, scripts/ensure-up.sh:138-145, scripts/ensure-up.sh:165-173, docs/live-switch.md:72-79, deploy/box.env:18 |

#### Summary

The live recovery script uses sudo to archive the root-owned Tailscale state directory, but shell redirection creates state.tgz in the unprivileged box account's persistence directory and chmods it 0600 for that account. The same user-controlled archive is later restored into /var/lib/tailscale without integrity or expected-identity verification. Compromise of the application account can therefore disclose reusable node identity material and can substitute a different valid state at the next reset.

#### Root Cause

A reset-survival design places privileged node credentials and root-consumed state in storage owned and writable by the application/operator account, collapsing the intended privilege boundary in both directions.

**The persistence directory belongs to the unprivileged box account** — `scripts/ensure-up.sh:71-81`

deploy/box.env sets TS_PERSIST_DIR=/home/box/.persist/tailscale, so the account running the application and recovery script owns the archive location.

```bash
PERSIST=$(_abs "${TS_PERSIST_DIR:-$HOME/.persist/tailscale}")
...
mkdir -p "$PERSIST" && chmod 700 "$PERSIST" ...
```

**Any nonempty user-controlled archive is restored as root daemon state** — `scripts/ensure-up.sh:138-145`

No owner, digest, signature, node identity, or tailnet identity check authenticates the archive before it becomes root tailscaled state.

```bash
if ! $SUDO test -s "$STATE"; then
  if [ -s "$PERSIST/state.tgz" ]; then
    ...
    $SUDO mkdir -p "$STATE_DIR" && $SUDO chmod 700 "$STATE_DIR" && $SUDO tar -xzf "$PERSIST/state.tgz" -C "$STATE_DIR" \
      && $SUDO chown -R root:root "$STATE_DIR" && log "restored tailscale state from $PERSIST/state.tgz" ...
```

**sudo reads root state but user-shell redirection owns the archive** — `scripts/ensure-up.sh:165-173`

sudo tar emits the complete root state to stdout; redirection and mktemp run as the box account, and chmod 600 protects the file for that account rather than root.

```bash
if [ "$BS" = Running ] && $SUDO test -s "$STATE"; then
  new=$($SUDO cat "$STATE" | sha256sum | cut -d' ' -f1)
  ...
  tmp=$(mktemp "$PERSIST/.state.XXXXXX")
  $SUDO tar -czf - -C "$STATE_DIR" --exclude='tailscaled.log*' --exclude='derpmap.cached.json' . > "$tmp" 2>>"$LOG"
  if [ -s "$tmp" ]; then chmod 600 "$tmp" && mv -f "$tmp" "$PERSIST/state.tgz" ...
```

**Documentation confirms the archive is reusable node identity** — `docs/live-switch.md:72-79`

The archive is not a benign cache: it preserves authentication identity sufficiently to avoid a new login after reset.

```markdown
`/workspace/ph-vote/scripts/ensure-up.sh` ... reinstalls Tailscale from the saved
`.deb`, restores the `picks` identity from `~/.persist/tailscale/state.tgz` and starts tailscaled.
... a server-side routine runs it every 15-30 min.
```

#### Validation

Confirmed root-only daemon state is exported to a user-readable archive and later trusted as root state without authenticity checks.

Validation method: Independent static credential-flow and recovery-path validation

- **Status:** validated
- **Disposition:** report

**sudo reads root state but user-shell redirection owns the archive** — `scripts/ensure-up.sh:165-173`

sudo tar emits the complete root state to stdout; redirection and mktemp run as the box account, and chmod 600 protects the file for that account rather than root.

```bash
if [ "$BS" = Running ] && $SUDO test -s "$STATE"; then
  new=$($SUDO cat "$STATE" | sha256sum | cut -d' ' -f1)
  ...
  tmp=$(mktemp "$PERSIST/.state.XXXXXX")
  $SUDO tar -czf - -C "$STATE_DIR" --exclude='tailscaled.log*' --exclude='derpmap.cached.json' . > "$tmp" 2>>"$LOG"
  if [ -s "$tmp" ]; then chmod 600 "$tmp" && mv -f "$tmp" "$PERSIST/state.tgz" ...
```

**Any nonempty user-controlled archive is restored as root daemon state** — `scripts/ensure-up.sh:138-145`

No owner, digest, signature, node identity, or tailnet identity check authenticates the archive before it becomes root tailscaled state.

```bash
if ! $SUDO test -s "$STATE"; then
  if [ -s "$PERSIST/state.tgz" ]; then
    ...
    $SUDO mkdir -p "$STATE_DIR" && $SUDO chmod 700 "$STATE_DIR" && $SUDO tar -xzf "$PERSIST/state.tgz" -C "$STATE_DIR" \
      && $SUDO chown -R root:root "$STATE_DIR" && log "restored tailscale state from $PERSIST/state.tgz" ...
```

**Documentation confirms the archive is reusable node identity** — `docs/live-switch.md:72-79`

The archive is not a benign cache: it preserves authentication identity sufficiently to avoid a new login after reset.

```markdown
`/workspace/ph-vote/scripts/ensure-up.sh` ... reinstalls Tailscale from the saved
`.deb`, restores the `picks` identity from `~/.persist/tailscale/state.tgz` and starts tailscaled.
... a server-side routine runs it every 15-30 min.
```

Assertions:
- sudo tar reads the entire Tailscale state directory except logs/cache.
- The output file is created and retained by the box account with mode 0600.
- Documentation says this archive restores the node identity without login.
- A replacement archive is extracted into the root state directory on reset without integrity or identity validation.

Counterevidence and remaining uncertainty:
- 0700/0600 modes exclude unrelated Unix users.
- The archive is outside version control.
- The attacker needs same-box-account execution.
- Substitution requires a valid Tailscale state to meaningfully alter network membership.

Limitations:
- The exact cryptographic contents, clone handling, node expiration, machine approval, and ACL permissions are external to this repository.

#### Dataflow

The canonical finding records the affected path at scripts/ensure-up.sh:71-81, scripts/ensure-up.sh:138-145, scripts/ensure-up.sh:165-173, docs/live-switch.md:72-79, deploy/box.env:18, but no expanded source-to-sink narrative was recorded.

Attack steps:
- Obtain code execution as the box/application account.
- Read /home/box/.persist/tailscale/state.tgz, which was created from root's live state.
- Copy the archive off-host for attempted identity reuse, or replace it with another valid state.
- After a reset removes the live state, recurring ensure-up.sh extracts the user-controlled archive into /var/lib/tailscale and starts root tailscaled.
- The attacker retains or changes node identity subject to external Tailscale controls.

#### Reachability

The documented reset-recovery lifecycle routinely exports and later consumes this archive.

- **Attacker:** Compromised box/application account

- **Entry point:** User ownership of TS_PERSIST_DIR

- **Source:** Root-owned /var/lib/tailscale state

- **Sink:** User-readable state.tgz and later sudo tar restoration

- **Outcome:** Credential disclosure and conditional node-identity substitution

Preconditions:
- The live deployment enables ensure-up Tailscale persistence and has successfully archived a Running state.
- Attacker compromises the box account.
- For substitution impact, a valid alternate state and favorable external ACLs exist.
- Tailscale persistence enabled and archive created.

Existing controls:
- Unix modes exclude other local users but intentionally grant the box account access.
- Recovery only restores when root state is missing and does not overwrite a healthy state.

**sudo reads root state but user-shell redirection owns the archive** — `scripts/ensure-up.sh:165-173`

sudo tar emits the complete root state to stdout; redirection and mktemp run as the box account, and chmod 600 protects the file for that account rather than root.

```bash
if [ "$BS" = Running ] && $SUDO test -s "$STATE"; then
  new=$($SUDO cat "$STATE" | sha256sum | cut -d' ' -f1)
  ...
  tmp=$(mktemp "$PERSIST/.state.XXXXXX")
  $SUDO tar -czf - -C "$STATE_DIR" --exclude='tailscaled.log*' --exclude='derpmap.cached.json' . > "$tmp" 2>>"$LOG"
  if [ -s "$tmp" ]; then chmod 600 "$tmp" && mv -f "$tmp" "$PERSIST/state.tgz" ...
```

**Any nonempty user-controlled archive is restored as root daemon state** — `scripts/ensure-up.sh:138-145`

No owner, digest, signature, node identity, or tailnet identity check authenticates the archive before it becomes root tailscaled state.

```bash
if ! $SUDO test -s "$STATE"; then
  if [ -s "$PERSIST/state.tgz" ]; then
    ...
    $SUDO mkdir -p "$STATE_DIR" && $SUDO chmod 700 "$STATE_DIR" && $SUDO tar -xzf "$PERSIST/state.tgz" -C "$STATE_DIR" \
      && $SUDO chown -R root:root "$STATE_DIR" && log "restored tailscale state from $PERSIST/state.tgz" ...
```

**Documentation confirms the archive is reusable node identity** — `docs/live-switch.md:72-79`

The archive is not a benign cache: it preserves authentication identity sufficiently to avoid a new login after reset.

```markdown
`/workspace/ph-vote/scripts/ensure-up.sh` ... reinstalls Tailscale from the saved
`.deb`, restores the `picks` identity from `~/.persist/tailscale/state.tgz` and starts tailscaled.
... a server-side routine runs it every 15-30 min.
```

#### Severity

**Medium** — The archive is deliberately sufficient to restore the node without login, so access can outlive the host and affect tailnet identity. Exploitation requires same-account code execution, while concrete off-host reachability and the effect of substituted state depend on external Tailscale ACLs and machine authorization.

Raise if the node has broad tailnet access or the box account is exposed through additional services; lower if state.tgz is independently encrypted to a root-only key and external ACLs make cloned state unusable.

Impact assessment:
- **Level:** medium
- **Rationale:** Reusable node identity material can extend compromise beyond the application host and may grant tailnet access; substituted state can alter service reachability.

Likelihood assessment:
- **Level:** medium
- **Rationale:** Archive access is direct after box-account compromise, but actual off-host reuse and privileges depend on Tailscale controls.

#### Remediation

Store Tailscale state backups in a root-owned directory inaccessible to the application account, or encrypt them with a key unavailable to that account. Have a small root-owned recovery unit perform backup and restore without user-shell redirection. Authenticate every backup with a root-only MAC/signature and verify expected node/tailnet identity before daemon start. Keep package cache and state backups separate and apply least-privilege sudo.

Tests:
- As the box account, assert state.tgz cannot be read, replaced, or deleted.
- Replace or modify a copied backup and assert restore fails authenticity verification before extraction.
- Restore an authentic backup and verify the resulting node/tailnet identity matches an expected pinned identity.
- Audit archive creation ownership and mode immediately after a healthy backup; require root ownership and no app-account access.

Preventive controls:
- Keep privileged daemon credentials out of application-user storage.
- Authenticate reset-survival artifacts before privileged restore.
- Separate operational recovery into root-owned code and data with explicit identity checks.

<a id="finding-5"></a>

### [5] Unbounded fuzzy duplicate matching performs quadratic work on every queue and home request

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | The nested comparisons, SequenceMatcher call, unbounded title normalization, and per-request queue/home call sites are explicit. Exact exhaustion thresholds depend on hardware and dataset shape. |
| Category | Algorithmic-complexity denial of service in source-driven request processing |
| CWE | CWE-407 |
| Affected lines | app/index.py:392-420, app/index.py:422-470, app/index.py:778-788, app/index.py:842-845, app/api.py:249-262 |

#### Summary

sent-log titles and collection sizes are unbounded, and suppress_tonight_dups compares every unvoted current-round title against all previously kept titles and recent history using difflib.SequenceMatcher. The queue endpoint performs this from scratch for each request, and home calls queue internally. A source-data writer can supply many long, equal-length, nonmatching titles and make ordinary or repeated admitted requests consume superlinear CPU.

#### Root Cause

Fuzzy deduplication operates directly over unbounded source collections and strings on synchronous request paths, without precomputation, indexing, caching, or a work budget.

**Long source-controlled titles reach SequenceMatcher** — `app/index.py:392-408`

No title-length budget precedes the fuzzy matcher; equal-length nonmatching titles bypass the only cheap length rejection.

```python
def normalize_title(t):
    if not isinstance(t, str): return ''
    s = t.lower()
    s = re.sub(r'[^a-z0-9\s]+', ' ', s)
    return re.sub(r'\s+', ' ', s).strip()
...
def title_near_dup(a, b, ratio=DUP_TITLE_RATIO):
    if not a or not b: return False
    if a == b: return True
    if abs(len(a) - len(b)) > max(12, int(0.4 * max(len(a), len(b)))): return False
    return difflib.SequenceMatcher(None, a, b).ratio() >= ratio
```

**Each current title scans all kept and recent titles** — `app/index.py:422-470`

For unique nonmatching titles, seen_titles grows on every iteration and the first nested loop performs approximately N²/2 fuzzy comparisons, plus recent-history comparisons.

```python
for vk, e in pairs:
    ...
    nt = normalize_title(title)
    ...
    for ot, ovk in seen_titles:
        if title_near_dup(nt, ot):
            ...
            break
    if not why and nt:
        for ot, ovk, od, _ou in recent:
            if title_near_dup(nt, ot):
                ...
                break
    ...
    if nt: seen_titles.append((nt, vk))
```

**Every queue request runs duplicate suppression** — `app/index.py:778-788`

There is no cache keyed by source version, comparison budget, or background precomputation.

```python
def queue(mode):
    votes = read_votes(); ...
    tn, info = tonight_round()
    ...
    tn_raw_n = len(tn)
    tn, dup_suppressed = suppress_tonight_dups(tn, {vk for vk, _ in tn if voted(vk)})
```

**The dashboard duplicates the same request-path work** — `app/index.py:842-845`

Both /api/queue and /api/home expose the unbounded algorithm on normal page/dashboard reads.

```python
def home(xh_limit=12, joined_limit=24):
    """Dashboard payload: queue counts (same as /api/queue) + read-only source hubs. No PH network."""
    q = queue('tonight')
```

#### Validation

Confirmed quadratic comparison growth over source-controlled titles and repeated execution on two HTTP read paths.

Validation method: Independent static algorithm and request-path validation

- **Status:** validated
- **Disposition:** report

**Long source-controlled titles reach SequenceMatcher** — `app/index.py:392-408`

No title-length budget precedes the fuzzy matcher; equal-length nonmatching titles bypass the only cheap length rejection.

```python
def normalize_title(t):
    if not isinstance(t, str): return ''
    s = t.lower()
    s = re.sub(r'[^a-z0-9\s]+', ' ', s)
    return re.sub(r'\s+', ' ', s).strip()
...
def title_near_dup(a, b, ratio=DUP_TITLE_RATIO):
    if not a or not b: return False
    if a == b: return True
    if abs(len(a) - len(b)) > max(12, int(0.4 * max(len(a), len(b)))): return False
    return difflib.SequenceMatcher(None, a, b).ratio() >= ratio
```

**Each current title scans all kept and recent titles** — `app/index.py:422-470`

For unique nonmatching titles, seen_titles grows on every iteration and the first nested loop performs approximately N²/2 fuzzy comparisons, plus recent-history comparisons.

```python
for vk, e in pairs:
    ...
    nt = normalize_title(title)
    ...
    for ot, ovk in seen_titles:
        if title_near_dup(nt, ot):
            ...
            break
    if not why and nt:
        for ot, ovk, od, _ou in recent:
            if title_near_dup(nt, ot):
                ...
                break
    ...
    if nt: seen_titles.append((nt, vk))
```

**Every queue request runs duplicate suppression** — `app/index.py:778-788`

There is no cache keyed by source version, comparison budget, or background precomputation.

```python
def queue(mode):
    votes = read_votes(); ...
    tn, info = tonight_round()
    ...
    tn_raw_n = len(tn)
    tn, dup_suppressed = suppress_tonight_dups(tn, {vk for vk, _ in tn if voted(vk)})
```

**The dashboard duplicates the same request-path work** — `app/index.py:842-845`

Both /api/queue and /api/home expose the unbounded algorithm on normal page/dashboard reads.

```python
def home(xh_limit=12, joined_limit=24):
    """Dashboard payload: queue counts (same as /api/queue) + read-only source hubs. No PH network."""
    q = queue('tonight')
```

Assertions:
- sent-log record count and title length are not capped before deduplication.
- Unique equal-length titles force the growing seen_titles loop to run to completion.
- SequenceMatcher is invoked for each pair.
- queue recomputes on every call, and home calls queue.

Counterevidence and remaining uncertainty:
- A length-difference check cheaply rejects titles with substantially different lengths.
- Exact duplicate or near-duplicate titles can break comparison loops early.
- The intended deployment is small and private.
- The attacker needs write influence over source data.

Limitations:
- Static review did not benchmark the input size needed to saturate production hardware.

#### Dataflow

The canonical finding records the affected path at app/index.py:392-420, app/index.py:422-470, app/index.py:778-788, app/index.py:842-845, app/api.py:249-262, but no expanded source-to-sink narrative was recorded.

Attack steps:
- Populate sent-log.json with many current-round entries whose long normalized titles have similar lengths but do not match.
- An admitted user opens the vote page/dashboard or an attacker repeatedly calls /api/queue or /api/home.
- suppress_tonight_dups compares each title with every earlier kept title using SequenceMatcher.
- CPU work grows quadratically with record count and with title-comparison cost, occupying request workers and the Python process.

#### Reachability

Authoritative sent-log data flows directly into synchronous read endpoints.

- **Attacker:** Source/scout automation writer, optionally paired with an admitted client

- **Entry point:** shared/sent-log.json

- **Source:** Current-round titles and record count

- **Sink:** difflib.SequenceMatcher in suppress_tonight_dups on /api/queue and /api/home

- **Outcome:** CPU exhaustion and degraded/denied service

Preconditions:
- Attacker can influence sent-log.json or its producer.
- A queue or home request occurs.
- Current-round adversarial data and a read request.

Existing controls:
- The service is network-private and the length-difference shortcut removes some pairs.
- No record, title, comparison, time, or per-source-version cache bound exists.

**Each current title scans all kept and recent titles** — `app/index.py:422-470`

For unique nonmatching titles, seen_titles grows on every iteration and the first nested loop performs approximately N²/2 fuzzy comparisons, plus recent-history comparisons.

```python
for vk, e in pairs:
    ...
    nt = normalize_title(title)
    ...
    for ot, ovk in seen_titles:
        if title_near_dup(nt, ot):
            ...
            break
    if not why and nt:
        for ot, ovk, od, _ou in recent:
            if title_near_dup(nt, ot):
                ...
                break
    ...
    if nt: seen_titles.append((nt, vk))
```

**Every queue request runs duplicate suppression** — `app/index.py:778-788`

There is no cache keyed by source version, comparison budget, or background precomputation.

```python
def queue(mode):
    votes = read_votes(); ...
    tn, info = tonight_round()
    ...
    tn_raw_n = len(tn)
    tn, dup_suppressed = suppress_tonight_dups(tn, {vk for vk, _ in tn if voted(vk)})
```

**The dashboard duplicates the same request-path work** — `app/index.py:842-845`

Both /api/queue and /api/home expose the unbounded algorithm on normal page/dashboard reads.

```python
def home(xh_limit=12, joined_limit=24):
    """Dashboard payload: queue counts (same as /api/queue) + read-only source hubs. No PH network."""
    q = queue('tonight')
```

#### Severity

**Medium** — The algorithm is persistent on common read paths and attacker-controlled title length multiplies the all-pairs cost, allowing service-level CPU denial. The attacker must influence authoritative source data; request amplification then requires only admitted client access or normal page loads.

Raise if remote automation can write sent-log directly or production rounds are large; lower if trusted producers enforce strict record/title limits and a front proxy rate-limits reads.

Impact assessment:
- **Level:** medium
- **Rationale:** Can make core page/dashboard reads consume sustained CPU and deny the single self-hosted service.

Likelihood assessment:
- **Level:** medium
- **Rationale:** Requires source-write influence, but the triggering read is routine and the adversarial dataset is straightforward.

#### Remediation

Cap accepted title length and current/recent record counts at ingestion. Precompute normalized titles when the index changes and cache suppression results by the sent-log/index version. Replace all-pairs SequenceMatcher with bounded candidate buckets or fingerprints, apply a strict comparison/time budget, and fall back to exact matching when exhausted. Perform expensive refresh work outside request handlers.

Tests:
- Generate N unique equal-length adversarial titles and assert comparison count/time stays within a linear or explicitly bounded budget as N grows.
- Send overlong titles and oversized rounds and assert they are truncated/rejected before fuzzy matching.
- Call /api/queue and /api/home repeatedly without source changes and assert duplicate suppression is computed once per source version.
- Exhaust the comparison budget and verify the endpoint returns promptly with a documented exact-match fallback.

Preventive controls:
- Budget source-controlled algorithmic work by records, bytes, comparisons, and time.
- Cache source-derived computations outside request paths.
- Use performance regression tests with adversarial, not only realistic, title distributions.

<a id="finding-6"></a>

### [6] The UI smoke test renders live third-party content with Chromium's sandbox disabled

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | The test explicitly waits for Pornhub embed URLs, disables the proxy, disables the browser sandbox, and contains a deterministic fixture that reaches that path. |
| Category | Developer-tool isolation failure for untrusted browser content |
| CWE | CWE-693 |
| Affected lines | tests/parity/README.md:3-10, tests/parity/ui_queue_smoke.py:48-49, tests/parity/ui_queue_smoke.py:62-67, tests/parity/ui_queue_smoke.py:92-104, tests/parity/fake_ytdlp/yt_dlp/__main__.py:5-8 |

#### Summary

tests/parity/ui_queue_smoke.py deliberately drives an unfetchable clip into a live Pornhub embed while launching Chromium with both --no-sandbox and --no-proxy-server. This contradicts the harness's offline isolation claim and exposes the developer or CI account to live third-party browser content without Chromium's normal process sandbox.

#### Root Cause

The smoke test relies on production's live external-embed fallback while disabling both the browser sandbox and proxy isolation, so its test fixture is not self-contained.

**The harness claims no network use** — `tests/parity/README.md:3-10`

The documented safety property is local, temporary, and offline execution.

```markdown
Proves that the FastAPI app (`app/api.py`) keeps stable behaviour across changes. Everything runs against
**temp data** on spare ports and never touches the live data or the network:
...
- ... `fake_ytdlp/` on `PYTHONPATH` stands in for yt-dlp ...; everything else goes to a dead proxy
```

**The test waits for a real Pornhub iframe** — `tests/parity/ui_queue_smoke.py:62-67`

The asserted success condition is creation of a live third-party iframe, not a local stub.

```python
# a clip the server can't fetch (fake yt-dlp fails viewkeys ending in 04): straight to the site's embed in his browser
...
if i04 >= 0:
    await ev(f"jumpTo({i04}); true")
    await wait(ev, "cur && cur.media === 'embed' && !!document.querySelector('#stage iframe[src^=\"https://www.pornhub.com/embed/\"]')", 20)
```

**The fixture guarantees the embed fallback** — `tests/parity/ui_queue_smoke.py:92-100`

The special 04 identifier is always added to the local queue.

```python
extra = [...]
extra.append({'viewkey': 'zz15queue04', ...})   # fake yt-dlp fails it
...
json.dump(json.load(open(al)) + [x['viewkey'] for x in extra], open(al, 'w'))
```

**Chromium is launched unsandboxed with direct network access** — `tests/parity/ui_queue_smoke.py:101-104`

A temporary profile limits persistent browser state, but --no-sandbox removes the OS containment layer and --no-proxy-server bypasses the harness's dead proxy.

```python
prof = tempfile.mkdtemp(prefix='phqueue-chrome-')
ch = subprocess.Popen([CHROME, '--headless=new', '--no-sandbox', f'--remote-debugging-port={DBG}', f'--user-data-dir={prof}',
                       '--no-proxy-server', 'about:blank'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
```

#### Validation

Confirmed deterministic live iframe creation in a Chromium process launched without its sandbox.

Validation method: Independent static control-flow validation

- **Status:** validated
- **Disposition:** report

**The test waits for a real Pornhub iframe** — `tests/parity/ui_queue_smoke.py:62-67`

The asserted success condition is creation of a live third-party iframe, not a local stub.

```python
# a clip the server can't fetch (fake yt-dlp fails viewkeys ending in 04): straight to the site's embed in his browser
...
if i04 >= 0:
    await ev(f"jumpTo({i04}); true")
    await wait(ev, "cur && cur.media === 'embed' && !!document.querySelector('#stage iframe[src^=\"https://www.pornhub.com/embed/\"]')", 20)
```

**The fixture guarantees the embed fallback** — `tests/parity/ui_queue_smoke.py:92-100`

The special 04 identifier is always added to the local queue.

```python
extra = [...]
extra.append({'viewkey': 'zz15queue04', ...})   # fake yt-dlp fails it
...
json.dump(json.load(open(al)) + [x['viewkey'] for x in extra], open(al, 'w'))
```

**Chromium is launched unsandboxed with direct network access** — `tests/parity/ui_queue_smoke.py:101-104`

A temporary profile limits persistent browser state, but --no-sandbox removes the OS containment layer and --no-proxy-server bypasses the harness's dead proxy.

```python
prof = tempfile.mkdtemp(prefix='phqueue-chrome-')
ch = subprocess.Popen([CHROME, '--headless=new', '--no-sandbox', f'--remote-debugging-port={DBG}', f'--user-data-dir={prof}',
                       '--no-proxy-server', 'about:blank'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
```

Assertions:
- The 04 fixture is deterministic and fake yt-dlp rejects it.
- The test waits until a https://www.pornhub.com/embed/ iframe exists.
- --no-proxy-server permits direct browser egress.
- --no-sandbox disables Chromium's process sandbox.

Counterevidence and remaining uncertainty:
- The browser uses a fresh temporary profile.
- The test server and data are temporary.
- Host compromise still requires a browser exploit or equivalent malicious external content.

Limitations:
- Whether this test runs in CI and what secrets or mounts are present is outside the repository.

#### Dataflow

The canonical finding records the affected path at tests/parity/README.md:3-10, tests/parity/ui_queue_smoke.py:48-49, tests/parity/ui_queue_smoke.py:62-67, tests/parity/ui_queue_smoke.py:92-104, tests/parity/fake_ytdlp/yt_dlp/__main__.py:5-8, but no expanded source-to-sink narrative was recorded.

Attack steps:
- A developer or CI job runs ui_queue_smoke.py.
- The fixture selects zz15queue04; fake yt-dlp returns an error.
- The application falls back to a live Pornhub embed.
- Chromium fetches and renders that external content because the proxy is disabled.
- A renderer exploit, if delivered, executes without the normal Chromium process sandbox under the test account.

#### Reachability

The external load is an asserted test path, and the browser launch flags remove the intended isolation.

- **Attacker:** Compromised or malicious third-party embed/CDN content

- **Entry point:** Live Pornhub iframe requested by the smoke test

- **Source:** External browser content

- **Sink:** Chromium renderer launched with --no-sandbox

- **Outcome:** Potential code execution with the test account's host access

Preconditions:
- Chromium is installed and the host permits outbound network access.
- The external content or delivery chain supplies a browser exploit.
- A compatible browser exploit is delivered.

Existing controls:
- A temporary user-data directory reduces persistent browser-state exposure.
- The test kills Chromium in finally, but only after content has executed.

**The test waits for a real Pornhub iframe** — `tests/parity/ui_queue_smoke.py:62-67`

The asserted success condition is creation of a live third-party iframe, not a local stub.

```python
# a clip the server can't fetch (fake yt-dlp fails viewkeys ending in 04): straight to the site's embed in his browser
...
if i04 >= 0:
    await ev(f"jumpTo({i04}); true")
    await wait(ev, "cur && cur.media === 'embed' && !!document.querySelector('#stage iframe[src^=\"https://www.pornhub.com/embed/\"]')", 20)
```

**Chromium is launched unsandboxed with direct network access** — `tests/parity/ui_queue_smoke.py:101-104`

A temporary profile limits persistent browser state, but --no-sandbox removes the OS containment layer and --no-proxy-server bypasses the harness's dead proxy.

```python
prof = tempfile.mkdtemp(prefix='phqueue-chrome-')
ch = subprocess.Popen([CHROME, '--headless=new', '--no-sandbox', f'--remote-debugging-port={DBG}', f'--user-data-dir={prof}',
                       '--no-proxy-server', 'about:blank'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
```

#### Severity

**Medium** — A successful renderer compromise would inherit the test user's host privileges and can affect the workspace or readable credentials, but exploitation requires a browser vulnerability or equivalent compromise in a third-party delivery chain and the smoke test to run.

Raise if the test runs automatically on privileged CI hosts with secrets or writable sensitive mounts; lower if it is always confined to an unprivileged, network-isolated container.

Impact assessment:
- **Level:** high
- **Rationale:** A renderer compromise can access the developer/CI account's files and credentials.

Likelihood assessment:
- **Level:** low
- **Rationale:** The test must run with egress and exploitation requires a browser vulnerability or supply-chain compromise.

#### Remediation

Remove --no-sandbox for developer-host execution. Block all non-loopback browser egress and replace the third-party embed with a local stub or CDP request interception. If the browser sandbox cannot run in the target environment, execute this test inside a dedicated unprivileged container or VM with no secrets, no sensitive writable mounts, and no Internet access.

Tests:
- Run the smoke test with outbound requests logged or intercepted and assert every destination is loopback.
- Assert the launched Chromium command does not contain --no-sandbox on ordinary developer hosts.
- Make the failed-media embed point at a local stub and preserve the existing fallback assertions.
- In the isolated fallback environment, verify no credentials or host workspace mounts are present.

Preventive controls:
- Treat browser tests as untrusted-content execution and require sandboxing or stronger outer isolation.
- Make offline-test network policy fail closed instead of relying on application proxy settings.
- Keep live third-party URLs out of deterministic regression fixtures.

<a id="finding-7"></a>

### [7] Admitted clients can create unbounded background threads on preview, queue, and webhook paths

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Each route-to-thread path is explicit, and duplicate work is not marked pending until after the thread is created. Existing per-item locks serialize work inside workers but do not bound waiting threads. |
| Category | Uncontrolled request-triggered background concurrency |
| CWE | CWE-770 |
| Affected lines | app/api.py:310-325, app/clips.py:431-485, app/api.py:249-262, app/index.py:813-818, app/api.py:610-624, app/store.py:515-542 |

#### Summary

Several HTTP-triggered operations start raw daemon threads without a shared executor, queue cap, rate limit, or pending-work coalescing. Repeated uncached preview requests create duplicate workers that block behind a per-viewkey lock, queue/home reads start duplicate 30-clip sweeps, and an enabled MLT webhook starts a new credentialed delivery thread for each repeated request. An admitted client can exhaust process threads, memory, sockets, and upstream capacity.

#### Root Cause

Expensive request-triggered background work uses ad hoc raw threads instead of a bounded, observable queue with per-key single-flight state.

**Every pending preview request starts a raw thread** — `app/api.py:310-323`

fetch=False does not expose an in-flight state, so every duplicate request observes 'not fetched yet' and schedules another worker.

```python
@app.get('/api/preview')
def api_preview(request: Request):
    ...
    r = dict(core.preview_for(vk, fetch=False))
    if r.get('kind') is None and r.get('why') == 'not fetched yet':
        core.prefetch_preview(vk)
        r = {'kind': None, 'why': 'preview is being fetched', 'pending': True}
```

**Duplicate threads are created before per-key serialization** — `app/clips.py:439-485`

The per-viewkey lock is acquired inside the spawned worker, so duplicate requests accumulate blocked operating-system threads rather than coalescing.

```python
with prev_lock:
    rec = PREV.get(vk); vl = prev_vk_locks.setdefault(vk, threading.Lock())
...
with vl:
    ...

def prefetch_preview(vk):
    threading.Thread(target=lambda: preview_for(vk), daemon=True).start()
```

**Each cold queue computation starts another sweep thread** — `app/index.py:813-818`

The per-request list is capped at 30, but there is no global pending set or worker bound, so concurrent queue/home requests duplicate the same upstream lookups.

```python
missing = [c['viewkey'] for c in tonight if c['viewkey'] not in CLIPS]
...
sweep = list(dict.fromkeys(missing + to_sweep))[:30]
if sweep:
    threading.Thread(target=lambda: [clip_info(v) for v in sweep], daemon=True).start()
```

**Repeated MLT-on requests spawn independent webhook threads** — `app/store.py:515-542`

An already-open request is refreshed and delivered again; each accepted POST creates a raw thread whose outbound call may block for eight seconds.

```python
if on:
    if idx:
        e = data[idx[0]]; e.update(f); e['updated_at'] = now.isoformat(timespec='seconds')
        if not refresh_only:
            wake = _write_mlt_wake(vk, f, now)
...
if wake is not None:
    threading.Thread(target=_mlt_webhook, args=(wake,), daemon=True).start()
```

#### Validation

Confirmed three admitted-client paths can create threads faster than the work completes, with no global thread or queue bound.

Validation method: Independent static concurrency-path validation

- **Status:** validated
- **Disposition:** report

**Every pending preview request starts a raw thread** — `app/api.py:310-323`

fetch=False does not expose an in-flight state, so every duplicate request observes 'not fetched yet' and schedules another worker.

```python
@app.get('/api/preview')
def api_preview(request: Request):
    ...
    r = dict(core.preview_for(vk, fetch=False))
    if r.get('kind') is None and r.get('why') == 'not fetched yet':
        core.prefetch_preview(vk)
        r = {'kind': None, 'why': 'preview is being fetched', 'pending': True}
```

**Duplicate threads are created before per-key serialization** — `app/clips.py:439-485`

The per-viewkey lock is acquired inside the spawned worker, so duplicate requests accumulate blocked operating-system threads rather than coalescing.

```python
with prev_lock:
    rec = PREV.get(vk); vl = prev_vk_locks.setdefault(vk, threading.Lock())
...
with vl:
    ...

def prefetch_preview(vk):
    threading.Thread(target=lambda: preview_for(vk), daemon=True).start()
```

**Each cold queue computation starts another sweep thread** — `app/index.py:813-818`

The per-request list is capped at 30, but there is no global pending set or worker bound, so concurrent queue/home requests duplicate the same upstream lookups.

```python
missing = [c['viewkey'] for c in tonight if c['viewkey'] not in CLIPS]
...
sweep = list(dict.fromkeys(missing + to_sweep))[:30]
if sweep:
    threading.Thread(target=lambda: [clip_info(v) for v in sweep], daemon=True).start()
```

**Repeated MLT-on requests spawn independent webhook threads** — `app/store.py:515-542`

An already-open request is refreshed and delivered again; each accepted POST creates a raw thread whose outbound call may block for eight seconds.

```python
if on:
    if idx:
        e = data[idx[0]]; e.update(f); e['updated_at'] = now.isoformat(timespec='seconds')
        if not refresh_only:
            wake = _write_mlt_wake(vk, f, now)
...
if wake is not None:
    threading.Thread(target=_mlt_webhook, args=(wake,), daemon=True).start()
```

Assertions:
- Repeated uncached previews each call prefetch_preview.
- The preview lock is acquired inside each spawned worker.
- Queue and home recompute the same cold sweep and start one raw thread per request.
- Repeated MLT-on POSTs for an existing open entry still produce a wake and webhook thread.

Counterevidence and remaining uncertainty:
- Preview and queue operations require known identifiers and admitted clients.
- Each queue sweep contains at most 30 clips.
- Preview data and clip data are cached after completion.
- The MLT webhook path is disabled unless both URL and key are configured.
- Full-video prewarming already uses a bounded executor and pending set, demonstrating a safe local pattern.

Limitations:
- Static review did not measure the host's thread limit or reverse-proxy throttling.

#### Dataflow

The canonical finding records the affected path at app/api.py:310-325, app/clips.py:431-485, app/api.py:249-262, app/index.py:813-818, app/api.py:610-624, app/store.py:515-542, but no expanded source-to-sink narrative was recorded.

Attack steps:
- Choose a known uncached clip or a cold queue, or use an enabled MLT webhook.
- Issue concurrent /api/preview or /api/queue requests, or repeated same-origin-style POST /api/mlt requests.
- Each request creates a daemon thread before pending work is coalesced.
- Workers wait on per-key locks or block on upstream calls while new threads continue to accumulate.
- Process scheduling, memory, sockets, or downstream request capacity is exhausted.

#### Reachability

All three sinks are directly reachable from documented HTTP routes after admission checks.

- **Attacker:** Admitted network client

- **Entry point:** GET /api/preview, GET /api/queue or /api/home, optional POST /api/mlt

- **Source:** Repeated requests while work is cold or pending

- **Sink:** threading.Thread creation and blocking upstream work

- **Outcome:** Service-wide resource exhaustion or authenticated downstream webhook flooding

Preconditions:
- Attacker can reach the application from ALLOWED_NETS.
- For preview/queue, relevant entries remain cold or in flight; for MLT, webhook URL/key are configured.
- Network admission; webhook configuration for the third variant.

Existing controls:
- Known-viewkey and request-origin controls limit who can trigger work but do not rate-limit admitted clients.
- Individual timeouts and cache completion eventually release workers but do not bound creation rate.

**Every pending preview request starts a raw thread** — `app/api.py:310-323`

fetch=False does not expose an in-flight state, so every duplicate request observes 'not fetched yet' and schedules another worker.

```python
@app.get('/api/preview')
def api_preview(request: Request):
    ...
    r = dict(core.preview_for(vk, fetch=False))
    if r.get('kind') is None and r.get('why') == 'not fetched yet':
        core.prefetch_preview(vk)
        r = {'kind': None, 'why': 'preview is being fetched', 'pending': True}
```

**Each cold queue computation starts another sweep thread** — `app/index.py:813-818`

The per-request list is capped at 30, but there is no global pending set or worker bound, so concurrent queue/home requests duplicate the same upstream lookups.

```python
missing = [c['viewkey'] for c in tonight if c['viewkey'] not in CLIPS]
...
sweep = list(dict.fromkeys(missing + to_sweep))[:30]
if sweep:
    threading.Thread(target=lambda: [clip_info(v) for v in sweep], daemon=True).start()
```

**Repeated MLT-on requests spawn independent webhook threads** — `app/store.py:515-542`

An already-open request is refreshed and delivered again; each accepted POST creates a raw thread whose outbound call may block for eight seconds.

```python
if on:
    if idx:
        e = data[idx[0]]; e.update(f); e['updated_at'] = now.isoformat(timespec='seconds')
        if not refresh_only:
            wake = _write_mlt_wake(vk, f, now)
...
if wake is not None:
    threading.Thread(target=_mlt_webhook, args=(wake,), daemon=True).start()
```

#### Severity

**Medium** — The attack is simple and can deny the whole service or flood an optional downstream integration, but the attacker must be admitted by the deployment's network authorization boundary and the webhook variant is configuration-dependent.

Raise if ALLOWED_NETS includes untrusted shared networks or the service is exposed without an authenticating proxy; lower if the deployment is provably single-user and request concurrency is constrained externally.

Impact assessment:
- **Level:** medium
- **Rationale:** Can deny the self-hosted service and, with webhook enabled, flood a credentialed downstream automation endpoint.

Likelihood assessment:
- **Level:** medium
- **Rationale:** Simple concurrency is sufficient once an attacker is inside an allowed network; caches and the private deployment reduce exposure.

#### Remediation

Route preview fetches, clip-info sweeps, and webhook deliveries through small bounded executors or queues with explicit capacity. Maintain per-viewkey pending Futures/single-flight state before worker creation, deduplicate queue sweep keys globally, coalesce repeated MLT wakes, and reject or drop work when capacity is full. Add per-client throttles and downstream delivery backoff/idempotency.

Tests:
- Hold a preview fetch open, issue hundreds of duplicate /api/preview requests, and assert only one worker exists for that viewkey and total worker count stays bounded.
- Issue concurrent cold /api/queue and /api/home requests and assert every viewkey is fetched once with a capped global queue.
- With a slow fake webhook, repeat POST /api/mlt and assert delivery is coalesced and thread count remains fixed.
- Fill the executor/queue and verify excess requests fail or degrade predictably without creating threads.

Preventive controls:
- Use the existing PREWARM_POOL plus pending-set pattern for all asynchronous media work.
- Centralize background-job capacity, metrics, and cancellation.
- Apply endpoint concurrency/rate limits at the application or authenticating reverse proxy.

<a id="finding-8"></a>

### [8] Namespaced clip metadata controls server-side media destinations without egress validation

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | The source URL, canonicalization, yt-dlp operand, extracted format acceptance, urllib relay, ffmpeg use, and direct-network fallback are explicit in source. Exact internal-service response disclosure remains target-dependent and is reflected in severity. |
| Category | Server-side request forgery through unvalidated integration metadata |
| CWE | CWE-918 |
| Affected lines | app/index.py:35-40, app/store.py:38-48, app/index.py:291-302, app/clips.py:577-629, app/clips.py:1213-1267, app/clips.py:927-956, app/clips.py:158-203, app/phnet.py:68-103 |

#### Summary

For xh: and ep: clips, recursively ingested source_url or url fields become the canonical page URL when they merely start with https://. The application passes that value to yt-dlp, accepts the extractor's HTTP(S) format and playlist URLs without host or address validation, and then fetches them through the full-video proxy, ffmpeg frame extraction, or persistent download path. A source-data writer can therefore induce server-side requests to unintended destinations; automatic preview prewarming supplies a trigger when a poisoned card is displayed.

#### Root Cause

Integration-supplied public display URLs and trusted server-fetch URLs share one field. A startswith('https://') check substitutes for a destination policy, and downstream extractor URLs are trusted solely by protocol.

**Namespaced source URLs are copied without destination validation** — `app/index.py:35-40`

Recursive source-file records can supply the server-fetch URL for namespaced identifiers.

```python
for src in ('reddit_post', 'reddit_sub', 'reddit_user', 'origin_url', 'origin_site',
            'xhamster_url', 'creator_url', 'uploader_url') + (('source', 'source_url', 'url') if ':' in vk else ()):
    v = o.get(src)
    if isinstance(v, str) and v.strip() and not m.get(src):
        m[src] = v.strip()
```

**The canonical URL control is only a string prefix check** — `app/store.py:38-48`

Arbitrary HTTPS hosts, ports, credentials, DNS results, and redirects are accepted. The xHamster rewrite changes recognized xHamster hosts but returns all other HTTPS URLs unchanged.

```python
def page_url_of(vk, meta=None):
    ...
    for k in ('source_url', 'url'):
        u = (meta or {}).get(k)
        if isinstance(u, str) and u.startswith('https://'):
            if site == 'xh': u = re.sub(..., f'https://{XH_HOST}/', u)
            return u
    ...
```

**yt-dlp receives the metadata URL and returned media URLs are accepted by protocol only** — `app/clips.py:592-623`

Neither the initial target nor extracted progressive/HLS destinations are constrained to expected public site/CDN hosts or public IP ranges.

```python
src = ph_page_url(vk)
p = subprocess.run([_sys.executable, '-m', 'yt_dlp', '-j', '--no-playlist', '--no-warnings', *phnet.ytdlp_args(src),
                    '--socket-timeout', '15', '-f', ..., phnet.route_url(src)], ...)
...
if f.get('url') and f.get('protocol') in ('https', 'http') and isinstance(h, int) ...:
    fmts[h] = {'url': f['url'], 'headers': ..., 'height': h}
```

**Non-allowlisted hosts bypass the optional proxy and use direct urllib** — `app/phnet.py:68-88`

The host suffix list chooses proxy routing; it is not an egress allowlist, so arbitrary metadata-selected hosts are fetched directly.

```python
def active_proxy(url=None):
    if url is not None and not is_walled_url(url): return None
    return proxy_url() if proxy_alive() else None
...
def urlopen(req, timeout=20, **kw):
    url = req.full_url if isinstance(req, urllib.request.Request) else req
    p = active_proxy(url)
    if not p: return urllib.request.urlopen(req, timeout=timeout, **kw)
```

#### Validation

Confirmed a source writer controls the initial yt-dlp target and indirectly controls unchecked URLs later consumed by urllib and ffmpeg.

Validation method: Independent static source-to-multiple-sinks validation

- **Status:** validated
- **Disposition:** report

**Namespaced source URLs are copied without destination validation** — `app/index.py:35-40`

Recursive source-file records can supply the server-fetch URL for namespaced identifiers.

```python
for src in ('reddit_post', 'reddit_sub', 'reddit_user', 'origin_url', 'origin_site',
            'xhamster_url', 'creator_url', 'uploader_url') + (('source', 'source_url', 'url') if ':' in vk else ()):
    v = o.get(src)
    if isinstance(v, str) and v.strip() and not m.get(src):
        m[src] = v.strip()
```

**The canonical URL control is only a string prefix check** — `app/store.py:38-48`

Arbitrary HTTPS hosts, ports, credentials, DNS results, and redirects are accepted. The xHamster rewrite changes recognized xHamster hosts but returns all other HTTPS URLs unchanged.

```python
def page_url_of(vk, meta=None):
    ...
    for k in ('source_url', 'url'):
        u = (meta or {}).get(k)
        if isinstance(u, str) and u.startswith('https://'):
            if site == 'xh': u = re.sub(..., f'https://{XH_HOST}/', u)
            return u
    ...
```

**yt-dlp receives the metadata URL and returned media URLs are accepted by protocol only** — `app/clips.py:592-623`

Neither the initial target nor extracted progressive/HLS destinations are constrained to expected public site/CDN hosts or public IP ranges.

```python
src = ph_page_url(vk)
p = subprocess.run([_sys.executable, '-m', 'yt_dlp', '-j', '--no-playlist', '--no-warnings', *phnet.ytdlp_args(src),
                    '--socket-timeout', '15', '-f', ..., phnet.route_url(src)], ...)
...
if f.get('url') and f.get('protocol') in ('https', 'http') and isinstance(h, int) ...:
    fmts[h] = {'url': f['url'], 'headers': ..., 'height': h}
```

**Extracted URL is opened and relayed to the client** — `app/clips.py:1221-1237`

The media proxy performs a server-side GET to the unchecked extracted URL and later streams its body.

```python
rec = resolve_full(vk, force=attempt > 0, fg=True)
f = pick_format(rec, q) if rec.get('url') else None
...
req = urllib.request.Request(f['url'], headers=hdr, method='GET')
try:
    up = phnet.urlopen(req, timeout=20)
```

**Non-allowlisted hosts bypass the optional proxy and use direct urllib** — `app/phnet.py:68-88`

The host suffix list chooses proxy routing; it is not an egress allowlist, so arbitrary metadata-selected hosts are fetched directly.

```python
def active_proxy(url=None):
    if url is not None and not is_walled_url(url): return None
    return proxy_url() if proxy_alive() else None
...
def urlopen(req, timeout=20, **kw):
    url = req.full_url if isinstance(req, urllib.request.Request) else req
    p = active_proxy(url)
    if not p: return urllib.request.urlopen(req, timeout=timeout, **kw)
```

Assertions:
- A known namespaced clip can acquire source_url/url from recursively scanned JSON.
- page_url_of returns arbitrary HTTPS metadata URLs.
- resolve_full passes that URL to yt-dlp.
- HTTP(S) format and HLS URLs are accepted without hostname/address validation.
- full streaming, frame extraction, and downloads consume the resulting locations.

Counterevidence and remaining uncertainty:
- Bare Pornhub IDs use a fixed canonical URL.
- The identifier must be syntactically valid and known to the index.
- Initial metadata text must start with https://.
- Useful response disclosure generally requires yt-dlp or ffmpeg to recognize attacker-selected content.
- HTTP triggers remain inside the admitted-client boundary.

Limitations:
- Redirect behavior and service-specific response extraction were not executed; concrete exfiltration depends on the targeted protocol/content.

#### Dataflow

The canonical finding records the affected path at app/index.py:35-40, app/store.py:38-48, app/index.py:291-302, app/clips.py:577-629, app/clips.py:1213-1267, app/clips.py:927-956, app/clips.py:158-203, app/phnet.py:68-103, but no expanded source-to-sink narrative was recorded.

Attack steps:
- Write an xh:/ep: source record for a known clip with attacker-selected HTTPS source_url/url.
- The index copies the URL and page_url_of adopts it as canonical.
- Displaying the card automatically calls /api/preview with cur=1, or an admitted client requests full media, frames, or download.
- yt-dlp requests the metadata URL and emits media or playlist URLs.
- urllib, ffmpeg, or the downloader accesses those unchecked destinations; the full proxy can relay decodable bytes.

#### Reachability

The source-to-sink path is complete for namespaced clips in supported multi-site ingestion.

- **Attacker:** Source/scout metadata writer

- **Entry point:** JSON under SOURCES_DIR or shared pool metadata

- **Source:** source_url/url

- **Sink:** yt-dlp, urllib full stream, ffmpeg frame extraction, and yt-dlp download

- **Outcome:** Server-side requests to unintended network destinations with conditional response relay or resource consumption

Preconditions:
- Attacker can influence a scanned source/scout record for a known namespaced clip.
- A user displays the poisoned current card or an admitted client requests a media operation.
- Known xh:/ep: clip and media trigger.

Existing controls:
- Known-viewkey validation and fixed Pornhub URLs narrow scope.
- yt-dlp and ffmpeg timeouts and concurrency limits reduce but do not prevent unintended egress.
- The image proxy has a separate host allowlist and is not part of this finding.

**Namespaced source URLs are copied without destination validation** — `app/index.py:35-40`

Recursive source-file records can supply the server-fetch URL for namespaced identifiers.

```python
for src in ('reddit_post', 'reddit_sub', 'reddit_user', 'origin_url', 'origin_site',
            'xhamster_url', 'creator_url', 'uploader_url') + (('source', 'source_url', 'url') if ':' in vk else ()):
    v = o.get(src)
    if isinstance(v, str) and v.strip() and not m.get(src):
        m[src] = v.strip()
```

**The canonical URL control is only a string prefix check** — `app/store.py:38-48`

Arbitrary HTTPS hosts, ports, credentials, DNS results, and redirects are accepted. The xHamster rewrite changes recognized xHamster hosts but returns all other HTTPS URLs unchanged.

```python
def page_url_of(vk, meta=None):
    ...
    for k in ('source_url', 'url'):
        u = (meta or {}).get(k)
        if isinstance(u, str) and u.startswith('https://'):
            if site == 'xh': u = re.sub(..., f'https://{XH_HOST}/', u)
            return u
    ...
```

**yt-dlp receives the metadata URL and returned media URLs are accepted by protocol only** — `app/clips.py:592-623`

Neither the initial target nor extracted progressive/HLS destinations are constrained to expected public site/CDN hosts or public IP ranges.

```python
src = ph_page_url(vk)
p = subprocess.run([_sys.executable, '-m', 'yt_dlp', '-j', '--no-playlist', '--no-warnings', *phnet.ytdlp_args(src),
                    '--socket-timeout', '15', '-f', ..., phnet.route_url(src)], ...)
...
if f.get('url') and f.get('protocol') in ('https', 'http') and isinstance(h, int) ...:
    fmts[h] = {'url': f['url'], 'headers': ..., 'height': h}
```

**Extracted URL is opened and relayed to the client** — `app/clips.py:1221-1237`

The media proxy performs a server-side GET to the unchecked extracted URL and later streams its body.

```python
rec = resolve_full(vk, force=attempt > 0, fg=True)
f = pick_format(rec, q) if rec.get('url') else None
...
req = urllib.request.Request(f['url'], headers=hdr, method='GET')
try:
    up = phnet.urlopen(req, timeout=20)
```

#### Severity

**Medium** — The vulnerable server-side egress can reach services unavailable to the source writer and can relay decodable media or consume bandwidth/storage, but exploitation requires write influence over local integration data for a known namespaced clip and useful disclosure depends on yt-dlp/media handling.

Raise if a less-trusted remote scout writes source metadata directly or the host can reach sensitive unauthenticated management services; lower if all source files are generated only by the fully trusted operator and outbound network policy blocks private destinations.

Impact assessment:
- **Level:** medium
- **Rationale:** Can probe or interact with internal services and conditionally expose media-like responses or consume server bandwidth/storage.

Likelihood assessment:
- **Level:** medium
- **Rationale:** Requires a compromised or malicious local integration writer but the trigger occurs automatically for displayed cards and through documented operations.

#### Remediation

Separate display links from trusted server-fetch targets. Construct fetch URLs from validated site plus native identifier wherever possible. When a slug is required, parse the URL and require the exact documented site host or properly delimited site suffix and expected port. Resolve DNS and reject loopback, link-local, private, multicast, and other non-public addresses; repeat validation after every redirect and for every extractor-returned playlist, segment, or format URL. Add network-level egress controls as defense in depth, and rebuild cached indexes after deployment.

Tests:
- Ingest xh:/ep: records whose URLs target loopback, link-local, RFC1918, IPv6 local, credentials-in-authority, alternate ports, and deceptive suffixes; assert media operations reject them before network access.
- Test an allowed public source that redirects to a private address; assert every redirect hop is revalidated.
- Return an allowed extraction page with a private progressive URL and HLS playlist/segment; assert urllib and ffmpeg never receive them.
- Confirm canonical xHamster and Eporner hosts continue to resolve and legitimate CDN formats remain usable.

Preventive controls:
- Use a single reusable egress-policy component for urllib, yt-dlp results, redirects, ffmpeg, and downloads.
- Keep browser navigation URLs distinct from server-fetch URLs in the data model.
- Apply an outbound firewall that blocks metadata and private address ranges for the application account/container.

<a id="finding-9"></a>

### [9] Synthetic range fallback amplifies a one-byte request into up to 32 MiB of upstream reads

| Field | Value |
| --- | --- |
| Severity | low |
| Confidence | high |
| Confidence rationale | The inclusive 32 MiB threshold and discard loop are explicit. Upstream-specific prevalence is unknown and reflected in attack complexity and severity. |
| Category | Asymmetric server-side bandwidth amplification |
| CWE | CWE-400 |
| Affected lines | app/clips.py:1213-1267, app/clips.py:1269-1279, app/api.py:405-414 |

#### Summary

When an upstream media server ignores Range and returns 200 with a numeric Content-Length, full_plan synthesizes a 206 response for offsets through 32 MiB. full_chunks implements this by reading and discarding every byte before the requested offset. An admitted client can repeatedly request one byte near the threshold and force roughly 32 MiB of hidden server-side bandwidth and blocking work per response.

#### Root Cause

The server emulates unsupported upstream seeking by linear prefix discard without accounting for the ratio between downstream response bytes and upstream work.

**A 206 is synthesized through a 32 MiB offset** — `app/clips.py:1245-1261`

A request such as bytes=33554432-33554432 produces a one-byte downstream response while setting skip to 32 MiB.

```python
elif rng and up.status == 200:
    # still ignored: serve the asked range ourselves when it is cheap, else an honest full 200
    start = int(mr.group(1)) if mr else None
    end = int(mr.group(2)) if mr and mr.group(2) else (total - 1 if total else None)
    if mr and total and start < total and start <= 32 * 1024 * 1024:
        end = min(end, total - 1)
        status, skip, limit = 206, start, end - start + 1
```

**The server reads and discards the complete prefix** — `app/clips.py:1269-1279`

Discarded upstream bytes are not charged to a global or per-client budget, and the client receives only limit bytes after the skip.

```python
def full_chunks(up, skip, limit):
    while skip > 0:
        chunk = up.read(min(65536, skip))
        if not chunk: return
        skip -= len(chunk)
    while limit is None or limit > 0:
        chunk = up.read(65536 if limit is None else min(65536, limit))
        if not chunk: break
        yield chunk
```

**The raw streaming route exposes the skip behavior** — `app/api.py:405-414`

An admitted HTTP client's Range header directly determines the hidden skip work performed during streaming.

```python
def _full(req, vk):
    plan = core.full_plan(vk, q1(req, 'q', None), req.headers.get('Range'))
    ...
    _, status, hdrs, up, skip, limit = plan
    def chunks():
        try: yield from core.full_chunks(up, skip, limit)
    ...
    return RawStream(status, hdrs, chunks(), on_close=up.close)
```

#### Validation

Confirmed an inclusive 32 MiB skip for a one-byte downstream range and no stream-wide byte/concurrency budget.

Validation method: Independent static arithmetic and source-to-sink validation

- **Status:** validated
- **Disposition:** report

**A 206 is synthesized through a 32 MiB offset** — `app/clips.py:1245-1261`

A request such as bytes=33554432-33554432 produces a one-byte downstream response while setting skip to 32 MiB.

```python
elif rng and up.status == 200:
    # still ignored: serve the asked range ourselves when it is cheap, else an honest full 200
    start = int(mr.group(1)) if mr else None
    end = int(mr.group(2)) if mr and mr.group(2) else (total - 1 if total else None)
    if mr and total and start < total and start <= 32 * 1024 * 1024:
        end = min(end, total - 1)
        status, skip, limit = 206, start, end - start + 1
```

**The server reads and discards the complete prefix** — `app/clips.py:1269-1279`

Discarded upstream bytes are not charged to a global or per-client budget, and the client receives only limit bytes after the skip.

```python
def full_chunks(up, skip, limit):
    while skip > 0:
        chunk = up.read(min(65536, skip))
        if not chunk: return
        skip -= len(chunk)
    while limit is None or limit > 0:
        chunk = up.read(65536 if limit is None else min(65536, limit))
        if not chunk: break
        yield chunk
```

**The raw streaming route exposes the skip behavior** — `app/api.py:405-414`

An admitted HTTP client's Range header directly determines the hidden skip work performed during streaming.

```python
def _full(req, vk):
    plan = core.full_plan(vk, q1(req, 'q', None), req.headers.get('Range'))
    ...
    _, status, hdrs, up, skip, limit = plan
    def chunks():
        try: yield from core.full_chunks(up, skip, limit)
    ...
    return RawStream(status, hdrs, chunks(), on_close=up.close)
```

Assertions:
- A valid bytes=start-end header controls start and end.
- An upstream 200 with numeric length enters the synthetic 206 branch.
- start equal to 32 MiB is accepted.
- full_chunks reads start bytes before yielding limit bytes.

Counterevidence and remaining uncertainty:
- The route requires an admitted client and known clip.
- The condition applies only when the upstream ignores Range.
- Offsets above 32 MiB receive an honest full 200 rather than synthetic 206.
- Per-read chunks are 64 KiB and individual network calls have timeouts.

Limitations:
- Static review did not measure how often production upstreams ignore Range or external bandwidth limits.

#### Dataflow

The canonical finding records the affected path at app/clips.py:1213-1267, app/clips.py:1269-1279, app/api.py:405-414, but no expanded source-to-sink narrative was recorded.

Attack steps:
- Choose a known clip whose selected upstream format returns 200 to Range requests with Content-Length.
- Request Range: bytes=33554432-33554432 on /media/full/\<viewkey\>.mp4.
- After one fresh-URL retry, full_plan returns a synthetic one-byte 206 plan with skip=33554432.
- full_chunks downloads and discards 32 MiB before yielding one byte.
- Repeat concurrently to consume server egress/ingress bandwidth and blocking stream capacity.

#### Reachability

The client's standard Range header reaches the fallback arithmetic and discard loop.

- **Attacker:** Admitted network client

- **Entry point:** GET /media/full/\<known\>.mp4 with Range

- **Source:** Range start/end

- **Sink:** full_chunks prefix discard

- **Outcome:** Large hidden upstream read for tiny downstream response

Preconditions:
- Admitted client access.
- Known clip and upstream that ignores Range with a length greater than the chosen offset.
- Upstream 200 response to Range.

Existing controls:
- 32 MiB bounds individual hidden prefix work.
- Cross-site read guards block ordinary foreign web pages but not admitted direct clients.

**A 206 is synthesized through a 32 MiB offset** — `app/clips.py:1245-1261`

A request such as bytes=33554432-33554432 produces a one-byte downstream response while setting skip to 32 MiB.

```python
elif rng and up.status == 200:
    # still ignored: serve the asked range ourselves when it is cheap, else an honest full 200
    start = int(mr.group(1)) if mr else None
    end = int(mr.group(2)) if mr and mr.group(2) else (total - 1 if total else None)
    if mr and total and start < total and start <= 32 * 1024 * 1024:
        end = min(end, total - 1)
        status, skip, limit = 206, start, end - start + 1
```

**The server reads and discards the complete prefix** — `app/clips.py:1269-1279`

Discarded upstream bytes are not charged to a global or per-client budget, and the client receives only limit bytes after the skip.

```python
def full_chunks(up, skip, limit):
    while skip > 0:
        chunk = up.read(min(65536, skip))
        if not chunk: return
        skip -= len(chunk)
    while limit is None or limit > 0:
        chunk = up.read(65536 if limit is None else min(65536, limit))
        if not chunk: break
        yield chunk
```

**The raw streaming route exposes the skip behavior** — `app/api.py:405-414`

An admitted HTTP client's Range header directly determines the hidden skip work performed during streaming.

```python
def _full(req, vk):
    plan = core.full_plan(vk, q1(req, 'q', None), req.headers.get('Range'))
    ...
    _, status, hdrs, up, skip, limit = plan
    def chunks():
        try: yield from core.full_chunks(up, skip, limit)
    ...
    return RawStream(status, hdrs, chunks(), on_close=up.close)
```

#### Severity

**Low** — The amplification factor is large and streaming lacks a global byte budget, but exploitation requires an admitted client and a known upstream that ignores Range and supplies a numeric length. Impact is primarily bandwidth and worker degradation on a self-hosted service.

Raise if many supported upstreams ignore Range or the route is exposed to untrusted networks; lower if a reverse proxy globally limits concurrent streams and upstream bytes.

Impact assessment:
- **Level:** low
- **Rationale:** Consumes bandwidth and streaming worker time but does not cross confidentiality or integrity boundaries.

Likelihood assessment:
- **Level:** medium
- **Rationale:** The request is trivial once a compatible upstream is known, but compatibility and private network admission are prerequisites.

#### Remediation

Do not synthesize nonzero ranges by linear discard. If upstream ignores a range, return an honest 200, reject the seek with 416/502, or use a bounded local cache designed for random access. If a small fallback is retained, reduce the threshold substantially and enforce global/per-client concurrent-stream and total-upstream-byte budgets based on bytes consumed, not bytes returned.

Tests:
- Use a fake upstream that ignores Range; request one byte at the maximum allowed offset and assert the server does not read the prefix.
- Verify unsupported nonzero ranges return the documented honest response without hidden discard.
- Issue concurrent range requests and assert a global stream/byte budget caps upstream consumption.
- Confirm normal upstream 206 pass-through remains seekable and preserves Content-Range.

Preventive controls:
- Account for upstream bytes separately from downstream response size.
- Bound concurrent full-media streams per client and globally.
- Prefer upstream-supported ranges or local random-access storage over discard-based emulation.

<a id="finding-10"></a>

### [10] An unvalidated clip identifier pollutes Object.prototype in browser watch state

| Field | Value |
| --- | --- |
| Severity | low |
| Confidence | high |
| Confidence rationale | The special __proto__ lookup semantics, unvalidated source identifier, reachable fallback call, and fixed prototype writes are all explicit in source. |
| Category | Client-side prototype pollution through an unvalidated object key |
| CWE | CWE-1321 |
| Affected lines | app/index.py:3-7, app/index.py:804-820, web/index.html:1738-1756, web/index.html:1849-1857, web/index.html:3050-3067 |

#### Summary

The index admits arbitrary string identifiers from already-sent.json, including __proto__, and returns them in queue items. When an invalid item reaches the normal media fallback, wpMark uses the identifier as a key in a plain object. WATCH\['__proto__'\] resolves Object.prototype, so the fixed watch-state fields are written to the global prototype and inherited by unrelated clip entries.

#### Root Cause

Server-side ingestion does not enforce its canonical clip identifier grammar, and client-side per-key state uses a prototype-bearing object without own-property checks.

**already-sent accepts every string as a clip key** — `app/index.py:3-7`

The canonical VK_RE exists but is not applied here, so __proto__ can become an indexed viewkey.

```python
def build_index():
    sent = load_json(SENT, []) or []
    sent = [k for k in sent if isinstance(k, str)]
    S = set(sent)
    meta = {k: {} for k in sent}
```

**Indexed picks are emitted without a validity filter** — `app/index.py:804-820`

An invalid string identifier can reach the browser queue even though mutation APIs later reject it.

```python
backlog_all = [p for p in INDEX['picks'] if p['viewkey'] not in tset and not voted(p['viewkey'])]
...
if mode == 'backlog':
    items = [card_of(p['viewkey'], None, cv, ms) for p in backlog_all]
```

**Plain object indexing reuses Object.prototype** — `web/index.html:3050-3067`

For vk='__proto__', w is Object.prototype; writes to w.e and w.t pollute every ordinary object and make unrelated absent watch records inherit the opened flag.

```html
const WATCH = LS.get('watch', {}) || {};
...
function wpMark(vk, k, v){
  if (!vk) return; const w = WATCH[vk] || (WATCH[vk] = {});
  if (k === 'f'){ ... } else { if (w[k]) return; w[k] = 1; }
  w.t = Date.now();
}
...
function wpBadge(vk){
  const w = WATCH[vk]; if (!w) return '';
  ...
  if (w.e) return '<span class="wp" ...>↗ opened</span>';
```

#### Validation

Confirmed the __proto__ key reaches a fixed Object.prototype write in the normal invalid-media fallback.

Validation method: Independent static data-flow validation

- **Status:** validated
- **Disposition:** report

**already-sent accepts every string as a clip key** — `app/index.py:3-7`

The canonical VK_RE exists but is not applied here, so __proto__ can become an indexed viewkey.

```python
def build_index():
    sent = load_json(SENT, []) or []
    sent = [k for k in sent if isinstance(k, str)]
    S = set(sent)
    meta = {k: {} for k in sent}
```

**Media fallback records the source-derived identifier** — `web/index.html:1849-1856`

Invalid IDs cause preview/full API errors and ultimately reach this fallback, which passes viewkey to wpMark.

```html
function startEmbed(c, why){
  if (!navigator.onLine) return waitOnline(c);
  const it = c.item, m = mediaEl(c);
  clearMedia(c); c.media = 'embed'; wpMark(it.viewkey, 'e');
  const f = document.createElement('iframe');
  ...
  f.src = embedUrlOf(it);
```

**Plain object indexing reuses Object.prototype** — `web/index.html:3050-3067`

For vk='__proto__', w is Object.prototype; writes to w.e and w.t pollute every ordinary object and make unrelated absent watch records inherit the opened flag.

```html
const WATCH = LS.get('watch', {}) || {};
...
function wpMark(vk, k, v){
  if (!vk) return; const w = WATCH[vk] || (WATCH[vk] = {});
  if (k === 'f'){ ... } else { if (w[k]) return; w[k] = 1; }
  w.t = Date.now();
}
...
function wpBadge(vk){
  const w = WATCH[vk]; if (!w) return '';
  ...
  if (w.e) return '<span class="wp" ...>↗ opened</span>';
```

Assertions:
- already-sent strings are not filtered by VK_RE.
- Backlog queue returns every indexed pick.
- Invalid identifiers fail preview/full requests and the online fallback calls startEmbed.
- WATCH is a normal object and WATCH\['__proto__'\] resolves Object.prototype.
- wpMark writes fixed e and t properties to the returned object.

Counterevidence and remaining uncertainty:
- Server write APIs require VK_RE and known_vk, preventing vote-state corruption with this identifier.
- Only fixed watch fields can be written.
- The pollution is in-memory; JSON.stringify does not persist inherited fields.

Limitations:
- The impact of inherited e/t fields outside the watch UI depends on other browser code's property checks.

#### Dataflow

The canonical finding records the affected path at app/index.py:3-7, app/index.py:804-820, web/index.html:1738-1756, web/index.html:1849-1857, web/index.html:3050-3067, but no expanded source-to-sink narrative was recorded.

Attack steps:
- Write __proto__ as a string entry in already-sent.json.
- The server builds and returns it as a backlog queue card.
- The page tries preview/full routes, which reject the invalid ID, then falls back to startEmbed.
- wpMark('__proto__','e') resolves Object.prototype and writes e and t.
- Unrelated object-backed watch lookups inherit the polluted flag until reload.

#### Reachability

Reachable through the documented file-backed source ingestion and backlog UI.

- **Attacker:** Shared clip-list writer

- **Entry point:** shared/already-sent.json

- **Source:** viewkey string __proto__

- **Sink:** WATCH\[vk\] and writes in wpMark

- **Outcome:** Object.prototype receives e and t properties in the application page

Preconditions:
- Attacker can write the authoritative clip list or influence its producer.
- An admitted user displays the poisoned card while online.
- Poisoned card is displayed.

Existing controls:
- HTTP mutation routes reject the invalid ID, limiting impact to client rendering/state.
- The existing watched_test.js covers ordinary keys but not special object keys.

**already-sent accepts every string as a clip key** — `app/index.py:3-7`

The canonical VK_RE exists but is not applied here, so __proto__ can become an indexed viewkey.

```python
def build_index():
    sent = load_json(SENT, []) or []
    sent = [k for k in sent if isinstance(k, str)]
    S = set(sent)
    meta = {k: {} for k in sent}
```

**Indexed picks are emitted without a validity filter** — `app/index.py:804-820`

An invalid string identifier can reach the browser queue even though mutation APIs later reject it.

```python
backlog_all = [p for p in INDEX['picks'] if p['viewkey'] not in tset and not voted(p['viewkey'])]
...
if mode == 'backlog':
    items = [card_of(p['viewkey'], None, cv, ms) for p in backlog_all]
```

**Plain object indexing reuses Object.prototype** — `web/index.html:3050-3067`

For vk='__proto__', w is Object.prototype; writes to w.e and w.t pollute every ordinary object and make unrelated absent watch records inherit the opened flag.

```html
const WATCH = LS.get('watch', {}) || {};
...
function wpMark(vk, k, v){
  if (!vk) return; const w = WATCH[vk] || (WATCH[vk] = {});
  if (k === 'f'){ ... } else { if (w[k]) return; w[k] = 1; }
  w.t = Date.now();
}
...
function wpBadge(vk){
  const w = WATCH[vk]; if (!w) return '';
  ...
  if (w.e) return '<span class="wp" ...>↗ opened</span>';
```

#### Severity

**Low** — The source writer must already control a local shared input and the resulting pollution is limited to fixed client-side watch fields for the lifetime of the page. It can corrupt UI state and destabilize property checks but does not provide arbitrary script execution or server-side state corruption.

Raise if future wpMark call sites permit attacker-selected property names or polluted fields become security decisions; lower to informational if all source writers are fully trusted operators.

Impact assessment:
- **Level:** low
- **Rationale:** Corrupts client-side watch state and may perturb property checks, without proven code execution or server-side mutation.

Likelihood assessment:
- **Level:** medium
- **Rationale:** The trigger is deterministic once a writable source contains the special key, but that writer is a local integration boundary.

#### Remediation

Validate every clip identifier with VK_RE while ingesting already-sent and sent-log data, dropping and logging invalid values before index construction. Independently represent WATCH with Map or Object.create(null), validate the parsed localStorage object, and use Object.hasOwn before reusing a keyed record.

Tests:
- Add __proto__, constructor, and prototype entries to an ingestion fixture; assert none appear in /api/queue.
- Call wpMark('__proto__','e') in watched_test.js and assert Object.prototype has no new e or t property.
- Verify an unrelated identifier receives no inherited badge after malicious-key input.
- Exercise valid bare, xh:, and ep: identifiers to prevent an over-restrictive fix.

Preventive controls:
- Centralize canonical identifier validation at every file-ingestion boundary.
- Use Map or prototype-free dictionaries for attacker-influenced keys.
- Include JavaScript special-property keys in keyed-state regression tests.

## Reviewed Surfaces

| Surface | Risk Area | Outcome | Notes |
| --- | --- | --- | --- |
| Candidate disposition: cand-http-xh-url-xss | Validated candidate disposition | Rejected | The href sink is target=_blank with rel=noopener noreferrer; source evidence does not establish script execution in the application document or origin. |
| Candidate disposition: cand-http-mlt-url-xss | Validated candidate disposition | Rejected | The href sink is target=_blank with rel=noopener; source evidence does not establish script execution in the application document or origin. |
| Candidate disposition: cand-baseline-url-xss | Validated candidate disposition | Rejected | Duplicate of the two URL-scheme candidates; both affected anchors create disowned targets and do not establish application-origin script execution. |
| Candidate disposition: cand-ingestion-mlt-url-xss | Validated candidate disposition | Rejected | Duplicate of cand-http-mlt-url-xss; target=_blank plus noopener defeats the claimed same-origin execution path. |
| Candidate disposition: cand-ingestion-xh-url-xss | Validated candidate disposition | Rejected | Duplicate of cand-http-xh-url-xss; target=_blank plus noopener/noreferrer defeats the claimed same-origin execution path. |
| Candidate disposition: cand-baseline-docker-unauth | Validated candidate disposition | Rejected | Docker binds loopback by default, and the documented remote mode intentionally authorizes configured network peers; README.md:169-174 warns against unauthenticated Internet exposure. |
| Candidate disposition: cand-egress-full-ssrf | Duplicate candidate reconciliation | Rejected | This candidate is a duplicate operation of the validated source-metadata SSRF root control and is merged into that finding rather than counted separately. |
| Candidate disposition: cand-egress-download-ssrf | Duplicate candidate reconciliation | Rejected | The unchecked-destination dataflow is merged into the validated source-metadata SSRF finding. Its separate no-quota observation does not establish a distinct authority gain: downloads are an admitted-client operation with one global active-job cap, so it remains hardening guidance. |
| Candidate disposition: cand-egress-ffmpeg-ssrf | Duplicate candidate reconciliation | Rejected | This candidate is a duplicate operation of the validated source-metadata SSRF root control and is merged into that finding rather than counted separately. |
| Candidate disposition: cand-baseline-ssrf | Duplicate candidate reconciliation | Rejected | This candidate is a duplicate operation of the validated source-metadata SSRF root control and is merged into that finding rather than counted separately. |
| Candidate disposition: cand-egress-preview-thread-dos | Duplicate candidate reconciliation | Rejected | This operation shares the validated unbounded request-triggered raw-thread control failure and bounded-executor remediation; it is merged into that finding rather than counted separately. |
| Candidate disposition: cand-http-queue-thread-dos | Duplicate candidate reconciliation | Rejected | This operation shares the validated unbounded request-triggered raw-thread control failure and bounded-executor remediation; it is merged into that finding rather than counted separately. |
| Candidate disposition: cand-egress-mlt-thread-dos | Duplicate candidate reconciliation | Rejected | This operation shares the validated unbounded request-triggered raw-thread control failure and bounded-executor remediation; it is merged into that finding rather than counted separately. |
| Candidate disposition: cand-deploy-state-restore | Duplicate candidate reconciliation | Rejected | The untrusted restore direction shares the same box-user-writable Tailscale persistence boundary and root-owned-storage remediation as the validated node-state exposure finding. It is merged there rather than counted separately; no archive-based root code execution was established. |
| Candidate disposition: cand-ingestion-dated-name-dos | Duplicate candidate reconciliation | Rejected | The malformed-filename rebuild abort is one manifestation of the validated source-scanner validation and per-file isolation failure and is merged into that finding rather than counted separately. |
| Candidate disposition: cand-ingestion-html-name-dos | Duplicate candidate reconciliation | Rejected | The malformed-filename rebuild abort is one manifestation of the validated source-scanner validation and per-file isolation failure and is merged into that finding rather than counted separately. |
| Architecture and trust-boundary mapping | Repository-wide architecture | No issue found | Completed architecture and trust-boundary mapping across all 63 tracked files. The resulting threat model was reconciled against every validated finding; no separate architecture-only vulnerability remained. |
| HTTP guards, browser rendering, and request-triggered work | HTTP/browser boundary | Reported | Completed HTTP guards, browser rendering, request inputs, API mutations, and request-triggered work. Findings cover stored markup injection, prototype pollution, and unbounded request-created threads; URL-link candidates were rejected after sink analysis. |
| Filesystem, outbound network, media, and credential sinks | Filesystem and egress | Reported | Completed filesystem boundaries, outbound HTTP, yt-dlp/ffmpeg use, media relay, downloads, and credential operations. Findings cover metadata-controlled server egress, synthetic range amplification, and shared thread-exhaustion paths. |
| Deployment scripts and Tailscale recovery | Privileged recovery and secrets | Reported | Completed deployment, container, reset-recovery, environment, and Tailscale review. Findings cover privileged installation of a user-writable cached package and downgrade of root node state into user-owned persistent storage. |
| Source ingestion, deserialization, duplicate suppression, and cache rebuilds | Data ingestion and stored metadata | Reported | Completed source ingestion, JSON/HTML parsing, cache rebuild, duplicate suppression, and shared-data contract review. Findings cover unbounded source scans, quadratic duplicate matching, and the ingestion side of metadata-controlled server egress. |
| Unvalidated integration URLs in target=_blank links | Browser URL-context injection | Rejected | Source review confirms MLT_POOL and XH_FAVORITES URLs reach href without scheme validation, but every affected anchor uses target=_blank with noopener (and the xHamster links also use noreferrer). A disowned newly created target does not have the application document's origin, so the candidates did not establish execution in the private application origin. Unsafe schemes should still be filtered as defense in depth. |
| Optional Docker remote publication without per-user login | Deployment authentication boundary | Rejected | The application deliberately authorizes clients by network and Host rather than per-user identity. Docker publishes loopback by default; README.md:169-174 explicitly requires loopback, VPN, or an authenticating reverse proxy and says not to expose the app to the internet. Setting PUBLISH_ADDR and ALLOWED_NETS intentionally admits those peers, so no control bypass was shown. |
| Remaining UI assets, test harnesses, and regression tests | Stored browser state and developer tooling | Reported | Completed remaining UI assets, test harness, regression tests, documentation, and packaging review. Validated the unsandboxed live-content browser-test finding; no additional distinct issue survived validation. |

## Open Questions And Follow Up

- What exact passwordless-sudo and scheduler policy is installed on the live reset-prone box?
- Which supported deployments permit less-trusted scout or digger automation to write configured source and shared-data directories?
- Which optional cookie and MLT webhook capabilities are enabled, and what external Tailscale ACLs constrain restored node identity?
