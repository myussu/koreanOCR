import unittest

from supertonic_server import VOICES, language_code, speech_speed


class SupertonicServerTests(unittest.TestCase):
    def test_the_language_maru_sends_by_name_becomes_the_models_code_and_korean_is_the_default(self):
        self.assertEqual(language_code("Korean"), "ko")
        self.assertEqual(language_code(" english "), "en")
        self.assertEqual(language_code("ko"), "ko")
        self.assertEqual(language_code(None), "ko")
        self.assertEqual(language_code(""), "ko")

    def test_normal_speed_is_the_models_own_pace_and_a_factor_scales_it(self):
        self.assertAlmostEqual(speech_speed(None), 1.05)
        self.assertAlmostEqual(speech_speed(1.0), 1.05)
        self.assertAlmostEqual(speech_speed(0.8), 0.84)

    def test_there_are_female_and_male_voices_to_compare(self):
        self.assertIn("F1", VOICES)
        self.assertIn("M1", VOICES)


if __name__ == "__main__":
    unittest.main()
