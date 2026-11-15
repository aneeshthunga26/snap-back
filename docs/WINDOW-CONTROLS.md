# Desktop window controls

Snap Back uses a 40 px caption area integrated into its interface, with Iced window decorations disabled. Dragging the empty caption moves the window; double-clicking toggles maximization. Caption buttons request minimize, maximize/restore, and close through Iced's window API. Narrow edge and corner regions allow resizing an unmaximized window.

Restored windows have subtle 8 px rounded corners on a transparent surface. Iced's antialiased background and sidebar edges form the shape, and the main scrollbar ends above the bottom corner. Maximizing sets the radius and scrollbar inset to zero. Content is drawn once, keeping the software renderer responsive. Sidebar hover and selection colors use a bounded 150 ms eased transition so slower frames cannot make their colors overshoot.

## Source review

T3 Code was pulled into `/home/aneesh/.cache/snap-back-reference/t3code` and reviewed at commit `7f81aa8386737ecb244e3fd21c95afdb0649aeb7`. Its [DesktopWindow.ts](https://github.com/pingdotgg/t3code/blob/7f81aa8386737ecb244e3fd21c95afdb0649aeb7/apps/desktop/src/window/DesktopWindow.ts) requests Electron's hidden title bar with a 40 px window-controls overlay. Electron supplies the caption buttons; T3 does not paint a separate set of React caption icons.

Electron's [FreedesktopNavButtonProvider](https://github.com/electron/electron/blob/main/shell/browser/ui/views/freedesktop_nav_button_provider.cc) obtains standard Freedesktop symbolic icons from the installed theme and applies Breeze or Adwaita button styling. Snap Back implements the corresponding integration in Rust, keeping its existing Iced renderer.

## Theme and layout

At startup, Snap Back reads the KDE icon theme and KWin button placement, or GNOME's icon theme and button layout through GSettings. Other GTK desktops can supply theme and decoration-layout settings through their GTK settings files. Explicitly empty button groups and omitted controls are respected. Left-side controls occupy a caption strip above both panes so they remain at the window edge.

The icon names are `window-minimize-symbolic`, `window-maximize-symbolic`, `window-restore-symbolic`, and `window-close-symbolic`. Theme inheritance is followed in order, with cycle protection and a hicolor fallback. Cached SVG icons are recolored to match the app. Missing icons, including themes that provide only raster icons, use built-in glyphs.

Breeze uses small glyphs without a resting background and circular hover feedback; Adwaita uses circular buttons. Inactive windows dim their controls. The VM resolves all four icons from its installed `breeze-dark` theme.

This reads desktop preferences and icon themes; it does not load KWin/Aurorae or Qt decoration plugins. Theme and button-layout changes take effect when the app is reopened. GNOME parsing is covered by VM-run regression tests; the graphical desktop available for interactive verification is KDE Plasma.
