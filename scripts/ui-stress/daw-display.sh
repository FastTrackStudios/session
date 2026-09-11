#!/usr/bin/env bash
# Where measurement windows open.
#
# A benchmark that opens on the screen you are working on is a benchmark
# you cannot leave running. This puts them on the RIGHT-HAND display and
# keeps the centre ultrawide free.
#
# The layout on THEBATTLESHIP, left to right (kscreen-doctor):
#   DP-3  x=0     1440x2560  portrait, left
#   DP-2  x=1440  5120x1440  ultrawide, centre
#   DP-1  x=6560  2560x1440  RIGHT  <- here
#
# Placement only works under X11/XWayland: a Wayland client cannot
# position its own surface, so a native-Wayland run lands wherever the
# compositor decides. Sourcing this therefore also selects XWayland,
# which measured the same frame rate as native Wayland when tested.
#
#   source scripts/ui-stress/daw-display.sh

# The right display's origin, and a 1600x900 window centred on it.
FTS_RIGHT_X="${FTS_RIGHT_X:-6560}"
FTS_RIGHT_Y="${FTS_RIGHT_Y:-0}"
FTS_RIGHT_W="${FTS_RIGHT_W:-2560}"
FTS_RIGHT_H="${FTS_RIGHT_H:-1440}"

export FTS_WINDOW_POS="$(( FTS_RIGHT_X + (FTS_RIGHT_W - 1600) / 2 )),$(( FTS_RIGHT_Y + (FTS_RIGHT_H - 900) / 2 ))"
export DISPLAY="${DISPLAY:-:0}"
export GDK_BACKEND=x11
