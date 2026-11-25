# Windows Server 2012 x64 compatibility build

The regular `x86_64-pc-windows-msvc` release targets Windows 10 / Server 2016
or newer and may fail at loader or CRT startup on Server 2012. No source
changes are needed: build the same source separately for
`x86_64-win7-windows-msvc` with a static CRT.

- Publish it as a separate asset (`p5099-server-win2012-x64.exe`); do not
  replace the regular release.
- Do not add a global `.cargo` target config or compatibility `cfg` branches.
- The target is Tier 3: it needs nightly with `rust-src` and `build-std`, and
  has no Rust release guarantees. Pin the dated nightly that first passes on
  real hardware (`rustup show`, `rustc +nightly -vV`).

## Build

On a Windows machine with the MSVC build tools and Windows SDK:

```powershell
rustup toolchain install nightly --component rust-src

$previousRustFlags = $env:RUSTFLAGS
$env:RUSTFLAGS = "-C target-feature=+crt-static"

cargo +nightly build `
  --release `
  --locked `
  -p p5099-server `
  --target x86_64-win7-windows-msvc `
  --target-dir target/p5099-win2012 `
  -Z build-std=std,panic_abort

$env:RUSTFLAGS = $previousRustFlags
```

Output: `target/p5099-win2012/x86_64-win7-windows-msvc/release/p5099-server.exe`

## Validation

**Not yet verified on a real Windows Server 2012 system.** Publish it as
experimental until it is.

1. Run `p5099-server-win2012-x64.exe --version`. If it fails before printing,
   check the PE loader, missing DLLs, CRT initialization, and unsupported APIs.
2. Then test startup, a `127.0.0.1` connection, a LAN connection, clean
   shutdown, and a second launch.
3. On failure, keep the dialog text, exit code, and the Event Viewer
   `Application Error` / `Windows Error Reporting` entries.
