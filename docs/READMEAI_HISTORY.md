# READMEAI History — Archived Codex Work Log

This file is the primary entry point for future work on this repository.
Before starting each new task, Codex must read `READMEAI` first and then inspect
only the files directly relevant to the current step.

## Logging Rules

- After every change, record the date, purpose, and files added, modified, or removed.
- Briefly explain what was done and how it was verified.
- Record important decisions, assumptions, known issues, and the next agreed step.
- Keep all previous entries so the log remains chronological.
- Never store passwords, tokens, keys, or other secrets here.
- Use `READMEAI` to restore context quickly. It does not replace inspecting the
  specific files required to complete the current task safely.

## Project Snapshot

- Working directory: `yt-dlp bd`
- Project type: TypeScript/Vite application with a Tauri component.
- At the time this log was created, the root contained `src`, `src-tauri`,
  `public`, `scripts`, `docs`, `dist`, `release`, `package.json`, and `README.md`.

## Change Log

### 2026-07-25 — Work Log Created

Purpose: establish a persistent, concise context for step-by-step work on the repository.

Added:

- `READMEAI` — working rules, a brief project snapshot, and a chronological change log.

Modified:

- Nothing.

Removed:

- Nothing.

Verification:

- Confirmed that the file exists in the repository root.
- No other files were changed during this step.

Decision:

- Keep this internal AI-facing file in English; continue user-facing communication in Ukrainian.

Next step:

- Await the user's next task.

### 2026-07-25 — Blocking Runtime Preparation Overlay

Purpose: make component checks, installations, and updates unmistakable and prevent interaction with the download form while maintenance is running.

Added:

- A blocking modal overlay covering the entire download card while `runtimeBusy` is true.
- Per-component status indicators for yt-dlp, ffmpeg, and Deno.
- Explicit guidance asking the user to wait and not close the application.

Modified:

- `src/App.tsx` — replaced the small inline runtime notice with the blocking preparation overlay and made the underlying form inert.
- `src/App.css` — added the overlay, modal, spinner, component-state, and dimmed-backdrop styles.

Removed:

- The old compact `.runtime-notice` UI and its styles.

Verification:

- `npm run build` completed successfully (TypeScript and Vite production build).
- Automated browser-based visual verification was unavailable because local addresses are blocked by the browser policy.
- Final appearance should be confirmed during the next native Tauri launch while runtime maintenance is active.

Decision:

- Limit the overlay to the full download card so the component status panel remains visible, while all download controls are blocked.

Next step:

- Confirm the overlay visually in the native application, then await the user's next task.

### 2026-07-25 — Eneida-Inspired Minimal Visual Redesign

Purpose: move the interface away from polished tattoo/collage styling toward a restrained modern visual language inspired exclusively by the 1991 animated film `Eneida`, while improving action spacing.

Added:

- `src/assets/logo-eneida.png` — an original transparent shaka-hand brand mark with hand-drawn irregular contours and no palm-eye symbol.
- A larger visual separation between the output-folder control and the primary download action.
- Organic, asymmetric silhouettes and flatter hand-painted surface treatment across cards, controls, buttons, and modals.

Modified:

- `src/App.tsx` — switched to the new logo and removed the decorative folk-thread collage element.
- `src/App.css` — simplified the background decoration, reduced glossy gradients, introduced irregular outlines, and updated the overall palette and control geometry.

Removed:

- The old logo from active UI use; the source file remains in the repository for safe rollback.
- The small multi-piece folk decoration from the download card.

Verification:

- Confirmed that `logo-eneida.png` is a 512×512 RGBA PNG with transparent corners and clean subject coverage.
- `npm run build` completed successfully (TypeScript and Vite production build).
- Automated local browser preview remains unavailable because local addresses are blocked by browser policy.
- Final proportions should be confirmed in the native Tauri application.

Decision:

- Interpret the reference through contour, caricature, flat color, and asymmetry rather than copying film characters or frames.
- Keep pink as an accent and preserve a modern, minimal information hierarchy.

Next step:

- Await the user's native-app visual review.

### 2026-07-25 — Living Shaka Chimera Logo Trial

Purpose: replace the earlier literal hand mark with the user-approved trial concept built from the full visual direction: a readable shaka gesture fused into one strange living organism.

Added:

- `src/assets/logo-shaka-chimera.png` — a transparent 512×512 logo with a burgundy beet/borscht-like body, an organic palm eye, root anatomy, and deliberately irregular hand-drawn contours.

Modified:

- `src/App.tsx` — switched the active brand image to `logo-shaka-chimera.png`.

Removed:

- Nothing. Earlier logo candidates remain available for safe rollback while this direction is evaluated in the native application.

Verification:

- Removed the flat chroma-key background with a soft alpha matte and despill before resizing the asset to 512×512.
- Confirmed the generated mark has transparent background pixels and a readable shaka silhouette.
- `npm run build` completed successfully.

Decision:

- Treat the logo as one fused living chimera rather than a hand combined with a recognizable tool, vessel, or interface symbol.
- Keep this as a trial integration until the user reviews it at native application scale.

Next step:

- Review the logo in the native application and refine only if the mark loses clarity or feels stylistically disconnected from the rest of the interface.

### 2026-07-25 — Fast Runtime Startup and Real URL Preview Validation

Purpose: remove the long blocking component check on every launch and verify pasted media URLs with yt-dlp before enabling downloads.

Added:

- A debounced `probe_url` Tauri command that asks managed yt-dlp for compact metadata in simulation mode, without downloading media.
- A 25-second process deadline and yt-dlp's 10-second socket timeout for URL probes.
- A compact preview card with thumbnail, title, uploader, duration, and extractor.
- Explicit checking, green-valid, and red-invalid URL states with a user-readable failure reason.
- Cached background update schedules: daily for yt-dlp and weekly for ffmpeg/Deno.
- Five-second connection and twenty-second total timeouts for component update requests.

Modified:

- `src/App.tsx` — startup now performs a quick local status check, installs only missing components behind the blocking overlay, runs due updates silently in the background, and requires a successful yt-dlp probe before enabling Download.
- `src/App.css` — added the URL checking indicators, green/red states, and media preview styling.
- `src-tauri/src/lib.rs` — component installers no longer repeat a complete runtime status scan, share a bounded HTTP client, and expose the metadata probe command.
- `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock` — enabled Tokio process/time support for cancellable probe execution.

Removed:

- The unconditional sequential online check/update of yt-dlp, ffmpeg, and Deno on every application launch.
- The former green check that only meant the URL was syntactically parseable.
- Repeated full runtime status scans after each individual component installer.

Verification:

- `npm run build` completed successfully.
- `cargo check` completed successfully.
- Native `tauri dev` rebuilt and restarted without repeating the ffmpeg/Deno network maintenance pass.
- A public YouTube URL returned compact title, thumbnail, duration, uploader, extractor, and webpage metadata in simulation mode without downloading media.
- An unavailable YouTube test URL returned an error instead of a false positive.

Decision:

- Treat yt-dlp extraction success with its default no-formats error behavior as the source of truth for downloadability.
- Use `--simulate`, `--no-playlist`, explicit no-formats failure, and a compact JSON print template, following the official yt-dlp option documentation.
- Keep the blocking preparation modal only for actual first-time installation or repair of missing components; routine update checks must not block the form.

Known limitation:

- Private, age-restricted, or login-only media can fail the anonymous preview probe even if a later retry with browser cookies could download it.

Next step:

- Let the user review startup speed and paste representative URLs from the services they use most.

### 2026-07-25 — Visible Startup Progress and Sub-Second YouTube Preview

Purpose: make application startup visibly responsive and remove the 14–18 second wait before a YouTube thumbnail and availability result appeared.

Added:

- A full-window startup overlay with the current phase, continuously moving progress bar, percentage, and a short completion state.
- Immediate YouTube thumbnail derivation from standard watch, short, live, embed, and `youtu.be` URLs.
- Fast provider-metadata validation for public YouTube and Vimeo URLs with two-second connection and four-second total request limits.
- A compact preview placeholder shown while the title and availability response are still being fetched.

Modified:

- `src/App.tsx` — tracks startup phases/progress, shows the launch overlay, starts URL validation after a 140 ms debounce, and renders the YouTube thumbnail before validation completes.
- `src/App.css` — styles the startup screen, progress bar, logo motion, and preview-loading state.
- `src-tauri/src/lib.rs` — uses fast oEmbed availability checks for YouTube/Vimeo; other providers still use yt-dlp with a six-second socket timeout, one extractor retry, and a twelve-second total process deadline.

Removed:

- The full yt-dlp format extraction pass from the YouTube/Vimeo paste-preview path.
- The 550 ms frontend debounce and the previous 25-second generic probe deadline.

Verification:

- The previous full yt-dlp YouTube probe was measured at approximately 14–18 seconds even after limiting clients and manifests.
- A public YouTube metadata check completed in approximately 0.21 seconds.
- An unavailable YouTube URL returned HTTP 404 in approximately 0.30 seconds.
- `npm run build` completed successfully.
- `cargo check` completed successfully.
- Native `tauri dev` rebuilt and restarted successfully.

Decision:

- Do not run a full yt-dlp format extraction merely to populate the paste preview for YouTube/Vimeo. Provider metadata confirms that the public media exists, and yt-dlp support is already supplied by the managed runtime; final extraction remains authoritative when the actual download starts.
- Preserve the full bounded yt-dlp probe as the fallback for other supported services.

Known limitation:

- Provider metadata confirms that public media exists but cannot guarantee every later format request, geo restriction, or account-only case. Those failures remain handled by the existing download/authentication flow.

Next step:

- Let the user review the startup overlay and paste-preview speed in the native application.

### 2026-07-25 — Download Progress Popup

Purpose: keep active download progress directly in front of the user instead of appending the progress card below the main download form.

Added:

- A centered, fixed download-progress popup above a dimmed and blurred application backdrop.
- The original media URL inside the popup, alongside title, status, percentage, speed, and ETA.
- A close action after completion, failure, or cancellation; the active state retains the stop action.

Modified:

- `src/App.tsx` — moved the job progress card outside the scrolling main content into a dialog-style overlay and added its source URL.
- `src/App.css` — added the fixed backdrop, elevated popup sizing/shadow, URL truncation, and completed-state close-button styles.

Removed:

- The inline download progress card previously rendered below the main form.

Verification:

- `npm run build` completed successfully.
- The running native development application accepted both frontend changes through hot reload.

Decision:

- Do not make the entire native application always-on-top. Only the active download progress is elevated above the application interface, matching the clarified request.

Next step:

- Let the user confirm the popup position and backdrop strength during a real download.

### 2026-07-25 — Fish-and-Crayfish Brand and High-Fashion UI Redesign

Purpose: replace the rejected hand/chimera direction with the user's fish-and-crayfish reference and modernize the entire interface from decorative 2010-era styling to restrained contemporary editorial minimalism.

Added:

- `src/assets/logo-fish-crayfish.png` — final 512×512 transparent application mark: a complementary petrol-blue fish and application-pink crayfish arranged as a compact circular pair.
- `src/assets/app-icon-source.png` — 1024×1024 warm-ivory system-icon master using the same mark.
- `src/assets/logo/fish-crayfish-source.png` and `src/assets/logo/fish-crayfish-transparent.png` — generated chroma source and cleaned high-resolution intermediate for future brand refinement.
- Newly generated Tauri desktop, Windows, iOS, and Android icon sizes under `src-tauri/icons/`.
- A restrained diamond rhythm as a small Ukrainian graphic accent in the sidebar and startup screen.

Modified:

- `src/App.tsx` — switched all application logo usage to the fish-and-crayfish mark and updated the hero copy to a more editorial tone.
- `src/App.css` — replaced the previous organic/glossy styling with a complete modern design system: near-black editorial canvas, warm neutral typography, petrol and pink accents, consistent spacing, thin borders, simple radii, cleaner controls, quieter states, modern dialogs, and an Iowan/Baskerville display-serif hierarchy.
- `src-tauri/icons/**` — regenerated the native application icon set from the new master.

Removed:

- The old shaka-chimera logo from active application use; previous candidates remain in the repository for rollback.
- Excessive irregular radii, thick decorative outlines, strong offset shadows, blob backgrounds, and ornamental treatment on every control.

Verification:

- Generated the logo with the built-in ImageGen workflow using the supplied fish-and-crayfish image as the edit target.
- Removed the flat chroma-key background with the official local helper, validated alpha, squared the mark, and downsampled it to 512×512.
- Confirmed the generated native icon remains recognizable at 32×32.
- `npm run build` completed successfully.
- Visually inspected the rendered application at the local Vite URL; layout hierarchy, contrast, field density, logo treatment, and responsive fit were all coherent.
- The native Tauri development process rebuilt after the icon set changed and is running with the new assets.

Decision:

- Concentrate Ukrainian-naive identity in one strong brand mark and a few micro-accents. Keep the functional UI international, quiet, and fashion-editorial rather than covering it in literal folk ornament.
- Use application pink `#ED78AA` for the crayfish and primary action, and complementary petrol blue `#14566E` for the fish and secondary brand accent.
- Use the transparent mark inside the UI and the warm-ivory square master for operating-system icons so black outlines remain legible at small sizes.

Image generation prompt summary:

- Conservative edit of the supplied circular fish-and-crayfish pair; preserve recognizable anatomy and central fin/claw contact; simplify internal detail for 32 px; pink crayfish, petrol-blue fish, warm-ivory eyes, near-black outlines; contemporary Ukrainian naive art refined through high-fashion editorial minimalism; flat chroma-key background; no text, ornament, 3D, gradients, or mascot treatment.

Next step:

- Let the user review the new brand mark and complete UI in the native application, then refine density or palette only from that direct visual feedback.

### 2026-07-25 — Kovalska-Inspired Shaka Identity and Dark Interface

Purpose: replace the rejected fish-and-crayfish/high-fashion direction with a recognizable shaka mark and an application layout informed by the structural language of the Kovalska and Spiilka websites, adapted to dark mode.

Added:

- `src/assets/logo-shaka-kovalska.png` — transparent 512×512 application logo with a compact distorted shaka silhouette: thumb up on the viewer's left, pinky extended right, and three curled middle fingers.
- `src/assets/logo/shaka-kovalska-source.png` — original chroma-key generation source retained for future brand refinement.
- `src/assets/app-icon-kovalska-source.png` — 1024×1024 dark system-icon master.

Modified:

- `src/App.tsx` — switched the active identity to the new shaka mark, introduced indexed navigation, and simplified the primary heading and brand copy.
- `src/App.css` — rebuilt the visual system around a dark fixed-column grid, oversized grotesk typography, square controls, thin rules, flat pink functional blocks, modular media preview, and poster-like modal/progress states.
- `src-tauri/icons/**` — regenerated the native application icon set from the new dark master.

Removed:

- The fish-and-crayfish mark from active application use; its files remain available for rollback.
- The former serif-led luxury styling, rounded cards, soft shadows, gradients, and ornamental diamond accents from the active interface.

Verification:

- Removed the chroma-key background with the installed image-processing helper and confirmed a 512×512 RGBA result with transparent corners.
- Normalized the logo fill to application pink `#ED78AA` for consistent small-size rendering.
- Visually inspected the redesigned application at the local Vite URL; the fixed navigation, oversized heading, form hierarchy, and dark modular grid fit coherently at 1280×720.
- Confirmed the gesture remains readable in the generated 32×32 native icon.
- `npm run build` completed successfully.

Decision:

- Concentrate strangeness in the shaka silhouette itself rather than adding eyes, mouths, tools, weapons, folk ornament, or internal illustrative objects.
- Translate the Kovalska reference through scale, grid, blunt geometry, flat color, and app-like fixed navigation rather than reproducing its wordmark or corporate content.
- Keep dark mode as the permanent canvas and use pink as a large functional surface, not a decorative glow.

Image generation prompt summary:

- Original vector-like shaka glyph; exact thumb-up/pinky-right orientation; one uninterrupted pink mass; uneven compressed finger arches; distorted sculptural contour; flat green chroma background; no black diagonal wedge, held object, knife, emoji styling, gradients, text, or mockup.

Next step:

- Let the user review the native application and refine only the logo silhouette or interface density from direct feedback.

### 2026-07-25 — Three-Hand Reference Logo and Loading Sequence

Purpose: replace the generated single-hand symbol with the user's supplied three-hand artwork, preserving its animation-like progression while adapting it to the dark application palette.

Added:

- `src/assets/logo-call-me-hands.png` — transparent 960×480 primary logo containing all three original call-me-hand poses in their left-to-right sequence.
- `src/assets/logo-call-me-hand-frame-1.png`, `logo-call-me-hand-frame-2.png`, and `logo-call-me-hand-frame-3.png` — individual transparent frames used by the startup loading animation.
- `src/assets/logo/call-me-hands-triptych-source.png` — chroma-key source retained for future refinement.
- `src/assets/app-icon-call-me-hands-source.png` — 1024×1024 dark native-icon master using the full triptych.

Modified:

- `src/App.tsx` — switched the active brand to the three-hand logo, added the three-frame startup sequence, and changed the brand caption to the exact requested `yt-dlp` / `baldojnyi downloader` wording.
- `src/App.css` — widened the sidebar and hero logo treatment and added a staggered three-frame startup animation.
- `src-tauri/icons/**` — regenerated all native icon sizes from the new triptych master.

Removed:

- Discarded single-hand intermediate files created during the initial misunderstanding of the request.

Verification:

- Removed the chroma-key background with edge contraction and confirmed a transparent 960×480 RGBA logo.
- Visually inspected the transparent triptych and its three separate startup frames.
- Confirmed all three silhouettes remain distinguishable in the generated 32×32 native icon.
- `npm run build` completed successfully.

Decision:

- Treat all three hands as one primary logo and as consecutive loading frames; do not isolate the center hand.
- Preserve the supplied composition and friendly irregular silhouettes. Limit adaptation to pink tonal hierarchy, near-black contour cleanup, transparency, and interface placement.
- Keep brand lettering as live UI text rather than baking it into the image, preserving clarity and localization flexibility.

Image generation prompt summary:

- Conservative edit of the supplied three-hand image; preserve all three poses, ordering, spacing, and animation-like progression; recolor to deep/application/pale pink; convert existing inner contours to near-black; flat chroma background; no pose redesign, added objects, gradients, text, or mockup.

Next step:

- Let the user review the triptych logo and three-frame startup animation in the running native application.

### 2026-07-25 — Readability and Header Composition Pass

Purpose: respond to the user's 27-inch screenshot review by removing the redundant right-side logo, improving the left brand lockup, and making all interface copy readable on both large and 13-inch displays.

Modified:

- `src/App.tsx` — removed the decorative translucent triptych from the right side of the main header.
- `src/App.css` — moved the `ІНСТРУМЕНТ / 01` label upward, centered the sidebar brand caption, increased the `yt-dlp` wordmark to 34 px, raised every remaining text size to at least 11 px, strengthened muted-text contrast, enlarged form/status/modal copy, improved disabled-button legibility, and constrained the brand artwork at compact widths.

Added:

- Nothing.

Removed:

- The redundant `.hero-mark` treatment and its responsive override.

Verification:

- Audited every `font-size` declaration in `src/App.css`; none remains below 11 px.
- Visually inspected the application at 1280×720 after hot reload. The sidebar lockup is centered, the header has no right-side duplicate logo, the instrument label sits higher, and the form/status copy remains readable without collisions.
- Confirmed the disabled primary action retains a clear label while remaining visibly inactive.
- Added compact-width brand sizing to prevent the 208 px triptych from overflowing the 220 px sidebar layout.
- `npm run build` completed successfully.

Decision:

- Preserve the oversized poster headline and rigid grid because they still carry the Kovalska-inspired identity well.
- Treat 11 px as the absolute floor for low-priority interface text; use 12–15 px for operational content and stronger contrast for all muted copy.
- Keep the header visually empty on the right rather than filling the space with repeated branding.

Next step:

- Let the user review the revised native window, especially the left brand size and 13-inch readability.

### 2026-07-25 — Forced macOS Dock Icon Refresh

Purpose: ensure the running native application and macOS Dock use the current three-hand brand instead of the first generated logo retained by the old development build.

Modified:

- No source files. The existing `src-tauri/icons/icon.icns` and configured bundle icon list were already correct.

Removed:

- Stale native build artifacts for the `yt-dlp-desktop` Cargo package.

Verification:

- Confirmed `src-tauri/tauri.conf.json` references `icons/icon.icns` and the current generated PNG/ICO sizes.
- Confirmed the icon files were generated from `src/assets/app-icon-call-me-hands-source.png` and are newer than the earlier branding.
- Stopped the old `tauri dev` process that predated the new icon set.
- Ran `cargo clean -p yt-dlp-desktop`, then launched a fresh `npm run tauri dev` build.
- Tauri recompiled the package successfully and launched `target/debug/yt-dlp-desktop` with the current native resources.

Decision:

- After any future native-icon change, restart and rebuild Tauri; frontend hot reload alone cannot replace the icon of an already running macOS Dock process.

Next step:

- Let the user confirm the three-hand icon is now visible in the Dock.

### 2026-07-25 — Readable Poster Typography System

Purpose: replace the overly condensed typography that remained difficult to read despite larger sizes, while preserving the Kovalska-inspired poster character.

Modified:

- `src/App.css` — removed `Arial Narrow` from the global stack, introduced separate UI and poster font stacks, and loosened the strongest negative letter spacing values.
- Operational copy, navigation, form controls, captions, and status text now use the macOS system/SF/Helvetica stack.
- The main headline, sidebar wordmark, startup headline, and modal headings now use a heavy `Arial Black`/SF Pro Display/Helvetica stack.

Verification:

- `npm run build` completed successfully.
- Visually inspected the running application at 1280×720.
- Confirmed the computed UI stack resolves through the macOS system font at 16 px base size.
- Confirmed the main headline renders at 92 px with `Arial Black`, moderate `-3.22 px` tracking, and a 776.8 px width that fits the content grid without clipping.
- Confirmed navigation renders at 14 px with positive `0.14 px` tracking.

Decision:

- Use neutral system typography for readability and reserve the heavy poster face for a small number of high-impact headings.
- Avoid condensed global font families and extreme negative tracking for Cyrillic text.
- Preserve uppercase labels, but give operational controls neutral or slightly positive tracking.

Next step:

- Let the user review the typography in the native window and adjust only the poster headline weight or tracking if desired.

### 2026-07-25 — Stronger Sidebar Brand Lockup

Purpose: make the product name beneath the three-hand logo expressive, readable, and complete.

Modified:

- `src/App.tsx` — changed the live brand caption to the exact two-line wording `yt-dlp BD` and `Baldojnyi Downloader`.
- `src/App.css` — increased the primary brand line to 38 px in the poster face, raised the descriptor to 15 px with stronger contrast and weight, reduced excessive descriptor tracking, and added 32/13 px compact-width values.

Verification:

- `npm run build` completed successfully.
- Visually inspected the 248 px sidebar at 1280×720.
- Confirmed both lines remain on one line inside the 207 px brand content width.
- Confirmed compact-width typography remains within the 180 px content area.

Decision:

- Treat `yt-dlp BD` as the primary wordmark and `Baldojnyi Downloader` as a readable product descriptor, not low-priority microcopy.

Next step:

- Let the user review the revised sidebar brand lockup in the native application.

### 2026-07-25 — Modern Rounded Cross-Platform App Icon

Purpose: replace the full-bleed square Dock artwork with a contemporary rounded application tile consistent with current macOS icons and regenerate every platform size.

Added:

- `src/assets/app-icon-call-me-hands-rounded.png` — final 1024×1024 master with transparent outer corners, a dark graphite rounded-square tile, subtle inner keyline, and an enlarged centered three-hand mark.

Modified:

- `src-tauri/icons/**` — regenerated macOS ICNS, Windows ICO/Appx, standard PNG, iOS, and Android icon assets from the rounded master.

Removed:

- Stale native Cargo build artifacts before the final icon rebuild. No source or user data was removed.

Verification:

- Visually inspected the 1024×1024 master and generated 32×32 PNG.
- Confirmed the outer corners are transparent and the rounded graphite tile remains distinct against a dark Dock.
- Corrected an initial scaling issue caused by a non-upscaling thumbnail operation; the final triptych now occupies roughly 70% of the tile width and all three silhouettes remain visible at 32×32.
- Ran `cargo clean -p yt-dlp-desktop`, rebuilt Tauri from scratch, and launched `target/debug/yt-dlp-desktop` successfully with the current native resources.

Decision:

- Use a rounded-square graphite tile for the native icon while keeping the transparent three-hand artwork inside the application UI.
- Regenerate every platform icon from the same master so macOS, Windows, iOS, and Android branding remains consistent.

Next step:

- Let the user confirm the new rounded icon visually matches neighboring applications in the macOS Dock.

### 2026-07-25 — Helvetica-Only Typography

Purpose: remove the inexpensive-looking Arial/SF mixture and give the whole application one consistent typographic voice.

Modified:

- `src/App.css` — replaced the global UI and poster font stacks with `"Helvetica Neue", Helvetica, sans-serif`.
- Headings, navigation, forms, status messages, modals, and the sidebar brand now all use the same Helvetica family.
- Preserved the poster hierarchy through font weight, scale, uppercase treatment, and controlled letter spacing instead of introducing another display font.

Verification:

- Confirmed there are no remaining references to Arial, SF Pro, `system-ui`, `-apple-system`, or BlinkMacSystemFont in `src/`.
- `npm run build` completed successfully.

Decision:

- Do not load a Google Font: the application now follows the user's explicit Helvetica-only direction and avoids adding a network-dependent desktop font.

Next step:

- Let the user review the Helvetica typography in the already-running native application.

### 2026-07-25 — Sidebar Separation and Restored Layout Rhythm

Purpose: visually separate the brand/navigation column from the download workspace and restore the whitespace lost during the typography and header revisions.

Modified:

- `src/App.css` — introduced a dedicated warm graphite-plum sidebar surface (`#141114`) with its own quieter dividers, while preserving the near-black main workspace.
- Widened the desktop sidebar slightly and increased its inner padding, brand separation, navigation rhythm, component-row height, and footer spacing.
- Increased the main workspace inset, header breathing room, download-card separation, and card padding.
- Rebuilt the download form rhythm so the format/quality controls, subtitles, folder picker, primary action, and legal note no longer touch or visually collapse into one table-like block.
- Added compact-width adjustments so the increased spacing remains usable near the application's minimum supported width.

Verification:

- `npm run build` completed successfully.
- Visually inspected the application at 1280×720 after hot reload.
- Confirmed the sidebar reads as a distinct surface, the header is separated from the form, and each functional group has visible whitespace around it.
- Confirmed the main area uses vertical scrolling instead of compressing controls when the window is shorter.

Decision:

- Keep the sidebar difference subtle and material-like rather than using a loud second accent color.
- Prefer natural vertical scrolling over shrinking typography or removing functional spacing on laptop-height windows.

Next step:

- Let the user review the updated native window and decide whether the sidebar should move one step lighter or darker.

### 2026-07-25 — macOS-Sized Squircle App Icon

Purpose: make the Dock icon match the optical size and continuous-corner silhouette of neighboring modern macOS applications such as QuickTime.

Added:

- `scripts/build_app_icon.py` — deterministic Pillow-based icon builder that preserves the existing three-hand artwork while controlling the native tile geometry, padding, color, and mark placement.

Modified:

- `src/assets/app-icon-call-me-hands-rounded.png` — rebuilt the 1024×1024 master with an 868 px centered tile, approximately 8% transparent padding per side, and a continuous 4.4-power superellipse instead of a large conventional rounded rectangle.
- `src-tauri/icons/**` — regenerated every macOS, Windows, iOS, Android, and PNG icon size from the corrected master.

Removed:

- Old package-specific native build artifacts via `cargo clean -p yt-dlp-desktop`; they were regenerated immediately. No source or user data was removed.

Verification:

- Inspected the new 1024×1024 master and its generated 32×32 representation.
- Confirmed the hands were reused from `src/assets/logo-call-me-hands.png` without generative redrawing.
- Confirmed the tile alpha bounds are intentionally smaller than the canvas and the corners follow a smooth squircle profile.
- Rebuilt and relaunched the native Tauri application successfully so macOS receives the new `icon.icns`.

Decision:

- Treat native app-icon scale separately from the in-app logo scale.
- Keep the icon-building process deterministic because this change concerns platform geometry, not artwork generation.

Next step:

- Let the user compare the relaunched icon directly with neighboring Dock icons.

### 2026-07-25 — Aligned Masthead and Reversed Brand Hierarchy

Purpose: make the left brand block and right application masthead end on one continuous horizontal line, while emphasizing the human-readable product name.

Modified:

- `src/App.tsx` — reordered the sidebar wordmark to show `yt-dlp BD` as the small supporting label and `Baldojnyi Downloader` as the primary name.
- `src/App.css` — assigned the sidebar brand block and main topbar the same 177 px height and synchronized their top insets at both desktop and compact breakpoints.
- Reduced the compact-width primary brand size so `Baldojnyi Downloader` remains on one line.

Verification:

- Measured both rendered blocks at 1280×720: the brand divider and topbar divider both end at exactly 211 px, with a computed difference of 0 px.
- Confirmed the primary brand title remains inside the sidebar content width.
- `npm run build` completed successfully.

Decision:

- Use exact shared block geometry for major cross-column alignment instead of relying on independent content-driven heights.

Next step:

- Let the user review the corrected masthead alignment in the native application.

### 2026-07-25 — Softer Rose Palette and Aligned Startup Progress

Purpose: reduce visual fatigue from the bright pink accent and make the startup progress indicator match the optical width of the startup headline.

Modified:

- `src/App.css` — changed the main accent from bright candy pink to muted dusty rose (`#c27898`) and softened the hover shade to `#d38aa6`.
- Updated the remaining hard-coded pink spinner and download-progress border colors to matching muted values.
- Applied reduced saturation and brightness to the in-app hand artwork in the sidebar and startup animation; the source logo files and native icon artwork remain unchanged.
- Limited the startup progress track and percentage label to 82% of the startup container with a 540 px maximum, keeping both aligned to the headline instead of the wider outer container.

Verification:

- Visually inspected both the startup overlay and the main interface at 1280×720.
- Confirmed the progress track and percentage label both render at 540 px inside the 660 px startup container.
- Confirmed the muted rose remains readable against the dark surfaces without dominating large buttons, selection states, navigation, or progress elements.
- `npm run build` completed successfully.

Decision:

- Keep pink as the identity color, but use a lower-chroma dusty rose for all operational UI accents.
- Adjust display saturation through CSS so the original hand assets remain reusable and unchanged.

Next step:

- Let the user review the softer palette on the 13-inch display.

### 2026-07-25 — Guaranteed MP4 Video Output

Purpose: fix video downloads that appeared as MP4 in the interface but remained WebM files on disk, especially with the `best` quality selector.

Root cause:

- The video branch selected the best available streams with `-f bv*+ba/b` but did not specify the final container.
- yt-dlp therefore correctly retained WebM whenever the best selected source or merged result used that container.

Modified:

- `src-tauri/src/lib.rs` — added `--merge-output-format mp4` and `--remux-video mp4` to every video-mode download.
- Audio-only downloads remain unchanged and continue to honor the selected MP3, M4A, Opus, or WAV format.

Verification:

- Confirmed against the official yt-dlp documentation that `--merge-output-format mp4` controls merged streams but is ignored when no merge is required, while `--remux-video mp4` handles an already-combined non-MP4 download.
- Preserved the existing best-quality and resolution-limit selectors; the fix changes the final container without intentionally lowering stream quality.
- `cargo check` completed successfully.
- The running Tauri development application rebuilt and relaunched successfully with the native change.
- `cargo fmt --check` still reports two older formatting-only differences in the oEmbed helper section; the new download-format block itself is rustfmt-compatible and those unrelated lines were left untouched.

Decision:

- Use merge plus remux instead of the yt-dlp `mp4` preset, because the preset also changes codec sorting and can prefer broadly compatible H.264/AAC streams over the highest available quality.
- Avoid `--recode-video mp4`; remuxing changes only the container and does not introduce a slow lossy re-encode.

Next step:

- Verify one new best-quality video download produces a final `.mp4` file; previously downloaded `.webm` files are not converted retroactively.

### 2026-07-25 — Adaptive Main-Column Scrolling

Purpose: keep the useful vertical scroll in compact windows while preventing the full-size download workspace from moving when all meaningful content already fits.

Modified:

- `src/App.tsx` — added a main-content element reference, resize observation, and adaptive overflow measurement.
- The right column now enables scrolling only when its content exceeds the available height by more than 56 px.
- When the remaining overflow is only padding or fractional layout slack, the column switches to fixed mode and returns to scroll position zero.
- The measurement automatically reruns when the window or observed header/form blocks resize, so previews and validation content can restore scrolling even in a large window when necessary.
- `src/App.css` — made fixed overflow the default, enabled `overflow-y: auto` only for `.is-scrollable`, and disabled vertical overscroll bounce.

Verification:

- `npm run build` completed successfully.
- At 1280×720, measured 871 px of content inside a 720 px viewport; the column correctly received `is-scrollable` with `overflow-y: auto`.
- Confirmed the running Tauri application hot-reloaded both the TSX and CSS changes.

Decision:

- Base scrolling on real content overflow rather than a hard-coded screen-size breakpoint, because previews, errors, and different display scaling can change the required height.
- Ignore up to 56 px of non-essential overflow so full-size windows remain visually anchored.

Next step:

- Let the user confirm the full-size native window no longer moves while compact-window scrolling remains available.

### 2026-07-25 — Completed-Download Checkmark Dismissal

Purpose: let the user close the completed-download popup from either side of the card.

Modified:

- `src/App.tsx` — changed the green completed-state checkmark area into an accessible `Закрити` button using the same `setJob(null)` action as the right-side cross.
- The left area remains a non-interactive progress indicator while a download is active.
- `src/App.css` — reset button chrome for the checkmark tile and added pointer and hover feedback without changing its layout.

Verification:

- Confirmed both completed-state close controls call the same popup-dismissal action.
- `npm run build` completed successfully.
- Confirmed the running Tauri application hot-reloaded the TSX and CSS changes.

Next step:

- Let the user confirm both the checkmark and cross dismiss the completed-download popup.

### 2026-07-25 — Rate-Limit-Friendly Multi-Item Delays

Purpose: reduce the risk of temporary blocking when yt-dlp processes playlists, carousels, or other sources containing multiple media items.

Modified:

- `src-tauri/src/lib.rs` — added the explicit delay values used by yt-dlp's official `sleep` preset to every download command:
  - `--sleep-requests 0.75` between extraction requests;
  - `--sleep-subtitles 5` before subtitle downloads;
  - `--sleep-interval 10` and `--max-sleep-interval 20` for a randomized 10–20 second pause before media downloads.

Verification:

- Confirmed the parameter meanings and values against the official yt-dlp README and preset definition.
- Confirmed the generated command block contains all four delay options.
- `cargo check` completed successfully.
- The running Tauri application rebuilt and relaunched successfully with the native change.

Decision:

- Use randomized official-preset timing instead of a fixed pause, which avoids a repetitive request cadence.
- Keep the arguments explicit rather than using `-t sleep`, so the application's behavior remains visible and stable if the preset is expanded in a future yt-dlp version.
- Accept that yt-dlp applies the download sleep before single downloads too; it does not expose a universal extractor-independent option that activates the delay only after detecting a playlist or carousel.

Next step:

- Let the user try a small multi-item playlist or carousel and confirm the site processes every entry without rapid consecutive requests.

### 2026-07-25 — QuickTime-Compatible MP4 Codecs

Purpose: fix MP4 files that downloaded and merged successfully but still triggered QuickTime's incompatible-media warning.

Root cause:

- Inspected `/Users/iad/Downloads/Як здурів нaцик Дугін： від opгiй до важких скрєп [1GVKutdrw-g].mp4` with the application's managed ffprobe.
- The container was valid MP4, but its internal streams were AV1 video at 1920×1080 plus Opus audio.
- The earlier merge/remux fix changed the container without changing those codecs, so no ffmpeg failure occurred; QuickTime rejected the media combination inside the MP4.

Modified:

- `src-tauri/src/lib.rs` — added the official yt-dlp MP4-compatible format sort `vcodec:h264,lang,quality,res,fps,hdr:12,acodec:aac` to video downloads while retaining MP4 merge and remux handling.
- `src/App.tsx` — renamed the default quality option to `Найкраща сумісна (MP4)` so the interface accurately communicates the compatibility-first behavior.

Verification:

- Confirmed the managed ffmpeg includes `libx264`, `h264_videotoolbox`, AAC, and AudioToolbox AAC encoders.
- Ran yt-dlp in simulation mode against the exact affected YouTube URL. The corrected command selected format `96-2` with `avc1.640028` H.264 video, `mp4a.40.2` AAC audio, MP4 container, and 1080p resolution instead of AV1 + Opus.
- `cargo check` completed successfully.
- `npm run build` completed successfully.
- The running Tauri application rebuilt and relaunched successfully with the native codec-sort change.

Decision:

- Prioritize H.264/AAC playback compatibility over AV1/Opus bitrate efficiency for the default desktop MP4 workflow.
- Do not blindly transcode every download: selecting compatible source streams avoids a long lossy re-encode in the common case.
- Keep resolution selections as upper bounds; some services, including YouTube, may not offer H.264 above 1080p even when AV1 4K exists.

Next step:

- Redownload the affected video to produce the corrected H.264/AAC file. The existing AV1/Opus MP4 is not modified retroactively.

### 2026-07-25 — Restored Proven VideoToolbox Download Pipeline

Purpose: adopt the user's previously reliable yt-dlp command so the app can retain the best source quality and still produce QuickTime-compatible output.

Modified:

- `src-tauri/src/lib.rs` — replaced compatibility-first format sorting with the user's proven `bestvideo+bestaudio/best` selection.
- Resolution limits now use equivalent `bestvideo[height<=…]+bestaudio/best[height<=…]` selectors.
- Added `--postprocessor-args "ffmpeg:-c:v h264_videotoolbox -c:a aac"` so merged or remuxed video is hardware-encoded to H.264 with AAC audio on macOS.
- Retained `--merge-output-format mp4` from the proven command and kept `--remux-video mp4` as protection for extractors that return a single WebM file without a merge step.
- `src/App.tsx` — renamed the default option to `Найкраща доступна (MP4)` because the app once again selects the best source before compatibility conversion.

Verification:

- Confirmed the managed ffmpeg exposes both `h264_videotoolbox` and AAC encoders.
- Ran a two-second real conversion from the affected AV1 + Opus MP4 through the exact VideoToolbox/AAC arguments.
- ffprobe confirmed the temporary result contained `h264|video` and `aac|audio`.
- Removed the temporary two-second test file after verification.
- `cargo check` completed successfully.
- `npm run build` completed successfully.
- The running Tauri application rebuilt and relaunched successfully with the restored pipeline.

Decision:

- Prefer the user's known-good hardware-transcoding workflow over compatibility-first source selection.
- Accept the post-processing time and one H.264 encode in exchange for preserving access to higher-resolution AV1/VP9 sources and producing predictable QuickTime-compatible files.
- This supersedes the previous decision to avoid transcoding and prioritize already-compatible H.264/AAC source streams.

Next step:

- Redownload the affected video once more and verify the complete file opens directly in QuickTime.

### 2026-07-25 — Conditional Short Playlist Delays

Purpose: remove the excessive 10–20 second sleep from ordinary single-video downloads while retaining modest rate-limit protection for explicit multi-item collections.

Modified:

- `src/App.tsx` — added `isLikelyMultiItemUrl`, which recognizes YouTube `/playlist` URLs, YouTube links containing a `list` parameter, and common collection paths such as playlists, sets, albums, collections, and showcases.
- Download requests now include a `multiItem` boolean derived from the submitted URL.
- `src-tauri/src/lib.rs` — added the corresponding `multi_item` request field and removed all sleep arguments from the common single-download command.
- Sleep arguments are now added only when `multi_item` is true.
- Reduced collection timing from 10–20 seconds to a randomized 3–7 seconds, extraction-request delay from 0.75 to 0.5 seconds, and subtitle delay from 5 to 2 seconds.

Verification:

- `cargo check` completed successfully.
- `npm run build` completed successfully.
- The running Tauri application rebuilt and relaunched successfully with both frontend URL detection and native conditional arguments.

Decision:

- Single-video URLs must never receive playlist-oriented sleep arguments.
- Use a short randomized delay only for explicitly recognizable collection URLs.
- This supersedes the earlier global official-preset sleep policy.

Next step:

- Confirm an ordinary YouTube watch URL starts immediately and a URL containing a YouTube `list` parameter shows only a 3–7 second randomized delay.

### 2026-07-25 — Sidebar Support Contact

Purpose: give users an obvious way to report a broken download or application problem.

Modified:

- `src/App.tsx` — imported Tauri's opener API and added a `contactSupport` action that opens the default mail application for `baldojnisyly@gmail.com` with the subject `Проблема з yt-dlp BD`.
- Added a compact `Щось не працює?` feedback block at the bottom of the sidebar with the visible support email address.
- `src/App.css` — styled the contact block with the muted rose accent and lightweight hover feedback.
- Added a max-height 760 px compact sidebar mode that reduces navigation and runtime-row spacing without changing the aligned masthead height.

Verification:

- Confirmed `opener:default` is already permitted for the main Tauri window and the opener plugin is initialized.
- Visually inspected the support block at 1280×720.
- Before the compact adjustment, the sidebar extended to 771 px; after the adjustment it ends at exactly 720 px, while the version footer ends at 698 px and remains fully visible.
- `npm run build` completed successfully.

Decision:

- Use a direct mailto action instead of collecting messages inside the application; no user report data is stored or transmitted by the app itself.
- Keep the contact address visible rather than hiding it behind an icon.

Next step:

- Let the user confirm the support link opens their preferred mail client in the native application.

### 2026-07-25 — Visible QuickTime Conversion Stage

Purpose: prevent the download popup from looking frozen after media reaches 100% while ffmpeg is merging and transcoding the final MP4.

Modified:

- `src-tauri/src/lib.rs` — added yt-dlp's official `postprocess:` progress template and emits a dedicated `postprocess` event when the merger, converter, or remuxer begins. Retained log-marker detection as a compatibility fallback.
- `src/App.tsx` — added a `postprocessing` job state. The modal now replaces the misleading `100%` label with `MP4`, explains that QuickTime conversion is running, identifies VideoToolbox, keeps the job active, and leaves the Stop action available.
- `src/App.css` — added a continuously moving indeterminate conversion bar plus a reduced-motion fallback.

Verification:

- `cargo check` completed successfully.
- `npm run build` completed successfully.

Decision:

- Do not invent a numeric conversion percentage: yt-dlp's postprocessor progress hook exposes stage start/finish but ffmpeg's frame progress is not forwarded as an accurate percentage through this CLI path.
- Continue selecting `bestvideo+bestaudio/best`; the conversion stage is required when the best source uses a codec that QuickTime does not support.

Next step:

- Confirm during a real high-quality download that the popup switches from determinate download progress to the animated MP4 conversion stage and remains visible until completion.

### 2026-07-25 — High-Quality Universal MP4 Pipeline

Purpose: improve the best-quality video workflow while retaining reliable QuickTime and Adobe Premiere Pro compatibility.

Modified:

- `src-tauri/src/lib.rs` — changed the video selectors to yt-dlp's current `bestvideo*+bestaudio/best` form, including the existing resolution caps.
- Scoped the ffmpeg arguments to output processing with `ffmpeg_o`.
- Added VideoToolbox constant-quality encoding at `q:v 80`, H.264 High profile, `yuv420p`, the `avc1` tag, AAC-LC at 256 kbit/s, and MP4 faststart metadata placement.

Verification:

- `cargo check` completed successfully.
- A one-second real encode through the managed ffmpeg and the exact new output arguments completed successfully outside the sandbox.
- ffprobe confirmed `H.264 High / yuv420p` video and `AAC-LC` audio in the resulting MP4.
- The temporary verification file was removed.

Decision:

- Use yt-dlp's most robust current best-format selector rather than the older video-only-first variant.
- Keep hardware H.264 transcoding because one universally usable QuickTime/Premiere MP4 is the product goal; accept one lossy generation in exchange for compatibility.
- Use explicit high-quality VideoToolbox and audio settings instead of relying on encoder defaults.

Next step:

- Download a representative 4K or VP9/AV1 source in the app and confirm the resulting file opens directly in both QuickTime and Premiere Pro.

### 2026-07-25 — Restored Previously Proven ffmpeg Scope

Purpose: fix a real regression where the supposedly universal MP4 still contained AV1 video and Opus audio and was rejected by QuickTime.

Modified:

- `src-tauri/src/lib.rs` — restored the previously proven `bestvideo+bestaudio/best` selectors, including all resolution caps.
- Restored the global `ffmpeg:-c:v h264_videotoolbox -c:a aac` postprocessor arguments exactly as they were before the high-quality pipeline experiment.

Verification:

- ffprobe showed the failed real download contained `AV1 + Opus` inside an MP4 container, confirming that the scoped `ffmpeg_o` arguments had not transcoded the yt-dlp merge.
- Ran the restored VideoToolbox/AAC arguments against one second of that exact AV1/Opus file.
- ffprobe confirmed the restored result contained `H.264 High / yuv420p` video and `AAC-LC` audio.
- `cargo check` completed successfully.
- Removed the temporary verification file.

Decision:

- Real end-to-end compatibility takes precedence over theoretically cleaner postprocessor scoping.
- Revert the complete experimental selector/quality change rather than retaining unproven pieces of it.
- The already-downloaded AV1/Opus file is not modified automatically and must be downloaded again.

Next step:

- Redownload the affected video and confirm the new file opens directly in QuickTime.

### 2026-07-25 — Advisory URL Validation Instead of False Blocking

Purpose: allow known-downloadable links to reach the authoritative yt-dlp download even when the lightweight preview probe is slow or inconclusive.

Modified:

- `src/App.tsx` — added an `unverified` probe state for syntactically valid URLs whose metadata check times out or fails.
- Added a four-second soft deadline: after it expires, the form becomes usable while the background probe may still complete and upgrade the state to verified.
- The primary action now remains enabled for `unverified` links and is labelled `Спробувати завантажити`.
- Kept malformed URLs as the only frontend validation state that blocks an attempted download.
- `src/App.css` — added a distinct amber warning state and explanatory message so uncertainty is not presented as a definite broken-link error.

Verification:

- `npm run build` completed successfully.
- TypeScript compilation confirmed all probe-state branches and button conditions are valid.

Decision:

- Treat preview validation as advisory. The real yt-dlp extraction/download is authoritative for every syntactically valid HTTP(S) URL.
- Prefer a possible later download error over a false negative that prevents a supported site from being attempted.

Next step:

- Paste the reported VK Video URL again; after at most four seconds the amber attempt action should become available even if preview metadata remains unavailable.

### 2026-07-25 — Dynamic Progress Typography

Purpose: make the pink percentage or conversion-format label visually balance the complete title/status text block in the download popup.

Modified:

- `src/App.tsx` — measures the rendered title and status block with `ResizeObserver` and dynamically scales the progress value from 24 to 68 px as that text grows across more lines.
- `src/App.css` — created a stable 180 px progress-value column with tabular numerals, tight display spacing, and vertical centering so changing percentages do not make the surrounding text jump.
- The same dynamic treatment applies to the `MP4` label during conversion.

Verification:

- `npm run build` completed successfully.
- TypeScript compilation confirmed the measurement lifecycle and dynamic inline font size are valid.

Decision:

- Size the progress typography from the actual rendered text height rather than estimating line count from message length.
- Cap the display size at 68 px and reserve its column width to prevent feedback loops where a larger percentage causes additional wrapping.

Next step:

- Review the popup during both downloading and MP4 conversion at native application size.

### 2026-07-26 — Reliable Cancel for yt-dlp and ffmpeg

Purpose: restore the popup Stop action when yt-dlp has spawned downloader or ffmpeg child processes.

Modified:

- `src-tauri/src/lib.rs` — starts every download in its own Unix process group and cancels the complete group, including yt-dlp and all spawned ffmpeg/downloader children, instead of killing only the parent process.
- `src/App.tsx` — immediately changes the successful Cancel action to the visible `Скасовано` state instead of waiting for the monitor thread to observe process termination.
- `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock` — added an explicit `libc` dependency for reliable Unix process-group signalling.

Verification:

- `cargo check` completed successfully.
- `npm run build` completed successfully.

Decision:

- Use a dedicated process group per download so cancellation remains correct during direct download, merging, and VideoToolbox conversion.
- Preserve forceful cancellation semantics to stop immediately, matching the previous direct child `kill` behavior.

Next step:

- Start a test download, press Stop during progress, and confirm the popup immediately changes to `Скасовано` and network/ffmpeg activity ends.

### 2026-07-26 — Single-Step Confirmed Cancellation

Purpose: remove the redundant cancelled-result screen and its second Close action.

Modified:

- `src/App.tsx` — the Stop action now opens a focused confirmation dialog with `Продовжити` and `Так, скасувати` choices.
- After successful cancellation, the complete download popup closes immediately.
- A native `cancelled` event also removes the popup directly, preventing any transient cancelled-result card.
- Added cancellation busy and inline error states so repeated confirmation clicks are blocked while the process group is being terminated.
- `src/App.css` — added the red cancellation confirmation treatment consistent with the existing modal system.

Verification:

- `npm run build` completed successfully.
- TypeScript compilation confirmed the confirmation, busy, terminal-event, and popup-removal paths are valid.

Decision:

- Cancellation is a terminal action with confirmation; it should not require a second dismissal step.
- Keep completed and failed result popups dismissible because those states contain useful outcome information.

Next step:

- Verify Stop → confirmation → `Так, скасувати` terminates activity and returns directly to the main form.

### 2026-07-26 — App-Managed FFmpeg Conversion Progress

Purpose: replace the opaque yt-dlp conversion stage with a separately managed ffmpeg process that exposes real progress.

Modified:

- `src-tauri/src/lib.rs` — video downloads now finish as a lossless intermediate MKV, and yt-dlp reports each final intermediate filepath through its documented `after_move:filepath` print interface.
- The application launches the managed ffmpeg itself with `-progress pipe:1` and `-stats_period 0.25`, reads `out_time_us` and `speed`, and calculates actual conversion percentage and ETA from ffprobe duration.
- Final output uses H.264 VideoToolbox video, AAC audio, `avc1`, and fast-start MP4 for QuickTime and Premiere compatibility.
- Conversion writes to a unique temporary file and publishes the MP4 only after success. Failed or cancelled conversion removes only the partial output and preserves the downloaded intermediate source.
- Existing output filenames are never overwritten; a numbered MP4 filename is selected when needed.
- Playlist and carousel items convert sequentially with aggregate progress across all downloaded files.
- The active-process registry now switches from yt-dlp to ffmpeg, preserving process-group cancellation during conversion.
- `src/App.tsx` — added a distinct real-progress conversion state. The popup retains the large `MP4` label while showing a determinate progress bar, percentage, ffmpeg speed, and ETA.
- Short yt-dlp merging/remuxing remains visibly indeterminate and is described separately from final encoding.
- Rust formatting was normalized by `cargo fmt`.

Verification:

- `cargo check` completed successfully.
- `npm run build` completed successfully.
- A five-second native VideoToolbox conversion emitted documented ffmpeg progress blocks with `out_time_us`, `speed`, and `progress=continue/end`.
- ffprobe confirmed the test output is MP4 with H.264/`avc1` video and AAC/`mp4a` audio.

Decision:

- Use yt-dlp only for extraction, download, and lossless stream merging; do not rely on yt-dlp's hidden postprocessor execution for the long compatibility transcode.
- Keep the intermediate MKV when conversion fails so the successfully downloaded source can be recovered or retried.

Next step:

- Run a native end-to-end download and visually confirm that the popup transitions from download progress to short merge activity and then to real MP4 conversion progress.

### 2026-07-26 — Pre-Download Size and Free-Space Advisory

Purpose: estimate storage requirements before media transfer begins and warn when the selected destination may not have the requested two-times safety margin.

Modified:

- `src-tauri/src/lib.rs` — added a documented yt-dlp `video:` print marker that reports selected-format `filesize`/`filesize_approx` before the first download byte.
- Added a bitrate-times-duration fallback for extractors that do not report a direct size estimate.
- Added a fast filesystem free-space check for the selected output directory using `statvfs` on Unix/macOS.
- Required working space is intentionally calculated as twice the estimated media size to cover the downloaded intermediate and the temporary compatible MP4.
- Playlist/carousel estimates accumulate as yt-dlp resolves each item.
- Storage checks emit advisory events only; insufficient space never cancels or blocks the download.
- Explicitly retained `--no-simulate` and `--progress` because yt-dlp's `--print` option otherwise changes those defaults.
- `src/App.tsx` — stores size estimates even if the event arrives before the native start command returns, preventing an early-event race.
- The progress popup now shows estimated file size, required working space, and currently available space.
- `src/App.css` — added compact normal and amber insufficient-space treatments inside the progress popup.

Verification:

- `cargo check` completed successfully.
- `npm run build` completed successfully.
- A no-download yt-dlp test returned the selected video-plus-audio estimate before transfer as a single machine-readable line.
- The tested 720p selection reported approximately 128.8 MB, matching the sum of its selected video and audio streams.

Decision:

- Keep the check advisory because reported sizes are estimates and some streaming services expose only bitrate-based approximations.
- Run the size calculation inside the authoritative download extraction instead of adding another slow metadata probe.

Next step:

- Start a native download and confirm the storage line appears before download progress; optionally test a nearly full destination to review the amber warning.

### 2026-07-26 — Numbered Download Stages and Folder Capacity

Purpose: make the popup's left rail communicate workflow state and simplify all storage messaging to available versus required space.

Modified:

- `src/App.tsx` — replaced the active download glyph in the popup rail with numbered stages: `0` connecting, `1` downloading, and `2` merging/converting; successful completion still becomes a checkmark and failure becomes an X.
- Simplified the storage estimate copy to show only currently available space and the maximum required working space (the existing two-times estimate).
- Added a live free-space value below the selected output path and refresh it when the folder or download stage changes.
- `src-tauri/src/lib.rs` — exposed a lightweight `folder_free_space` command backed by the existing filesystem capacity check.
- `src/App.css` — styled the large stage number and added a subdued free-space line to the folder selector without crowding its path.

Verification:

- `cargo check` completed successfully.
- `npm run build` completed successfully.
- The running Tauri development application rebuilt and restarted successfully.

Decision:

- Follow the explicit zero-based workflow requested by the user: connection `0`, download `1`, conversion `2`, then completion checkmark.
- Keep storage language focused on the actionable comparison: `Вільно` versus `потрібно до`.

Next step:

- Review the stage rail and folder-space line in the native window at both compact and full-screen sizes.

### 2026-07-26 — Automatic YouTube 403 Recovery

Purpose: recover large YouTube downloads that begin normally but receive HTTP 403 at a repeatable early percentage, including the reported 4K `f401` stream failure at approximately 3%.

Modified:

- `src-tauri/src/lib.rs` — records the last actionable yt-dlp error instead of discarding it at terminal failure.
- Added up to two automatic recovery attempts for YouTube HTTP 403 failures.
- The first recovery re-extracts a fresh signed media URL and resumes the existing `.part` file.
- The second recovery additionally prefers the no-token `web_embedded` client while retaining `android_vr` as fallback.
- The active process registry switches to each retry child, preserving cancellation.
- Storage estimates are keyed by media ID so retry extraction replaces the previous estimate instead of double-counting it.
- `src/App.tsx` — added a reconnecting state that returns the stage rail to `0`, explains that the stream is being refreshed, and returns to stage `1` when progress resumes.
- Final failures now display the captured yt-dlp reason in the popup.

Verification:

- Reproduced the same public video and format selection (`401+251`) in a bounded yt-dlp test.
- Confirmed the reported symptom exactly matches yt-dlp's documented/known pattern: large format `f401` can receive HTTP 403 at roughly 3%.
- Confirmed the same video exposes the selected 4K formats through the no-token `web_embedded` fallback client.
- `cargo check` completed successfully.
- `npm run build` completed successfully.

Decision:

- Treat this specific mid-transfer YouTube 403 as recoverable rather than immediately terminal.
- Preserve partial data and resume it after re-extraction; do not restart a gigabyte-scale download from zero.
- Limit automatic recovery to two attempts so a persistent server/IP restriction cannot loop indefinitely.

Next step:

- Retry the same video; the existing 30 MB `.part` file should resume automatically and visibly reconnect if YouTube returns another 403.

### 2026-07-26 — Disconnected Drive Fallback

Purpose: prevent a previously selected external SSD from remaining the active destination after it is disconnected.

Modified:

- `src/App.tsx` — the selected output folder is checked immediately and every 2.5 seconds using the existing free-space command.
- If the selected folder becomes unavailable, the application automatically switches to the operating system's Downloads directory.
- The Downloads fallback is written to local storage so the disconnected external path does not return on the next launch.
- The same validation also repairs a stale external-drive path restored from a previous session.

Verification:

- `npm run build` completed successfully.
- The running Tauri development application received the frontend update through hot reload.

Decision:

- Treat folder unavailability as a stale destination selection and recover silently to Downloads.
- Do not automatically switch back if the external drive is later reconnected; the user must explicitly select it again.

Next step:

- Disconnect the selected external SSD and confirm the folder row changes to Downloads within approximately 2.5 seconds.

### 2026-07-26 — Conversion-Aware Storage Estimate and Friendly Link Errors

Purpose: replace the misleading two-times source-size warning with a realistic conversion-aware working-space estimate, and make failed-link reasons understandable without exposing raw yt-dlp diagnostics.

Modified:

- `src-tauri/src/lib.rs` — selected media estimates now include duration, resolution, FPS, and source bitrate metadata.
- Added a high-quality H.264 bitrate profile by resolution and frame rate, and applied the same profile to both the pre-download estimate and the VideoToolbox conversion.
- Required working space now sums the downloaded intermediate file and a conservative maximum final MP4 size, including audio and container overhead; playlist items are accumulated without double-counting retries.
- The final MP4 conversion now sets explicit video bitrate, maximum rate, buffer size, and 192 kbps AAC audio so output size is bounded and the pre-download estimate remains meaningful.
- ffprobe now reads duration, height, and average frame rate before conversion.
- Added Ukrainian user-facing explanations for truncated links, private or removed media, login/cookie requirements, unsupported services, DRM, regional restrictions, missing pages, and access rejection.
- Both URL probes and terminal download failures now use the friendly error mapping while internal raw errors remain available for automatic YouTube 403 recovery.

Verification:

- `cargo fmt` completed successfully.
- `cargo check` completed successfully.
- `npm run build` completed successfully.
- `git diff --check` reported no whitespace errors.

Decision:

- Exact H.264 file size cannot be known before encoding, so the UI reports a conservative working maximum derived from a controlled output bitrate rather than pretending the compressed source size predicts the final file.
- Preserve high visual quality while bounding 4K/60 fps output separately from lower-resolution and lower-frame-rate media.

Next step:

- Verify the previously reported 4K link and a deliberately truncated YouTube link in the restarted native application.

### 2026-07-26 — Clickable Failed-State Rail

Purpose: make the large X in the pink error rail behave as the close control users naturally expect.

Modified:

- `src/App.tsx` — changed the failed-state rail from a decorative container into an accessible close button with the same action as the small X on the right.
- Improved the completed-state button label for screen readers.
- `src/App.css` — added failed-state hover feedback and a visible keyboard focus treatment.

Verification:

- `npm run build` completed successfully.
- `git diff --check` reported no whitespace errors.

Decision:

- Keep the pink rail clickable only after completion or failure; active download stages remain non-clickable so they cannot bypass the existing cancellation confirmation.

Next step:

- Confirm that clicking either X closes a failed download popup.

### 2026-07-26 — Clear Download Field Label

Purpose: remove the unintended login association from the download form section marker.

Modified:

- `src/App.css` — changed the decorative section label from `01 / ВХІД` to `01 / ПОЛЕ`.

Verification:

- Confirmed that authentication-related copy remains unchanged and only the form marker was renamed.

Decision:

- Use `ПОЛЕ` as a neutral interface label; reserve `вхід` exclusively for actual browser authentication states.

Next step:

- Await further interface review.

### 2026-07-26 — GitHub Publication Preparation

Purpose: prepare the complete current application for GitHub publication and replace the outdated public project description.

Modified:

- `README.md` — updated the product name, summary, and feature list to match the current native macOS application.
- Removed the public mention of the internal `.ru`/`.рф` VPN warning while leaving the internal feature itself unchanged.
- Included current capabilities such as fast link previews, Ukrainian error explanations, staged progress, compatible H.264/AAC output, storage estimation, playlist pacing, interrupted-download recovery, and disconnected-drive fallback.

Verification:

- Reviewed ignored files and confirmed `.secrets`, build output, dependencies, and signing credentials remain excluded from Git.
- Scanned the publishable working tree for common private-key and token signatures; no embedded credentials were found.
- Confirmed the existing remote points to `BaldojniSylyUkrainy/yt-dlp-BD`.

Decision:

- Keep the VPN warning as an undocumented internal safety feature.
- Publish the full current working tree, including the AI work log and generated application icon assets, while excluding ignored build artifacts and secrets.

Next step:

- Complete GitHub re-authentication, identify the exact obsolete repository, publish the current repository, update its GitHub description, and then remove only the confirmed obsolete repository.

### 2026-07-26 — Current Repository Published to GitHub

Purpose: publish the complete current application and replace the outdated public-facing repository state.

Published:

- Pushed commit `5f590df` (`Publish current Baldojnyi Downloader app`) to the `main` branch of `BaldojniSylyUkrainy/yt-dlp-BD`.
- Updated the GitHub description to: `Нативний macOS-завантажувач відео й аудіо з українським інтерфейсом на базі yt-dlp.`
- Added the public topics `macos`, `tauri`, `ukrainian`, and `yt-dlp`.

Verification:

- Confirmed the repository is public at `https://github.com/BaldojniSylyUkrainy/yt-dlp-BD`.
- Confirmed local `main` and GitHub `main` both point to commit `5f590dff4bc4a865691df498efd599ff29689529`.
- Confirmed the updated `README.md` is available on GitHub and does not advertise the internal `.ru`/`.рф` VPN warning.

Decision:

- The GitHub account contains only one repository, and that repository is also this project's configured remote. It was updated in place instead of deleting the repository entity, preserving its URL and update infrastructure while replacing the outdated default-branch contents.

Next step:

- If explicitly required, replace the Git history with a fresh single-snapshot history; this is separate from the now-completed publication and would remove rollback history.
