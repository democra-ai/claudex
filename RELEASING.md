# Releasing Claudex (with auto-update)

Claudex ships an in-app auto-updater (`tauri-plugin-updater`). The app checks
`https://github.com/democra-ai/claudex/releases/latest/download/latest.json` on
launch; if it reports a newer **signed** build, the user gets an "Install &
restart" banner. For that to work, every release MUST be signed with the SAME
key and MUST publish a matching `latest.json`.

## Signing key (keep secret, never commit)

- Private key: `~/.claudex-signing/claudex.key` (passwordless)
- Public key:  `~/.claudex-signing/claudex.key.pub` — this value is pinned in
  `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`. If you ever rotate the
  key, old installs can no longer auto-update (they only trust the pinned key).

If the key is lost, generate a new one with
`npx tauri signer generate -w ~/.claudex-signing/claudex.key -p "" --ci -f`
and update the pubkey in `tauri.conf.json` (note: existing installs won't
auto-update across a key change — they'd need a manual reinstall once).

## Cut a release

1. Bump the version in `src-tauri/tauri.conf.json` and `package.json`.
2. Build, signed:
   ```sh
   export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.claudex-signing/claudex.key)"
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
   npm run tauri:build
   ```
   Produces, under `src-tauri/target/release/bundle/`:
   - `dmg/Claudex_<v>_aarch64.dmg` (manual download)
   - `macos/Claudex.app.tar.gz` + `.sig` (the updater payload)
3. Stage versioned updater assets + the manifest (the asset name must match the
   `url` in `latest.json`):
   ```sh
   cd src-tauri/target/release/bundle/macos
   cp Claudex.app.tar.gz     "Claudex_<v>_aarch64.app.tar.gz"
   cp Claudex.app.tar.gz.sig "Claudex_<v>_aarch64.app.tar.gz.sig"
   ```
   Write `latest.json` with `version: <v>`, the `.sig` contents as `signature`,
   and `url` pointing at the versioned `.app.tar.gz` in this release.
4. `git tag v<v> && git push origin main v<v>`
5. Publish — attach the DMG, the versioned `.app.tar.gz`, its `.sig`, and
   `latest.json`:
   ```sh
   gh release create v<v> \
     "…/dmg/Claudex_<v>_aarch64.dmg" \
     "…/macos/Claudex_<v>_aarch64.app.tar.gz" \
     "…/macos/Claudex_<v>_aarch64.app.tar.gz.sig" \
     "…/macos/latest.json" \
     --title "v<v> — …" --notes-file notes.md
   ```
6. Install locally: `ditto …/bundle/macos/Claudex.app ~/Applications/Claudex.app`.

Because the endpoint is `releases/latest/download/latest.json`, the newest
release's manifest is what every installed app sees — no extra hosting needed.
