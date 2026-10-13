# SPDX-License-Identifier: GPL-3.0-or-later
# Einstellungen für dmgbuild (aufgerufen von scripts/make-dmg.sh): Fenster mit der App links,
# Verknüpfung zu „Programme“ rechts und dem Hintergrund aus assets/dmg-background.tiff.
import os.path

app = defines["app"]  # noqa: F821 (von dmgbuild gesetzt)
root = defines["root"]  # noqa: F821

format = "UDZO"
filesystem = "HFS+"
files = [app]
symlinks = {"Applications": "/Applications"}
hide_extensions = [os.path.basename(app)]

background = os.path.join(root, "assets", "dmg-background.tiff")
icon = os.path.join(root, "assets", "FadeWerk DJ.icns")
# Höher als die 400 pt Inhalt: neuere Finder blenden Tab- und Pfadleiste trotzdem ein
window_rect = ((200, 120), (640, 500))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
icon_size = 112
text_size = 13
icon_locations = {
    os.path.basename(app): (150, 178),
    "Applications": (490, 178),
}
