#!/usr/bin/env python3
"""Turn a Logi Options+ settings database into an OpenLogi config.

Reads the Options+ `settings.db` (a SQLite file whose newest `data` row holds
one JSON document) and prints an OpenLogi `config.toml` for the mice it knows
about: the gesture button's actions, the buttons Options+ has remapped to
something OpenLogi can express, and smooth scrolling.

It only ever reads the database and never touches an existing config. By
default the result goes to stdout so it can be reviewed; `--output` writes a
new file and refuses to replace one that exists. Merge the result into your
own `config.toml` by hand if you already have one.

Whatever cannot be translated is listed on stderr instead of guessed at.
Standard library only.
"""

from __future__ import annotations

import argparse
import json
import re
import sqlite3
import sys
from pathlib import Path
from typing import Dict, List, Optional, Tuple

# The config schema the generated TOML is written for
# (`SCHEMA_VERSION` in crates/openlogi-core/src/config.rs). OpenLogi migrates an
# older file on load, so a stale value stays loadable; the unit tests compare
# it with the source so a bump does not go unnoticed.
SCHEMA_VERSION = 10

DEFAULT_DB = (
    Path.home() / "Library" / "Application Support" / "LogiOptionsPlus" / "settings.db"
)

# Options+ names a button's slot `<device slot prefix>_<suffix>`. These are the
# HID++ control ids (decimal) of the buttons OpenLogi can rebind, as OpenLogi's
# `ButtonId` names them.
BUTTON_SLOTS = {
    "c82": "MiddleClick",
    "c83": "Back",
    "c86": "Forward",
    "c195": "GestureButton",
    "c196": "DpiToggle",
}

# Options+ gesture slots, as OpenLogi's `GestureDirection` names them.
GESTURE_DIRECTIONS = {
    "click": "Click",
    "up": "Up",
    "down": "Down",
    "left": "Left",
    "right": "Right",
}

QUICK_LAUNCH_ACTIONS = {
    "MISSION_CONTROL": "MissionControl",
    "APP_EXPOSE": "AppExpose",
    "LAUNCHPAD": "LaunchpadShow",
}
SYSTEM_ACTIONS = {
    "SWITCH_BETWEEN_DESKTOPS_LEFT": "PreviousDesktop",
    "SWITCH_BETWEEN_DESKTOPS_RIGHT": "NextDesktop",
}
MEDIA_ACTIONS = {
    "PLAY_PAUSE": "PlayPause",
    "VOLUME_UP": "VolumeUp",
    "VOLUME_DOWN": "VolumeDown",
    "NEXT_TRACK": "NextTrack",
    "PREVIOUS_TRACK": "PrevTrack",
}
# USB HID button usages: 1 primary, 2 secondary, 3 middle, 4 back, 5 forward.
MOUSE_BUTTON_ACTIONS = {
    1: "LeftClick",
    2: "RightClick",
    3: "MiddleClick",
    4: "MouseBack",
    5: "MouseForward",
}

# A serial becomes part of a TOML table name; anything else is not a serial
# Options+ would have written.
SERIAL_RE = re.compile(r"^[A-Za-z0-9._-]+$")

DEFAULT_PROFILE_NAME = "PROFILE_NAME_DEFAULT"


class ImportFailure(Exception):
    """The database is missing, unreadable, or not an Options+ database."""


def read_settings(db_path: Path) -> dict:
    """Return the newest settings document in an Options+ `settings.db`."""
    if not db_path.is_file():
        raise ImportFailure(f"no Logi Options+ database at {db_path}")
    # Read-only, so a running Options+ is never disturbed.
    uri = db_path.resolve().as_uri() + "?mode=ro"
    try:
        con = sqlite3.connect(uri, uri=True)
        try:
            row = con.execute(
                "SELECT file FROM data ORDER BY _id DESC LIMIT 1"
            ).fetchone()
        finally:
            con.close()
    except sqlite3.Error as error:
        raise ImportFailure(f"cannot read {db_path}: {error}") from error
    if row is None:
        raise ImportFailure(f"{db_path} holds no settings")
    try:
        data = json.loads(row[0])
    except (TypeError, ValueError) as error:
        raise ImportFailure(f"{db_path} holds an unreadable settings document") from error
    if not isinstance(data, dict):
        raise ImportFailure(f"{db_path} holds an unexpected settings document")
    return data


def device_serials(data: dict) -> Dict[str, str]:
    """Map each Options+ device slot prefix to the device's serial number.

    OpenLogi keys a device by `serial:<serial>` (lower-cased), so a device is
    only importable when Options+ recorded its serial.
    """
    devices = (data.get("ever_connected_devices") or {}).get("devices") or []
    serials: Dict[str, str] = {}
    for device in devices:
        prefix = device.get("slotPrefix")
        serial = device.get("serialNumber")
        if prefix and isinstance(serial, str) and SERIAL_RE.match(serial):
            serials[prefix] = serial
    return serials


def default_profile(data: dict) -> Optional[dict]:
    """The profile that applies outside any per-application override."""
    profiles = [data.get(key) for key in data.get("profile_keys") or []]
    profiles = [p for p in profiles if isinstance(p, dict)]
    for profile in profiles:
        if profile.get("name") == DEFAULT_PROFILE_NAME:
            return profile
    return None


def card_action(card: Optional[dict]) -> Optional[str]:
    """The OpenLogi action an Options+ card performs, or `None` if unknown."""
    macro = (card or {}).get("macro") or {}
    kind = macro.get("type")
    if kind == "QUICK_LAUNCH":
        return QUICK_LAUNCH_ACTIONS.get((macro.get("quickLaunch") or {}).get("action"))
    if kind == "SYSTEM":
        return SYSTEM_ACTIONS.get((macro.get("system") or {}).get("action"))
    if kind == "MEDIA":
        return MEDIA_ACTIONS.get((macro.get("media") or {}).get("usage"))
    if kind == "DO_NOTHING":
        return "None"
    if kind == "MOUSE":
        mouse = macro.get("mouse") or {}
        if mouse.get("action") == "BUTTON":
            return MOUSE_BUTTON_ACTIONS.get(mouse.get("hidUsage"))
    return None


def card_label(card: Optional[dict]) -> str:
    """A name for a card in the report of what was left out."""
    card = card or {}
    return str(card.get("name") or card.get("id") or "unnamed assignment")


class Device:
    """What was learned about one mouse."""

    def __init__(self, serial: str) -> None:
        self.serial = serial
        self.bindings: Dict[str, str] = {}
        self.gesture: Dict[str, str] = {}
        self.smooth_wheel = False

    @property
    def key(self) -> str:
        return "serial:" + self.serial.lower()

    def is_empty(self) -> bool:
        return not (self.bindings or self.gesture or self.smooth_wheel)


def gesture_actions(card: dict, skipped: List[str], device: str) -> Dict[str, str]:
    """The five direction actions of the gesture mode the user selected."""
    selected = card.get("selectedNestedCard")
    mode = (card.get("nestedCards") or {}).get(selected)
    sub_cards = (mode or {}).get("nestedCards")
    if not isinstance(sub_cards, dict):
        skipped.append(f"{device}: gesture mode {selected!r} is not a four-way mode")
        return {}
    actions: Dict[str, str] = {}
    for options_name, direction in GESTURE_DIRECTIONS.items():
        sub = sub_cards.get(options_name)
        action = card_action(sub)
        if action is None:
            skipped.append(
                f"{device}: gesture {direction.lower()} ({card_label(sub)}) has no OpenLogi equivalent"
            )
        else:
            actions[direction] = action
    return actions


def extract_devices(data: dict) -> Tuple[List[Device], List[str]]:
    """Read every importable device and list what could not be imported."""
    skipped: List[str] = []
    serials = device_serials(data)
    profile = default_profile(data)
    if profile is None:
        raise ImportFailure("no default profile found in the Logi Options+ settings")
    if len(data.get("profile_keys") or []) > 1:
        skipped.append("per-application profiles are not imported")

    devices: Dict[str, Device] = {}
    for assignment in profile.get("assignments") or []:
        prefix, _, suffix = str(assignment.get("slotId", "")).partition("_")
        serial = serials.get(prefix)
        if serial is None:
            continue  # a virtual device, or one Options+ never recorded a serial for
        device = devices.setdefault(prefix, Device(serial))
        card = assignment.get("card") or {}
        if suffix == "mouse_scroll_wheel_settings":
            settings = card.get("mouseScrollWheelSettings") or {}
            device.smooth_wheel = settings.get("isSmooth") is True
        elif suffix == "c195" and card.get("attribute") == "ONE_OF":
            device.gesture = gesture_actions(card, skipped, prefix)
        elif suffix in BUTTON_SLOTS:
            action = card_action(card)
            if action is not None:
                device.bindings[BUTTON_SLOTS[suffix]] = action
            elif card.get("macro"):
                skipped.append(
                    f"{prefix}: {BUTTON_SLOTS[suffix]} ({card_label(card)}) has no OpenLogi equivalent"
                )

    return [d for d in devices.values() if not d.is_empty()], skipped


def toml_string(value: str) -> str:
    """A TOML basic string (JSON string escaping is a subset of it)."""
    return json.dumps(value)


def render_config(devices: List[Device]) -> str:
    """The OpenLogi `config.toml` for `devices`."""
    lines = [f"schema_version = {SCHEMA_VERSION}"]
    for device in sorted(devices, key=lambda d: d.key):
        table = f"devices.{toml_string(device.key)}"
        lines += ["", f"[{table}]"]
        if device.smooth_wheel:
            lines.append("smooth_wheel = true")
        if device.bindings:
            lines += ["", f"[{table}.bindings]"]
            lines += [
                f"{button} = {toml_string(action)}"
                for button, action in sorted(device.bindings.items())
            ]
        if device.gesture:
            lines += ["", f"[{table}.bindings.GestureButton]"]
            lines += [
                f"{direction} = {toml_string(device.gesture[direction])}"
                for direction in GESTURE_DIRECTIONS.values()
                if direction in device.gesture
            ]
    return "\n".join(lines) + "\n"


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--db",
        type=Path,
        default=DEFAULT_DB,
        help="Options+ settings.db (default: the macOS location)",
    )
    parser.add_argument(
        "-o",
        "--output",
        type=Path,
        help="write the config to this new file instead of stdout; an existing file is never replaced",
    )
    args = parser.parse_args(argv)

    try:
        devices, skipped = extract_devices(read_settings(args.db))
        if not devices:
            raise ImportFailure("no importable settings found")
        config = render_config(devices)
        if args.output is None:
            sys.stdout.write(config)
        else:
            try:
                with args.output.open("x", encoding="utf-8") as out:
                    out.write(config)
            except FileExistsError:
                raise ImportFailure(
                    f"{args.output} already exists; refusing to replace it"
                ) from None
            print(f"wrote {args.output}", file=sys.stderr)
    except ImportFailure as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    for line in skipped:
        print(f"not imported: {line}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
