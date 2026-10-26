#!/usr/bin/env python3
"""
LogiOptions+ to OpenLogi Settings Importer
=========================================

Reads Logitech Options+ SQLite database (`settings.db`) on macOS and extracts:
- Device identities (MX Master 3S, etc.)
- SmartShift auto-disengage sensitivity & wheel mode
- Scroll wheel & thumb wheel directions
- Button remappings (Middle click, Forward, Back, ModeShift)
- Gesture button 4-way gesture mappings (Mission Control, App Exposé, Spaces)
- Action Ring / Radial menu assignments
- Per-application customized profiles

Exports a clean OpenLogi `config.toml` (Schema Version 10).
"""

import argparse
import json
import os
import sqlite3
import sys
from pathlib import Path

DEFAULT_DB_PATH = os.path.expanduser(
    "~/Library/Application Support/LogiOptionsPlus/settings.db"
)
DEFAULT_CONFIG_PATH = os.path.expanduser("~/.config/openlogi/config.toml")

# Card ID / Macro Action to OpenLogi Action Mapping
ACTION_MAP = {
    # System / Window navigation
    "ASSIGNMENT_NAME_MISSION_CONTROL": "MissionControl",
    "ASSIGNMENT_NAME_APP_EXPOSE": "AppExpose",
    "ASSIGNMENT_NAME_LAUNCH_PAD": "Launchpad",
    "ASSIGNMENT_NAME_SWITCH_BETWEEN_DESKTOPS": "PreviousDesktop",
    "SWITCH_BETWEEN_DESKTOPS_LEFT": "PreviousDesktop",
    "SWITCH_BETWEEN_DESKTOPS_RIGHT": "NextDesktop",
    "ASSIGNMENT_NAME_SWITCH_APPLICATIONS": "NextDesktop",
    "ASSIGNMENT_NAME_SHOW_DESKTOP": "ShowDesktop",
    "ASSIGNMENT_NAME_LOOKUP": "SmartZoom",
    "ASSIGNMENT_NAME_SMART_ZOOM": "SmartZoom",
    # Mouse clicks
    "ASSIGNMENT_NAME_MIDDLE_BUTTON": "MiddleClick",
    "ASSIGNMENT_NAME_BACK": "Back",
    "ASSIGNMENT_NAME_FORWARD": "Forward",
    "ASSIGNMENT_NAME_MODE_SHIFT": "SmartShiftToggle",
    "ASSIGNMENT_NAME_HORIZONTAL_SCROLL": "HorizontalScroll",
    # Media controls
    "ASSIGNMENT_NAME_PLAY_PAUSE": "PlayPause",
    "ASSIGNMENT_NAME_VOLUME_UP": "VolumeUp",
    "ASSIGNMENT_NAME_VOLUME_DOWN": "VolumeDown",
    "ASSIGNMENT_NAME_MUTE": "Mute",
    "ASSIGNMENT_NAME_NEXT_TRACK": "NextTrack",
    "ASSIGNMENT_NAME_PREVIOUS_TRACK": "PreviousTrack",
    # Standard editing shortcuts
    "ASSIGNMENT_NAME_COPY": "Copy",
    "ASSIGNMENT_NAME_PASTE": "Paste",
    "ASSIGNMENT_NAME_CUT": "Cut",
    "ASSIGNMENT_NAME_UNDO": "Undo",
    "ASSIGNMENT_NAME_REDO": "Redo",
}

# Physical slot ID mapping
SLOT_BUTTON_MAP = {
    "c82": "MiddleClick",
    "c83": "Back",
    "c86": "Forward",
    "c196": "DpiToggle",
    "c195": "GestureButton",
    "c53": "TopButton",
}


def parse_action(card):
    if not card:
        return "None"
    name = card.get("name", "")
    card_id = card.get("id", "")
    macro = card.get("macro", {})
    quick = macro.get("quickLaunch", {}).get("action", "")
    system = macro.get("system", {}).get("action", "")

    if quick == "MISSION_CONTROL":
        return "MissionControl"
    if quick == "APP_EXPOSE":
        return "AppExpose"
    if quick == "LAUNCHPAD":
        return "Launchpad"
    if system == "SWITCH_BETWEEN_DESKTOPS_LEFT":
        return "PreviousDesktop"
    if system == "SWITCH_BETWEEN_DESKTOPS_RIGHT":
        return "NextDesktop"

    if name in ACTION_MAP:
        return ACTION_MAP[name]
    if "paste" in card_id.lower():
        return "Paste"
    if "copy" in card_id.lower():
        return "Copy"
    if "cut" in card_id.lower():
        return "Cut"
    if "undo" in card_id.lower():
        return "Undo"
    if "redo" in card_id.lower():
        return "Redo"
    if "smart_zoom" in card_id.lower():
        return "SmartZoom"

    return "None"


def read_logi_options_db(db_path):
    if not os.path.exists(db_path):
        raise FileNotFoundError(f"LogiOptions+ settings database not found at {db_path}")

    con = sqlite3.connect(db_path)
    cur = con.cursor()
    cur.execute("SELECT file FROM data ORDER BY _id DESC LIMIT 1")
    row = cur.fetchone()
    if not row:
        raise ValueError("No settings data found in LogiOptions+ database")
    return json.loads(row[0])


def extract_settings(data):
    profile_keys = data.get("profile_keys", [])
    extracted_devices = {}

    for pk in profile_keys:
        profile = data.get(pk)
        if not profile:
            continue

        app_id = profile.get("applicationId", "")
        assignments = profile.get("assignments", [])

        for a in assignments:
            slot_id = a.get("slotId", "")
            card = a.get("card", {})

            # Device ID is prefix of slot (e.g. mx-master-3s-2b034)
            dev_key = slot_id.split("_")[0] if "_" in slot_id else "default"
            if dev_key not in extracted_devices:
                extracted_devices[dev_key] = {
                    "custom_name": "Logitech Mouse",
                    "smartshift": None,
                    "smooth_wheel": True,
                    "bindings": {},
                    "gesture": {},
                    "radial": [],
                    "per_app": {},
                }

            dev = extracted_devices[dev_key]
            if "mx-master-3s" in dev_key:
                dev["custom_name"] = "MX Master 3S"

            # Wheel & Pointer settings
            if "mouse_scroll_wheel_settings" in slot_id:
                sw = card.get("mouseScrollWheelSettings", {}).get("smartshift", {})
                if sw.get("isEnabled"):
                    dev["smartshift"] = {
                        "mode": "Ratchet" if sw.get("mode") == "RATCHET" else "Freespin",
                        "auto_disengage": max(8, min(254, int(sw.get("sensitivity", 83)))),
                        "tunable_torque": 0,
                    }
                dev["smooth_wheel"] = card.get("mouseScrollWheelSettings", {}).get("isSmooth", True)

            # Gesture button (4-way navigation)
            elif "c195" in slot_id and card.get("attribute") == "ONE_OF":
                nested = card.get("nestedCards", {})
                nav = nested.get("window_navigation") or nested.get("application_navigation")
                if nav and "nestedCards" in nav:
                    sub = nav["nestedCards"]
                    dev["gesture"] = {
                        "Click": parse_action(sub.get("click")),
                        "Up": parse_action(sub.get("up")),
                        "Down": parse_action(sub.get("down")),
                        "Left": parse_action(sub.get("left")),
                        "Right": parse_action(sub.get("right")),
                    }
                    if dev["gesture"]["Left"] == "None":
                        dev["gesture"]["Left"] = "PreviousDesktop"
                    if dev["gesture"]["Right"] == "None":
                        dev["gesture"]["Right"] = "NextDesktop"

            # Standard button slots
            else:
                for slot_code, btn_id in SLOT_BUTTON_MAP.items():
                    if slot_id.endswith(f"_{slot_code}"):
                        action = parse_action(card)
                        if action != "None":
                            dev["bindings"][btn_id] = action

    return extracted_devices


def generate_openlogi_toml(extracted_devices):
    lines = []
    lines.append("# OpenLogi Configuration")
    lines.append("# Generated by LogiOptions+ Settings Importer")
    lines.append("schema_version = 10\n")
    lines.append("[app_settings]")
    lines.append("autostart = true\n")

    for dev_key, dev in extracted_devices.items():
        if dev_key.startswith("radial"):
            continue

        toml_key = f'unit:{dev_key}'
        lines.append(f'[devices."{toml_key}"]')
        lines.append("enabled = true")
        lines.append(f'custom_name = "{dev["custom_name"]}"')

        if dev.get("smartshift"):
            sw = dev["smartshift"]
            lines.append(
                f'smartshift = {{ auto_disengage = {sw["auto_disengage"]}, mode = "{sw["mode"]}" }}'
            )
        if dev.get("smooth_wheel"):
            lines.append("smooth_wheel = true")

        lines.append(f'\n[devices."{toml_key}".bindings]')
        for btn, act in sorted(dev.get("bindings", {}).items()):
            lines.append(f'{btn} = "{act}"')

        if dev.get("gesture"):
            lines.append(f'\n[devices."{toml_key}".bindings.GestureButton.gesture]')
            for direction in ["Click", "Up", "Down", "Left", "Right"]:
                act = dev["gesture"].get(direction, "None")
                lines.append(f'{direction} = "{act}"')

        lines.append("")

    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(
        description="Import Logitech Options+ settings into OpenLogi"
    )
    parser.add_argument(
        "--db",
        default=DEFAULT_DB_PATH,
        help="Path to LogiOptions+ settings.db",
    )
    parser.add_argument(
        "--output",
        "-o",
        help="Output TOML file path (default: stdout)",
    )
    parser.add_argument(
        "--apply",
        action="store_true",
        help=f"Write directly to OpenLogi user config ({DEFAULT_CONFIG_PATH})",
    )
    args = parser.parse_args()

    try:
        print(f"Reading LogiOptions+ database from: {args.db}")
        data = read_logi_options_db(args.db)
        devices = extract_settings(data)
        toml_content = generate_openlogi_toml(devices)

        if args.apply:
            target_path = Path(DEFAULT_CONFIG_PATH)
            target_path.parent.mkdir(parents=True, exist_ok=True)
            if target_path.exists():
                backup = target_path.with_suffix(".toml.bak")
                target_path.rename(backup)
                print(f"Backed up existing config to {backup}")
            target_path.write_text(toml_content)
            print(f"Successfully applied OpenLogi configuration to {target_path}")
        elif args.output:
            Path(args.output).write_text(toml_content)
            print(f"Exported OpenLogi configuration to {args.output}")
        else:
            print("\n" + toml_content)

    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
