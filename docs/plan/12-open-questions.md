# 12 — Open questions

Decisions the plan made provisionally that the owner may want to overrule. Each
has the default the implementation follows until told otherwise.

Answered by the owner (2026-11-04): the name (`dsplr`), the GUI stack (crcbl, as
pikr and lyr), and X11 in the MVP.

1. **Wayland protocol bindings.** Default: `wayland-client` +
   `wayland-protocols-wlr` + `wayland-protocols-plasma` from crates.io. The
   alternative is generating them with crcbl's own protocol scanner
   (`crcbl-wl-scanner`), which keeps dsplr on one Wayland stack with its GUI but
   means a second client connection either way. New dependencies; needs the
   owner's approval before M2.
2. **X11 bindings.** Default: `x11rb` with the `randr` feature. Alternative:
   crcbl-shell's own X11 FFI, which would have to grow RandR. New dependency;
   needs approval before M7.
3. **Async runtime.** Default: a current-thread tokio runtime on the backend
   thread, because zbus is house precedent with tokio. Alternative: zbus's own
   executor and plain blocking loops for the Wayland and X11 backends.
4. **How Hyprland outputs are named in `monitors.lua`.** Default: by description
   (`desc:…`) when the serial is non-empty, so settings follow the monitor
   across ports. nwg-displays writes connector names; switching costs nothing on
   the owner's single external monitor but changes the file.
5. **Confirm timeout.** Default: 20 seconds, the same as Mutter's
   `DEFAULT_DISPLAY_CONFIGURATION_TIMEOUT`, in the `CONFIRM_TIMEOUT` constant
   with a config override. KDE's own value was not checked.
6. **Linux only.** Default: yes; Windows and macOS declined. Revisit only if
   crcbl's Win32 and AppKit shells make a port nearly free and someone asks.
7. **HDR and colour management.** Default: read-only display of what Hyprland
   and KDE report; setting them is post-MVP.
8. **KDE protocol bindings.** KWin 6.8 has custom modes and mirroring
   (`set_custom_modes`, `set_replication_source`) that
   `wayland-protocols-plasma` 0.3.12 does not carry. Default: ship KDE without
   both in the MVP and mark them `Unsupported`; the alternative is generating
   bindings from KWin's own protocol XML, which tracks KWin but takes on its
   unversioned changes.
9. **MVP scope.** The owner put X11 in the MVP (2026-11-04), so M0–M7 is the
   MVP: five backends, an EDID parser and the xrandr CRTC algorithm. The owner's
   own need is met at M3. Raised in review as a scope risk; the owner's decision
   stands unless they change it.
