# THROWAWAY theme directions

Question: what should Retro, Cyberpunk and Omarchy-inspired file-browser themes look like?

These are design mocks, not supported themes or a release change. All three use
the real mock filesystem and virtualized view. The prototypes do not change
the installed app or persist a theme choice. No third-party icons or fonts are
bundled; folder icons are the project's own, tinted for each study.

```sh
npm run dev:mock -- --port 1422
```

- http://localhost:1422/?demo&variant=retro&path=/Users/me
- http://localhost:1422/?demo&variant=cyberpunk&path=/Users/me
- http://localhost:1422/?demo&variant=omarchy&path=/Users/me

The floating DESIGN MOCK bar switches variants without navigating away from
the folder. Retro has a classic menu strip and beveled controls; Cyberpunk
has bordered file tiles and a neon workstation status strip; Omarchy-inspired
has an inset tiling-window frame, warm terminal typography and quiet amber
selection. This is an original interpretation, not an official Omarchy theme.

The variant entry is guarded by Vite's development flag. Do not merge this
throwaway branch into main. Rewrite a chosen direction as a supported theme
with proper tokens, accessibility, light/dark behavior and tests.
