# Feature posts on UO Tavern

New user-facing features should have a short introduction and actual gameplay
screenshots in the forum. The publisher uses an existing thread for the same
release tag or feature slug, preserving replies when a note is corrected.

## Automatic publishing

- Publishing or editing a **stable, public GitHub release** announces it.
- Adding or editing `docs/updates/*.json` on `main` publishes that note.
- **Actions → Publish client updates to UO Tavern → Run workflow** can publish
  a particular release or feature note manually.
- Draft and prerelease versions are not automatically announced. The existing
  bundle workflow still creates drafts; publication remains the release step.

The workflow uses the scoped `UOTAVERN_PUBLISH_TOKEN` repository secret. The
forum validates it with `CLIENT_UPDATES_TOKEN`. No database key is in GitHub.

## A note for a feature

Create `docs/updates/<feature-slug>.json`:

```json
{
  "kind": "feature",
  "slug": "a-stable-feature-name",
  "title": "A short, player-facing headline",
  "content": "What you can now do, how to try it, and any relevant limits. Markdown is supported.",
  "images": [
    {
      "path": "docs/updates/images/feature-name.png",
      "alt": "Describe the visible new behavior"
    }
  ]
}
```

Use a real capture showing the feature. Follow `scripts/devbrowser.sh` and the
renderer verification guidance in `CLAUDE.md`; do not invent a gameplay capture.
Commit the image with the note. Local image paths become URLs pinned to the
published git revision. Existing HTTPS screenshot URLs are also supported with
`url` instead of `path`. Use up to five PNG, JPEG, or WebP screenshots.

Describe completed behavior accurately. State setup requirements and validation
limits. Internal refactors do not need a feature post. Keep the slug stable when
correcting a note, so the publisher updates the existing conversation.

For release-specific images or a shorter introduction, add
`docs/updates/vX.Y.Z.json` with `kind: "release"`, `tag`, and optional `title`,
`content`, and `images`. Otherwise the publisher uses the GitHub release notes.

## Preview before publishing

```sh
python3 scripts/publish-forum-update.py --note docs/updates/v0.6.0.json
python3 scripts/publish-forum-update.py --release v0.6.0
```

These commands only print the prepared payload. `--publish` explicitly sends it.
Use GitHub Actions for normal publication; never put the publisher token in a
note, screenshot, issue, or command argument.

## Writing directly in the forum

The post and reply editors support **paste (⌘V / Ctrl+V)**, drag and drop, and
file selection. Uploads accept still PNG, JPEG, and WebP images up to 4 MB each.
The editor inserts image Markdown and provides a **Preview** tab. Uploaded
images are public attachments; replace the default “Screenshot” label with a
useful description before posting.
