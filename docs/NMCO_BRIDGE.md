# P5099 messenger / friend bridge (NMCO replacement)

The stock client has no game-server packets for friends or the messenger.
Every friend, presence, request, note and chat feature goes through the
Nexon Messenger client SDK that is statically linked into `KartRider.exe`
(291 `CNM*Func` / `CNM*Event` classes) and reaches the outside world only
through `nmcogame.dll`. The stock DLL connects to Nexon's separate
`NMService` process, so on a private server the whole feature is dead.

P5099 replaces `nmcogame.dll` with `native/p5099-nmco`, a thin bridge that
forwards every call to the P5099 server and injects server events back into
the client. All messenger logic lives in Rust.

## DLL ABI used by the client

The client loads `nmcogame.dll` dynamically (`0x00873F47`) and resolves:

| Export | Signature (cdecl) |
|---|---|
| `NMCO_CallNMFunction` | `int(u32 code, const u8* frame, u8 codec, u8** out_frame, u32* out_capacity)` |
| `NMCO_MemoryFree` | `void(void* frame)` |
| `NMCO_SetPatchOption` | `int(int)` |

`code` is the function code stored at `CNMFunc+0x0C` (for example
`CNMGetFriendListFunc` = `0x4001`). `*out_frame` initially points at a
client-owned 1 MiB buffer and `*out_capacity` is `0x100000`; a bridge may
instead return its own allocation, which the client releases through
`NMCO_MemoryFree` when the pointer differs from its buffer. The call
succeeds only when it returns `1`; the client then deserializes the reply
into the same function object and treats `CNMFunc+0x28 != 0` as success.

`NMCO_SetVersionFileUrlA/W` and `NMCO_SetLocaleAndRegion` are exported for
completeness; the client does not resolve them.

## Frames

Every buffer crossing the DLL boundary is a frame:

```text
u32 codec magic | u32 payload length | payload | u32 codec trailer
```

| Codec | id | magic | trailer | payload |
|---|---|---|---|---|
| plain | 0 | `0x001FCA34` | `0x008A119E` | serialized object |
| scramble | 1 | `0x0A98C32A` | `0x820A9E12` | encoded object (`len + 10`) |

The client always encodes requests with the default scramble codec but
selects the decoder from each frame's magic, so the bridge answers with the
plain codec.

Scramble decoding (`0x0088B8D0`), over the payload `p` of length `n`:

1. `u16 check = p[n-2..n]` must equal `checksum(p[..n-2])`, where
   `checksum` folds every byte into `sum: u8` (wrapping add) and
   `xor: u8`, and returns `((!sum as u16) << 1) | xor`.
2. `t = reverse(p[..n-2])`; `t[1]` is the codec id `1`, `t[3..7]` is a
   little-endian `u32` key.
3. For `i in 0..n-10`: `plain[i] = t[8+i] ^ (prev ^ key as u8)` with
   `prev = t[8+i-1]` (`0` for the first byte).

## Object serialization

An object is `u32 tag | u32 length | fields`, with
`tag = class_code << 16 | version << 8 | flag` and `length` covering the
fields. `CNMFunc` and `CNMEvent` objects use the generic class codes
`0x666F` / `0x656F`; the concrete function or event is identified by the
code passed beside the frame.

Primitive encodings (`0x008390B0` ...):

| Writer | Encoding |
|---|---|
| `u8` / `u16` / `u32` / `u64` | little-endian |
| wide string (`0x00839250`) | compact length (UTF-16 units) + units, no NUL |
| bytes (`0x00839400`) | compact length + bytes |

Compact length (`0x00838EB0`): `< 0x40` one byte `n << 2`; `< 0x4000`
`u16 n << 2 | 1`; `< 0x4000_0000` `u32 n << 2 | 2`; otherwise byte `3`
followed by `u32 n`.

`CNMFunc` contributes `u32 +0x24` (request argument) and `u32 +0x28`
(result) after the version check; each class then writes its own fields. A
class writes the same fields for the request and the reply.

## Events

`CNMRegisterCallbackFunc` (`0x1002`) carries the client window and message.
The client window procedure (`0x00E68CF3`) accepts messages `0x40C` and
`0x464` with `wParam = event code` and `lParam = frame`, deserializes the
event (`0x00885DF0`, factory `0x00884AB0`) and dispatches it
(`0x00885F10`). The bridge delivers server events with `SendMessageW` from
its network thread so the frame stays valid during dispatch.

## Bridge transport

The connector writes `p5099-xun-session.ini` beside the client with the
server address and nickname; the bridge reads the same file and connects to
the NMCO port (login port + 4). Frames on that socket are
`u32 length | body`:

| Direction | Body |
|---|---|
| DLL → server hello | `u8 1 | u32 protocol | wide string nickname` |
| DLL → server call | `u8 2 | u32 sequence | u32 code | client frame` |
| server → DLL reply | `u8 3 | u32 sequence | u32 result | object bytes` |
| server → DLL event | `u8 4 | u32 code | object bytes` |

A call carries the client's frame unchanged (the server decodes either
codec); replies and events carry plain serialized objects, which the DLL
wraps in plain-codec frames. A missing server makes every call return `0`,
matching the stock behaviour on a private server.

Events are delivered from a dedicated dispatcher thread: the client thread
may be blocked waiting for a reply, so the socket reader never calls
`SendMessageW` itself.

## Scope: friend window only

Chat is served by the in-game chat server (`messenger_tcp`, base port + 2):
`PqGetRecommandChatServerInfo` is answered with that endpoint
(`PrGetRecommandChatServerInfo`: `u8 enabled | IPv4 octets | u16 port`).
The bridge implements only the friend window.

The Korean client (locale 1002, game table entry `0x3EA`) selects the SDK's
social backend (vtable `0x0100C6B4`, event message `0x464`); the classic
messenger backend (`0x0100C684`, message `0x40C`) is unused. KartRider's
event listener ignores the social events `0x8001`–`0x8004`, so the window
refreshes by calling the list functions again and the bridge never needs to
push events for friends.

Implemented functions (`crates/p5099-core/src/nm_social.rs`):

| Code | Function | Request fields | Reply fields |
|---|---|---|---|
| `0x200E`/`0x200F` | Get social / game friend list | — | `u32 n`, n × friend |
| `0x2012`/`0x2015` | Get received / sent requests | — | `u32 n`, n × request |
| `0x2018` | Get blocked users | — | `u32 0` |
| `0x2017` | Send friend request | `u64 id`, `wstr name` | `u32 code` |
| `0x2013`/`0x2014`/`0x2016` | Accept / reject / cancel | `u64 id` | `u32 code` |
| `0x2010`/`0x2011` | Delete social / game friend | `u64 id` | `u32 code` |

Objects (all `tag | length`, version 1, flag 1): `0x5330` presence
(`u32`, `wstr`); `0x5331` (presence, `wstr`); `0x5332` user (`u64 id`,
`wstr`, `wstr` display name, `wstr`); `0x5333` friend (user, presence,
`u8 online`, `u8`, `u32` character count); `0x5335` request (user, `0x5331`,
`wstr` message). Result codes (`0x00E14240`): `0` success, `1000` no such
user, `1002`/`1003` blocked, `1004` already friends, `1005` error, `1006`
limit, `1007` cancelled, `1010` inactive, `1013` already requested.

Friendships, requests and stable social ids are stored in
`Profile/Friends.json`; a friend is shown online while their bridge is
connected.
