"""Tests for find_subtitles.py and the naming in sample_subtitle_frames.py (no video is needed)."""
import unittest

import find_subtitles as fs
import sample_subtitle_frames as sampler

try:
    import numpy as np
except ImportError:  # the pixel tests need it; the rest do not
    np = None


def same(a, b):
    return 1.0 if a == b else 0.0


class Stretches(unittest.TestCase):
    def test_lettering_that_stays_is_one_stretch_and_a_new_line_starts_another(self):
        times = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0]
        grids = [None, "a", "a", None, "b", "b", "b"]
        self.assertEqual(fs.stretches(times, grids, same), [(0.0, 1.0, "a"), (1.5, 3.0, "b")])

    def test_one_line_replacing_another_without_a_gap_is_two_stretches(self):
        times = [0.0, 0.5, 1.0, 1.5, 2.0]
        grids = [None, "a", "a", "b", "b"]
        self.assertEqual(fs.stretches(times, grids, same), [(0.0, 1.0, "a"), (1.0, 2.0, "b")])

    def test_nothing_found_is_no_stretch(self):
        self.assertEqual(fs.stretches([0.0, 0.5], [None, None], same), [])


class Spread(unittest.TestCase):
    def test_few_stretches_are_all_kept_in_order(self):
        found = [(5.0, 6.0, "b"), (1.0, 2.0, "a")]
        self.assertEqual(fs.spread(found, 10, 100.0), sorted(found))

    def test_a_crowded_start_does_not_take_every_place(self):
        # forty subtitles in the first minute, then one every minute for nine more
        early = [(float(i), i + 0.9, f"e{i}") for i in range(40)]
        late = [(60.0 * m, 60.0 * m + 2, f"l{m}") for m in range(1, 10)]
        kept = fs.spread(early + late, 10, 600.0)
        self.assertEqual(len(kept), 10)
        self.assertEqual(sum(1 for s in kept if s[0] >= 60.0), 9, "every later minute keeps its subtitle")

    def test_empty_slices_are_made_up_with_the_longest_of_the_rest(self):
        found = [(0.0, 1.0, "a"), (1.0, 5.0, "b"), (6.0, 8.0, "c"), (90.0, 91.0, "d")]
        kept = fs.spread(found, 3, 100.0)
        self.assertEqual([s[2] for s in kept], ["b", "c", "d"])


@unittest.skipIf(np is None, "numpy is not installed")
class Lettering(unittest.TestCase):
    def grid(self, rows=30, cols=100):
        return np.zeros((rows, cols), dtype=bool)

    def test_a_centred_line_is_a_subtitle_and_scenery_rows_are_left_out_of_it(self):
        g = self.grid()
        g[12:16, 30:70] = True  # the line
        g[2, 5:14] = True  # a HUD's edge: far less lettering than the line
        self.assertEqual(fs.box_of(g), (12, 16, 31, 69))
        part = fs.subtitle_part(g)
        self.assertIsNotNone(part)
        self.assertFalse(part[2].any(), "what is outside the lines is dropped")

    def test_two_lines_close_together_are_one_subtitle(self):
        g = self.grid()
        g[10:13, 35:65] = True
        g[15:18, 30:70] = True
        top, bottom, _, _ = fs.box_of(g)
        self.assertEqual((top, bottom), (10, 18))

    def test_lettering_at_the_side_or_across_the_whole_band_is_not_a_subtitle(self):
        side = self.grid()
        side[12:16, 2:30] = True
        self.assertIsNone(fs.subtitle_part(side))
        horizon = self.grid()
        horizon[14:16, :] = True
        self.assertIsNone(fs.subtitle_part(horizon))
        self.assertIsNone(fs.subtitle_part(self.grid()), "an empty band")

    def test_thin_light_strokes_are_lettering_and_a_bright_wall_is_not(self):
        gray = np.full((40, 120), 30, dtype=np.uint8)
        gray[:, 80:] = 230  # a bright wall
        gray[18:21, 10:60] = 240  # a stroke three pixels thick
        mask = fs.lettering(gray)
        self.assertTrue(mask[19, 12:58].all())
        self.assertFalse(mask[:, 90:].any())

    def test_the_same_lettering_overlaps_fully_and_different_lettering_does_not(self):
        a, b = self.grid(), self.grid()
        a[12:16, 30:70] = True
        b[12:16, 30:50] = True  # the line still typing out
        self.assertEqual(fs.overlap(a, b), 1.0)
        c = self.grid()
        c[20:24, 30:70] = True
        self.assertEqual(fs.overlap(a, c), 0.0)


class Names(unittest.TestCase):
    def test_without_a_prefix_names_are_as_before(self):
        self.assertEqual(sampler.picture_name(None, 400.0, 12, 3), "s012_3")

    def test_a_prefix_puts_the_five_minute_stretch_first_so_a_scene_is_split_as_one(self):
        self.assertEqual(sampler.picture_name("er", 0.0, 1, 0), "er00_s001_0")
        self.assertEqual(sampler.picture_name("er", 299.9, 2, 0).split("_")[0], "er00")
        self.assertEqual(sampler.picture_name("er", 300.0, 3, 1), "er01_s003_1")


if __name__ == "__main__":
    unittest.main()
