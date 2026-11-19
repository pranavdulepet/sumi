"""Generate the app's dry-brush enso icon at macOS iconset resolutions."""
import math
import random
from pathlib import Path
from PIL import Image, ImageDraw

def icon():
    image = Image.new('RGBA', (1024, 1024))
    pen = ImageDraw.Draw(image)
    pen.rounded_rectangle((46, 46, 978, 978), radius=209, fill='#f4efe6')
    rng = random.Random(12)
    for i in range(230):
        radius = 270 + rng.uniform(-35, 35)
        begin = -63 + rng.uniform(-4, 6)
        end = 258 + rng.uniform(-17, 12)
        points = []
        for step in range(226):
            t = step / 225
            angle = math.radians(begin + (end - begin) * t)
            r = radius + 11 * math.sin(angle * 2) + 5 * math.sin(angle * 3)
            points.append((514 + r * math.cos(angle), 508 + r * .96 * math.sin(angle)))
        pen.line(points, fill=rng.choice(['#262321','#35312b','#201d1b']), width=rng.randint(1, 5))
    return image

if __name__ == '__main__':
    out = Path('src-tauri/icons')
    source = icon()
    source.save(out / 'icon.png')
    source.save(out / 'icon.icns', format='ICNS')
