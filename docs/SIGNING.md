# macOS signing & notarization — restore guide

This repo's macOS releases are signed with a **Developer ID Application**
certificate (HUCONN Co.,Ltd.) and notarized by Apple, so Gatekeeper accepts
them. CI does it automatically from repository secrets; nothing local is needed
to cut a release.

**No key material lives in this repo, and none belongs here — it is a public
repository.** All of it lives in one encrypted bundle you keep yourself
(`anima-signing-bundle.tar.gz.enc`). This file is only the procedure.

## To cut a signed + notarized release (the common case)

Nothing on your Mac is required. Bump the bundle version and push a tag:

```sh
# edit crates/anima-desktop/tauri.conf.json  ->  "version": "0.4.x"
git commit -am "chore: bump the desktop bundle version to 0.4.x"
git tag -a v0.4.x -m "Anima v0.4.x"
git push origin main --tags
```

`.github/workflows/release.yml` builds both platforms, signs + notarizes the
macOS bundle, and creates a **draft** release. Publish it with
`gh release edit v0.4.x --draft=false --latest`.

Verify the artifact after download:

```sh
spctl -a -vvv -t exec /Applications/Anima.app   # -> accepted, source=Notarized Developer ID
xcrun stapler validate /Applications/Anima.app  # -> has a ticket stapled
```

## On a freshly-wiped Mac

Everything needed is in your encrypted bundle. Restore it:

```sh
# 1. decrypt (prompts for the bundle passphrase — the one you keep in your
#    password manager, NOT any Apple password)
openssl enc -d -aes-256-cbc -pbkdf2 -iter 600000 \
  -in anima-signing-bundle.tar.gz.enc | tar xz

# 2. one command does the rest: verify, import the signing identity into the
#    login keychain, check the notarization key, and (with --github) re-register
#    all six repository secrets
cd bundle-stage
sh restore.sh            # local signing only
sh restore.sh --github   # also re-registers the CI secrets (needs `gh auth login`)
```

`restore.sh` and `MANIFEST.txt` inside the bundle document every file and every
identifier (Team ID, Key ID, Issuer ID). They are deliberately **not** written
here, because this file is public.

## If the bundle is lost

The private key cannot be recovered — Apple never had it. You must re-issue:

1. Generate a key + CSR:
   ```sh
   openssl req -new -newkey rsa:2048 -nodes -sha256 \
     -keyout DeveloperID.key -out DeveloperID.csr \
     -subj "/emailAddress=<you>/CN=<name>/C=KR"
   ```
2. developer.apple.com → Certificates → `+` → **Developer ID Application** →
   Profile Type **G2 Sub-CA** → upload the CSR → download the `.cer`.
   (Developer ID Application is limited to **5 per team**; revoke an unused one
   first if the list is full. Revoking does **not** break already-notarized,
   already-released builds — the stapled ticket stands.)
3. Bundle key + cert + Apple's G2 CA into a `.p12`, then rebuild the encrypted
   bundle and re-run `restore.sh --github`.

The notarization API key (`.p8`) is separate from the certificate: losing the
cert does not affect it, and vice versa. A new API key is minted at
App Store Connect → Users and Access → Integrations.

See [`DISTRIBUTION.md`](DISTRIBUTION.md) for the underlying Tauri/Gatekeeper
details.
