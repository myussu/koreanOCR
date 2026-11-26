import json
import os
import struct
import tempfile
import unittest
import wave

from voice_candidates import (
    eval_rows,
    measured_row,
    model_servers,
    others_gpu_mb,
    peaks,
    plausible_seconds,
    process_gpu_bytes,
    quiet_seconds,
    render_page,
    retakes,
    speed_range,
    wait_until_quiet,
    wav_facts,
    wav_problem,
)

GREETING = "안녕하세요. 만나서 반갑습니다. 저는 한국어를 공부하고 있어요."
EVAL_OUTPUT = """The local speech server was ready in 10.2 s (process 4242).
voice              sentence            audio     took    RTF
sohee              greeting             5.9s     0.8s   0.14
sohee              paragraph           23.2s     3.0s   0.13
Supertonic 3 (F1)  greeting             6.7s     1.6s   0.24

Open C:\\somewhere\\index.html in a browser and listen. RTF below 1 means it speaks faster than real time.
"""
WARNING = "2026-11-26T10:00:00Z  WARN maru_tts: speech much longer than its text; asking again take=1 seconds=22.4 limit=5.95\n"


def write_wav(path, seconds, level=8000, rate=24000):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(struct.pack("<h", level) * int(seconds * rate))


def run(ready, working_set, private, gpu, stdout=EVAL_OUTPUT, stderr="", system=None):
    result = {"ready_seconds": ready, "peaks_mb": {"working_set": working_set, "private": private, "gpu_process": gpu}, "client": {"stdout": stdout, "stderr": stderr}}
    if system:
        result["system_peaks_mb"] = {"working_set": system[0], "private": system[1]}
    return result


class VoiceCandidatesTests(unittest.TestCase):
    def test_only_model_servers_are_found_in_the_task_list_whatever_their_case(self):
        listing = '"explorer.exe","12740","Console","1","210,000 K"\n"LLAMA-SERVER.EXE","5120","Console","1","4,100,000 K"\n"tts-server.exe","77","Console","1","9 K"\n\n'
        self.assertEqual(model_servers(listing), [("LLAMA-SERVER.EXE", 5120), ("tts-server.exe", 77)])
        self.assertEqual(model_servers('INFO: No tasks are running which match the specified criteria.\n'), [])

    def test_waiting_for_other_servers_asks_once_a_minute_and_gives_up_at_the_limit(self):
        answers = iter([[("llama-server.exe", 1)], [("llama-server.exe", 1)], []])
        naps = []
        self.assertEqual(wait_until_quiet(lambda: next(answers), 20, naps.append), ([], 2))
        self.assertEqual(naps, [60, 60])
        naps.clear()
        self.assertEqual(wait_until_quiet(lambda: [("maru-app.exe", 9)], 3, naps.append), ([("maru-app.exe", 9)], 3))
        self.assertEqual(len(naps), 3)
        self.assertEqual(wait_until_quiet(lambda: [], 20, naps.append), ([], 0))

    def test_a_process_gpu_memory_is_the_sum_of_its_adapters_and_never_another_process_with_a_longer_number(self):
        instances = {"pid_42_luid_0x00000000_0x0000A1B2_phys_0": 3_000, "pid_42_luid_0x00000000_0x0000C3D4_phys_0": 500, "pid_421_luid_0x00000000_0x0000A1B2_phys_0": 9_000}
        self.assertEqual(process_gpu_bytes(instances, 42), 3_500)
        self.assertEqual(process_gpu_bytes(instances, 7), 0)

    def test_peaks_are_the_highest_sample_of_each_counter_in_megabytes(self):
        samples = [
            {"t": 1, "working_set": 100 * 1_048_576, "private": 900 * 1_048_576, "gpu_process": 0},
            {"t": 2, "working_set": 2447 * 1_048_576, "private": 4480 * 1_048_576, "gpu_process": 3996 * 1_048_576},
            {"t": 3, "working_set": 2000 * 1_048_576, "private": 4000 * 1_048_576, "gpu_process": 3000 * 1_048_576},
        ]
        self.assertEqual(peaks(samples), {"working_set": 2447, "private": 4480, "gpu_process": 3996})
        self.assertEqual(peaks([]), {"working_set": 0, "private": 0, "gpu_process": 0})

    def test_another_program_loading_a_model_in_the_middle_shows_in_what_the_others_hold(self):
        quiet = [{"gpu_others": 2856 * 1_048_576}, {"gpu_others": 2850 * 1_048_576}]
        self.assertEqual(others_gpu_mb(quiet), [2850, 2856])
        self.assertEqual(others_gpu_mb(quiet + [{"gpu_others": 6000 * 1_048_576}]), [2850, 6000])
        self.assertIsNone(others_gpu_mb([{"t": 1}]), "older measurements did not record it")

    def test_quiet_is_counted_in_all_and_before_the_first_and_after_the_last_sound(self):
        rate = 1000
        samples = [0] * 500 + [8000] * 1000 + [100] * 300 + [8000] * 200 + [0] * 1000
        found = quiet_seconds(samples, rate)
        self.assertAlmostEqual(found["quiet"], 1.8)
        self.assertAlmostEqual(found["quiet_before"], 0.5)
        self.assertAlmostEqual(found["quiet_after"], 1.0)
        self.assertEqual(quiet_seconds([0] * 100, rate), {"quiet": 0.1, "quiet_before": 0.1, "quiet_after": 0.1})

    def test_the_eval_table_is_read_row_by_row_even_when_a_voice_name_has_spaces(self):
        rows = eval_rows(EVAL_OUTPUT)
        self.assertEqual([(r["voice"], r["sentence"]) for r in rows], [("sohee", "greeting"), ("sohee", "paragraph"), ("Supertonic 3 (F1)", "greeting")])
        self.assertEqual((rows[1]["audio"], rows[1]["took"], rows[1]["rtf"]), (23.2, 3.0, 0.13))
        self.assertEqual(speed_range(rows), "0.13–0.24")
        self.assertEqual(speed_range([]), "—")

    def test_a_take_that_ran_long_is_counted_from_marus_warning_with_or_without_colours(self):
        coloured = WARNING.replace(" WARN ", " \x1b[33m WARN\x1b[0m ").replace("take=1", "\x1b[3mtake\x1b[0m\x1b[2m=\x1b[0m3")
        self.assertEqual(retakes(WARNING + "something else take=9\n" + coloured), [{"take": 1, "seconds": 22.4, "limit": 5.95}, {"take": 3, "seconds": 22.4, "limit": 5.95}])
        self.assertEqual(retakes(""), [])

    def test_a_reading_may_last_from_a_tenth_of_a_second_to_about_half_a_second_a_syllable(self):
        shortest, longest = plausible_seconds(GREETING)
        self.assertLess(shortest, 5.9)
        self.assertGreater(longest, 6.7, "both run 1 readings of this sentence fit")
        self.assertLess(longest, 20)
        _, phone = plausible_seconds("전화번호는 010-1234-5678이에요.")
        self.assertGreater(phone, 7, "a phone number is read digit by digit")

    def test_a_wav_is_measured_and_a_silent_short_or_overlong_one_is_named(self):
        with tempfile.TemporaryDirectory() as d:
            good, silent, short, long = (os.path.join(d, n) for n in ("good.wav", "silent.wav", "short.wav", "long.wav"))
            write_wav(good, 6.0)
            write_wav(silent, 6.0, level=0)
            write_wav(short, 0.5)
            write_wav(long, 40.0)
            facts = wav_facts(good)
            self.assertAlmostEqual(facts["seconds"], 6.0)
            self.assertEqual((facts["rate"], facts["channels"], facts["bits"]), (24000, 1, 16))
            self.assertAlmostEqual(facts["loudest"], 8000 / 32768)
            self.assertEqual(facts["quiet"], 0)
            self.assertAlmostEqual(wav_facts(silent)["quiet"], 6.0)
            self.assertIsNone(wav_problem(facts, GREETING))
            self.assertEqual(wav_problem(wav_facts(silent), GREETING), "silent")
            self.assertIn("too short", wav_problem(wav_facts(short), GREETING))
            self.assertIn("longer than", wav_problem(wav_facts(long), GREETING))

    def test_a_measured_voice_shows_every_ready_time_the_worst_memory_and_the_takes_asked_again(self):
        row = measured_row([run(10.2, 2400, 4480, 3990), run(11.7, 2447, 4400, 3996, stderr=WARNING)])
        self.assertEqual(row["ready"], "10.2 s and 11.7 s")
        self.assertEqual((row["working_set"], row["private"], row["gpu"]), ("2,447 MB", "4,480 MB", "3,996 MB"))
        seen = measured_row([run(1, 2452, 4448, 1, system=(3127, 4457)), run(1, 2413, 4385, 1, system=(3128, 4387))])
        self.assertEqual(seen["working_set"], "2,452 MB sampled, 3,128 MB at its highest instant")
        self.assertEqual(seen["private"], "4,448 MB sampled, 4,457 MB at its highest instant")
        self.assertEqual(row["speed"], "0.13–0.24 × real time")
        self.assertEqual(row["whole"], "5.4 s for 35.8 s of audio and 5.4 s for 35.8 s of audio")
        self.assertEqual(row["takes"], "1 asked again")
        self.assertEqual(measured_row([run(1, 1, 1, 1)])["takes"], "none")
        kept = measured_row([run(1, 1, 1, 1, stderr=WARNING + WARNING.replace("take=1", "take=3"))])["takes"]
        self.assertEqual(kept, "2 asked again, 1 kept although too long")

    def test_the_page_has_a_player_for_every_voice_under_every_sentence_and_names_audio_that_is_wrong(self):
        with tempfile.TemporaryDirectory() as d:
            for folder, seconds in (("measured", 5.9), ("reported", 6.7)):
                os.makedirs(os.path.join(d, folder))
                write_wav(os.path.join(d, folder, "greeting.wav"), seconds)
            write_wav(os.path.join(d, "measured", "paragraph.wav"), 0.2)
            with open(os.path.join(d, "one.json"), "w", encoding="utf-8") as f:
                json.dump(run(10.2, 2447, 4480, 3996, stdout=EVAL_OUTPUT.replace("Supertonic 3 (F1)  greeting", "sohee              numbers ")), f)
            spec = {
                "title": "Voices <run 2>",
                "method": ["Looked at once a second."],
                "sentences": [{"name": "greeting", "text": GREETING}, {"name": "paragraph", "text": "한국어를 공부하는 것은 쉽지 않지만, 매일 조금씩 연습하면 분명히 늘어요."}],
                "voices": [
                    {"name": "Measured & new", "folder": "measured", "measurements": ["one.json"]},
                    {"name": "Reported", "folder": "reported", "reported": {"ready": "0.9 s", "gpu": "0"}, "made": {"greeting": {"took": 1.6}}},
                ],
            }
            text, problems = render_page(spec, d)
        self.assertIn("Voices &lt;run 2&gt;", text)
        self.assertIn("Measured &amp; new", text)
        self.assertEqual(text.count("<audio "), 4)
        self.assertLess(text.index("<h2>Measurements"), text.index("<h2>Listen"))
        self.assertLess(text.rindex("<audio "), text.index("Looked at once a second."), "the method is read after listening")
        self.assertIn('src="measured/greeting.wav"', text)
        self.assertIn("5.9 s, 0.0 s of it quiet · made in 0.8 s (0.14 × real time)", text)
        self.assertIn("6.7 s, 0.0 s of it quiet · made in 1.6 s<", text)
        self.assertIn("<td>2,447 MB</td>", text)
        self.assertIn("<td>0.9 s</td><td>—</td>", text, "a number nobody reported is left blank, not invented")
        self.assertIn("<td>5.4 s for 35.8 s of audio</td>", text)
        self.assertEqual(len(problems), 2)
        self.assertTrue(problems[0].startswith("measured/paragraph.wav") and "too short" in problems[0])
        self.assertEqual(problems[1], "reported/paragraph.wav: the file is missing")


if __name__ == "__main__":
    unittest.main()
