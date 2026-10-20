# Agent contribution rules

These rules apply to the entire repository and every proposed public artifact.

- Publish only reviewed technical project content, original fixtures and
  appropriately licensed assets. Keep repository text and code in English.
- Never publish credentials, tokens, keys, environment files, personal email
  addresses, absolute local user/project paths, private machine details, chats,
  prompts, transcripts, operational state, memory dumps, screenshots of private
  applications, saves, firmware, proprietary SDK files or commercial game data.
- Local access is not permission to publish local files or their metadata.
  Explicitly supplied project assets still require metadata and path review.
- Preserve public upstream licenses, copyright notices and provenance. Do not
  remove required attribution while removing private development metadata.
- Delegated agents work locally only. They must not push, create public issues
  or comments, upload assets, publish releases or change repository visibility.
  The coordinating agent publishes owner-authorized technical changes after
  reviewing the exact source, diff, metadata and appropriate verification.
- Never push inherited private branches, tags or history into this public
  repository. Transfer approved source patches onto its clean public ancestry.
  Use the owner's GitHub noreply identity for new commit metadata.
- Scan the exact staged/public payload before publication, including binaries,
  archives and generated notices. Build releases with source-path remapping;
  inspect their contents and strings for private paths and sensitive material.
  Keep diagnostics closed and do not include raw guest payloads or addresses.
- Keep tests original and bounded. Record what actually ran; distinguish source
  review, compilation, diagnostics, a rendered game frame and measured FPS.
  Missing behavior must remain explicit; do not add fabricated success.
- Public CI uses standard GitHub-hosted runners, read-only permissions and no
  private local inputs. External contributions must not execute on a developer's
  computer through a public CI runner. Do not add secrets or paid runners without
  explicit owner authorization.
- Coordinate file ownership and use the agreed shared resource gate for local
  heavy work. Do not stop another project's processes or alter host memory,
  pagefile, priority or affinity policy to force a test through.
