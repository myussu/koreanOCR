"""Propose subtitle boundary brackets from an existing clip; visual review is required."""
import argparse
import hashlib
import io
import json
import subprocess
from pathlib import Path


def gray_frames(video, start, duration, rate, crop):
    x, y, width, height = crop
    result = subprocess.run(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-ss", str(start),
         "-i", str(video), "-t", str(duration), "-vf",
         f"fps={rate},crop={width}:{height}:{x}:{y}:exact=1", "-f", "rawvideo", "-pix_fmt", "gray", "pipe:1"],
        capture_output=True, check=True, timeout=60,
    )
    size = width * height
    if len(result.stdout) % size:
        raise ValueError("Decoder returned a partial frame")
    return [result.stdout[i:i + size] for i in range(0, len(result.stdout), size)]


def glyph_mask(frame, width, height):
    from PIL import Image, ImageChops, ImageFilter
    image = Image.frombytes("L", (width, height), frame)
    # Bright lettering next to its dark outline excludes most smooth background highlights.
    dark = image.filter(ImageFilter.MinFilter(5))
    contrast = ImageChops.subtract(image, dark).tobytes()
    return bytes(int(value >= 165 and edge >= 75) for value, edge in zip(frame, contrast))


def similarity(template, sample):
    count = sum(template)
    if count < 30:
        raise ValueError("The reference frame has too little lettering; inspect the crop")
    return sum(a and b for a, b in zip(template, sample)) / count


def propose(video, item, rate, crop):
    _, _, width, height = crop
    midpoint = (item["start_seconds"] + item["end_seconds"]) / 2
    template = glyph_mask(gray_frames(video, midpoint, 1 / rate, rate, crop)[0], width, height)
    boundaries = {}
    for side, old_key, revised_key in (
        ("onset", "start_seconds", "first_visible_seconds"),
        ("offset", "end_seconds", "first_absent_seconds"),
    ):
        old = item[old_key]
        revised = item.get(revised_key, old)
        lower = max(0, min(old, revised) - 1.5)
        upper = max(old, revised) + 1.5
        frames = gray_frames(video, lower, upper - lower, rate, crop)
        scores = [similarity(template, glyph_mask(frame, width, height)) for frame in frames]
        transitions = []
        for index in range(1, len(scores)):
            previous, current = scores[index - 1] >= .55, scores[index] >= .55
            if (side == "onset" and not previous and current) or (side == "offset" and previous and not current):
                transitions.append(index)
        if transitions:
            index = min(transitions, key=lambda n: abs(lower + n / rate - old))
            bracket = [round(lower + (index - 1) / rate, 4), round(lower + index / rate, 4)]
        else:
            bracket = None
        boundaries[side] = dict(bracket_seconds=bracket, sample_start_seconds=lower,
                                scores=scores, automatic_threshold=.55, reviewed=False)
    return dict(id=item["id"], source=item["exact_korean_text"], midpoint_seconds=midpoint,
                boundaries=boundaries, status="proposal_requires_visual_review")


def run(args):
    from PIL import Image, ImageDraw
    raw = args.dataset.read_bytes()
    dataset = json.loads(raw.decode("utf-8-sig"))
    video = args.source_root / dataset["video_path"]
    args.out.mkdir(parents=True, exist_ok=False)
    records = []
    for item in dataset["items"]:
        record = propose(video, item, args.rate, args.crop)
        records.append(record)
        # Exact seek samples on either side are independently decoded for the reviewer.
        tiles = []
        for side, boundary in record["boundaries"].items():
            if boundary["bracket_seconds"] is None:
                continue
            for when in (boundary["bracket_seconds"][0] - .15, boundary["bracket_seconds"][1] + .15):
                x, y, width, height = args.crop
                decoded = subprocess.run(
                    ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-ss", str(max(0, when)),
                     "-i", str(video), "-frames:v", "1", "-vf", f"crop={width}:{height}:{x}:{y}",
                     "-f", "image2pipe", "-vcodec", "png", "pipe:1"], capture_output=True, check=True, timeout=60,
                )
                image = Image.open(io.BytesIO(decoded.stdout)).convert("RGB")
                tile = Image.new("RGB", (width, height + 25), "white")
                ImageDraw.Draw(tile).text((5, 3), f"{item['id']} {side} exact sample {when:.4f}s", fill="black")
                tile.paste(image, (0, 25))
                tiles.append(tile)
        if tiles:
            sheet = Image.new("RGB", (args.crop[2], len(tiles) * (args.crop[3] + 25)), "white")
            for n, tile in enumerate(tiles):
                sheet.paste(tile, (0, n * tile.height))
            sheet.save(args.out / f"{item['id']}.png")
        print(f"Prepared {item['id']}", flush=True)
    report = dict(source_dataset=str(args.dataset.resolve()), source_sha256=hashlib.sha256(raw).hexdigest(),
                  video=str(video.resolve()), rate=args.rate, crop=args.crop, items=records,
                  limitation="Pixel matching proposes boundaries; it does not verify text or frame-exact timing. "
                  "fps resampling can select a nearby source frame. Review exact-seek evidence and record conservative brackets.")
    (args.out / "proposals.json").write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", type=Path, required=True)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--rate", type=int, choices=(5, 10, 20), default=10)
    parser.add_argument("--crop", type=int, nargs=4, default=(300, 900, 1320, 150), metavar=("X", "Y", "WIDTH", "HEIGHT"))
    run(parser.parse_args())
