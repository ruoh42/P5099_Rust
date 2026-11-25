# Porting status

Rules and open work for the independent Rust port. Implementation history is
in git; this file keeps only what a contributor must know before changing code.

## State

The retained-capture surface is implemented and covered by automated tests:
transport/login/migration, rooms, races, ceremony, persistence, MyRoom, item
GameSlot relay, the launcher, accounts, the RCON console, and the Discord bot.
Automated coverage is **not** proof of stock-client compatibility; the live
gates below are still open.

Explicitly out of scope: Lucci world objects, bonus-item world objects, and
team flags (`GameSlot` types 4/5/6/7/8). Their codecs stay as diagnostic
boundaries only.

## Source policy

- The C# server is a read-only protocol and product-intent reference, not a
  bug-for-bug spec. Preserve client-visible wire behavior; never reproduce its
  data loss, races, spoofing, partial publication, or unsafe failure handling.
  A C# defect is a Rust requirement or a capture question, not a reason to
  edit C#.
- Captures, `PhysicsSim`, IDBs, decompiler output, client binaries, and RHO
  payloads must not be copied into this repository.
- Do not copy a dirty tree between the Rust and C# repositories.

## Non-negotiable invariants

- Keep workspace `unsafe_code = "forbid"`. Any real `unsafe` block is a
  stop-and-review condition.
- Do not make identity leases cloneable or reconstruct them from numeric
  session/generation fields.
- Preserve registry-instance validation at every admitted World-handle
  boundary.
- Freeze in the actor, wait for drain outside the actor, and acquire the
  profile lane only after drain.
- Actor/durable work must own a retained child lease independently of its
  request future.
- Keep migration ACK/peer queue permits and every transition reservation before
  the result-free commit boundary.
- Do not add a fallible send, allocation, serialization, or validation after
  the ordered ACK is published and before all migration state commits.
- Keep UDP source admission separate from recipient lookup. Freezing a source
  must not hide valid outbound recipients.
- Keep TCP GameSlot and the opaque UDP relay as separate codecs and policy
  domains. The TCP logical/blob caps must not constrain UDP relay bodies.
- Keep parsed TCP GameSlot commands move-only and parser-minted. Do not expose
  mutable action, audience, claimed-identity, or raw-wire fields.
- Reserve the complete exact-generation GameSlot audience before publishing
  any recipient. Queue-full drops are atomic and must not enter a retry queue.
- Do not replace pre-reserved completion permits with untracked spawned
  callbacks or best-effort `try_send`.
- Do not release a profile lane before World has revalidated and published the
  durable outcome.
- Reserve required outbound capacity before mutating state. Expected
  backpressure is a typed request error; impossible actor-state contradictions
  remain typed terminal errors.
- Keep global identity-operation admission outermost. After admission, parse
  before packet-specific mutation; any request that deliberately authorizes
  before parsing must document and test that error-priority boundary.
- Bounded mailboxes, collections, wire fields, and identity counts must remain
  bounded on all error paths.
- An unclassified packet is logged and consumed without a reply. A new packet
  family needs its own evidence and owning domain before it may mutate state
  or reply.

## Deliberate differences from C#

- Messenger does not treat one socket read as one packet, trust
  packet-supplied identity, or issue concurrent writes on one connection.
- Migration prepares capabilities first and crosses a result-free commit
  boundary instead of best-effort rollback.
- Room membership, team-booster state, MyRoom entry, and character-position
  fanout reserve every recipient queue before publishing, so there is no
  partial delivery window.
- UDP source admission is generation-, owner-, IP-, room-, and logical-epoch
  fenced. Movement tick ordering is not guessed.
- Rider-info lookup returns only a non-disclosing failure; the C#
  ten-byte-short projection must not be exposed.
- Visitors never receive stored room/item passwords. Protected owner-item
  reads need a one-shot item-password grant.
- Reenter is not aliased to random entry, and random entry cannot create a
  room for an arbitrary nickname.

## Client facts that are easy to get wrong

- `GameResult` human records are **208 bytes** for P5099 (reader
  `0x007115C0`), not the newer client's 217. The couple-ring marker `0xFF` is
  at byte 132 and the club logo at dword 170.
- Ceremony TX order must be `GameControl(state=4)`, `GameNextStage`, then
  `GameResult`.
- AI difficulty is not in the add/remove-AI request. `GrCommandStartPacket`
  carries six floats per AI: `spec[0] * spec[1]` is the base target speed,
  `spec[2]` the boost window (ms), `spec[3]` the boost acceleration multiplier;
  `spec[4]` and `spec[5]` are never read. Speed modes (game types 1/3) and item
  modes (2/4) use separate vectors.

## Open work

Live gates (these block a "port complete" claim):

- [ ] stock client launches on native Windows
- [ ] the same client launches through Wine, CrossOver, or Sikarugir
- [ ] two clients on two machines login, migrate, join a room, race, persist,
  and shut down; both return to the room after the ceremony
- [ ] Windows, macOS, and Linux CI pass

Capture-dependent:

- [ ] per-sender movement sequence, tick wrap, envelope fields, and fallback
- [ ] type-1/type-2 item award visible in the stock client, with the same
  sender-inclusive 73-byte reply reaching both clients
- [ ] multiplayer type-16 routing and remaining class-specific type-12
  source/target/effect meanings
- [ ] directional `PcGameRequestRelay` / `GameRelayBroadcasting` pairing
- [ ] production AI start and nonzero AI-master payloads
- [ ] held-slot/type-10 use and type-11 reaction side effects. Do not add them
  until fixtures prove both the state change and any extra wire packet.
- [ ] endpoint-report behavior under NAT before adding peer refresh

Design work:

- [ ] TCP-issued nonce/challenge for the first UDP bind (one-time, expiring,
  Game/P2P separated) without weakening existing fences
- [ ] race-wide multi-profile reward journal and crash recovery
- [ ] actor-owned club repository with atomic create/join/rename
- [ ] shop purchase, pricing, and currency transactions
- [ ] rider-info success path with field-level privacy rules
- [ ] nonempty MyRoom Career ownership, X-parts outside category 63, remaining
  kart tuning, and quest/attendance/progression
- [ ] actor-owned live race position and track-box spawn/collision authority
- [ ] password storage with zeroization and per-requester throttling

Evidence harness:

- [ ] C#-derived golden fixture for every supported serializer
- [ ] differential request/response harness for the supported flow
- [ ] an independent client-consumer oracle for every supported
  server-to-client serializer (currently 12 of 63 core consumers)

## Before every checkpoint

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --workspace --all-features -- --ignored
git diff --check
```
