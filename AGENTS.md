# DGI agent instructions

These instructions apply across the repository.

## Ghost Acrylic design system

- Follow the Ghost Acrylic reference supplied for DGI. Use the cool white canvas (`#f8f9fb`), raised white surfaces, obsidian ink (`#0b0c0e`), soft ambient shadows, and locally bundled Plus Jakarta Sans. Keep the stipple inside asset previews and thumbnails. Use compact page titles and leave instructions next to the controls that need them. Red is reserved for errors and actions that can discard work.
- Treat `src/ui.ts` and `src/design-system.css` as the local component library. Follow a shadcn-style workflow by using the shared primitive first, adding a named variant when the same need recurs, and fixing shared behavior in the primitive. Keep feature-specific layout in the feature's own classes.
- Build action controls with `button()` and `icon()` from `src/ui.ts`. Use `primary` for the principal action, `secondary` for supporting actions, `ghost` for low emphasis, and `danger` for destructive actions. Use the `default`, `small`, and `icon` sizes consistently. Supply accessible names for icon-only buttons. Navigation, tabs, and asset selection should expose their current or selected state to assistive technology.
- Reuse `.panel`, `.field`, `.badge`, `.segment`, `.notice`, and `.empty-state` before creating new patterns. Use the shared variables for spacing, rounding, color, type, and elevation. Avoid page-specific button treatments, remote fonts, emoji icons, and decorative UI copy.
- Preserve keyboard navigation, visible focus, responsive layouts, reduced motion, and distinguishable hover, disabled, selected, loading, and error states. Review the app at its default window size and a narrow window before committing changes to layout or controls.
- On Windows, keep the custom title bar at the same height as the native caption controls. Reserve 138 logical pixels at the right edge and keep drag regions clear of interactive controls.

## Project workflow

- Keep the interface in TypeScript and place cache access, downloads, verification, and backup operations in Rust. Keep credentials out of files, logs, tests, screenshots, and commits. Do not overwrite the first original backup when replacing an asset again.
- Run `npm run build` for interface updates and the relevant Cargo checks for Rust changes. Verify affected workflows on disposable fixtures. Use focused Git commits with a clear summary and keep unrelated working tree changes out of the commit.
