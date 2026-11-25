# P5099 protocol-visible client FSM

The part of the Korean P5099 client's state machine that constrains a
compatible server. UI-only pages, events, store, and social features are not
modeled. The executable oracle is
`crates/p5099-client-oracle/src/protocol_fsm.rs` and has no dependency on the
production packet writers.

## State model

Transport and scene are orthogonal:

```text
Transport
  Disconnected -> AwaitingFirstMessage -> EncryptedUnauthenticated -> Authenticated

Scene
  Offline -> Login -> RiderBootstrap -> Menu
                                   Menu -> Migration -> Menu
                                   Menu -> RoomLobby
                                           -> Loading
                                           -> Racing
                                           -> Settling
                                           -> Ceremony
                                              AwaitingNextStage
                                              -> AwaitingResult
                                              -> Podium
                                              -> local final-stage scheduler
                                              -> RoomLobby (`GameReadyStage`)
```

Room leave escapes from any room phase back to Menu. A connection loss clears
the scene; only an established channel migration keeps its epoch across
reconnect.

## Server-owned stage exclusivity

One authenticated generation must never hold two server-owned memberships:

```text
ProtocolRoom  -- MyRoom entry ------------> MyRoom
ProtocolRoom  -- validated UI/solo entry --> No room membership
MyRoom        -- validated UI/solo entry --> No MyRoom membership
```

UI/solo entry covers Shop, Magic Hat, Club, Rider School, Time Attack,
Challenger, Scenario, Single Player, and Matching. Passive inventory, cash,
event-timer, and cosmetic queries are not transition markers. All peer
publications reserve outbound capacity before either membership changes.

## Login and migration

- The server sends `PcFirstMessage` first; then the encrypted stream starts.
- The complete inventory preload precedes `PrGetRider`, which enters the menu.
  Post-rider queries are unordered UI initialization, not FSM states.
- Account presets (pmap 0, 590, 718, 1798) are login attributes, not scenes.
- `PrChannelSwitch` creates a migration epoch: the client reconnects, consumes
  another first message, sends `PqChannelMovein`, and is authenticated again on
  `PrChannelMoveIn`. Mode 1 (local club UI) is a same-socket hand-off, not a
  migration.

## Room, loading, and race

- Successful create/join reply enters the room; rejection stays in the menu.
  `GrSessionDataPacket` + `GrSlotDataPacket` form the room snapshot.
- `GrCommandStartPacket` is self-contained (it embeds session, slot, track, and
  the AI specs) and enters Loading. **A standalone lobby snapshot after Loading
  begins is a crash hazard** and is rejected by the oracle.
- Loading: client sends `GameControl(0)`, normally does UDP time-sync, then the
  server sends `GameControl(1)`. UDP sync is not a client guard; the server
  times out and starts anyway.
- `PqStartCollectRecord` (hash only) may follow the start command. The reply is
  hash + one flag byte (1 when category-12 replay-camera equipment is set),
  accepted only in Loading/Racing/Settling, with no scene change.
  `PcReportUserCollectedRecord` and `PqReportGameCollectedRecord` are
  no-reply telemetry.

## Settlement and podium

```text
Loading
  -- server GameControl(1) --> Racing
  -- client GameControl(2) --> local finish reported
  -- server GameControl(3) --> Settling
  -- server GameControl(4) --> Ceremony/AwaitingNextStage
  -- GameNextStage         --> Ceremony/AwaitingResult
  -- GameResult            --> Ceremony/Podium
  -- local scheduler       --> RoomLobby
```

The order **`GameControl(4)` -> `GameNextStage` -> `GameResult`** comes from
the working deployment; do not reorder it.

The podium-to-room transition is client-side and automatic (no server packet,
no RHO duration). Timer comparisons are strict `>`, and an animation gate makes
the real duration longer than the sum:

```text
Individual: >1000 ms -> animation done -> >100 ms -> >5000 ms -> ready stage
Team:       >1000 ms -> animation done -> >100 ms -> >7000 ms -> ready stage
```

Mode flag `0x40` selects the observer path and `ObserverReadyStage`. Flag
`0x80` is a manual result path that waits for local action 13 after the team
7-second phase.

## Still open

- product names for every global mode bit, especially flag `0x80`;
- meanings of internal stage fields such as the byte at offset 224;
- mini-FSMs for non-core game types;
- a socket-driving mock client (the oracle returns markers instead of
  fabricating packets).

## Tests

```bash
cargo test -p p5099-client-oracle --test protocol_fsm
cargo test -p p5099-client-oracle --test final_stage_scheduler
cargo test -p p5099-client-oracle --test item_operation_semantics
```
