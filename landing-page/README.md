# Cutokyo Frontend

Cutokyo is a desktop AI traffic control plane for Claude, Codex, and Gemini-style CLI traffic. The Next.js UI is packaged with Tauri and talks to the local API on `http://localhost:49321`.

## Quick Start

```bash
bun install
bun run dev
```

To run the backend, website/admin, local API, and capture proxy as one
supervised development stack from the repository root:

```bash
./scripts/dev-stack.sh
```

The development command deliberately leaves the machine-wide proxy untouched.
Set `CUTOKYO_DEV_APPLY_AUTO_CONNECT=1` only when you explicitly want to exercise
automatic system capture. Headless Linux sessions without Secret Service must
also provide 32-byte base64url `CUTOKYO_EVENT_ENCRYPTION_KEY` and
`CUTOKYO_AUTH_SESSION_ENCRYPTION_KEY` values; packaged desktops use the OS
credential store.

For the web/dev AuthKit flow, copy `.env.example` to `.env.local` and set the WorkOS values:

```bash
WORKOS_CLIENT_ID=
WORKOS_API_KEY=
WORKOS_COOKIE_PASSWORD=
NEXT_PUBLIC_WORKOS_REDIRECT_URI=http://localhost:3000/auth/callback
```

## Desktop Builds

Desktop builds use `bun run build:desktop` to create a static Next.js export, then Tauri packages the app.

```bash
bun run build:desktop
bun run desktop:build:linux
bun run desktop:build:linux:appimage
bun run desktop:build:macos
bun run desktop:build:windows
```

Windows release installers should be built on native Windows CI. For this WSL/Linux
workspace, use the local cross-build helper when you need a setup exe on the
Windows Desktop:

```bash
bun run desktop:build:windows:desktop
```

The helper downloads MinGW and NSIS packages into `src-tauri/target`, builds the
`x86_64-pc-windows-gnu` Tauri app, runs NSIS with the local data directory, and
fails if the executable imports unbundled MinGW runtime DLLs. macOS DMG builds
must run on macOS or in the `macos-latest` GitHub Actions job.

The packaged desktop app embeds the proxy and uses a local WorkOS OAuth bridge. Release builds must provide these public build-time values:

```bash
CUTOKYO_WORKOS_AUTHKIT_DOMAIN=https://auth.example.com
CUTOKYO_WORKOS_CLIENT_ID=client_...
CUTOKYO_WORKOS_REDIRECT_URI=http://localhost:49321/cutokyo/auth/callback
```

The root `.github/workflows/desktop-build.yml` workflow builds downloadable Linux DEB, Linux AppImage, macOS arm64/x64 DMGs, and a Windows setup artifact. Configure `CUTOKYO_WORKOS_AUTHKIT_DOMAIN` and `CUTOKYO_WORKOS_CLIENT_ID` as repository variables before running release builds. The exact release names are `Cutokyo-Linux.deb`, `Cutokyo-Linux.AppImage`, `Cutokyo-macOS-arm64.dmg`, `Cutokyo-macOS-x64.dmg`, and `Cutokyo-Windows-Setup.exe`; the app uses `NEXT_PUBLIC_CUTOKYO_RELEASE_BASE_URL` as the base URL for those download links.

Pull requests may build unsigned installers for native testing. Tagged releases fail closed unless the Windows installer passes Authenticode verification and both macOS apps pass strict codesign and Gatekeeper assessment; configure the platform signing/notarization credentials before creating a release tag.

The installed executable also provides the `cutokyo` CLI. DEB installs expose it
through `/usr/bin`, Windows setup adds the application directory to the user
PATH, and release builds on macOS/AppImage create an owner-safe
`~/.local/bin/cutokyo` link on first launch. Cutokyo never overwrites an
unrelated command at that path.

## Validation

```bash
bun run validate
bun run build
(cd .. && cargo audit)
bun tauri build --no-bundle --ci
```

## Useful Routes

- `GET /cutokyo/status` - proxy readiness and desktop auth configuration
- `GET /cutokyo/auth/session` - local desktop auth session state
- `POST /cutokyo/auth/start` - starts the desktop WorkOS OAuth flow
- `POST /cutokyo/compression/preview` - previews token-saving compression
- `POST /cutokyo/activation` - enables managed capture and optional gateway fallback settings
- `GET /cutokyo/sessions` - searches and filters locally encrypted session metadata
- `GET|PUT /cutokyo/settings` - reads or updates every endpoint capability switch
- `GET /cutokyo/detection` - reports detected supported harnesses and evidence
