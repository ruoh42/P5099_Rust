# P5099 XUN 시험용 사이드카

A 32-bit, version-pinned experimental layer that backports XUN driving to the
P5099 client. It builds a startup `dinput8.dll` proxy and a standalone
`p5099-xun.dll` (loadable into a running client with `p5099-xun-attach.exe`).
Both verify the exact client executable and log to `p5099-xun-sidecar.log`
beside the DLL.

## What it does

With `enabled=1`, it hooks the tachometer factory/allocators, V1 display
update, four-stat converter, drive-event handler, and main physics tick. XUN
charger state lives in a side table keyed by `GoPlayKart*`, so P5099's
`GoPlayKart` layout is never grown.

- S/B/L XUN types 2/3/4: booster-count charging, activation, duration and
  expiry, all six speed-mode physics consumers (boost-gauge and Exceed-gauge
  gains, collision responses), continuous charger gauge UI, an independent
  charger aura (`effect/charger/카트바디차저발동`), and modern four-stat display
  conversion. Acceleration and top speed are not changed.
- Type 1 needs no DLL consumer: the server grants the starting item.
- Types 5-10 stay disabled. XUN-only animations and lead-charge packet/state
  behavior are not implemented.
- Charger and ordinary Exceed are separate states and UI surfaces.

The profile comes from the server's private TCP channel (base port + 3) using
`p5099-xun-session.ini`, which the launcher writes beside the game executable.

## Pinned executable

Any mismatch leaves the DLL inert and is logged:

- PE32/x86 timestamp `0x6407DD11`
- image size `0x0141A000`
- entry RVA `0x00BE4D56`
- SHA-256 `FD9444C057090C3BB524AF03BFF5EC995620FBB951B9A823D2CD4E9B0494956F`

Installation checks sixteen instruction boundaries, suspends other client
threads, patches all sites together, and stays inert if any check fails. No
client-derived bytes, RHO payload, IDB, or decompiler output are included.

## Build

```powershell
cmake -S native\p5099-xun-sidecar `
  -B target\xun-sidecar-win32 `
  -G "Visual Studio 17 2022" -A Win32 `
  -DP5099_TEST_EXECUTABLE=C:\Nexon\KartRider_5099\KartRiderU.exe
cmake --build target\xun-sidecar-win32 --config Release
ctest --test-dir target\xun-sidecar-win32 -C Release --output-on-failure
cmake --install target\xun-sidecar-win32 --config Release --prefix target\xun-release
```

## Deploy

The launcher has no attach step, so distributed clients use the startup
proxy: put `startup-proxy/dinput8.dll` and `p5099-xun.dll` in the client folder
**before** writing the client manifest. Do not place the proxy beside the
launcher.

For attach mode, start the exact client and run `p5099-xun-attach.exe` with the
DLL path (pass the PID if several clients run). It must run at the same
elevation as the client.

## Diagnostics

Success logs `status=6 hooks=xun-tacho+physics-state+six-consumers+charger-visual`.
The log records drive events, charger transitions, charger-visual
create/start/stop, and Exceed gauge changes at 5% steps. Hook threads only
enqueue into a 256-entry queue; a background writer flushes at most once per
second, and overflow is reported as a dropped-entry summary instead of
blocking the physics tick.
