"""Tests for import_logi_options.py, on synthetic settings only.

Run from the repository root:

    python3 -m unittest discover -s tools
"""

from __future__ import annotations

import contextlib
import io
import json
import re
import sqlite3
import tempfile
import unittest
from pathlib import Path

import import_logi_options as importer

REPO_ROOT = Path(__file__).resolve().parent.parent

SLOT = "synth-mouse-0001"
SERIAL = "SYNTH0001ABCD"


def macro_card(card_id: str, macro: dict) -> dict:
    return {"id": card_id, "name": card_id.upper(), "macro": macro, "attribute": "MACRO_PLAYBACK"}


def quick_launch(action: str) -> dict:
    return macro_card(action, {"type": "QUICK_LAUNCH", "quickLaunch": {"action": action}})


def four_way(click: dict, up: dict, down: dict, left: dict, right: dict) -> dict:
    return {
        "attribute": "ADAPTER_4WAYS",
        "nestedCards": {
            "click": click,
            "up": up,
            "down": down,
            "left": left,
            "right": right,
        },
    }


def window_navigation() -> dict:
    desktop = lambda side: macro_card(  # noqa: E731
        side,
        {"type": "SYSTEM", "system": {"action": f"SWITCH_BETWEEN_DESKTOPS_{side.upper()}"}},
    )
    return four_way(
        quick_launch("MISSION_CONTROL"),
        quick_launch("MISSION_CONTROL"),
        quick_launch("APP_EXPOSE"),
        desktop("left"),
        desktop("right"),
    )


def application_navigation() -> dict:
    unmapped = {"id": "switch_applications", "name": "ASSIGNMENT_NAME_SWITCH_APPLICATIONS"}
    return four_way(
        quick_launch("LAUNCHPAD"),
        quick_launch("MISSION_CONTROL"),
        quick_launch("APP_EXPOSE"),
        unmapped,
        unmapped,
    )


def gesture_card(selected: str) -> dict:
    return {
        "attribute": "ONE_OF",
        "selectedNestedCard": selected,
        "nestedCards": {
            "window_navigation": window_navigation(),
            "application_navigation": application_navigation(),
        },
    }


def assignment(suffix: str, card: dict, slot: str = SLOT) -> dict:
    return {"slotId": f"{slot}_{suffix}", "card": card}


def settings(assignments: list, extra_profiles: int = 0) -> dict:
    data = {
        "ever_connected_devices": {
            "devices": [
                {"slotPrefix": SLOT, "serialNumber": SERIAL},
                {"slotPrefix": "synth-keyboard-0002", "serialNumber": "bad serial!"},
            ]
        },
        "profile_keys": ["profile-default"],
        "profile-default": {
            "name": importer.DEFAULT_PROFILE_NAME,
            "assignments": assignments,
        },
    }
    for index in range(extra_profiles):
        key = f"profile-app-{index}"
        data["profile_keys"].append(key)
        data[key] = {"name": "PROFILE_NAME_APP", "assignments": []}
    return data


def wheel_card(smooth: bool) -> dict:
    return {
        "attribute": "MOUSE_SCROLL_WHEEL_SETTINGS",
        "mouseScrollWheelSettings": {"isSmooth": smooth},
    }


def write_db(path: Path, documents: list) -> None:
    con = sqlite3.connect(path)
    con.execute("CREATE TABLE data(_id INTEGER PRIMARY KEY, file BLOB NOT NULL)")
    for document in documents:
        con.execute("INSERT INTO data(file) VALUES (?)", (json.dumps(document),))
    con.commit()
    con.close()


class CardActionTests(unittest.TestCase):
    def test_known_macros_map_to_openlogi_actions(self) -> None:
        cases = [
            ({"type": "QUICK_LAUNCH", "quickLaunch": {"action": "LAUNCHPAD"}}, "LaunchpadShow"),
            ({"type": "SYSTEM", "system": {"action": "SWITCH_BETWEEN_DESKTOPS_LEFT"}}, "PreviousDesktop"),
            ({"type": "MEDIA", "media": {"usage": "PREVIOUS_TRACK"}}, "PrevTrack"),
            ({"type": "MOUSE", "mouse": {"action": "BUTTON", "hidUsage": 3}}, "MiddleClick"),
            ({"type": "DO_NOTHING", "doNothing": {}}, "None"),
        ]
        for macro, expected in cases:
            with self.subTest(macro=macro):
                self.assertEqual(importer.card_action({"macro": macro}), expected)

    def test_unknown_or_missing_macros_have_no_action(self) -> None:
        cases = [
            None,
            {},
            {"macro": {}},
            {"macro": {"type": "QUICK_LAUNCH", "quickLaunch": {"action": "SOMETHING_NEW"}}},
            {"macro": {"type": "MOUSE", "mouse": {"action": "OSX_GESTURE_BACK"}}},
            {"macro": {"type": "MOUSE", "mouse": {"action": "BUTTON", "hidUsage": 9}}},
            {"macro": {"type": "KEYSTROKE"}},
        ]
        for card in cases:
            with self.subTest(card=card):
                self.assertIsNone(importer.card_action(card))

    def test_every_emitted_action_is_an_openlogi_action_name(self) -> None:
        source = (REPO_ROOT / "crates/openlogi-core/src/binding/action.rs").read_text()
        emitted = {
            *importer.QUICK_LAUNCH_ACTIONS.values(),
            *importer.SYSTEM_ACTIONS.values(),
            *importer.MEDIA_ACTIONS.values(),
            *importer.MOUSE_BUTTON_ACTIONS.values(),
        }
        for action in emitted:
            with self.subTest(action=action):
                self.assertRegex(source, rf"\b{action}\b")


class ReadSettingsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.db = Path(self.dir.name) / "settings with space.db"

    def test_reads_the_newest_row(self) -> None:
        write_db(self.db, [{"generation": 1}, {"generation": 2}])
        self.assertEqual(importer.read_settings(self.db), {"generation": 2})

    def test_missing_file(self) -> None:
        with self.assertRaisesRegex(importer.ImportFailure, "no Logi Options"):
            importer.read_settings(self.db)

    def test_empty_table(self) -> None:
        write_db(self.db, [])
        with self.assertRaisesRegex(importer.ImportFailure, "no settings"):
            importer.read_settings(self.db)

    def test_not_a_sqlite_database(self) -> None:
        self.db.write_text("not a database")
        with self.assertRaisesRegex(importer.ImportFailure, "cannot read"):
            importer.read_settings(self.db)

    def test_unreadable_document(self) -> None:
        con = sqlite3.connect(self.db)
        con.execute("CREATE TABLE data(_id INTEGER PRIMARY KEY, file BLOB NOT NULL)")
        con.execute("INSERT INTO data(file) VALUES ('{broken')")
        con.commit()
        con.close()
        with self.assertRaisesRegex(importer.ImportFailure, "unreadable"):
            importer.read_settings(self.db)

    def test_does_not_modify_the_database(self) -> None:
        write_db(self.db, [{"generation": 1}])
        before = self.db.read_bytes()
        importer.read_settings(self.db)
        self.assertEqual(self.db.read_bytes(), before)


class ExtractTests(unittest.TestCase):
    def test_device_is_keyed_by_lowercased_serial_never_unit(self) -> None:
        data = settings([assignment("c82", macro_card("mb3", {
            "type": "MOUSE", "mouse": {"action": "BUTTON", "hidUsage": 3}}))])
        devices, _ = importer.extract_devices(data)
        self.assertEqual([d.key for d in devices], ["serial:synth0001abcd"])
        self.assertNotIn("unit:", importer.render_config(devices))

    def test_device_without_a_usable_serial_is_not_imported(self) -> None:
        data = settings([assignment("c82", quick_launch("LAUNCHPAD"), slot="synth-keyboard-0002"),
                         assignment("c82", quick_launch("LAUNCHPAD"), slot="virtual-device-9")])
        devices, _ = importer.extract_devices(data)
        self.assertEqual(devices, [])

    def test_gesture_follows_the_selected_mode(self) -> None:
        window = importer.extract_devices(settings([assignment("c195", gesture_card("window_navigation"))]))[0][0]
        self.assertEqual(
            window.gesture,
            {"Click": "MissionControl", "Up": "MissionControl", "Down": "AppExpose",
             "Left": "PreviousDesktop", "Right": "NextDesktop"},
        )

    def test_unmappable_gesture_directions_are_reported_not_guessed(self) -> None:
        devices, skipped = importer.extract_devices(
            settings([assignment("c195", gesture_card("application_navigation"))])
        )
        self.assertEqual(
            devices[0].gesture,
            {"Click": "LaunchpadShow", "Up": "MissionControl", "Down": "AppExpose"},
        )
        self.assertEqual(len([s for s in skipped if "no OpenLogi equivalent" in s]), 2)

    def test_unknown_gesture_mode_imports_nothing(self) -> None:
        card = gesture_card("window_navigation")
        card["selectedNestedCard"] = "mode_from_the_future"
        devices, skipped = importer.extract_devices(settings([assignment("c195", card)]))
        self.assertEqual(devices, [])
        self.assertTrue(any("not a four-way mode" in s for s in skipped))

    def test_unrecognised_button_is_reported_and_left_out(self) -> None:
        card = macro_card("custom", {"type": "KEYSTROKE"})
        devices, skipped = importer.extract_devices(
            settings([assignment("c83", card), assignment("c82", quick_launch("APP_EXPOSE"))])
        )
        self.assertEqual(devices[0].bindings, {"MiddleClick": "AppExpose"})
        self.assertTrue(any("Back" in s for s in skipped))

    def test_smooth_scrolling_follows_the_wheel_settings(self) -> None:
        on = importer.extract_devices(settings([assignment("mouse_scroll_wheel_settings", wheel_card(True))]))[0]
        off = importer.extract_devices(settings([assignment("mouse_scroll_wheel_settings", wheel_card(False))]))[0]
        self.assertTrue(on[0].smooth_wheel)
        self.assertEqual(off, [])

    def test_only_the_default_profile_is_read_and_the_rest_reported(self) -> None:
        data = settings([assignment("c82", quick_launch("LAUNCHPAD"))], extra_profiles=2)
        _, skipped = importer.extract_devices(data)
        self.assertIn("per-application profiles are not imported", skipped)

    def test_missing_default_profile_fails(self) -> None:
        data = settings([])
        data["profile-default"]["name"] = "PROFILE_NAME_APP"
        with self.assertRaisesRegex(importer.ImportFailure, "default profile"):
            importer.extract_devices(data)


class RenderTests(unittest.TestCase):
    def test_renders_a_complete_document(self) -> None:
        device = importer.Device("AbC123")
        device.smooth_wheel = True
        device.bindings = {"Back": "MouseBack", "MiddleClick": "None"}
        device.gesture = {"Click": "MissionControl", "Left": "PreviousDesktop"}
        self.assertEqual(
            importer.render_config([device]),
            "\n".join([
                "schema_version = 10",
                "",
                '[devices."serial:abc123"]',
                "smooth_wheel = true",
                "",
                '[devices."serial:abc123".bindings]',
                'Back = "MouseBack"',
                'MiddleClick = "None"',
                "",
                '[devices."serial:abc123".bindings.GestureButton]',
                'Click = "MissionControl"',
                'Left = "PreviousDesktop"',
                "",
            ]),
        )

    def test_schema_version_matches_the_config_source(self) -> None:
        source = (REPO_ROOT / "crates/openlogi-core/src/config.rs").read_text()
        match = re.search(r"pub const SCHEMA_VERSION: u32 = (\d+);", source)
        self.assertIsNotNone(match, "SCHEMA_VERSION moved; update this test")
        self.assertEqual(
            importer.SCHEMA_VERSION,
            int(match.group(1)),
            "config schema changed: re-check the generated layout, then bump SCHEMA_VERSION",
        )


class CommandLineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.root = Path(self.dir.name)
        self.db = self.root / "settings.db"
        write_db(self.db, [settings([assignment("c82", quick_launch("LAUNCHPAD"))])])

    def run_main(self, *args: str) -> tuple:
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = importer.main(list(args))
        return code, out.getvalue(), err.getvalue()

    def test_default_is_a_dry_run_on_stdout(self) -> None:
        code, out, _ = self.run_main("--db", str(self.db))
        self.assertEqual(code, 0)
        self.assertIn('MiddleClick = "LaunchpadShow"', out)
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), ["settings.db"])

    def test_output_writes_a_new_file(self) -> None:
        target = self.root / "config.toml"
        code, out, _ = self.run_main("--db", str(self.db), "-o", str(target))
        self.assertEqual(code, 0)
        self.assertEqual(out, "")
        self.assertIn("schema_version", target.read_text())

    def test_existing_output_is_never_replaced(self) -> None:
        target = self.root / "config.toml"
        target.write_text("# precious\n")
        code, _, err = self.run_main("--db", str(self.db), "-o", str(target))
        self.assertEqual(code, 1)
        self.assertIn("refusing", err)
        self.assertEqual(target.read_text(), "# precious\n")

    def test_missing_database_is_an_error(self) -> None:
        code, out, err = self.run_main("--db", str(self.root / "nope.db"))
        self.assertEqual((code, out), (1, ""))
        self.assertIn("no Logi Options+ database", err)

    def test_nothing_importable_is_an_error(self) -> None:
        write_db(self.root / "empty.db", [settings([])])
        code, out, err = self.run_main("--db", str(self.root / "empty.db"))
        self.assertEqual((code, out), (1, ""))
        self.assertIn("no importable settings", err)


if __name__ == "__main__":
    unittest.main()
