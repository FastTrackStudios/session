"""The template writer's GROUP_FLAGS field order, pinned to REAPER's.

REAPER writes the 25 per-parameter group masks in an order that is NOT
the grouping dialog's: seven lead/follow pairs, then the reverse flags,
width only at 19/20, VCA at 21/22. A writer that puts width at 5/6 has
every pair after it off by two — mute lands on solo, solo on rec-arm —
which is what happened to the Electric and Acoustic folders (session
issue #41). The reference here is a project REAPER 6.69 saved itself,
`helgobox/resources/test-projects/issue-45-grouping-vca-test.RPP`,
lines 95-111 and 1949-1965: its `GROUP_FLAGS` lines are embedded below
so the check needs no file outside the repo, and compared against the
file itself when that checkout is present.
"""
import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("make_template_rpp", HERE / "make-template-rpp.py")
writer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(writer)

FIXTURE = pathlib.Path("/run/media/Development/helgobox/resources/test-projects/issue-45-grouping-vca-test.RPP")

# What REAPER wrote for a track that LEADS every parameter in group 1,
# and for one that follows all of group 1 while leading all of group 2.
LEAD_ALL_1 = "1 0 1 0 1 0 1 0 1 0 1 0 1 0 0 0 0 0 1"
FOLLOW_1_LEAD_2 = "2 1 2 1 2 1 2 1 2 1 2 1 2 1 0 0 2 0 2 1"

PAIRS = ["volume", "pan", "mute", "solo", "recarm", "polarity", "automode", "width"]
LEADS = {f"{p}_lead" for p in PAIRS}
FOLLOWS = {f"{p}_follow" for p in PAIRS}


def decode(line, group):
    """The field names whose mask holds `group`, read with GROUP_FIELDS."""
    masks = [int(f) for f in line.split()]
    return {
        name for name, mask in zip(writer.GROUP_FIELDS, masks)
        if mask & (1 << (group - 1))
    }


def fields_set(flags):
    """1-based field numbers with any bit set in a written line."""
    return [i + 1 for i, f in enumerate(flags.split()) if int(f)]


class FieldOrder(unittest.TestCase):
    def test_twenty_five_fields(self):
        self.assertEqual(len(writer.GROUP_FIELDS), 25)
        self.assertEqual(len(set(writer.GROUP_FIELDS)), 25)

    def test_reaper_lead_of_everything_names_the_lead_fields(self):
        # Fields 1,3,5,7,9,11,13 and 19 are set: seven pairs first, width
        # at 19/20. Read with our names, that must be exactly the leads.
        self.assertEqual(decode(LEAD_ALL_1, 1), LEADS)

    def test_reaper_follow_of_everything_names_the_follow_fields(self):
        # Group 1 bit: exactly the follow side — 2,4,…,14 and 20.
        self.assertEqual(decode(FOLLOW_1_LEAD_2, 1), FOLLOWS)
        # Group 2 bit: the leads, plus field 17 — "no lead when
        # following", which sits BEFORE width reverse (18), not after.
        self.assertEqual(decode(FOLLOW_1_LEAD_2, 2), LEADS | {"no_lead_when_following"})

    def test_reaper_vca_pair_is_21_and_22(self):
        self.assertEqual(decode("0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 4", 3), {"vca_lead"})
        self.assertEqual(decode("0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 4", 3), {"vca_follow"})

    def test_named_positions(self):
        one_based = {name: i + 1 for i, name in enumerate(writer.GROUP_FIELDS)}
        self.assertEqual(one_based["mute_lead"], 5)
        self.assertEqual(one_based["mute_follow"], 6)
        self.assertEqual(one_based["solo_lead"], 7)
        self.assertEqual(one_based["solo_follow"], 8)
        self.assertEqual(one_based["recarm_lead"], 9)
        self.assertEqual(one_based["width_lead"], 19)
        self.assertEqual(one_based["width_follow"], 20)
        self.assertEqual(one_based["vca_lead"], 21)
        self.assertEqual(one_based["vca_follow"], 22)

    def test_folder_is_vca_mute_solo_lead_of_its_bus(self):
        # The Electric folder: VCA lead (21), mute lead (5), solo lead (7)
        # of group 1 — and NOT rec-arm lead (9), which the old order gave it.
        self.assertEqual(fields_set(writer.group_flags(writer.vca_lead(1))), [5, 7, 21])
        self.assertEqual(fields_set(writer.group_flags(writer.vca_follow(1))), [6, 8, 22])
        self.assertEqual(decode(writer.group_flags(writer.vca_lead(2)), 2),
                         {"vca_lead", "mute_lead", "solo_lead"})

    @unittest.skipUnless(FIXTURE.is_file(), "REAPER-saved fixture not checked out")
    def test_embedded_lines_match_the_reaper_fixture(self):
        lines = [l.strip() for l in FIXTURE.read_text().splitlines() if l.strip().startswith("GROUP_FLAGS ")]
        flags = [l[len("GROUP_FLAGS "):] for l in lines]
        self.assertIn(LEAD_ALL_1, flags)
        self.assertIn(FOLLOW_1_LEAD_2, flags)


if __name__ == "__main__":
    unittest.main()
