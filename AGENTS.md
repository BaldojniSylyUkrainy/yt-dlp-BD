# yt-dlp-BD Agent Instructions

Read `READMEAI` completely before every task. It is the current architecture, build, testing, release, and security handoff. Preserve unrelated local changes and ignored signing/build material.

## Platform setup

- Supported targets are Windows x64 and macOS Apple Silicon. Reproduce platform-specific and packaged failures on the affected native platform.
- On Windows, validate the published NSIS installer and the managed app-local runtimes (`yt-dlp.exe`, Deno, FFmpeg, and FFprobe), including their actual paths, versions, and child-process arguments.
- Never expose or copy browser cookie values. Prefer the existing Edge cookie flow on Windows, and do not forcibly close the browser unless the test requires it and the user approves.
- Keep managed-runtime behavior cross-platform unless the fix is necessarily target-gated.

## Review and tests

- Work in this order: code, full diff review, tests, fixes, one release version bump, docs, explicit staging, commit, and push.
- Do not commit or push before the relevant review and tests pass.
- Run structural checks before normal suites when applicable:

```text
sg scan
sg test
npm test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run tauri build -- --debug --no-bundle
```

- For Windows release fixes, also build the native Tauri target and NSIS installer, or document a proportional packaged smoke test when signing/bundling credentials prevent a full release build.
- Repeat the exact affected GUI flow in the installed packaged app after implementing a fix.

## Version and release rules

- Public versions use `MAJOR.MINOR.PATCH.HOTFIX`; urgent targeted follow-ups increment `HOTFIX`.
- Tauri uses the mapped three-part version documented in `docs/VERSIONING_UK.md`.
- Update every version and documentation mirror listed in `READMEAI` and validate `RELEASE_NOTES.md`.
- Use the locally authenticated `gh` CLI for GitHub operations.
- Before protected merge or deployment approval, verify the exact PR head SHA.
- Before release handoff, verify `HEAD == origin/main == dereferenced tag`.
- The workflow may create a draft release. Never publish it without direct user approval.
- Never commit runtime binaries, artifacts, signing material, secrets, or browser cookies.
